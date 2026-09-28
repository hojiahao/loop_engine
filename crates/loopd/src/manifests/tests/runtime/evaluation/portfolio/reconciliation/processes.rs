//! Independent processes reconstruct actual evidence before racing the commit.

use super::super::processes::{Pinned, Worker, finish, wait_file};
use super::*;
use crate::runtime::{ReconciliationExecutor, ValidationTask};
use crate::store::{JobMutation, StoreResult};
use std::process::{Command, Stdio};

// Each writer replays the primary (180s) before independent reconstruction
// (180s). Include 60s for its bounded fixture/database setup; the production
// mTLS operation still has its separate outer 180s deadline.
const PREPARE_SECONDS: u64 = 420;

async fn prepare(case: &ValidationCase) {
    case.primary().await;
    let request = case.request().await;
    let root = case.portfolio.base.fixture.directory.path();
    let record = case
        .portfolio
        .base
        .store
        .get("job.validation")
        .await
        .unwrap()
        .unwrap();
    let started = crate::store::timestamp_millis(
        record
            .active_lease
            .as_ref()
            .unwrap()
            .issued_at
            .as_ref()
            .unwrap(),
        false,
    )
    .unwrap();
    std::fs::write(root.join("validation-start"), started.to_string()).unwrap();
    for (index, job) in case.portfolio.jobs.iter().enumerate() {
        std::fs::write(
            root.join(format!("validation-job-{index}.pb")),
            job.encode_to_vec(),
        )
        .unwrap();
    }
    std::fs::write(root.join("validation-request.pb"), request.encode_to_vec()).unwrap();
    std::fs::write(
        root.join("validation-config.json"),
        serde_json::to_vec(case.portfolio.validation.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("validation-primary.json"),
        serde_json::to_vec(&case.portfolio.pin).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("validation-data.json"),
        serde_json::to_vec(&case.portfolio.base.fixture.context.data).unwrap(),
    )
    .unwrap();
    // The race tests one coherent transaction time; live expiry has a separate
    // real-clock mTLS case. Sequential process startup cannot extend a lease.
    std::fs::write(
        root.join("validation-clock"),
        SystemClock.now_millis().unwrap().to_string(),
    )
    .unwrap();
}

fn spawn(case: &ValidationCase, index: usize, point: Option<&str>) -> Worker {
    let root = case.portfolio.base.fixture.directory.path();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact","manifests::tests::runtime::evaluation::portfolio::reconciliation::processes::validation_writer","--ignored","--nocapture"])
        .env("LOOP_VALIDATION_TEST_ROOT",root).env("LOOP_VALIDATION_TEST_INDEX",index.to_string())
        .env("TMPDIR",root).stdout(Stdio::null()).stderr(Stdio::inherit());
    if let Some(point) = point {
        command
            .env("LOOP_TEST_FAULT_POINT", point)
            .env("LOOP_TEST_FAULT_READY", root.join("fault"));
    } else {
        command
            .env_remove("LOOP_TEST_FAULT_POINT")
            .env_remove("LOOP_TEST_FAULT_READY");
    }
    Worker(command.spawn().unwrap())
}

#[test]
#[ignore = "launched by independent process and kill/restart tests"]
fn validation_writer() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(write())
        .unwrap();
}

