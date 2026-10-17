//! Build numerical runs from the same real panel/tape as standalone acceptance.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use loop_protocol::wire::{research::v1 as research, runs::v1 as runs, v1};
use prost::Message;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    Case, Clock, FrozenPlan, ObjectRef, operations::Operations, private_dir, private_file, put,
};

// Match the recipe's canonical field order instead of serializing an unordered
// JSON object. The resolver verifies both the digest and the typed encoding.
#[derive(Serialize)]
struct Recipe {
    schema: &'static str,
    id: &'static str,
    revision: &'static str,
    evaluation_context: ObjectRef,
    direction: &'static str,
    deterministic_seed: String,
    execution_tape: ObjectRef,
    policies: BTreeMap<String, String>,
    hypothesis: &'static str,
    required_observations: Vec<&'static str>,
    falsification_criteria: Vec<&'static str>,
}

impl Case {
    pub(in crate::runtime::discovery::tests) async fn install_numerical(
        &mut self,
        rounds: u32,
        missing: bool,
    ) -> Operations {
        self.install_workflow(rounds, missing, false).await
    }

    pub(super) async fn install_history(&mut self, rounds: u32) -> Operations {
        self.install_workflow(rounds, false, true).await
    }

    pub(super) async fn install_sparse(&mut self, rounds: u32) -> Operations {
        self.install_workflow(rounds, true, true).await
    }

