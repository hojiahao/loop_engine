//! Frozen variants for real parent-budget and coverage-memory acceptance.

use std::fs;

use loop_protocol::wire::{runs::v1 as runs, v1};
use prost::Message;
use serde_json::{Value, json};

use super::{Case, ObjectRef, operations::Operations, private_file, put};
use crate::manifests::feedback::{FeedbackPolicy, Proposal};

impl Case {
    pub(in crate::runtime::discovery::tests) fn window_candidates(&self) {
        // Two model-originated members of one family. Dense fixtures evaluate
        // both; on the sparse panel only ma(5,2) has sufficient coverage.
        let sequence: Vec<_> = [5, 2]
            .into_iter()
            .map(|window| {
                json!({"node":"call", "operator":"ma", "operator_version":"2", "arguments":[
                    {"node":"field", "field":"market.close"},
                    {"node":"decimal", "value":window.to_string()},
                    {"node":"decimal", "value":"2"}
                ]})
            })
            .collect();
        private_file(
            &self.directory.path().join("candidate-sequence.json"),
            &serde_json::to_vec(&sequence).unwrap(),
        );
    }

    pub(in crate::runtime::discovery::tests) fn selection_work(
        &self,
        job: &v1::JobRecord,
    ) -> v1::PerturbationWork {
        let Some(v1::job_specification::Input::Perturbation(input)) = job
            .specification
            .as_ref()
            .and_then(|specification| specification.input.as_ref())
        else {
            panic!("actual perturbation input")
        };
        let digest = &input
            .work
            .as_ref()
            .unwrap()
            .artifact_id
            .as_ref()
            .unwrap()
            .value;
        let bytes = fs::read(self.directory.path().join("data").join(&digest[7..])).unwrap();
        v1::PerturbationWork::decode(bytes.as_slice()).unwrap()
    }
}

impl Operations {
    pub(in crate::runtime::discovery::tests) fn limit_parent(&mut self, steps: u64) {
        self.edit_adaptive(|specification, _| {
            specification.budget.as_mut().unwrap().maximum_steps = steps;
        });
    }

    pub(in crate::runtime::discovery::tests) fn family_schedule(&mut self) {
        self.edit_adaptive(|specification, policy| {
            policy.proposal_schedule = vec![
                Proposal::Discovery,
                Proposal::Discovery,
                Proposal::Perturbation,
                Proposal::Discovery,
            ];
            policy.eligible_windows = vec![2, 5];
            specification.maximum_rounds = 4;
            let budget = specification.budget.as_mut().unwrap();
            budget.maximum_steps = 12;
            budget.maximum_input_tokens = 4 * 4096;
            budget.maximum_output_tokens = 4 * 128;
            budget
                .maximum_cost
                .as_mut()
                .unwrap()
                .amount
                .as_mut()
                .unwrap()
                .value = "4".into();
        });
    }

    fn edit_adaptive(
        &mut self,
        change: impl FnOnce(&mut runs::RunSpecification, &mut FeedbackPolicy),
    ) {
        let mut deployment = self.run_deployment.as_ref().unwrap().clone();
        let root = self.directory().join("run-plans");
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
        let reference: ObjectRef =
            serde_json::from_value(document["feedback_policy"].clone()).unwrap();
        let mut policy: FeedbackPolicy =
            serde_json::from_slice(&fs::read(root.join(&reference.sha256[7..])).unwrap()).unwrap();
        change(&mut specification, &mut policy);
        let feedback = put(&root, &serde_json::to_vec(&policy).unwrap());
        specification.feedback_policy = Some(v1::PolicyReference {
            policy_id: Some(v1::PolicyId { value: policy.id }),
            revision: policy.revision,
            sha256: Some(v1::Sha256Digest {
                value: feedback.digest().unwrap().to_vec(),
            }),
        });
        document["feedback_policy"] = serde_json::to_value(feedback).unwrap();
        document["specification"] =
            serde_json::to_value(put(&root, &specification.encode_to_vec())).unwrap();
        let reference = put(&root, &serde_json::to_vec(&document).unwrap());
        deployment["runs"]["plans"] = json!([reference]);
        let config: Value = serde_json::from_slice(&fs::read(&self.run_config).unwrap()).unwrap();
        self.configure_run(
            deployment,
            &config,
            &v1::PolicyReference {
                policy_id: Some(v1::PolicyId {
                    value: "run.synthetic".into(),
                }),
                revision: "1".into(),
                sha256: Some(v1::Sha256Digest {
                    value: reference.digest().unwrap().to_vec(),
                }),
            },
        );
    }
}
