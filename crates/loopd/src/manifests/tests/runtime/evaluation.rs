use std::path::{Path, PathBuf};
use std::time::Duration;

use loop_protocol::wire::v1::*;
use serde::Deserialize;

use super::*;
use crate::manifests::{EvaluationPin, EvaluationResolver, ObjectRef};
use crate::runtime::FactorExecutor;
use crate::store::{Clock, SystemClock};

struct Case {
    fixture: Fixture,
    store: PgJobStore,
    tls: tls::Credentials,
    authority: Arc<RuntimeAuthority>,
    broker: Arc<ArtifactBroker>,
    executor: Arc<FactorExecutor>,
    clock: Arc<TestClock>,
    views: PathBuf,
    output: PathBuf,
    address: std::net::SocketAddr,
    task: tokio::task::JoinHandle<crate::store::StoreResult<()>>,
}

struct TestClock(AtomicI64);

impl Clock for TestClock {
    fn now_millis(&self) -> crate::store::StoreResult<i64> {
        SystemClock
            .now_millis()?
            .checked_add(self.0.load(Ordering::SeqCst))
            .ok_or(crate::store::StoreError::Invalid("test clock range"))
    }
}

impl Case {
    async fn start() -> Self {
        Self::open(fixture(false).await).await
    }

    async fn open(fixture: Fixture) -> Self {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let clock = Arc::new(TestClock(AtomicI64::new(0)));
        let store = fixture.open(clock.clone(), fixture.policy().await).await;
        store.submit(fixture.job.clone()).await.unwrap();
        let tls = tls::Credentials::new(fixture.directory.path());
        let now = SystemClock.now_millis().unwrap();
        let actor = actor();
        let job = &fixture.job.specification;
        let authority = Arc::new(
            RuntimeAuthority::new(
                vec![Identity {
                    actor_id: actor.actor_id.as_ref().unwrap().value.clone(),
                    subject: actor.authenticated_subject,
                    display_name: actor.display_name,
                    role: Role::Research,
                    certificate_sha256: vec![tls.client_digest()],
                    not_before_ms: now - 1000,
                    expires_at_ms: now + 3_600_000,
                    run_ids: vec![job.run_id.as_ref().unwrap().value.clone()],
                }],
                vec![JobPin {
                    job_id: job.job_id.as_ref().unwrap().value.clone(),
                    specification_sha256: format!(
                        "sha256:{:x}",
                        Sha256::digest(job.encode_to_vec())
                    ),
                }],
                clock.clone(),
            )
            .unwrap(),
        );
        let private = |name: &str| {
            let path = fixture.directory.path().join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path
        };
        let protected = private("protected");
        let views = private("views");
        let output = private("output");
        let broker = Arc::new(
            ArtifactBroker::open(
                &fixture.root,
                &protected,
                &views,
                vec![DataPin {
                    job_id: job.job_id.as_ref().unwrap().value.clone(),
                    manifest: fixture.context.data.clone(),
                    protected: false,
                }],
            )
            .unwrap(),
        );
        let resolver = Arc::new(
            EvaluationResolver::open(
                &fixture.root,
                vec![EvaluationPin {
                    job_id: job.job_id.as_ref().unwrap().value.clone(),
                    context: fixture.json(&fixture.context),
                }],
            )
            .unwrap(),
        );
        let executor = Arc::new(FactorExecutor::open(&python(), &output, resolver).unwrap());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let service = RuntimeService::new(store.clone(), authority.clone(), broker.clone())
            .with_factor_executor(executor.clone());
        let task = tokio::spawn(crate::runtime::serve(
            service,
            listener,
            tls.server(),
            std::future::pending(),
        ));
        Self {
            fixture,
            store,
            tls,
            authority,
            broker,
            executor,
            clock,
            views,
            output,
            address,
            task,
        }
    }

    async fn client(&self) -> job_service_client::JobServiceClient<tonic::transport::Channel> {
        // The server allows a 90-second RPC envelope, including byte-backed
        // build/data checks and its bounded numerical worker. The metadata
        // fixture's 30-second client timeout must not cut this test short.
        self.tls
            .timed_client(self.address, Some("client"), Duration::from_secs(95))
            .await
            .unwrap()
    }

