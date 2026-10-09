use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use loop_protocol::wire::{discovery::v1 as wire, provider::v1 as provider, v1};
use prost::Message;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};

use super::super::{
    DiscoveryConfig, DiscoveryExecutor, ProviderConfig, plan::FrozenPlan,
    transport::ProviderConnection,
};
use super::tls;
use crate::manifests::ObjectRef;
use crate::runtime::{ArtifactBroker, Role, RuntimeAuthority, RuntimeService, model_codec};
use crate::store::{AdmissionPolicy, Clock, JobRepository, PgJobStore, StoreResult, SystemClock};
use crate::test_support;

pub(super) mod operations;
mod run_operations;

pub(super) struct OffsetClock(pub(super) AtomicI64);

impl Clock for OffsetClock {
    fn now_millis(&self) -> StoreResult<i64> {
        Ok(SystemClock.now_millis()? + self.0.load(Ordering::SeqCst))
    }
}

pub(super) struct Case {
    directory: TempDir,
    tls: tls::Credentials,
    child: tokio::process::Child,
    port: u16,
    plan: ObjectRef,
    pub(super) input: wire::DiscoveryJobInput,
    pub(super) invocation: v1::ModelInvocation,
    pub(super) store: PgJobStore,
    pub(super) clock: Arc<OffsetClock>,
    authority: Arc<RuntimeAuthority>,
    executor: Arc<DiscoveryExecutor>,
    address: std::net::SocketAddr,
    task: tokio::task::JoinHandle<StoreResult<()>>,
}

impl Case {
    pub(super) async fn open() -> Self {
        Self::open_for(false, false).await
    }

    pub(super) async fn protected() -> Self {
        Self::open_for(true, false).await
    }

    pub(super) async fn controlled() -> Self {
        Self::open_for(false, true).await
    }

    pub(super) async fn controlled_protected() -> Self {
        Self::open_for(true, true).await
    }

