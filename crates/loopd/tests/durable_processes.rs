mod support;

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, atomic::AtomicI64};
use std::time::{Duration, Instant};

use loop_core::audit::verify_audit_chain;
use loop_protocol::wire::jobs::v1::{AcquireJobLeaseRequest, CompleteJobRequest};
use loop_protocol::wire::v1::JobId;
use loopd::store::{
    BacktestRepository, CloseGrant, GrantClosure, HoldoutRepository, JobMutation, JobRepository,
    PerturbationRepository, PgJobStore, StoreError,
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
        if mode.starts_with("result") || mode.starts_with("export") || mode.starts_with("rejection") {
            options.admission = Arc::new(backtest::Admission);
            options.backtest_policy = Arc::new(backtest::Policy::default());
        }
        if mode.starts_with("perturbation") {
            options.admission = Arc::new(perturbation::Admission);
            options.backtest_policy = Arc::new(perturbation::Policy::default());
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
        if mode.starts_with("batch") {
            options.holdout_policy = Arc::new(batch::Policy::default());
            options.admission = Arc::new(batch::Admission);
        }
        let store = PgJobStore::open(options).await.unwrap();
        let directory = path.parent().unwrap();
        std::fs::write(directory.join(format!("ready.{index}")), b"ready").unwrap();
        wait_for(&directory.join("start")).await;
        let result = match mode.as_str() {
            "perturbation" | "perturbation-race" => {
                let key = if mode.ends_with("race") { format!("perturbation.{index}") } else { "perturbation.retry".to_owned() };
                store.advance_perturbation(&actor(), perturbation::command(1, 0, &key), &perturbation::worker()).await.map(|value| value.replayed)
            }
            "export" | "export-distinct" => {
                let key = if mode.ends_with("distinct") { format!("export.{index}") } else { "export.retry".to_owned() };
                store.export_current(&actor(), backtest::export(&key)).await.map(|value| value.replayed)
            }
            "rejection-filter" => store.submit(rejection::command(index + 2)).await.map(|value| value.replayed),
            "result" | "result-race" | "rejection" | "rejection-race" => {
                let bytes = std::fs::read(directory.join("input")).unwrap();
                let mut request = CompleteJobRequest::decode(bytes.as_slice()).unwrap();
                if mode.ends_with("race") {
                    request.context = Some(context(&format!("finish.{index}")));
                }
                store.mutate(&actor(), JobMutation::Complete(request)).await.map(|value| value.replayed)
            }
            "batch" | "batch-race" => {
                let bytes = std::fs::read(directory.join("input")).unwrap();
                let mut request = loop_protocol::wire::holdout::v1::ConsumeGrantAndEnqueueBacktestRequest::decode(bytes.as_slice()).unwrap();
                if mode.ends_with("race") { request.context = Some(context(&format!("batch.{index}"))); }
                store.consume_grant(&actor(), request, research::metadata()).await.map(|value| value.replayed)
            }
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
            Err(StoreError::PreviouslyRejected) if mode == "rejection-filter" => "blocked",
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
            "batch",
            "batch-race",
            "result",
            "result-race",
            "export",
            "export-distinct",
            "rejection",
            "rejection-race",
            "rejection-filter",
            "perturbation",
            "perturbation-race",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("state");
            if mode.starts_with("perturbation") {
                let config = perturbation::options(
                    &path,
                    Arc::new(FixtureClock(AtomicI64::new(NOW))),
                    Arc::new(perturbation::Policy::default()),
                );
                let store = PgJobStore::open(config).await.unwrap();
                perturbation::seed(&store, 1, 15, false).await;
                store.close().await;
            }
            if mode.starts_with("result")
                || mode.starts_with("export")
                || mode.starts_with("rejection")
            {
                let config = backtest::options(
                    &path,
                    Arc::new(FixtureClock(AtomicI64::new(NOW))),
                    Arc::new(backtest::Policy::default()),
                );
                let store = PgJobStore::open(config).await.unwrap();
                let request = if mode.starts_with("rejection") {
                    rejection::seed(&store).await
                } else {
                    backtest::seed(&store).await
                };
                if mode.starts_with("export") || mode == "rejection-filter" {
                    store
                        .mutate(&actor(), JobMutation::Complete(request))
                        .await
                        .unwrap();
                } else {
                    std::fs::write(directory.path().join("input"), request.encode_to_vec())
                        .unwrap();
                }
                store.close().await;
            }
            if mode.starts_with("batch") {
                let mut config = options(&path, Arc::new(FixtureClock(AtomicI64::new(NOW))));
                config.holdout_policy = Arc::new(batch::Policy::default());
                config.admission = Arc::new(batch::Admission);
                let store = PgJobStore::open(config).await.unwrap();
                let (_, request) = batch::seed(&store).await;
                std::fs::write(directory.path().join("input"), request.encode_to_vec()).unwrap();
                store.close().await;
            }
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
                    "committed" | "replayed" | "fenced" | "blocked"
                ));
                if mode == "rejection-filter" {
                    assert_eq!(result, "blocked");
                }
                committed += usize::from(result == "committed");
            }
            assert_eq!(
                committed,
                if mode == "rejection-filter" {
                    0
                } else if mode.ends_with("distinct") {
                    count
                } else {
                    1
                }
            );
            let mut config = options(&path, Arc::new(FixtureClock(AtomicI64::new(NOW))));
            if mode.starts_with("perturbation") {
                config.admission = Arc::new(perturbation::Admission);
                config.backtest_policy = Arc::new(perturbation::Policy::default());
            }
            if mode.starts_with("result")
                || mode.starts_with("export")
                || mode.starts_with("rejection")
            {
                config.admission = Arc::new(backtest::Admission);
                config.backtest_policy = Arc::new(backtest::Policy::default());
            }
            let store = PgJobStore::open(config).await.unwrap();
            let events = store.audit_events(0, 500).await.unwrap();
            let baseline = if mode.starts_with("batch") {
                4
            } else if mode.starts_with("export")
                || mode == "rejection-filter"
                || mode.starts_with("perturbation")
            {
                3
            } else if mode.starts_with("result") || mode.starts_with("rejection") {
                2
            } else if mode.starts_with("grant") {
                3
            } else if mode.starts_with("close") {
                4
            } else {
                usize::from(mode == "acquire" || mode.starts_with("approval"))
            };
            let events_per_commit = if mode.starts_with("batch") { 3 } else { 1 };
            assert_eq!(events.len(), baseline + committed * events_per_commit);
            verify_audit_chain(&events).unwrap();
            if mode.starts_with("perturbation") {
                let (revision, blob, receipts): (i64, Vec<u8>, i64) = sqlx::query_as(
                    "SELECT revision, state_blob, (SELECT count(*) FROM command_receipts WHERE operation = 'loop.perturbation.advance') FROM perturbation_states",
                ).fetch_one(&mut connection(&directory).await).await.unwrap();
                let state =
                    loop_protocol::wire::v1::PerturbationState::decode(blob.as_slice()).unwrap();
                assert_eq!(
                    (revision, receipts, state.history.len(), state.random_draws),
                    (1, 1, 1, 1)
                );
                assert_eq!(state.proposed_factor_ids.len(), 1);
            }
            if mode.starts_with("rejection") {
                let rejected: i64 = sqlx::query_scalar("SELECT count(*) FROM backtest_rejections")
                    .fetch_one(&mut connection(&directory).await)
                    .await
                    .unwrap();
                assert_eq!(rejected, 1);
                assert_eq!(
                    store.get("job.1").await.unwrap().unwrap().state,
                    loop_protocol::wire::v1::JobState::FactorRejected as i32
                );
                assert!(matches!(
                    store.submit(rejection::command(2)).await,
                    Err(StoreError::PreviouslyRejected)
                ));
            }
            if mode.starts_with("result") || mode.starts_with("export") {
                assert_eq!(
                    store
                        .current_backtest(&actor(), "job.1", "context.fixture")
                        .await
                        .unwrap(),
                    backtest::result()
                );
                let results: i64 = sqlx::query_scalar("SELECT count(*) FROM backtest_results")
                    .fetch_one(&mut connection(&directory).await)
                    .await
                    .unwrap();
                assert_eq!(results, 1);
            }
            if mode.starts_with("export") {
                let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM command_receipts WHERE operation = 'loop.backtests.export_current'")
                    .fetch_one(&mut connection(&directory).await).await.unwrap();
                assert_eq!(receipts, committed as i64);
                assert_eq!(store.get("job.1").await.unwrap().unwrap().revision, 3);
            }
            store.verify_configuration().await.unwrap();
            store.close().await;
        }
    }
}