    async fn request(&self) -> EvaluateFactorRequest {
        let job = self
            .client()
            .await
            .acquire_job_lease(AcquireJobLeaseRequest {
                context: Some(context("evaluation-acquire")),
                job_id: self.fixture.job.specification.job_id.clone(),
                expected_revision: 1,
                requested_duration: Some(prost_types::Duration {
                    seconds: 120,
                    nanos: 0,
                }),
            })
            .await
            .unwrap()
            .into_inner()
            .job
            .unwrap();
        EvaluateFactorRequest {
            context: Some(context("evaluate")),
            job_id: self.fixture.job.specification.job_id.clone(),
            lease_id: job.active_lease.unwrap().lease_id,
            expected_revision: job.revision,
        }
    }

    async fn restart(&mut self) {
        self.task.abort();
        self.store = self
            .fixture
            .open(self.clock.clone(), self.fixture.policy().await)
            .await;
        // Reopen every file cache and numerical executor. A replay must resolve
        // durable bytes rather than depend on an in-memory evidence object.
        self.broker = Arc::new(
            ArtifactBroker::open(
                &self.fixture.root,
                &self.fixture.directory.path().join("protected"),
                &self.views,
                vec![DataPin {
                    job_id: self
                        .fixture
                        .job
                        .specification
                        .job_id
                        .as_ref()
                        .unwrap()
                        .value
                        .clone(),
                    manifest: self.fixture.context.data.clone(),
                    protected: false,
                }],
            )
            .unwrap(),
        );
        self.executor = Arc::new(
            FactorExecutor::open(
                &python(),
                &self.output,
                Arc::new(
                    EvaluationResolver::open(
                        &self.fixture.root,
                        vec![EvaluationPin {
                            job_id: self
                                .fixture
                                .job
                                .specification
                                .job_id
                                .as_ref()
                                .unwrap()
                                .value
                                .clone(),
                            context: self.fixture.json(&self.fixture.context),
                        }],
                    )
                    .unwrap(),
                ),
            )
            .unwrap(),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        self.address = listener.local_addr().unwrap();
        let service = RuntimeService::new(
            self.store.clone(),
            self.authority.clone(),
            self.broker.clone(),
        )
        .with_factor_executor(self.executor.clone());
        self.task = tokio::spawn(crate::runtime::serve(
            service,
            listener,
            self.tls.server(),
            std::future::pending(),
        ));
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        self.task.abort();
        if let Ok(entries) = std::fs::read_dir(&self.views) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    let _ = std::fs::set_permissions(
                        entry.path(),
                        std::fs::Permissions::from_mode(0o700),
                    );
                }
            }
        }
    }
}

fn python() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.venv/bin/python")
}

fn context(key: &str) -> CommandContext {
    let mut context = support::context(key);
    context.requested_at = Some(support::timestamp(SystemClock.now_millis().unwrap()));
    context
}

