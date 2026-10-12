use loop_protocol::wire::{runs::v1 as wire, v1};
use prost::Message;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::process::Command;

use super::operations::Operations;
use super::{Case, Clock, private_dir, private_file, put};

impl Case {
    pub(in crate::runtime::discovery::tests) async fn agent_run(
        &self,
    ) -> wire::run_service_client::RunServiceClient<tonic::transport::Channel> {
        wire::run_service_client::RunServiceClient::new(
            super::channel(&self.tls, self.address.port(), "client").await,
        )
    }

    pub(in crate::runtime::discovery::tests) fn install_run(
        &self,
        rounds: u32,
        allowance: u64,
    ) -> Operations {
        let mut operations = self.install();
        let root = operations.directory();
        let plans = root.join("run-plans");
        private_dir(&plans);
        let owner = v1::Actor {
            actor_id: Some(v1::ActorId {
                value: "operator.research".into(),
            }),
            authenticated_subject: "human:research".into(),
            display_name: "Run owner".into(),
            kind: v1::ActorKind::Human as i32,
        };
        let budget = self.input.budget.as_ref().unwrap();
        let metadata = self
            .executor
            .submission(&self.actor(), &self.input)
            .unwrap();
        let template = wire::RunSpecification {
            plan: None,
            run_id: Some(metadata.run_id),
            owner: Some(owner.clone()),
            executor: Some(self.actor()),
            discovery: Some(self.input.clone()),
            protocol_selection: Some(metadata.protocol_selection),
            maximum_rounds: rounds,
            budget: Some(wire::RunBudget {
                maximum_steps: u64::from(budget.maximum_steps) * allowance,
                maximum_input_tokens: budget.maximum_input_tokens * allowance,
                maximum_output_tokens: budget.maximum_output_tokens * allowance,
                maximum_cost: Some(v1::Money {
                    currency_code: "USD".into(),
                    amount: Some(v1::ExactDecimal {
                        value: ((if budget.maximum_steps == 3 { 2 } else { 1 }) * allowance)
                            .to_string(),
                    }),
                }),
                maximum_wall_time: Some(prost_types::Duration {
                    seconds: 600,
                    nanos: 0,
                }),
            }),
        };
        let document = json!({"schema":"loop.research-run/v1","id":"run.synthetic","revision":"1",
            "specification":put(&plans, &template.encode_to_vec())});
        let reference = put(&plans, &serde_json::to_vec(&document).unwrap());
        let policy = v1::PolicyReference {
            policy_id: Some(v1::PolicyId {
                value: "run.synthetic".into(),
            }),
            revision: "1".into(),
            sha256: Some(v1::Sha256Digest {
                value: reference.digest().unwrap().to_vec(),
            }),
        };
        let result = Command::new("openssl")
            .args(["x509", "-in"])
            .arg(self.tls.path("unknown.pem"))
            .args(["-outform", "DER"])
            .output()
            .unwrap();
        assert!(result.status.success());
        let digest = format!("sha256:{:x}", Sha256::digest(result.stdout));
        let now = self.clock.now_millis().unwrap();
        let config = json!({
            "schema":"loop.operator/v1","endpoint":format!("https://localhost:{}",self.address.port()),
            "server_name":"localhost","ca_file":self.tls.path("ca.pem"),
            "certificate_file":self.tls.path("unknown.pem"),"private_key_file":self.tls.path("unknown.key"),
            "actor":{"actor_id":"operator.research","subject":"human:research","display_name":"Run owner"}
        });
        let deployment = json!({"identity": {
            "actor_id":"operator.research","subject":"human:research","display_name":"Run owner","role":"operator",
            "certificate_sha256":[digest],"not_before_ms":now-1000,"expires_at_ms":now+3600000,"run_ids":["run.discovery"]
        },"runs":{"plan_store":plans,"plans":[reference]}});
        operations.configure_run(deployment, &config, &policy);
        operations
    }
}

impl Operations {
    // Retain the legitimate caller identity to exercise server plan verification.
    pub(in crate::runtime::discovery::tests) fn corrupt_run(&self) {
        let root = self.directory().join("run-plans");
        for file in fs::read_dir(root).unwrap() {
            private_file(&file.unwrap().path(), b"changed");
        }
    }
}