async fn write() -> StoreResult<()> {
    let root = PathBuf::from(std::env::var_os("LOOP_VALIDATION_TEST_ROOT").unwrap());
    let index = std::env::var("LOOP_VALIDATION_TEST_INDEX").unwrap();
    let jobs: Vec<JobSpecification> = (0..3)
        .map(|index| {
            JobSpecification::decode(
                std::fs::read(root.join(format!("validation-job-{index}.pb")))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap()
        })
        .collect();
    let request = ExecuteReconciliationRequest::decode(
        std::fs::read(root.join("validation-request.pb"))?.as_slice(),
    )
    .unwrap();
    let config: ReconciliationConfig =
        serde_json::from_slice(&std::fs::read(root.join("validation-config.json"))?).unwrap();
    let pin: PortfolioPin =
        serde_json::from_slice(&std::fs::read(root.join("validation-primary.json"))?).unwrap();
    let manifest: ObjectRef =
        serde_json::from_slice(&std::fs::read(root.join("validation-data.json"))?).unwrap();
    let broker = Arc::new(ArtifactBroker::open(
        &root.join("objects"),
        &root.join("protected"),
        &root.join("views"),
        vec![DataPin {
            job_id: pin.job_id.clone(),
            manifest,
            protected: false,
        }],
    )?);
    let executor = Arc::new(PortfolioExecutor::open(
        &python(),
        &root.join("objects"),
        &root.join("portfolio-output"),
        vec![pin],
        broker,
    )?);
    let now = std::fs::read_to_string(root.join("validation-clock"))?
        .parse::<i64>()
        .unwrap();
    let mut options = support::base_options(&root.join("state"));
    options.clock = Arc::new(support::FixtureClock(AtomicI64::new(now)));
    options.admission = Arc::new(Pinned(jobs.clone()));
    let store = PgJobStore::open(options).await?;
    let record = store.get("job.portfolio").await?.unwrap();
    let Some(job_outcome::Outcome::Success(success)) =
        record.outcome.as_ref().unwrap().outcome.as_ref()
    else {
        panic!("registered primary required")
    };
    let inputs = executor.inputs(&jobs[1]).await?;
    let primary = Arc::new(executor.replay(inputs, &jobs[1], success).await?);
    let validator = ReconciliationExecutor::open(config, executor)?;
    let evidence = Arc::new(
        validator
            .execute(
                ValidationTask {
                    statistics: None,
                    job: &jobs[2],
                    lease: &request.lease_id.as_ref().unwrap().value,
                    record,
                    primary: primary.clone(),
                    prior: None,
                    started_ms: std::fs::read_to_string(root.join("validation-start"))?
                        .parse::<i64>()
                        .unwrap(),
                },
                Duration::from_secs(180),
            )
            .await?,
    );
    let command = CompleteJobRequest {
        context: request.context,
        job_id: request.job_id,
        lease_id: request.lease_id,
        expected_revision: request.expected_revision,
        outcome: Some(JobOutcome {
            outcome: Some(job_outcome::Outcome::Success(evidence.success()?)),
        }),
    };
    std::fs::write(root.join(format!("ready-{index}")), b"ready")?;
    // Up to eight bounded reconstructions start sequentially on small hosts.
    // Only their database commits race. The first writer must be able to wait
    // for the remaining seven without extending any production lease/deadline.
    wait_file(&root.join("release"), 8 * PREPARE_SECONDS).await;
    let store = store
        .with_backtest_policy(primary)
        .with_validation_evidence(evidence);
    store
        .mutate(&actor(), JobMutation::Complete(command))
        .await?;
    store.close().await;
    Ok(())
}

async fn assert_committed(case: &ValidationCase, before: usize) {
    let job = case
        .portfolio
        .base
        .store
        .get("job.validation")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.state, JobState::Succeeded as i32);
    assert_eq!(job.revision, 3);
    let events = case
        .portfolio
        .base
        .store
        .audit_events(0, 100)
        .await
        .unwrap();
    loop_core::audit::verify_audit_chain(&events).unwrap();
    assert_eq!(events.len(), before + 1);
}

async fn validation_writers(writers: usize) {
    let case = ValidationCase::new(false).await;
    prepare(&case).await;
    let root = case.portfolio.base.fixture.directory.path();
    let before = case
        .portfolio
        .base
        .store
        .audit_events(0, 100)
        .await
        .unwrap()
        .len();
    let mut children = Vec::new();
    for index in 0..writers {
        children.push(spawn(&case, index, None));
        wait_file(&root.join(format!("ready-{index}")), PREPARE_SECONDS).await;
    }
    std::fs::write(root.join("release"), b"ready").unwrap();
    for child in &mut children {
        finish(child, 120).await;
    }
    assert_committed(&case, before).await;
}

#[tokio::test]
async fn two_validation_writers() {
    validation_writers(2).await;
}

#[tokio::test]
async fn four_validation_writers() {
    validation_writers(4).await;
}

#[tokio::test]
async fn eight_validation_writers() {
    validation_writers(8).await;
}

#[tokio::test]
async fn killed_validation_atomic() {
    for point in ["validation_before_commit", "validation_after_commit"] {
        let case = ValidationCase::new(false).await;
        prepare(&case).await;
        let root = case.portfolio.base.fixture.directory.path();
        let before = case
            .portfolio
            .base
            .store
            .audit_events(0, 100)
            .await
            .unwrap()
            .len();
        std::fs::write(root.join("release"), b"ready").unwrap();
        let mut child = spawn(&case, 0, Some(point));
        wait_file(&root.join("fault"), PREPARE_SECONDS + 30).await;
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
        let state = case
            .portfolio
            .base
            .store
            .get("job.validation")
            .await
            .unwrap()
            .unwrap()
            .state;
        assert_eq!(
            state,
            if point == "validation_after_commit" {
                JobState::Succeeded
            } else {
                JobState::Leased
            } as i32
        );
        let mut retry = spawn(&case, 1, None);
        // Restart repeats both bounded reconstructions before its transaction.
        finish(&mut retry, PREPARE_SECONDS + 30).await;
        assert_committed(&case, before).await;
    }
}