async fn fixture(transformed: bool) -> Fixture {
    let mut fixture = Fixture::new();
    let captured = tokio::time::timeout(
        Duration::from_secs(65),
        tokio::process::Command::new(python())
            .args([
                "-I",
                "-m",
                "loop_research.cli",
                "build-manifests",
                "--profile",
                "evaluation",
                "--store",
            ])
            .arg(&fixture.root)
            .env_clear()
            .env("OPENBLAS_NUM_THREADS", "1")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        captured.status.success(),
        "{}",
        String::from_utf8_lossy(&captured.stderr)
    );
    #[derive(Deserialize)]
    struct Build {
        source: ObjectRef,
        environment: ObjectRef,
    }
    let build: Build = serde_json::from_slice(&captured.stdout).unwrap();
    fixture.context.source = build.source;
    fixture.context.environment = build.environment;
    fixture.registry = Arc::new(loop_core::factor::us_equities::registry().unwrap());
    fixture.context.registry =
        super::super::fixture::put(&fixture.root, &fixture.registry.canonical_bytes());
    let mut configuration: model::Configuration = serde_json::from_slice(
        &std::fs::read(fixture.path(&fixture.context.configuration)).unwrap(),
    )
    .unwrap();
    configuration.backtest_engine_version = if transformed {
        "factor-evaluator.3"
    } else {
        "factor-evaluator.2"
    }
    .to_owned();
    if transformed {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/market/transforms/transform.json");
        let recipe: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for role in ["preprocess", "neutralization"] {
            let document: model::PolicyDocument =
                serde_json::from_value(recipe[role].clone()).unwrap();
            let policy = configuration
                .policies
                .iter_mut()
                .find(|entry| entry.policy_id == document.policy_id)
                .unwrap();
            policy.document = fixture.json(&document);
        }
    }
    fixture.context.configuration = fixture.json(&configuration);
    let data = build_panel_input(&mut fixture, transformed).await;
    fixture.context.family = None;
    let (_, job_specification::Input::FactorEvaluation(mut input)) =
        support::research::inputs().remove(1)
    else {
        unreachable!()
    };
    let factor = input.factor.as_mut().unwrap();
    bind_moving_average(factor, &fixture.registry);
    factor.operator_registry_sha256 = Some(model::digest(*fixture.registry.identity().as_bytes()));
    let frozen = factor.frozen_policy.as_mut().unwrap();
    for (wire, role) in [
        (&mut frozen.universe_policy, "universe"),
        (&mut frozen.data_policy, "data"),
        (&mut frozen.calendar_policy, "calendar"),
        (&mut frozen.preprocess_policy, "preprocess"),
        (&mut frozen.neutralization_policy, "neutralization"),
        (&mut frozen.portfolio_policy, "portfolio"),
        (&mut frozen.execution_policy, "execution"),
        (&mut frozen.cost_policy, "cost"),
        (&mut frozen.evaluation_policy, "evaluation"),
    ] {
        let document = configuration
            .policies
            .iter()
            .find(|policy| policy.policy_id == format!("policy.{role}"))
            .unwrap();
        wire.as_mut().unwrap().sha256 = Some(model::digest(document.document.digest().unwrap()));
    }
    factor.factor_spec_id.as_mut().unwrap().value = format!(
        "sha256:{}",
        loop_protocol::job::factor_spec_identity_sha256(factor)
            .unwrap()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    input.dataset = Some(data.reference(&fixture.context.data).unwrap());
    input.provenance = Some(
        fixture
            .context
            .provenance(*fixture.registry.identity().as_bytes())
            .unwrap(),
    );
    input.deterministic_seed = Some(Sha256Digest { value: vec![1; 32] });
    fixture.job.specification.kind = JobKind::FactorEvaluation as i32;
    fixture.job.specification.input = Some(job_specification::Input::FactorEvaluation(input));
    let selection = fixture
        .job
        .specification
        .protocol_selection
        .as_mut()
        .unwrap();
    selection.schema_descriptor_sha256 = Some(Sha256Digest {
        value: Sha256::digest(loop_protocol::FILE_DESCRIPTOR_SET).to_vec(),
    });
    selection.selection_sha256 = Some(Sha256Digest {
        value: loop_protocol::job::protocol_selection_sha256(selection)
            .unwrap()
            .to_vec(),
    });
    fixture.job.specification.submitted_at =
        Some(support::timestamp(SystemClock.now_millis().unwrap()));
    fixture
}

async fn build_panel_input(fixture: &mut Fixture, transformed: bool) -> model::Dataset {
    // Exercise the installed data-owner workflow before TLS/lease execution.
    // Source history stays outside the development store and broker views.
    std::fs::set_permissions(&fixture.root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let sources = fixture.directory.path().join("panel-sources");
    std::fs::create_dir(&sources).unwrap();
    std::fs::set_permissions(&sources, std::fs::Permissions::from_mode(0o700)).unwrap();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join(if transformed {
        "../../fixtures/market/transforms"
    } else {
        "../../fixtures/market/panels"
    });
    let sources_names: &[&str] = if transformed {
        &[
            "source.json",
            "capture.json",
            "exposures.json",
            "transform.json",
        ]
    } else {
        &["source.json", "capture.json"]
    };
    for name in sources_names {
        super::super::fixture::put(&sources, &std::fs::read(examples.join(name)).unwrap());
    }
    let built = tokio::time::timeout(
        Duration::from_secs(35),
        tokio::process::Command::new(python())
            .args(["-I", "-m", "loop_research.cli", "panel-build"])
            .arg(examples.join("request.json"))
            .arg("--sources")
            .arg(&sources)
            .arg("--store")
            .arg(&fixture.root)
            .env_clear()
            .env("OPENBLAS_NUM_THREADS", "1")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    #[derive(Deserialize)]
    struct BuiltPanel {
        dataset: ObjectRef,
        calendar: ObjectRef,
        rows: u64,
        eligible_rows: u64,
        observed_rows: u64,
        production_eligible: bool,
    }
    let report: BuiltPanel = serde_json::from_slice(&built.stdout).unwrap();
    let expected = if transformed { 24 } else { 6 };
    assert_eq!(
        (report.rows, report.eligible_rows, report.observed_rows),
        (expected, expected, expected)
    );
    assert!(!report.production_eligible);
    fixture.context.data = report.dataset;
    fixture.context.calendar = report.calendar;
    serde_json::from_slice(&std::fs::read(fixture.path(&fixture.context.data)).unwrap()).unwrap()
}

fn change_preprocess(fixture: &mut Fixture) {
    // Rebind the entire frozen context to a different valid document. The test
    // must reach panel/policy validation, not merely fail a stale digest check.
    let mut configuration: model::Configuration = serde_json::from_slice(
        &std::fs::read(fixture.path(&fixture.context.configuration)).unwrap(),
    )
    .unwrap();
    let policy = configuration
        .policies
        .iter_mut()
        .find(|entry| entry.policy_id == "policy.preprocess")
        .unwrap();
    let mut document: model::PolicyDocument =
        serde_json::from_slice(&std::fs::read(fixture.path(&policy.document)).unwrap()).unwrap();
    document
        .settings
        .insert("standardize".to_owned(), "zscore".to_owned());
    policy.document = fixture.json(&document);
    let digest = policy.document.digest().unwrap();
    fixture.context.configuration = fixture.json(&configuration);
    let provenance = fixture
        .context
        .provenance(*fixture.registry.identity().as_bytes())
        .unwrap();
    let Some(job_specification::Input::FactorEvaluation(input)) =
        &mut fixture.job.specification.input
    else {
        unreachable!()
    };
    input.provenance = Some(provenance);
    let factor = input.factor.as_mut().unwrap();
    factor
        .frozen_policy
        .as_mut()
        .unwrap()
        .preprocess_policy
        .as_mut()
        .unwrap()
        .sha256 = Some(model::digest(digest));
    factor.factor_spec_id.as_mut().unwrap().value = format!(
        "sha256:{}",
        loop_protocol::job::factor_spec_identity_sha256(factor)
            .unwrap()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
}

fn bind_moving_average(
    factor: &mut FactorSpec,
    registry: &loop_core::factor::OperatorPolicyRegistry,
) {
    use loop_core::factor as domain;
    let expression = domain::FactorExpr::Call(domain::OperatorCall::new(
        domain::OperatorRef::new(
            domain::Identifier::new("ma").unwrap(),
            domain::PositiveInteger::new("2").unwrap(),
        ),
        vec![
            domain::FactorExpr::Field(domain::FieldRef::new(
                domain::Identifier::new("market.close").unwrap(),
            )),
            domain::FactorExpr::Literal(domain::Literal::Decimal(
                domain::CanonicalDecimal::new("2").unwrap(),
            )),
            domain::FactorExpr::Literal(domain::Literal::Decimal(
                domain::CanonicalDecimal::new("2").unwrap(),
            )),
        ],
    ));
    let limits = domain::ValidationLimits::default();
    let id = FactorExpressionId {
        value: domain::expression_id(&expression, registry, limits)
            .unwrap()
            .to_string(),
    };
    let decimal = || FactorAstNode {
        node: Some(factor_ast_node::Node::Literal(FactorLiteral {
            value: Some(factor_literal::Value::Decimal(ExactDecimal {
                value: "2".to_owned(),
            })),
        })),
    };
    factor.expression_id = Some(id.clone());
    factor.expression = Some(CanonicalFactorAst {
        expression_id: Some(id),
        ast: Some(FactorAst {
            schema_version: 1,
            root: Some(FactorAstNode {
                node: Some(factor_ast_node::Node::Call(OperatorCall {
                    operator: Some(OperatorReference {
                        operator: "ma".to_owned(),
                        operator_version: "2".to_owned(),
                    }),
                    arguments: vec![
                        FactorAstNode {
                            node: Some(factor_ast_node::Node::Field(FieldReference {
                                field: "market.close".to_owned(),
                            })),
                        },
                        decimal(),
                        decimal(),
                    ],
                })),
            }),
        }),
        canonicalization_profile: "loop.factor-ast/v1".to_owned(),
        canonical_json: domain::canonical_expression_bytes(&expression, registry, limits).unwrap(),
    });
}

#[tokio::test]
async fn numerical_execution_commits_bound_values() {
    let case = Case::start().await;
    let request = case.request().await;
    let started = std::time::Instant::now();
    let job = case
        .client()
        .await
        .evaluate_factor(request)
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    eprintln!(
        "authorized evaluation: 3 sessions, 2 securities, ma(2,2), elapsed={:?}",
        started.elapsed()
    );
    assert_eq!(job.state, JobState::Succeeded as i32);
    let Some(job_outcome::Outcome::Success(success)) = job.outcome.unwrap().outcome else {
        panic!("numerical success");
    };
    assert_eq!(success.outputs.len(), 2);
    let content = std::fs::read(
        case.output
            .join(&success.outputs[0].artifact_id.as_ref().unwrap().value[7..]),
    )
    .unwrap();
    assert_eq!(content, b"session,security_id,eligible,value\n2010-01-04,US.001,1,\n2010-01-04,US.002,1,\n2010-01-05,US.001,1,9\n2010-01-05,US.002,1,9\n2010-01-06,US.001,1,11\n2010-01-06,US.002,1,11\n");
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            case.output
                .join(&success.outputs[1].artifact_id.as_ref().unwrap().value[7..]),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(report["eligible_observations"], 6);
    assert_eq!(report["valid_observations"], 4);
    assert_eq!(success.outputs[0].row_count, Some(6));
    assert_eq!(case.store.audit_events(0, 20).await.unwrap().len(), 4);
}

#[tokio::test]
async fn completed_evaluation_replays_after_restart() {
    let mut case = Case::start().await;
    let request = case.request().await;
    let first = case
        .client()
        .await
        .evaluate_factor(request.clone())
        .await
        .unwrap()
        .into_inner();
    let before = std::fs::read_dir(&case.output).unwrap().count();
    case.restart().await;
    let replay = case
        .client()
        .await
        .evaluate_factor(request)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first, replay);
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), before);
    assert_eq!(case.store.audit_events(0, 20).await.unwrap().len(), 4);
}

#[tokio::test]
async fn generic_completion_cannot_skip_numerical_evidence() {
    let case = Case::start().await;
    let request = case.request().await;
    let error = case
        .client()
        .await
        .complete_job(CompleteJobRequest {
            context: request.context,
            job_id: request.job_id.clone(),
            lease_id: request.lease_id,
            expected_revision: request.expected_revision,
            outcome: Some(JobOutcome {
                outcome: Some(job_outcome::Outcome::Success(JobSuccess {
                    outputs: vec![],
                })),
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    assert_eq!(
        case.store
            .get(&request.job_id.unwrap().value)
            .await
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), 0);
}

#[tokio::test]
async fn cancelled_lease_cannot_start_computation() {
    let case = Case::start().await;
    let request = case.request().await;
    case.store
        .mutate(
            &actor(),
            crate::store::JobMutation::Cancel(CancelJobRequest {
                context: Some(context("cancel-evaluation")),
                job_id: request.job_id.clone(),
                expected_revision: request.expected_revision,
                reason: "synthetic cancellation".to_owned(),
            }),
        )
        .await
        .unwrap();
    let mut request = request;
    request.expected_revision += 1;
    let error = case
        .client()
        .await
        .evaluate_factor(request)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&case.views).unwrap().count(), 0);
}

#[tokio::test]
async fn expired_lease_denies_computation() {
    let case = Case::start().await;
    let request = case.request().await;
    case.clock.0.store(120_001, Ordering::SeqCst);
    let error = case
        .client()
        .await
        .evaluate_factor(request.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert_eq!(
        case.store
            .get(&request.job_id.unwrap().value)
            .await
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&case.views).unwrap().count(), 0);
}

#[tokio::test]
async fn changed_input_cannot_reach_the_worker() {
    let case = Case::start().await;
    let request = case.request().await;
    let data: model::Dataset = serde_json::from_slice(
        &std::fs::read(case.fixture.path(&case.fixture.context.data)).unwrap(),
    )
    .unwrap();
    let values = data.snapshots[0]
        .artifacts
        .iter()
        .find(|artifact| artifact.media_type == "text/csv")
        .unwrap();
    std::fs::write(case.fixture.path(&values.object), b"corrupt").unwrap();
    let error = case
        .client()
        .await
        .evaluate_factor(request.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Unavailable);
    assert_eq!(
        case.store
            .get(&request.job_id.unwrap().value)
            .await
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&case.views).unwrap().count(), 0);
}

#[tokio::test]
async fn changed_output_cannot_replay_a_successful_receipt() {
    let case = Case::start().await;
    let request = case.request().await;
    let job = case
        .client()
        .await
        .evaluate_factor(request.clone())
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    let Some(job_outcome::Outcome::Success(success)) = job.outcome.unwrap().outcome else {
        panic!("numerical success");
    };
    std::fs::write(
        case.output
            .join(&success.outputs[0].artifact_id.as_ref().unwrap().value[7..]),
        b"corrupt",
    )
    .unwrap();
    let error = case
        .client()
        .await
        .evaluate_factor(request)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Unavailable);
    assert_eq!(case.store.audit_events(0, 20).await.unwrap().len(), 4);
}

