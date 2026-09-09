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

use super::{
    BacktestRepository, GrantClosure, HoldoutRepository, JobMutation, JobRepository, PgJobStore,
    RecoveryCommand,
};
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
            let mut options = options(
                Path::new(&path),
                Arc::new(FixtureClock(AtomicI64::new(NOW))),
            );
            if mode.starts_with("role_") {
                options.admission = Arc::new(research::Admission);
            }
            if mode.starts_with("result_") {
                options.admission = Arc::new(backtest::Admission);
                options.backtest_policy = Arc::new(backtest::Policy::default());
            }
            if mode.starts_with("period_") {
                options.holdout_policy = Arc::new(holdout::Policy);
            }
            if mode.starts_with("approval_") {
                options.holdout_policy = Arc::new(approval::Policy::default());
            }
            if mode.starts_with("grant_") || mode.starts_with("close_") {
                options.holdout_policy = Arc::new(grant::Policy::default());
            }
            if mode.starts_with("batch_") {
                options.holdout_policy = Arc::new(batch::Policy::default());
                options.admission = Arc::new(batch::Admission);
            }
            let store = PgJobStore::open(options).await.unwrap();
            if mode.starts_with("result_") {
                let request = backtest::seed(&store).await;
                store
                    .mutate(&actor(), JobMutation::Complete(request))
                    .await
                    .unwrap();
                panic!("result fault point not reached");
            }
            if mode.starts_with("batch_") {
                let (_, request) = batch::seed(&store).await;
                store
                    .consume_grant(&actor(), request, research::metadata())
                    .await
                    .unwrap();
                panic!("batch fault point not reached");
            }
            if mode.starts_with("grant_") || mode.starts_with("close_") {
                let request = grant::seed(&store, 2, false).await;
                let issued = store.issue_grant(&actor(), request).await.unwrap();
                if mode.starts_with("close_") {
                    store
                        .close_grant(
                            &actor(),
                            grant::close(&issued, GrantClosure::Revoke, "close.retry"),
                        )
                        .await
                        .unwrap();
                }
                panic!("grant fault point not reached");
            }
            if mode.starts_with("approval_") {
                store
                    .register_period(&actor(), holdout::command(0, "period.0"))
                    .await
                    .unwrap();
                store
                    .record_approval(
                        &approval::human(1),
                        approval::command(0, "approval.retry", &approval::human(1)),
                    )
                    .await
                    .unwrap();
                panic!("approval fault point not reached");
            }
            if mode.starts_with("period_") {
                store
                    .register_period(&actor(), holdout::command(0, "period.retry"))
                    .await
                    .unwrap();
                panic!("period fault point not reached");
            }
            if mode.starts_with("role_") {
                store
                    .submit_role(
                        &actor(),
                        research::command("role.retry"),
                        research::metadata(),
                    )
                    .await
                    .unwrap();
                panic!("role fault point not reached");
            }
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
async fn killed_writer_preserves_atomicity() {
    for point in [
        "before_commit",
        "after_commit",
        "active_lease",
        "role_before_commit",
        "role_after_commit",
        "period_before_commit",
        "period_after_commit",
        "approval_before_commit",
        "approval_after_commit",
        "grant_before_commit",
        "grant_after_commit",
        "close_before_commit",
        "close_after_commit",
        "batch_mid_insert",
        "batch_before_commit",
        "batch_after_commit",
        "result_after_insert",
        "result_before_commit",
        "result_after_commit",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state");
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
        let mut options = options(&path, clock.clone());
        if point.starts_with("role_") {
            options.admission = Arc::new(research::Admission);
        }
        if point.starts_with("result_") {
            options.admission = Arc::new(backtest::Admission);
            options.backtest_policy = Arc::new(backtest::Policy::default());
        }
        if point.starts_with("period_") {
            options.holdout_policy = Arc::new(holdout::Policy);
        }
        if point.starts_with("approval_") {
            options.holdout_policy = Arc::new(approval::Policy::default());
        }
        if point.starts_with("grant_") || point.starts_with("close_") {
            options.holdout_policy = Arc::new(grant::Policy::default());
        }
        if point.starts_with("batch_") {
            options.holdout_policy = Arc::new(batch::Policy::default());
            options.admission = Arc::new(batch::Admission);
        }
        let store = PgJobStore::open(options).await.unwrap();
        store.verify_configuration().await.unwrap();
        if point.starts_with("result_") {
            let committed = point == "result_after_commit";
            assert_eq!(
                store.audit_events(0, 500).await.unwrap().len(),
                2 + usize::from(committed)
            );
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM backtest_results")
                .fetch_one(&mut connection(&directory).await)
                .await
                .unwrap();
            assert_eq!(count, i64::from(committed));
            let request = backtest::seed(&store).await;
            let result = store
                .mutate(&actor(), JobMutation::Complete(request))
                .await
                .unwrap();
            assert_eq!(result.replayed, committed);
            assert_eq!(
                store
                    .current_backtest(&actor(), "job.1", "context.fixture")
                    .await
                    .unwrap(),
                backtest::result()
            );
            let events = store.audit_events(0, 500).await.unwrap();
            assert_eq!(events.len(), 3);
            verify_audit_chain(&events).unwrap();
            store.close().await;
            continue;
        }
        if point.starts_with("batch_") {
            let committed = point == "batch_after_commit";
            assert_eq!(
                store.audit_events(0, 500).await.unwrap().len(),
                if committed { 7 } else { 4 }
            );
            let (_, request) = batch::seed(&store).await;
            let result = store
                .consume_grant(&actor(), request, research::metadata())
                .await
                .unwrap();
            assert_eq!(result.replayed, committed);
            let jobs = result.response.job_batch.unwrap().job_ids;
            assert_eq!(jobs.len(), 2);
            for id in jobs {
                assert!(store.get(&id.value).await.unwrap().is_some());
            }
            let events = store.audit_events(0, 500).await.unwrap();
            assert_eq!(events.len(), 7);
            verify_audit_chain(&events).unwrap();
            store.close().await;
            continue;
        }
        if point.starts_with("grant_") || point.starts_with("close_") {
            let committed = point.ends_with("after_commit");
            let closing = point.starts_with("close_");
            let baseline = if closing { 4 } else { 3 };
            assert_eq!(
                store.audit_events(0, 500).await.unwrap().len(),
                baseline + usize::from(committed)
            );
            let request = grant::seed(&store, 2, false).await;
            let issued = store.issue_grant(&actor(), request).await.unwrap();
            if closing {
                assert!(issued.replayed);
                let closed = store
                    .close_grant(
                        &actor(),
                        grant::close(&issued, GrantClosure::Revoke, "close.retry"),
                    )
                    .await
                    .unwrap();
                assert_eq!(closed.replayed, committed);
            } else {
                assert_eq!(issued.replayed, committed);
            }
            let events = store.audit_events(0, 500).await.unwrap();
            assert_eq!(events.len(), baseline + 1);
            verify_audit_chain(&events).unwrap();
            store.close().await;
            continue;
        }
        if point.starts_with("approval_") {
            let committed = point == "approval_after_commit";
            assert_eq!(
                store.audit_events(0, 500).await.unwrap().len(),
                1 + usize::from(committed)
            );
            let replay = store
                .record_approval(
                    &approval::human(1),
                    approval::command(0, "approval.retry", &approval::human(1)),
                )
                .await
                .unwrap();
            assert_eq!(replay.replayed, committed);
            let events = store.audit_events(0, 500).await.unwrap();
            assert_eq!(events.len(), 2);
            verify_audit_chain(&events).unwrap();
            store.close().await;
            continue;
        }
        if point.starts_with("period_") {
            let committed = point == "period_after_commit";
            assert_eq!(
                store.audit_events(0, 500).await.unwrap().len(),
                usize::from(committed)
            );
            let replay = store
                .register_period(&actor(), holdout::command(0, "period.retry"))
                .await
                .unwrap();
            assert_eq!(replay.replayed, committed);
            let events = store.audit_events(0, 500).await.unwrap();
            assert_eq!(events.len(), 1);
            verify_audit_chain(&events).unwrap();
            store.close().await;
            continue;
        }
        if point.starts_with("role_") {
            let events = store.audit_events(0, 500).await.unwrap();
            assert_eq!(events.len(), usize::from(point == "role_after_commit"));
            let replay = store
                .submit_role(
                    &actor(),
                    research::command("role.retry"),
                    research::metadata(),
                )
                .await
                .unwrap();
            assert_eq!(replay.replayed, point == "role_after_commit");
            let events = store.audit_events(0, 500).await.unwrap();
            assert_eq!(events.len(), 1);
            verify_audit_chain(&events).unwrap();
            store.close().await;
            continue;
        }
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