    async fn open_for(protected: bool, controlled: bool) -> Self {
        let directory = tempfile::Builder::new()
            .prefix("loop-discovery-e2e-")
            .tempdir()
            .unwrap();
        let tls = tls::Credentials::new(directory.path());
        for name in ["ca.pem", "server.pem", "client.pem"] {
            fs::set_permissions(tls.path(name), fs::Permissions::from_mode(0o600)).unwrap();
        }
        for name in ["plans", "data", "protected", "views"] {
            private_dir(&directory.path().join(name));
        }
        let schema = model_codec::ast_schema().unwrap();
        private_file(
            &directory.path().join("ast-schema.json"),
            &schema.canonical_json,
        );
        private_file(
            &directory.path().join("schema-reference.json"),
            &serde_json::to_vec(&json!({
                "id":schema.schema_id,"version":schema.schema_version,
                "sha256":format!("{:x}", Sha256::digest(&schema.canonical_json)),
            }))
            .unwrap(),
        );
        for (name, schema) in [
            (
                "tool-input",
                model_codec::describe_tool().unwrap().input_schema.unwrap(),
            ),
            ("tool-result", super::super::tools::result_schema().unwrap()),
        ] {
            private_file(
                &directory.path().join(format!("{name}.json")),
                &schema.canonical_json,
            );
            private_file(
                &directory.path().join(format!("{name}-reference.json")),
                &serde_json::to_vec(
                    &json!({"id":schema.schema_id,"version":schema.schema_version,
                    "sha256":format!("{:x}", Sha256::digest(&schema.canonical_json))}),
                )
                .unwrap(),
            );
        }
        let (child, ready) = worker(directory.path(), "normal").await;
        let port = ready["port"].as_u64().unwrap() as u16;
        let model = v1::ModelResolutionSnapshot::decode(
            STANDARD
                .decode(ready["model"].as_str().unwrap())
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let policy = v1::PolicyReference::decode(
            STANDARD
                .decode(ready["policy"].as_str().unwrap())
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let connector = ProviderConnection::open(provider_config(directory.path(), port)).unwrap();
        let data = dataset(&directory.path().join("data"), protected);
        let cost = v1::Money {
            currency_code: "USD".into(),
            amount: Some(v1::ExactDecimal { value: "1".into() }),
        };
        let input = wire::DiscoveryJobInput {
            dataset: Some(v1::DevelopmentDatasetReference {
                snapshot_ids: vec![v1::SnapshotId {
                    value: "snapshot.discovery".into(),
                }],
                manifest_sha256: Some(v1::Sha256Digest {
                    value: data.digest().unwrap().to_vec(),
                }),
            }),
            research_policy: None,
            maker_model: Some(model.clone()),
            checker_model: Some(model.clone()),
            budget: Some(wire::DiscoveryJobBudget {
                maximum_steps: if controlled { 3 } else { 1 },
                maximum_input_tokens: if controlled { 8192 } else { 4096 },
                maximum_output_tokens: if controlled { 256 } else { 128 },
                maximum_cost: Some(if controlled {
                    v1::Money {
                        currency_code: "USD".into(),
                        amount: Some(v1::ExactDecimal { value: "2".into() }),
                    }
                } else {
                    cost.clone()
                }),
                maximum_wall_time: Some(prost_types::Duration {
                    seconds: 120,
                    nanos: 0,
                }),
            }),
            maximum_candidates: 1,
        };
        let mut invocation = v1::ModelInvocation {
            model: Some(model), request_policy: Some(policy),
            messages: vec![v1::ModelMessage { role: v1::ModelRole::User as i32,
                content: vec![v1::ContentBlock { content: Some(v1::content_block::Content::Text(v1::TextContent {
                    text: "Return one AST using market.close from the approved synthetic development context.".into(),
                })) }] }],
            structured_output: Some(v1::StructuredOutputDefinition { name: "factor_candidate".into(),
                json_schema: Some(schema), strict: Some(true), ..Default::default() }),
            budget: Some(v1::InvocationBudget { maximum_input_tokens: 4096, maximum_output_tokens: 128,
                maximum_cost: Some(cost), maximum_wall_time: Some(prost_types::Duration { seconds: 5, nanos: 0 }) }),
            ..Default::default()
        };
        let tool_invocation = controlled.then(|| {
            invocation.tools = vec![model_codec::describe_tool().unwrap()];
            invocation.tool_choice = Some(v1::ToolChoice {
                mode: v1::ToolChoiceMode::None as i32,
                named_tool: String::new(),
            });
            let mut first = invocation.clone();
            first.structured_output = None;
            first.tool_choice.as_mut().unwrap().mode = v1::ToolChoiceMode::Required as i32;
            first
        });
        let plans = directory.path().join("plans");
        let mut protocol = test_support::command(1)
            .specification
            .protocol_selection
            .unwrap();
        protocol.selected_at = Some(test_support::timestamp(
            SystemClock.now_millis().unwrap() - 1000,
        ));
        protocol.enabled_features = [
            "discovery.model-step.v1",
            "jobs.envelope.v1",
            "jobs.kind-input.v1",
            "jobs.prelease-terminal.v1",
            "provider.invocation-lookup.v1",
        ]
        .map(str::to_owned)
        .to_vec();
        if controlled {
            protocol
                .enabled_features
                .push("discovery.tool-context.v1".into());
            protocol.enabled_features.sort();
        }
        protocol.schema_descriptor_sha256 = Some(v1::Sha256Digest {
            value: Sha256::digest(loop_protocol::FILE_DESCRIPTOR_SET).to_vec(),
        });
        protocol.selection_sha256 = Some(v1::Sha256Digest {
            value: loop_protocol::job::protocol_selection_sha256(&protocol)
                .unwrap()
                .to_vec(),
        });
        let mut document = json!({
            "schema":if controlled { "loop.discovery-plan/v2" } else { "loop.discovery-plan/v1" }, "id":"discovery.synthetic", "revision":"1",
            "actor_id":"agent.discovery", "run_id":"run.discovery", "provider_sha256":connector.sha256,
            "input":put(&plans, &input.encode_to_vec()), "invocation":put(&plans, &invocation.encode_to_vec()),
            "protocol":put(&plans, &protocol.encode_to_vec()), "data":data,
            "registry":put(&plans, &loop_core::factor::us_equities::registry().unwrap().canonical_bytes()),
        });
        if let Some(first) = &tool_invocation {
            document["tool_invocation"] =
                serde_json::to_value(put(&plans, &first.encode_to_vec())).unwrap();
        }
        let plan = put(&plans, &serde_json::to_vec(&document).unwrap());
        let frozen = FrozenPlan::load(&plans, &plan).unwrap();
        let clock = Arc::new(OffsetClock(AtomicI64::new(0)));
        let (store, address, task, authority, executor) =
            runtime(directory.path(), &tls, port, &plan, clock.clone()).await;
        Self {
            directory,
            tls,
            child,
            port,
            plan,
            input: frozen.input,
            invocation: frozen.tool_invocation.unwrap_or(frozen.invocation),
            store,
            clock,
            authority,
            executor,
            address,
            task,
        }
    }

    pub(super) fn actor(&self) -> v1::Actor {
        actor()
    }
    pub(super) fn corrupt_data(&self) {
        let bytes = b"session,security,close\n2016-01-04,security.1,100\n";
        let digest = format!("{:x}", Sha256::digest(bytes));
        private_file(
            &self.directory.path().join("data").join(digest),
            b"corrupted data",
        );
    }
    pub(super) fn invalid_ast(&self) {
        private_file(&self.directory.path().join("invalid-ast"), b"1");
    }
    pub(super) fn invalid_response(&self, mode: &str) {
        private_file(
            &self.directory.path().join("invalid-response"),
            mode.as_bytes(),
        );
    }
    pub(super) fn delay_supplier(&self) {
        private_file(&self.directory.path().join("supplier-delay"), b"2000");
    }
    pub(super) fn corrupt_plan(&self) {
        private_file(
            &self
                .directory
                .path()
                .join("plans")
                .join(&self.plan.sha256[7..]),
            b"invalid plan",
        );
    }
    pub(super) fn invalid_tool(&self, mode: &str) {
        private_file(&self.directory.path().join("invalid-tool"), mode.as_bytes());
    }
    pub(super) fn supplier_bodies(&self) -> Vec<Value> {
        serde_json::from_slice(
            &fs::read(self.directory.path().join("supplier-bodies.json")).unwrap(),
        )
        .unwrap()
    }
    pub(super) async fn tool_result(
        &self,
        step: &crate::store::ModelStep,
    ) -> v1::ToolResultContent {
        let plan = FrozenPlan::load(&self.directory.path().join("plans"), &self.plan).unwrap();
        let source =
            crate::manifests::LocalArtifacts::open(&self.directory.path().join("data")).unwrap();
        let call =
            model_codec::response_call(&step.request, step.response.as_ref().unwrap()).unwrap();
        super::super::tools::describe(
            &source,
            &plan.data,
            step.job.specification.as_ref().unwrap(),
            &plan.registry,
            &call,
        )
        .await
        .unwrap()
    }
    pub(super) fn final_invocation(&self) -> v1::ModelInvocation {
        FrozenPlan::load(&self.directory.path().join("plans"), &self.plan)
            .unwrap()
            .invocation
    }
    pub(super) async fn verify_deployment(&self, job: &wire::DiscoveryJobHandle) {
        let root = self.directory.path();
        let mut value = self.deployment();
        let path = root.join("runtime.json");
        private_file(&path, &serde_json::to_vec(&value).unwrap());
        let deployment = crate::runtime::RuntimeDeployment::load(&path).unwrap();
        assert!(deployment.discovery.is_some());
        let record = self
            .store
            .get(&job.job_id.as_ref().unwrap().value)
            .await
            .unwrap()
            .unwrap();
        deployment
            .authority
            .validate_submission(record.specification.as_ref().unwrap())
            .unwrap();
        value["identities"][0]["role"] = "research".into();
        private_file(&path, &serde_json::to_vec(&value).unwrap());
        assert!(crate::runtime::RuntimeDeployment::load(&path).is_err());
        value["identities"][0]["role"] = "discovery".into();
        value["discovery"]["plan_store"] = json!(root.join("data"));
        private_file(&path, &serde_json::to_vec(&value).unwrap());
        assert!(crate::runtime::RuntimeDeployment::load(&path).is_err());
    }

    fn deployment(&self) -> Value {
        let root = self.directory.path();
        let now = self.clock.now_millis().unwrap();
        json!({
            "schema":"loop.runtime/v1", "bind":"127.0.0.1:8443",
            "server_certificate_file":self.tls.path("server.pem"),
            "server_key_file":self.tls.path("server.key"), "client_ca_file":self.tls.path("ca.pem"),
            "identities":[{"actor_id":"agent.discovery", "subject":"agent:discovery",
                "display_name":"Discovery fixture", "role":"discovery",
                "certificate_sha256":[self.tls.client_digest()], "not_before_ms":now-1000,
                "expires_at_ms":now+3600000, "run_ids":["run.discovery"]}],
            "jobs":[], "data":[], "development_store":root.join("data"),
            "protected_store":root.join("protected"), "view_store":root.join("views"),
            "discovery":{"plan_store":root.join("plans"), "plans":[self.plan], "provider":{
                "endpoint":format!("https://localhost:{}", self.port), "domain":"localhost",
                "actor_id":"agent.discovery", "subject":"agent:discovery", "display_name":"Discovery fixture",
                "ca_file":self.tls.path("ca.pem"), "certificate_file":self.tls.path("client.pem"),
                "private_key_file":self.tls.path("client.key")}},
        })
    }
    pub(super) fn context(&self) -> v1::CommandContext {
        let id = uuid::Uuid::new_v4().to_string();
        v1::CommandContext {
            request_id: Some(v1::RequestId { value: id.clone() }),
            correlation_id: Some(v1::CorrelationId {
                value: "discovery.e2e".into(),
            }),
            causation_id: Some(v1::CausationId {
                value: "discovery.e2e".into(),
            }),
            idempotency_key: Some(v1::IdempotencyKey { value: id }),
            actor: Some(actor()),
            requested_at: Some(test_support::timestamp(self.clock.now_millis().unwrap())),
        }
    }
    pub(super) fn calls(&self) -> u64 {
        fs::read(self.directory.path().join("supplier-calls.json"))
            .ok()
            .map_or(0, |bytes| serde_json::from_slice(&bytes).unwrap())
    }
    pub(super) async fn client(
        &self,
        identity: &str,
    ) -> wire::discovery_service_client::DiscoveryServiceClient<Channel> {
        wire::discovery_service_client::DiscoveryServiceClient::new(
            channel(&self.tls, self.address.port(), identity).await,
        )
    }
    pub(super) async fn provider(
        &self,
    ) -> provider::provider_service_client::ProviderServiceClient<Channel> {
        provider::provider_service_client::ProviderServiceClient::new(
            channel(&self.tls, self.port, "client").await,
        )
    }
    pub(super) async fn start(&self) -> wire::DiscoveryJobHandle {
        self.client("client")
            .await
            .start_discovery(timed(wire::StartDiscoveryRequest {
                context: Some(self.context()),
                discovery: Some(self.input.clone()),
            }))
            .await
            .unwrap()
            .into_inner()
            .job
            .unwrap()
    }
    pub(super) async fn read(&self, job: &wire::DiscoveryJobHandle) -> wire::DiscoveryStepView {
        self.client("client")
            .await
            .get_discovery(timed(wire::GetDiscoveryRequest {
                context: Some(self.context()),
                job_id: job.job_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner()
            .step
            .unwrap()
    }
    pub(super) async fn execute(
        &self,
        job: &wire::DiscoveryJobHandle,
    ) -> Result<wire::DiscoveryStepView, tonic::Status> {
        self.client("client")
            .await
            .execute_discovery(timed(wire::ExecuteDiscoveryRequest {
                context: Some(self.context()),
                job_id: job.job_id.clone(),
                expected_revision: job.revision,
            }))
            .await
            .map(|reply| reply.into_inner().step.unwrap())
    }
    pub(super) async fn stale_execute(
        &self,
        job: v1::JobRecord,
    ) -> StoreResult<wire::DiscoveryStepView> {
        let revision = job.revision;
        self.executor
            .execute(&self.store, &self.authority, &actor(), job, revision)
            .await
    }
    pub(super) async fn restart(&mut self) {
        self.task.abort();
        self.store.close().await;
        let (store, address, task, authority, executor) = runtime(
            self.directory.path(),
            &self.tls,
            self.port,
            &self.plan,
            self.clock.clone(),
        )
        .await;
        self.store = store;
        self.address = address;
        self.task = task;
        self.authority = authority;
        self.executor = executor;
    }
    pub(super) async fn stop_only(&mut self) {
        self.task.abort();
        self.store.close().await;
        let now = self.clock.now_millis().unwrap();
        let authority = Arc::new(
            RuntimeAuthority::new(
                vec![crate::runtime::Identity {
                    actor_id: "agent.discovery".into(),
                    subject: "agent:discovery".into(),
                    display_name: "Discovery fixture".into(),
                    role: Role::Discovery,
                    certificate_sha256: vec![self.tls.client_digest()],
                    not_before_ms: now - 1000,
                    expires_at_ms: now + 3_600_000,
                    run_ids: vec!["run.discovery".into()],
                }],
                vec![],
                self.clock.clone(),
            )
            .unwrap(),
        );
        let mut options = test_support::base_options(&self.directory.path().join("state"));
        options.clock = self.clock.clone();
        options.admission = authority.clone();
        self.store = PgJobStore::open(options).await.unwrap();
        let broker = Arc::new(
            ArtifactBroker::open(
                &self.directory.path().join("data"),
                &self.directory.path().join("protected"),
                &self.directory.path().join("views"),
                vec![],
            )
            .unwrap(),
        );
        let service = RuntimeService::new(self.store.clone(), authority.clone(), broker);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        self.address = listener.local_addr().unwrap();
        self.task = tokio::spawn(crate::runtime::serve(
            service,
            listener,
            self.tls.server(),
            std::future::pending(),
        ));
        self.authority = authority;
    }
    pub(super) async fn restart_provider(&mut self) {
        self.child.kill().await.unwrap();
        self.child.wait().await.unwrap();
        let (child, _) = worker(self.directory.path(), "recover").await;
        self.child = child;
    }
    pub(super) async fn resume_provider(&mut self) {
        self.child.kill().await.unwrap();
        self.child.wait().await.unwrap();
        let (child, _) = worker(self.directory.path(), "resume").await;
        self.child = child;
    }
    pub(super) async fn close(&mut self) {
        self.task.abort();
        self.child.kill().await.unwrap();
        self.child.wait().await.unwrap();
        self.store.close().await;
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        self.task.abort();
        let _ = self.child.start_kill();
    }
}

impl Case {
    pub(super) fn lookup_faults(&self, failures: u32) {
        private_file(
            &self.directory.path().join("lookup-failures"),
            failures.to_string().as_bytes(),
        );
    }

    pub(super) fn lookups(&self) -> u64 {
        fs::read(self.directory.path().join("lookup-calls.json"))
            .ok()
            .map_or(0, |bytes| serde_json::from_slice(&bytes).unwrap())
    }

    pub(super) async fn prior_descriptor(&mut self) {
        // Create a distinct immutable plan before submission. Never rewrite
        // submitted jobs or the prior content-addressed protocol artifact.
        assert!(self.store.audit_events(0, 1).await.unwrap().is_empty());
        let plans = self.directory.path().join("plans");
        let mut document: Value =
            serde_json::from_slice(&fs::read(plans.join(&self.plan.sha256[7..])).unwrap()).unwrap();
        let reference: ObjectRef = serde_json::from_value(document["protocol"].clone()).unwrap();
        let mut protocol = v1::ProtocolSelectionSnapshot::decode(
            fs::read(plans.join(&reference.sha256[7..]))
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        protocol.schema_descriptor_sha256 = Some(v1::Sha256Digest {
            value: vec![
                0x2c, 0x60, 0xbd, 0x74, 0x3b, 0x8f, 0x4d, 0xb0, 0x9f, 0xdb, 0x19, 0x20, 0x0e, 0xf8,
                0x30, 0x7d, 0x49, 0xad, 0xc3, 0xb2, 0x7a, 0xdf, 0x28, 0xbf, 0xf6, 0x81, 0x84, 0x58,
                0x98, 0xbd, 0x3e, 0x1c,
            ],
        });
        protocol.selection_sha256 = Some(v1::Sha256Digest {
            value: loop_protocol::job::protocol_selection_sha256(&protocol)
                .unwrap()
                .to_vec(),
        });
        document["protocol"] =
            serde_json::to_value(put(&plans, &protocol.encode_to_vec())).unwrap();
        self.plan = put(&plans, &serde_json::to_vec(&document).unwrap());
        // Loading binds research_policy to this new immutable plan. Submit its
        // matching input, rather than the previous plan's policy reference.
        self.input = FrozenPlan::load(&plans, &self.plan).unwrap().input;
        self.restart().await;
    }
}

pub(super) fn timed<T>(value: T) -> tonic::Request<T> {
    let mut request = tonic::Request::new(value);
    request.set_timeout(Duration::from_secs(120));
    request
}

fn actor() -> v1::Actor {
    v1::Actor {
        actor_id: Some(v1::ActorId {
            value: "agent.discovery".into(),
        }),
        kind: v1::ActorKind::Agent as i32,
        authenticated_subject: "agent:discovery".into(),
        display_name: "Discovery fixture".into(),
    }
}

fn provider_config(root: &Path, port: u16) -> ProviderConfig {
    ProviderConfig {
        endpoint: format!("https://localhost:{port}"),
        domain: "localhost".into(),
        actor_id: "agent.discovery".into(),
        subject: "agent:discovery".into(),
        display_name: "Discovery fixture".into(),
        ca_file: root.join("tls/ca.pem"),
        certificate_file: root.join("tls/client.pem"),
        private_key_file: root.join("tls/client.key"),
    }
}

type Running = (
    PgJobStore,
    std::net::SocketAddr,
    tokio::task::JoinHandle<StoreResult<()>>,
    Arc<RuntimeAuthority>,
    Arc<DiscoveryExecutor>,
);

async fn runtime(
    root: &Path,
    tls: &tls::Credentials,
    port: u16,
    plan: &ObjectRef,
    clock: Arc<OffsetClock>,
) -> Running {
    let executor = Arc::new(
        DiscoveryExecutor::open(
            DiscoveryConfig {
                plan_store: root.join("plans"),
                plans: vec![plan.clone()],
                provider: provider_config(root, port),
            },
            &root.join("data"),
        )
        .unwrap(),
    );
    let now = clock.now_millis().unwrap();
    let authority = Arc::new(
        RuntimeAuthority::new(
            vec![crate::runtime::Identity {
                actor_id: "agent.discovery".into(),
                subject: "agent:discovery".into(),
                display_name: "Discovery fixture".into(),
                role: Role::Discovery,
                certificate_sha256: vec![tls.client_digest()],
                not_before_ms: now - 1000,
                expires_at_ms: now + 3_600_000,
                run_ids: vec!["run.discovery".into()],
            }],
            vec![],
            clock.clone(),
        )
        .unwrap()
        .with_discovery(executor.clone())
        .unwrap(),
    );
    let mut options = test_support::base_options(&root.join("state"));
    options.clock = clock;
    options.admission = authority.clone();
    let store = PgJobStore::open(options).await.unwrap();
    let broker = Arc::new(
        ArtifactBroker::open(
            &root.join("data"),
            &root.join("protected"),
            &root.join("views"),
            vec![],
        )
        .unwrap(),
    );
    let service = RuntimeService::new(store.clone(), authority.clone(), broker)
        .with_discoverer(executor.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(crate::runtime::serve(
        service,
        listener,
        tls.server(),
        std::future::pending(),
    ));
    (store, address, task, authority, executor)
}

async fn channel(tls: &tls::Credentials, port: u16, identity: &str) -> Channel {
    Endpoint::from_shared(format!("https://localhost:{port}"))
        .unwrap()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(15))
        .tls_config(
            ClientTlsConfig::new()
                .domain_name("localhost")
                .ca_certificate(Certificate::from_pem(fs::read(tls.path("ca.pem")).unwrap()))
                .identity(Identity::from_pem(
                    fs::read(tls.path(&format!("{identity}.pem"))).unwrap(),
                    fs::read(tls.path(&format!("{identity}.key"))).unwrap(),
                )),
        )
        .unwrap()
        .connect()
        .await
        .unwrap()
}

async fn worker(root: &Path, mode: &str) -> (tokio::process::Child, Value) {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/providerd/test/harness-worker.mjs");
    let mut command = tokio::process::Command::new("node");
    command
        .arg(script)
        .arg(root)
        .arg(mode)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = command
        .spawn()
        .expect("build the Provider and install Node 24 before Rust integration tests");
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    let line = tokio::time::timeout(Duration::from_secs(15), output.next_line())
        .await
        .unwrap()
        .unwrap()
        .expect("compiled Provider fixture failed to start");
    (child, serde_json::from_str(&line).unwrap())
}

fn private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn private_file(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
fn put(root: &Path, bytes: &[u8]) -> ObjectRef {
    let digest = format!("{:x}", Sha256::digest(bytes));
    private_file(&root.join(&digest), bytes);
    ObjectRef {
        sha256: format!("sha256:{digest}"),
        byte_size: bytes.len() as u64,
    }
}
fn dataset(root: &Path, protected: bool) -> ObjectRef {
    let object = put(root, b"session,security,close\n2016-01-04,security.1,100\n");
    // Manifest byte order is part of the established writer contract. Literal
    // fixture documents independently exercise the production reader, including
    // nested ObjectRef ordering, instead of bypassing it with an in-memory view.
    let document = put(
        root,
        concat!(
            r#"{"schema":"loop.artifact-schema/v1","name":"loop.synthetic_prices","version":1,"#,
            r#""media_type":"text/csv","columns":["session","security","close"]}"#,
        )
        .as_bytes(),
    );
    let (role, year) = if protected {
        ("first_locked_confirmation", "2021")
    } else {
        ("in_sample", "2016")
    };
    let bytes = format!(
        concat!(
            r#"{{"schema":"loop.development-dataset/v1","sample":{{"role":"{role}","#,
            r#""start":"{year}-01-04","end":"{year}-01-08"}},"quality":"synthetic","#,
            r#""snapshots":[{{"snapshot_id":"snapshot.discovery","source":"synthetic","#,
            r#""dataset":"discovery-test","entitlement":"synthetic","known_through_ms":1452286800000,"#,
            r#""artifacts":[{{"object":{object},"schema":{{"name":"loop.synthetic_prices","#,
            r#""version":1,"document":{document}}},"media_type":"text/csv","created_at_ms":1452286800000}}]}}]}}"#,
        ),
        role = role,
        year = year,
        object = serde_json::to_string(&object).unwrap(),
        document = serde_json::to_string(&document).unwrap()
    );
    put(root, bytes.as_bytes())
}
