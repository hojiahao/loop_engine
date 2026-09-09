mod support;

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, atomic::AtomicI64};
use std::time::{Duration, Instant};

use loop_core::audit::verify_audit_chain;
use loop_protocol::wire::jobs::v1::AcquireJobLeaseRequest;
use loop_protocol::wire::v1::JobId;
use loopd::store::{
    CloseGrant, GrantClosure, HoldoutRepository, JobMutation, JobRepository, PgJobStore, StoreError,
};
use prost::Message;
use support::*;

struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "worker barrier timed out: {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[test]
#[ignore = "invoked only as an isolated subprocess by the process-race test"]
fn process_worker() {
    let path = std::env::var_os("LOOP_TEST_DB").expect("subprocess database");
    let index: u32 = std::env::var("LOOP_TEST_INDEX").unwrap().parse().unwrap();
    let mode = std::env::var("LOOP_TEST_MODE").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let path = Path::new(&path);
        let mut options = options(path, Arc::new(FixtureClock(AtomicI64::new(NOW))));
        if mode == "role" {
            options.admission = Arc::new(research::Admission);
        }
        if mode == "period" {
            options.holdout_policy = Arc::new(holdout::Policy);
        }
        if mode.starts_with("approval") {
            options.holdout_policy = Arc::new(approval::Policy::default());
        }
        if mode.starts_with("grant") || mode.starts_with("close") {
            options.holdout_policy = Arc::new(grant::Policy::default());
        }
        let store = PgJobStore::open(options).await.unwrap();
        let directory = path.parent().unwrap();
        std::fs::write(directory.join(format!("ready.{index}")), b"ready").unwrap();
        wait_for(&directory.join("start")).await;
        let result = match mode.as_str() {
            "grant" | "grant-race" => {
                let bytes = std::fs::read(directory.join("input")).unwrap();
                let mut request =
                    loop_protocol::wire::holdout::v1::RequestHoldoutGrantRequest::decode(
                        bytes.as_slice(),
                    )
                    .unwrap();
                if mode.ends_with("race") {
                    request.context = Some(context(&format!("grant.{index}")));
                }
                store
                    .issue_grant(&actor(), request)
                    .await
                    .map(|value| value.replayed)
            }
            "close" | "close-race" => {
                let bytes = std::fs::read(directory.join("input")).unwrap();
                let mut request = CloseGrant::decode(bytes.as_slice()).unwrap();
                if mode.ends_with("race") {
                    request.context = Some(context(&format!("close.{index}")));
                }
                store
                    .close_grant(&actor(), request)
                    .await
                    .map(|value| value.replayed)
            }
            "approval" | "approval-distinct" => store
                .record_approval(
                    &approval::human(1),
                    approval::command(
                        0,
                        &if mode == "approval" {
                            "approval.retry".to_owned()
                        } else {
                            format!("approval.{index}")
                        },
                        &approval::human(1),
                    ),
                )
                .await
                .map(|value| value.replayed),
            "period" => store
                .register_period(&actor(), holdout::command(0, "period.retry"))
                .await
                .map(|value| value.replayed),
            "submit" => store.submit(command(1)).await.map(|value| value.replayed),
            "distinct" => store
                .submit(command(index + 1))
                .await
                .map(|value| value.replayed),
            "role" => store
                .submit_role(
                    &actor(),
                    research::command("role.retry"),
                    research::metadata(),
                )
                .await
                .map(|value| value.replayed),
            "acquire" => store
                .mutate(
                    &actor(),
                    JobMutation::Acquire(AcquireJobLeaseRequest {
                        context: Some(context(&format!("claim.{index}"))),
                        job_id: Some(JobId {
                            value: "job.1".to_owned(),
                        }),
                        expected_revision: 1,
                        requested_duration: Some(prost_types::Duration {
                            seconds: 30,
                            nanos: 0,
                        }),
                    }),
                )
                .await
                .map(|value| value.replayed),
            _ => panic!("unknown worker mode"),
        };
        let outcome = match result {
            Ok(true) => "replayed",
            Ok(false) => "committed",
            Err(StoreError::RevisionConflict) if mode == "acquire" || mode.ends_with("race") => {
                "fenced"
            }
            other => panic!("unexpected worker outcome: {other:?}"),
        };
        std::fs::write(directory.join(format!("result.{index}")), outcome).unwrap();
        store.close().await;
    });
}

