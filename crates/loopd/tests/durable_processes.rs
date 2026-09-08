mod support;

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, atomic::AtomicI64};
use std::time::{Duration, Instant};

use loop_core::audit::verify_audit_chain;
use loop_protocol::wire::jobs::v1::AcquireJobLeaseRequest;
use loop_protocol::wire::v1::JobId;
use loopd::store::{JobMutation, JobRepository, SqliteJobStore, StoreError};
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
        let store =
            SqliteJobStore::open(options(path, Arc::new(FixtureClock(AtomicI64::new(NOW)))))
                .await
                .unwrap();
        let directory = path.parent().unwrap();
        std::fs::write(directory.join(format!("ready.{index}")), b"ready").unwrap();
        wait_for(&directory.join("start")).await;
        let result = match mode.as_str() {
            "submit" => store.submit(command(1)).await,
            "distinct" => store.submit(command(index + 1)).await,
            "acquire" => {
                store
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
            }
            _ => panic!("unknown worker mode"),
        };
        let outcome = match result {
            Ok(value) if value.replayed => "replayed",
            Ok(_) => "committed",
            Err(StoreError::RevisionConflict) if mode == "acquire" => "fenced",
            other => panic!("unexpected worker outcome: {other:?}"),
        };
        std::fs::write(directory.join(format!("result.{index}")), outcome).unwrap();
        store.close().await;
    });
}

#[tokio::test]
async fn independent_processes_preserve_commit_once_and_revision_fencing() {
    for count in [2, 4, 8] {
        for mode in ["submit", "distinct", "acquire"] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("state.sqlite3");
            if mode == "acquire" {
                let store = SqliteJobStore::open(options(
                    &path,
                    Arc::new(FixtureClock(AtomicI64::new(NOW))),
                ))
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
            assert_eq!(committed, if mode == "distinct" { count } else { 1 });
            let store =
                SqliteJobStore::open(options(&path, Arc::new(FixtureClock(AtomicI64::new(NOW)))))
                    .await
                    .unwrap();
            let events = store.audit_events(0, 500).await.unwrap();
            assert_eq!(events.len(), if mode == "acquire" { 2 } else { committed });
            verify_audit_chain(&events).unwrap();
            store.verify_configuration().await.unwrap();
            store.close().await;
        }
    }
}
