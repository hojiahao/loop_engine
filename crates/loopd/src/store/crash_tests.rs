//! Fault injection is compiled only into the library test executable.

#[path = "../../tests/support/mod.rs"]
mod support;

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use std::time::{Duration, Instant};

use loop_core::audit::verify_audit_chain;
use loop_protocol::wire::jobs::v1::AcquireJobLeaseRequest;
use loop_protocol::wire::v1::{JobId, JobState};

use super::{JobMutation, JobRepository, RecoveryCommand, SqliteJobStore};
use support::*;

pub(super) async fn fault_point(point: &str) {
    if std::env::var("LOOP_TEST_FAULT_POINT").ok().as_deref() == Some(point) {
        let path = std::env::var_os("LOOP_TEST_FAULT_READY").expect("test fault ready path");
        std::fs::write(path, b"ready").unwrap();
        std::future::pending::<()>().await;
    }
}

struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "invoked by kill/restart test as an isolated subprocess"]
fn crash_worker() {
    let path = std::env::var_os("LOOP_TEST_DB").expect("test database");
    let mode = std::env::var("LOOP_TEST_FAULT_POINT").unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let store = SqliteJobStore::open(options(
                Path::new(&path),
                Arc::new(FixtureClock(AtomicI64::new(NOW))),
            ))
            .await
            .unwrap();
            store.submit(command(1)).await.unwrap();
            if mode == "active_lease" {
                store
                    .mutate(
                        &actor(),
                        JobMutation::Acquire(AcquireJobLeaseRequest {
                            context: Some(context("claim")),
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
                    .unwrap();
                fault_point("active_lease").await;
            }
            panic!("fault point not reached");
        });
}

#[tokio::test]
async fn killed_writer_preserves_atomicity_before_after_commit_and_during_lease() {
    for point in ["before_commit", "after_commit", "active_lease"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.sqlite3");
        let ready = directory.path().join("ready");
        let mut worker = Worker(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "store::crash_tests::crash_worker",
                    "--ignored",
                    "--nocapture",
                ])
                .env("LOOP_TEST_DB", &path)
                .env("LOOP_TEST_FAULT_POINT", point)
                .env("LOOP_TEST_FAULT_READY", &ready)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ready.exists() {
            assert!(
                worker.0.try_wait().unwrap().is_none(),
                "fault worker exited early"
            );
            assert!(Instant::now() < deadline, "fault worker timed out");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        worker.0.kill().unwrap();
        assert!(!worker.0.wait().unwrap().success());
        let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
        let store = SqliteJobStore::open(options(&path, clock.clone()))
            .await
            .unwrap();
        store.verify_configuration().await.unwrap();
        if point == "before_commit" {
            assert!(store.get("job.1").await.unwrap().is_none());
            assert!(store.audit_events(0, 500).await.unwrap().is_empty());
            assert!(!store.submit(command(1)).await.unwrap().replayed);
        } else {
            assert!(store.submit(command(1)).await.unwrap().replayed);
        }
        if point == "active_lease" {
            assert_eq!(store.get("job.1").await.unwrap().unwrap().revision, 2);
            clock.0.store(NOW + 30_000, Ordering::SeqCst);
            let recovered = store
                .mutate(
                    &actor(),
                    JobMutation::Recover(RecoveryCommand {
                        context: Some(context("recover")),
                        job_id: Some(JobId {
                            value: "job.1".to_owned(),
                        }),
                        expected_revision: 2,
                    }),
                )
                .await
                .unwrap();
            assert_eq!(recovered.job.state, JobState::InfrastructureFailed as i32);
        }
        let events = store.audit_events(0, 500).await.unwrap();
        assert_eq!(events.len(), if point == "active_lease" { 3 } else { 1 });
        verify_audit_chain(&events).unwrap();
        store.close().await;
    }
}