#[tokio::test]
async fn process_writers_preserve_fencing() {
    for count in [2, 4, 8] {
        for mode in [
            "submit",
            "distinct",
            "acquire",
            "role",
            "period",
            "approval",
            "approval-distinct",
            "grant",
            "grant-race",
            "close",
            "close-race",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("state");
            if mode.starts_with("grant") || mode.starts_with("close") {
                let mut config = options(&path, Arc::new(FixtureClock(AtomicI64::new(NOW))));
                config.holdout_policy = Arc::new(grant::Policy::default());
                let store = PgJobStore::open(config).await.unwrap();
                let request = grant::seed(&store, 2, false).await;
                let bytes = if mode.starts_with("close") {
                    let issued = store.issue_grant(&actor(), request).await.unwrap();
                    grant::close(&issued, GrantClosure::Revoke, "close.retry").encode_to_vec()
                } else {
                    request.encode_to_vec()
                };
                std::fs::write(directory.path().join("input"), bytes).unwrap();
                store.close().await;
            }
            if mode.starts_with("approval") {
                let mut config = options(&path, Arc::new(FixtureClock(AtomicI64::new(NOW))));
                config.holdout_policy = Arc::new(approval::Policy::default());
                let store = PgJobStore::open(config).await.unwrap();
                store
                    .register_period(&actor(), holdout::command(0, "period.0"))
                    .await
                    .unwrap();
                store.close().await;
            }
            if mode == "acquire" {
                let store =
                    PgJobStore::open(options(&path, Arc::new(FixtureClock(AtomicI64::new(NOW)))))
                        .await
                        .unwrap();
                store.submit(command(1)).await.unwrap();
                store.close().await;
            }
            let mut workers = vec![];
            for index in 0..count {
                workers.push(Worker(
                    Command::new(std::env::current_exe().unwrap())
                        .args(["--exact", "process_worker", "--ignored", "--nocapture"])
                        .env("LOOP_TEST_DB", &path)
                        .env("LOOP_TEST_INDEX", index.to_string())
                        .env("LOOP_TEST_MODE", mode)
                        .stdout(Stdio::null())
                        .stderr(Stdio::inherit())
                        .spawn()
                        .unwrap(),
                ));
            }
            for index in 0..count {
                wait_for(&directory.path().join(format!("ready.{index}"))).await;
            }
            std::fs::write(directory.path().join("start"), b"start").unwrap();
            let deadline = Instant::now() + Duration::from_secs(30);
            for worker in &mut workers {
                loop {
                    if let Some(status) = worker.0.try_wait().unwrap() {
                        assert!(status.success());
                        break;
                    }
                    assert!(Instant::now() < deadline, "worker failed to exit");
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
            let mut committed = 0;
            for index in 0..count {
                let result =
                    std::fs::read_to_string(directory.path().join(format!("result.{index}")))
                        .unwrap();
                assert!(matches!(
                    result.as_str(),
                    "committed" | "replayed" | "fenced"
                ));
                committed += usize::from(result == "committed");
            }
            assert_eq!(
                committed,
                if mode.ends_with("distinct") { count } else { 1 }
            );
            let store =
                PgJobStore::open(options(&path, Arc::new(FixtureClock(AtomicI64::new(NOW)))))
                    .await
                    .unwrap();
            let events = store.audit_events(0, 500).await.unwrap();
            let baseline = if mode.starts_with("grant") {
                3
            } else if mode.starts_with("close") {
                4
            } else {
                usize::from(mode == "acquire" || mode.starts_with("approval"))
            };
            assert_eq!(events.len(), baseline + committed);
            verify_audit_chain(&events).unwrap();
            store.verify_configuration().await.unwrap();
            store.close().await;
        }
    }
}