    async fn install_workflow(&mut self, rounds: u32, missing: bool, history: bool) -> Operations {
        // The helper runs the installed Python producer and freezes its actual
        // source/environment manifests, but never submits a predecessor job.
        let mut fixture = if history {
            crate::manifests::tests::run_history().await
        } else {
            crate::manifests::tests::run_fixture().await
        };
        if missing {
            fixture.context.data = missing_panel(&fixture.root, &fixture.context.data, history);
        }
        let data_root = self.directory.path().join("data");
        let (dataset, tape) = fixture.numerical_data();
        let context = fixture.json(&fixture.context);
        for entry in fs::read_dir(&fixture.root).unwrap() {
            let entry = entry.unwrap();
            assert!(entry.file_type().unwrap().is_file());
            let target = data_root.join(entry.file_name());
            let bytes = fs::read(entry.path()).unwrap();
            if target.exists() {
                assert_eq!(fs::read(&target).unwrap(), bytes);
            } else {
                private_file(&target, &bytes);
            }
        }
        let plans = self.directory.path().join("plans");
        let mut discovery: Value =
            serde_json::from_slice(&fs::read(plans.join(&self.plan.sha256[7..])).unwrap()).unwrap();
        let mut input = self.input.clone();
        input.dataset = Some(dataset);
        input.research_policy = None;
        discovery["input"] = serde_json::to_value(put(&plans, &input.encode_to_vec())).unwrap();
        discovery["data"] = serde_json::to_value(&fixture.context.data).unwrap();
        self.plan = put(&plans, &serde_json::to_vec(&discovery).unwrap());
        self.input = FrozenPlan::load(&plans, &self.plan).unwrap().input;
        self.restart().await;

        let mut operations = self.install_run(rounds, u64::from(rounds));
        let mut deployment = operations.run_deployment.as_ref().unwrap().clone();
        let run_root = operations.directory().join("run-plans");
        let reference: ObjectRef =
            serde_json::from_value(deployment["runs"]["plans"][0].clone()).unwrap();
        let mut document: Value =
            serde_json::from_slice(&fs::read(run_root.join(&reference.sha256[7..])).unwrap())
                .unwrap();
        let template: ObjectRef =
            serde_json::from_value(document["specification"].clone()).unwrap();
        let mut specification = runs::RunSpecification::decode(
            fs::read(run_root.join(&template.sha256[7..]))
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let policies: BTreeMap<String, String> = [
            "universe",
            "data",
            "calendar",
            "preprocess",
            "neutralization",
            "portfolio",
            "execution",
            "cost",
            "evaluation",
        ]
        .into_iter()
        .map(|role| (format!("{role}_policy"), format!("policy.{role}")))
        .collect();
        let recipe = put(
            &run_root,
            &serde_json::to_vec(&Recipe {
                schema: "loop.run-numerical/v1",
                id: "recipe.synthetic",
                revision: "1",
                evaluation_context: context,
                direction: "higher_is_better",
                deterministic_seed: format!("sha256:{}", "01".repeat(32)),
                execution_tape: tape,
                policies,
                hypothesis: "Closing prices rank the declared synthetic cross section.",
                required_observations: vec![
                    "Causal closing prices and next-session raw execution prices.",
                ],
                falsification_criteria: vec!["Coverage falls below the frozen numerical policy."],
            })
            .unwrap(),
        );
        let mut protocol = specification.protocol_selection.clone().unwrap();
        protocol.selected_package = "loop.v1".to_owned();
        protocol.enabled_features = [
            "jobs.envelope.v1",
            "jobs.kind-input.v1",
            "jobs.prelease-terminal.v1",
        ]
        .map(str::to_owned)
        .to_vec();
        protocol.selection_sha256 = Some(v1::Sha256Digest {
            value: loop_protocol::job::protocol_selection_sha256(&protocol)
                .unwrap()
                .to_vec(),
        });
        specification.numerical = Some(runs::NumericalWorkflow {
            research_executor: Some(v1::Actor {
                actor_id: Some(v1::ActorId {
                    value: "service.numerical".into(),
                }),
                kind: v1::ActorKind::Service as i32,
                authenticated_subject: "service:numerical".into(),
                display_name: "Numerical fixture".into(),
            }),
            recipe: Some(v1::PolicyReference {
                policy_id: Some(v1::PolicyId {
                    value: "recipe.synthetic".into(),
                }),
                revision: "1".into(),
                sha256: Some(v1::Sha256Digest {
                    value: recipe.digest().unwrap().to_vec(),
                }),
            }),
            evaluation_budget: Some(numerical_budget()),
            portfolio_budget: Some(numerical_budget()),
            protocol_selection: Some(protocol),
            perturbation_budget: None,
        });
        let budget = specification.budget.as_mut().unwrap();
        budget.maximum_steps += u64::from(rounds) * 2;
        budget.maximum_wall_time.as_mut().unwrap().seconds = 1200;
        document["schema"] = "loop.research-run/v2".into();
        document["numerical_recipe"] = serde_json::to_value(recipe).unwrap();
        document["specification"] =
            serde_json::to_value(put(&run_root, &specification.encode_to_vec())).unwrap();
        let reference = put(&run_root, &serde_json::to_vec(&document).unwrap());
        deployment["runs"]["plans"] = json!([reference]);
        let now = self.clock.now_millis().unwrap();
        deployment["research_identity"] = json!({
            "actor_id":"service.numerical","subject":"service:numerical","display_name":"Numerical fixture","role":"research",
            "certificate_sha256":[certificate(self.tls.path("ca.pem").parent().unwrap())],
            "not_before_ms":now-1000,"expires_at_ms":now+3600000,"run_ids":["run.discovery"]
        });
        let python = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.venv/bin/python");
        for section in ["evaluation", "portfolio"] {
            let output = operations.directory().join(format!("{section}-output"));
            private_dir(&output);
            deployment[section] = json!({"python":python,"output_store":output});
            deployment[section][if section == "evaluation" {
                "contexts"
            } else {
                "jobs"
            }] = json!([]);
        }
        let config: Value =
            serde_json::from_slice(&fs::read(&operations.run_config).unwrap()).unwrap();
        let policy = v1::PolicyReference {
            policy_id: Some(v1::PolicyId {
                value: "run.synthetic".into(),
            }),
            revision: "1".into(),
            sha256: Some(v1::Sha256Digest {
                value: reference.digest().unwrap().to_vec(),
            }),
        };
        operations.configure_run(deployment, &config, &policy);
        operations
    }

    pub(in crate::runtime::discovery::tests) async fn agent_jobs(
        &self,
    ) -> loop_protocol::wire::jobs::v1::job_service_client::JobServiceClient<
        tonic::transport::Channel,
    > {
        self.tls
            .named_client(self.address, Some("client"))
            .await
            .unwrap()
    }

    pub(in crate::runtime::discovery::tests) async fn numerical_db(&self) -> sqlx::PgConnection {
        crate::test_support::connection(&self.directory).await
    }
}

fn missing_panel(root: &Path, reference: &ObjectRef, history: bool) -> ObjectRef {
    // Publish an explicit synthetic variant; retain every eligible grid cell
    // while selected observations become genuinely absent, never mocked rejects.
    let mut dataset = fs::read_to_string(root.join(&reference.sha256[7..])).unwrap();
    let metadata: Value = serde_json::from_str(&dataset).unwrap();
    let snapshot = metadata["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|snapshot| {
            snapshot["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|artifact| artifact["schema"]["name"] == "loop.factor_panel")
        })
        .unwrap();
    let panel: ObjectRef = serde_json::from_value(
        snapshot["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|artifact| artifact["schema"]["name"] == "loop.factor_panel")
            .unwrap()["object"]
            .clone(),
    )
    .unwrap();
    let mut declaration = fs::read_to_string(root.join(&panel.sha256[7..])).unwrap();
    let metadata: Value = serde_json::from_str(&declaration).unwrap();
    let values: ObjectRef = serde_json::from_value(metadata["values"].clone()).unwrap();
    let csv = fs::read_to_string(root.join(&values.sha256[7..])).unwrap();
    let mut lines = csv.lines();
    let mut missing = format!("{}\n", lines.next().unwrap());
    let mut count = 0;
    for (index, row) in lines.enumerate() {
        let mut fields: Vec<_> = row.split(',').collect();
        assert_eq!(fields.len(), 5);
        assert_eq!(fields[2], "1");
        // In the longer panel, observations arrive every other session. ma(5,2)
        // has 16 valid evaluation sessions; ma(2,2) never sees two observations.
        let absent = if history {
            (index / 6) % 2 == 1
        } else {
            index < 4
        };
        if absent {
            fields[3] = "";
            fields[4] = "";
        }
        missing.push_str(&fields.join(","));
        missing.push('\n');
        count += 1;
    }
    assert_eq!(count, if history { 18 * 6 } else { 6 });
    let new_values = put(root, missing.as_bytes());
    replace_reference(&mut declaration, &values, &new_values);
    let new_panel = put(root, declaration.as_bytes());
    replace_reference(&mut dataset, &values, &new_values);
    replace_reference(&mut dataset, &panel, &new_panel);
    let identity = snapshot["snapshot_id"].as_str().unwrap();
    dataset = dataset.replace(
        &format!("\"snapshot_id\":\"{identity}\""),
        &format!("\"snapshot_id\":\"{identity}.missing\""),
    );
    put(root, dataset.as_bytes())
}

fn replace_reference(document: &mut String, previous: &ObjectRef, next: &ObjectRef) {
    // Replacing only a typed ObjectRef preserves the producer's canonical key
    // order; serializing serde_json::Value would reorder unrelated fields.
    let previous = serde_json::to_string(previous).unwrap();
    let next = serde_json::to_string(next).unwrap();
    assert_eq!(document.matches(&previous).count(), 1);
    *document = document.replace(&previous, &next);
}

fn numerical_budget() -> research::ResearchJobBudget {
    research::ResearchJobBudget {
        maximum_steps: 1,
        maximum_input_tokens: 0,
        maximum_output_tokens: 0,
        maximum_cost: Some(v1::Money {
            currency_code: "USD".into(),
            amount: Some(v1::ExactDecimal { value: "0".into() }),
        }),
        maximum_wall_time: Some(prost_types::Duration {
            seconds: 180,
            nanos: 0,
        }),
    }
}

fn certificate(root: &Path) -> String {
    for arguments in [
        vec![
            "req",
            "-new",
            "-newkey",
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:prime256v1",
            "-nodes",
            "-keyout",
            "research.key",
            "-out",
            "research.csr",
            "-subj",
            "/CN=research",
        ],
        vec![
            "x509",
            "-req",
            "-in",
            "research.csr",
            "-CA",
            "ca.pem",
            "-CAkey",
            "ca.key",
            "-CAcreateserial",
            "-out",
            "research.pem",
            "-days",
            "1",
        ],
    ] {
        assert!(
            Command::new("openssl")
                .args(arguments)
                .current_dir(root)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }
    let output = Command::new("openssl")
        .args(["x509", "-in", "research.pem", "-outform", "DER"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success());
    format!("sha256:{:x}", Sha256::digest(output.stdout))
}