#[tokio::test]
async fn raw_policy_is_not_ignored() {
    let mut fixture = fixture(false).await;
    change_preprocess(&mut fixture);
    let case = Case::open(fixture).await;
    let error = case
        .client()
        .await
        .evaluate_factor(case.request().await)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Unavailable);
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&case.views).unwrap().count(), 0);
}

#[tokio::test]
async fn transformed_policy_must_match_panel() {
    let mut fixture = fixture(true).await;
    change_preprocess(&mut fixture);
    let case = Case::open(fixture).await;
    let error = case
        .client()
        .await
        .evaluate_factor(case.request().await)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Unavailable);
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&case.views).unwrap().count(), 0);
}

#[tokio::test]
async fn transformed_values_commit() {
    let case = Case::open(fixture(true).await).await;
    let job = case
        .client()
        .await
        .evaluate_factor(case.request().await)
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    let Some(job_outcome::Outcome::Success(success)) = job.outcome.unwrap().outcome else {
        panic!("transformed numerical success");
    };
    assert!(
        success
            .outputs
            .iter()
            .all(|artifact| artifact.schema.as_ref().unwrap().version == 2)
    );
    let content = std::fs::read_to_string(
        case.output
            .join(&success.outputs[0].artifact_id.as_ref().unwrap().value[7..]),
    )
    .unwrap();
    let rows = content.lines().skip(1).collect::<Vec<_>>();
    assert_eq!(rows.len(), 24);
    let expected = [1.0_f64, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0];
    for (index, row) in rows.iter().enumerate() {
        let value = row.rsplit(',').next().unwrap();
        if index < 8 {
            assert!(value.is_empty());
        } else {
            assert!((value.parse::<f64>().unwrap() - expected[index % 8]).abs() < 1e-12);
        }
    }
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            case.output
                .join(&success.outputs[1].artifact_id.as_ref().unwrap().value[7..]),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(report["schema"], "loop.factor-evaluation-result/v2");
    assert_eq!(report["eligible_observations"], 24);
    assert_eq!(report["valid_observations"], 16);
    assert_eq!(report["transform"]["raw_valid_observations"], 16);
    assert_eq!(
        report["transform"]["outcomes"],
        serde_json::json!(["insufficient", "ok", "ok"])
    );
}

#[tokio::test]
async fn transformed_completion_replays() {
    let mut case = Case::open(fixture(true).await).await;
    let request = case.request().await;
    let first = case
        .client()
        .await
        .evaluate_factor(request.clone())
        .await
        .unwrap()
        .into_inner();
    let before = std::fs::read_dir(&case.output).unwrap().count();
    case.restart().await;
    let repeated = case
        .client()
        .await
        .evaluate_factor(request)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first, repeated);
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), before);
    assert_eq!(case.store.audit_events(0, 20).await.unwrap().len(), 4);
}

#[tokio::test]
async fn changed_exposures_block_execution() {
    let case = Case::open(fixture(true).await).await;
    let request = case.request().await;
    let data: model::Dataset = serde_json::from_slice(
        &std::fs::read(case.fixture.path(&case.fixture.context.data)).unwrap(),
    )
    .unwrap();
    let exposure = data.snapshots[0]
        .artifacts
        .iter()
        .find(|artifact| artifact.schema.name == "loop.factor_exposures")
        .unwrap();
    std::fs::write(case.fixture.path(&exposure.object), b"corrupt").unwrap();
    let error = case
        .client()
        .await
        .evaluate_factor(request)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Unavailable);
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&case.views).unwrap().count(), 0);
}
