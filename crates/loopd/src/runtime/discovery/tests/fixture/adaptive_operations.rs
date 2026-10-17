//! Real v3 plan and loopback supplier; all numerical workers remain installed.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use loop_protocol::wire::{research::v1 as research, runs::v1 as runs, v1};
use prost::Message;
use serde_json::{Value, json};

use super::{Case, ObjectRef, operations::Operations, private_file, put};
use crate::manifests::feedback::{FeedbackPolicy, Proposal};

impl Case {
    pub(in crate::runtime::discovery::tests) fn corrupt_evidence(&self, digest: &str) {
        let digest = digest.strip_prefix("sha256:").unwrap();
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|value| value.is_ascii_hexdigit()));
        let path = self.directory.path().join("data").join(digest);
        assert!(path.is_file());
        private_file(&path, b"corrupt frozen evidence");
    }

    pub(in crate::runtime::discovery::tests) async fn install_adaptive(&mut self) -> Operations {
        self.install_feedback(true).await
    }

    pub(in crate::runtime::discovery::tests) async fn install_feedback(
        &mut self,
        history: bool,
    ) -> Operations {
        let operations = if history {
            self.install_history(3).await
        } else {
            self.install_numerical(3, false).await
        };
        self.configure_feedback(operations)
    }

    pub(in crate::runtime::discovery::tests) async fn install_memory(&mut self) -> Operations {
        let operations = self.install_sparse(3).await;
        self.configure_feedback(operations)
    }

    fn configure_feedback(&self, mut operations: Operations) -> Operations {
        let mut deployment = operations.run_deployment.as_ref().unwrap().clone();
        let root = operations.directory().join("run-plans");
        let reference: ObjectRef =
            serde_json::from_value(deployment["runs"]["plans"][0].clone()).unwrap();
        let mut document: Value =
            serde_json::from_slice(&fs::read(root.join(&reference.sha256[7..])).unwrap()).unwrap();
        let template: ObjectRef =
            serde_json::from_value(document["specification"].clone()).unwrap();
        let mut specification = runs::RunSpecification::decode(
            fs::read(root.join(&template.sha256[7..]))
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let policy = FeedbackPolicy {
            schema: "loop.run-feedback/v1".into(),
            id: "feedback.synthetic".into(),
            revision: "1".into(),
            proposal_schedule: vec![
                Proposal::Discovery,
                Proposal::Perturbation,
                Proposal::Discovery,
            ],
            window_path: vec![1],
            eligible_windows: vec![2, 3, 4, 5],
            random_seed: format!("sha256:{}", "01".repeat(32)),
            max_observations: 16,
            max_bytes: 16_384,
        };
        let feedback = put(&root, &serde_json::to_vec(&policy).unwrap());
        specification.feedback_policy = Some(v1::PolicyReference {
            policy_id: Some(v1::PolicyId { value: policy.id }),
            revision: policy.revision,
            sha256: Some(v1::Sha256Digest {
                value: feedback.digest().unwrap().to_vec(),
            }),
        });
        let numerical = specification.numerical.as_mut().unwrap();
        numerical.perturbation_budget = Some(research::ResearchJobBudget {
            maximum_steps: 1,
            maximum_input_tokens: 0,
            maximum_output_tokens: 0,
            maximum_cost: Some(v1::Money {
                currency_code: "USD".into(),
                amount: Some(v1::ExactDecimal { value: "0".into() }),
            }),
            maximum_wall_time: Some(prost_types::Duration {
                seconds: 20,
                nanos: 0,
            }),
        });
        let protocol = numerical.protocol_selection.as_mut().unwrap();
        protocol
            .enabled_features
            .push("jobs.perturbation.v1".into());
        protocol.enabled_features.sort();
        protocol.selection_sha256 = Some(v1::Sha256Digest {
            value: loop_protocol::job::protocol_selection_sha256(protocol)
                .unwrap()
                .to_vec(),
        });
        document["schema"] = "loop.research-run/v3".into();
        document["feedback_policy"] = serde_json::to_value(feedback).unwrap();
        document["specification"] =
            serde_json::to_value(put(&root, &specification.encode_to_vec())).unwrap();
        let reference = put(&root, &serde_json::to_vec(&document).unwrap());
        deployment["runs"]["plans"] = json!([reference]);
        let python = deployment["portfolio"]["python"].as_str().unwrap();
        let worker = operations.directory().join("counted-python");
        let counter = operations.directory().join("perturbation-starts");
        // The shim counts launches, then execs the actual frozen interpreter.
        // No result, worker response or numerical operation is mocked.
        let script = format!(
            "#!/bin/sh\nif [ \"$3\" = loop_research.perturbation ]; then\n  printf 'start\\n' >> {}\nfi\nexec {} \"$@\"\n",
            shell_quote(counter.to_str().unwrap()),
            shell_quote(python),
        );
        private_file(&counter, b"");
        private_file(&worker, script.as_bytes());
        fs::set_permissions(&worker, fs::Permissions::from_mode(0o700)).unwrap();
        deployment["portfolio"]["python"] = serde_json::to_value(worker).unwrap();
        let config: Value =
            serde_json::from_slice(&fs::read(&operations.run_config).unwrap()).unwrap();
        let plan = v1::PolicyReference {
            policy_id: Some(v1::PolicyId {
                value: "run.synthetic".into(),
            }),
            revision: "1".into(),
            sha256: Some(v1::Sha256Digest {
                value: reference.digest().unwrap().to_vec(),
            }),
        };
        operations.configure_run(deployment, &config, &plan);
        let sequence = json!([
            {"node":"call", "operator":"ma", "operator_version":"2", "arguments":[
                {"node":"field", "field":"market.close"},
                {"node":"decimal", "value":"5"},
                {"node":"decimal", "value":"2"}
            ]},
            {"node":"field", "field":"market.close"}
        ]);
        private_file(
            &self.directory.path().join("candidate-sequence.json"),
            &serde_json::to_vec(&sequence).unwrap(),
        );
        operations
    }
}

impl Operations {
    pub(in crate::runtime::discovery::tests) fn perturbation_starts(&self) -> usize {
        fs::read_to_string(self.directory().join("perturbation-starts"))
            .unwrap()
            .lines()
            .count()
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
