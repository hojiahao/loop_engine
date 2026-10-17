//! Alternate frozen policy for a bounded, real window-exhaustion scenario.

use std::fs;
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use loop_protocol::wire::jobs::v1::CancelJobRequest;
use loop_protocol::wire::{runs::v1 as runs, v1};
use prost::Message;
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
use serde_json::{Value, json};

use super::{Case, ObjectRef, operations::Operations, private_file, put};
use crate::manifests::feedback::{FeedbackPolicy, Proposal};
use crate::store::{
    AdmissionPolicy, Clock, JobMutation, JobRepository, PgJobStore, StoreError, StoreResult,
    SystemClock,
};
use crate::test_support;

struct ExactCancel(v1::JobSpecification);

pub(in crate::runtime::discovery::tests) struct HeldWorker {
    process: PathBuf,
    descriptor: OwnedFd,
}

impl HeldWorker {
    pub(in crate::runtime::discovery::tests) async fn wait_exit(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.process.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled worker must exit and be reaped");
    }
}

impl Drop for HeldWorker {
    fn drop(&mut self) {
        // A failed assertion must not orphan a stopped process. The kernel
        // handle identifies this child even if its numeric PID is later reused.
        let _ = pidfd_send_signal(&self.descriptor, Signal::KILL);
    }
}

impl AdmissionPolicy for ExactCancel {
    fn validate_submission(&self, _: &v1::JobSpecification) -> StoreResult<()> {
        Err(StoreError::AdmissionDenied)
    }

    fn authorize_job_command(
        &self,
        operation: &str,
        actor: &v1::Actor,
        record: &v1::JobRecord,
    ) -> StoreResult<()> {
        if operation != "loop.jobs.cancel"
            || self.0.kind != v1::JobKind::Perturbation as i32
            || record.specification.as_ref() != Some(&self.0)
            || self.0.submitted_by.as_ref() != Some(actor)
        {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(())
    }
}

impl Case {
    pub(in crate::runtime::discovery::tests) async fn cancel_selection(
        &self,
        job: &v1::JobRecord,
    ) -> v1::JobRecord {
        // Exercise the existing transactional cancellation command under an
        // exact test-only grant. This is not a public operator/RPC permission.
        let specification = job.specification.as_ref().unwrap();
        let actor = specification.submitted_by.as_ref().unwrap();
        let mut options = test_support::base_options(&self.directory.path().join("state"));
        options.apply_migrations = false;
        options.admission = Arc::new(ExactCancel(specification.clone()));
        let store = PgJobStore::open(options).await.unwrap();
        let mut context = test_support::context("limits.cancel");
        context.actor = Some(actor.clone());
        context.requested_at = Some(test_support::timestamp(SystemClock.now_millis().unwrap()));
        let result = store
            .mutate(
                actor,
                JobMutation::Cancel(CancelJobRequest {
                    context: Some(context),
                    job_id: specification.job_id.clone(),
                    expected_revision: job.revision,
                    reason: "Stop the leased-selection acceptance fixture".into(),
                }),
            )
            .await
            .unwrap();
        assert!(!result.replayed);
        store.close().await;
        result.job
    }
}

impl Operations {
    pub(in crate::runtime::discovery::tests) fn hold_selection(&mut self) {
        let mut deployment = self.run_deployment.as_ref().unwrap().clone();
        let python = deployment["evaluation"]["python"].as_str().unwrap();
        let normal = deployment["portfolio"]["python"].as_str().unwrap();
        let worker = self.directory().join("held-python");
        let marker = self.directory().join("held-worker-pid");
        let counter = self.directory().join("perturbation-starts");
        // The managed process is the real fixed interpreter. Import the real
        // module, then stop before calculation; no grandchild or result stub is
        // involved. Resuming this process would execute the unchanged worker.
        let code = "import os, signal, sys\n\
            with open(sys.argv[1], 'x') as marker: marker.write(str(os.getpid()))\n\
            from loop_research.perturbation import main\n\
            with open(sys.argv[2], 'a') as counter: counter.write('start\\n')\n\
            os.kill(os.getpid(), signal.SIGSTOP)\n\
            raise SystemExit(main())\n";
        let script = format!(
            "#!/bin/sh\nif [ \"$3\" = loop_research.perturbation ]; then\n  exec {} -I -c {} {} {}\nfi\nexec {} \"$@\"\n",
            shell_quote(python),
            shell_quote(code),
            shell_quote(marker.to_str().unwrap()),
            shell_quote(counter.to_str().unwrap()),
            shell_quote(normal),
        );
        private_file(&worker, script.as_bytes());
        fs::set_permissions(&worker, fs::Permissions::from_mode(0o700)).unwrap();
        deployment["portfolio"]["python"] = serde_json::to_value(worker).unwrap();
        self.run_deployment = Some(deployment);
    }

    pub(in crate::runtime::discovery::tests) async fn held_worker(&self) -> HeldWorker {
        let marker = self.directory().join("held-worker-pid");
        let pid = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(value) = fs::read_to_string(&marker)
                    && let Ok(value) = value.parse::<i32>()
                {
                    break Pid::from_raw(value).expect("positive worker PID");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("actual worker must publish its PID");
        let worker = HeldWorker {
            process: PathBuf::from(format!("/proc/{}", pid.as_raw_nonzero())),
            descriptor: pidfd_open(pid, PidfdFlags::empty()).unwrap(),
        };
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let status = fs::read_to_string(worker.process.join("status")).unwrap();
                if status.lines().any(|line| line.starts_with("State:\tT")) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("installed worker must reach the controlled stop");
        worker
    }

    pub(in crate::runtime::discovery::tests) fn limit_windows(&mut self) {
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
        policy.proposal_schedule = vec![
            Proposal::Discovery,
            Proposal::Perturbation,
            Proposal::Perturbation,
            Proposal::Discovery,
        ];
        // The first real model candidate uses five. The first perturbation has
        // exactly one choice (two); the second must emit actual exhaustion.
        policy.eligible_windows = vec![2, 5];
        let feedback = put(&root, &serde_json::to_vec(&policy).unwrap());
        specification.feedback_policy = Some(v1::PolicyReference {
            policy_id: Some(v1::PolicyId { value: policy.id }),
            revision: policy.revision,
            sha256: Some(v1::Sha256Digest {
                value: feedback.digest().unwrap().to_vec(),
            }),
        });
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
        specification.maximum_rounds = 4;
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

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
