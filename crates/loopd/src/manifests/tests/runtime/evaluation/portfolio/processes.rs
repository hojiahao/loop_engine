//! Independent database writers consume real installed-producer evidence.

use super::*;
use crate::store::{AdmissionPolicy, BacktestPolicy, JobMutation, StoreError, StoreResult};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Pinned(Vec<JobSpecification>);
impl AdmissionPolicy for Pinned {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        if self.0.contains(job) {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
    fn authorize_job_command(
        &self,
        _: &str,
        principal: &Actor,
        job: &JobRecord,
    ) -> StoreResult<()> {
        if principal != &actor() {
            return Err(StoreError::AdmissionDenied);
        }
        self.validate_submission(
            job.specification
                .as_ref()
                .ok_or(StoreError::AdmissionDenied)?,
        )
    }
}

async fn wait_file(path: &Path, seconds: u64) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "portfolio writer marker deadline"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn finish(worker: &mut Worker) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = worker.0.try_wait().unwrap() {
            assert!(status.success(), "portfolio writer failed: {status}");
            return;
        }
        assert!(Instant::now() < deadline, "portfolio writer deadline");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn prepare(case: &PortfolioCase) {
    let request = case.request_seconds(300).await;
    let record = case.record().await;
    let prepared = case
        .base
        .broker
        .prepare(
            &case.base.store,
            &actor(),
            &PrepareJobArtifactsRequest {
                context: request.context.clone(),
                job_id: request.job_id.clone(),
                lease_id: request.lease_id.clone(),
                expected_revision: request.expected_revision,
            },
            &record,
        )
        .await
        .unwrap();
    let view = case.base.broker.evaluation_view(&prepared).await.unwrap();
    let executor = PortfolioExecutor::open(
        &python(),
        &case.base.fixture.root,
        &case.output,
        vec![case.pin.clone()],
        case.base.broker.clone(),
    )
    .unwrap();
    let inputs = executor.inputs(&case.job).await.unwrap();
    let trials = case.base.store.trial_ledger(&actor()).await.unwrap();
    let work = inputs.work(&request.lease_id.as_ref().unwrap().value, trials, None);
    let proof = executor
        .execute(inputs, &work, &view, Duration::from_secs(180))
        .await
        .unwrap();
    let command = CompleteJobRequest {
        context: request.context,
        job_id: request.job_id,
        lease_id: request.lease_id,
        expected_revision: request.expected_revision,
        outcome: Some(JobOutcome {
            outcome: Some(job_outcome::Outcome::Success(proof.success.unwrap())),
        }),
    };
    let root = case.base.fixture.directory.path();
    for (index, job) in case.jobs.iter().enumerate() {
        std::fs::write(
            root.join(format!("portfolio-job-{index}.pb")),
            job.encode_to_vec(),
        )
        .unwrap();
    }
    std::fs::write(root.join("portfolio-command.pb"), command.encode_to_vec()).unwrap();
    std::fs::write(
        root.join("portfolio-pin.json"),
        serde_json::to_vec(&case.pin).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("portfolio-data.json"),
        serde_json::to_vec(&case.base.fixture.context.data).unwrap(),
    )
    .unwrap();
    // Commit races use one coherent logical clock after real computation. Slow
    // sequential child startup must not accidentally become a lease-expiry
    // scenario; expiry is separately tested over the live mTLS runtime.
    std::fs::write(
        root.join("portfolio-clock.json"),
        SystemClock.now_millis().unwrap().to_string(),
    )
    .unwrap();
}

fn spawn(case: &PortfolioCase, index: usize, point: Option<&str>) -> Worker {
    let root = case.base.fixture.directory.path();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "manifests::tests::runtime::evaluation::portfolio::processes::portfolio_writer",
            "--ignored",
            "--nocapture",
        ])
        .env("LOOP_PORTFOLIO_TEST_ROOT", root)
        .env("LOOP_PORTFOLIO_TEST_INDEX", index.to_string())
        .env("TMPDIR", root)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
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
#[ignore = "launched by portfolio process/crash acceptance"]
fn portfolio_writer() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let root = PathBuf::from(std::env::var_os("LOOP_PORTFOLIO_TEST_ROOT").unwrap());
            let index = std::env::var("LOOP_PORTFOLIO_TEST_INDEX").unwrap();
            let jobs: Vec<_> = (0..2)
                .map(|index| {
                    JobSpecification::decode(
                        std::fs::read(root.join(format!("portfolio-job-{index}.pb")))
                            .unwrap()
                            .as_slice(),
                    )
                    .unwrap()
                })
                .collect();
            let command = CompleteJobRequest::decode(
                std::fs::read(root.join("portfolio-command.pb"))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
            let pin: PortfolioPin =
                serde_json::from_slice(&std::fs::read(root.join("portfolio-pin.json")).unwrap())
                    .unwrap();
            let manifest: ObjectRef =
                serde_json::from_slice(&std::fs::read(root.join("portfolio-data.json")).unwrap())
                    .unwrap();
            let broker = Arc::new(
                ArtifactBroker::open(
                    &root.join("objects"),
                    &root.join("protected"),
                    &root.join("views"),
                    vec![DataPin {
                        job_id: pin.job_id.clone(),
                        manifest,
                        protected: false,
                    }],
                )
                .unwrap(),
            );
            let executor = PortfolioExecutor::open(
                &python(),
                &root.join("objects"),
                &root.join("portfolio-output"),
                vec![pin],
                broker,
            )
            .unwrap();
            let Some(job_outcome::Outcome::Success(success)) = command
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.outcome.as_ref())
            else {
                unreachable!()
            };
            let proof = executor
                .prepare(&jobs[1], None, Some(success))
                .await
                .unwrap()
                .unwrap();
            let mut options = support::base_options(&root.join("state"));
            let now = std::fs::read_to_string(root.join("portfolio-clock.json"))
                .unwrap()
                .parse::<i64>()
                .unwrap();
            options.clock = Arc::new(support::FixtureClock(AtomicI64::new(now)));
            options.admission = Arc::new(Pinned(jobs));
            options.backtest_policy = proof;
            let store = PgJobStore::open(options).await.unwrap();
            std::fs::write(root.join(format!("ready-{index}")), b"ready").unwrap();
            wait_file(&root.join("release"), 300).await;
            store
                .mutate(&actor(), JobMutation::Complete(command))
                .await
                .unwrap();
            store.close().await;
        });
}

async fn count(case: &PortfolioCase) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM backtest_results")
        .fetch_one(&mut support::connection(&case.base.fixture.directory).await)
        .await
        .unwrap()
}

#[tokio::test]
async fn independent_portfolio_writers() {
    for writers in [2, 4, 8] {
        let case = PortfolioCase::new().await;
        prepare(&case).await;
        let before = case.base.store.audit_events(0, 100).await.unwrap().len();
        let mut children = Vec::new();
        // Numerical reconstruction is sequential; commits race across distinct
        // OS processes at the shared barrier, without cold-hash resource noise.
        for index in 0..writers {
            children.push(spawn(&case, index, None));
            wait_file(
                &case
                    .base
                    .fixture
                    .directory
                    .path()
                    .join(format!("ready-{index}")),
                120,
            )
            .await;
        }
        std::fs::write(case.base.fixture.directory.path().join("release"), b"ready").unwrap();
        for child in &mut children {
            finish(child).await;
        }
        assert_eq!(count(&case).await, 1);
        assert_eq!(case.record().await.revision, 3);
        let events = case.base.store.audit_events(0, 100).await.unwrap();
        loop_core::audit::verify_audit_chain(&events).unwrap();
        assert_eq!(events.len(), before + 1);
    }
}

#[tokio::test]
async fn killed_portfolio_atomic() {
    for point in [
        "result_after_insert",
        "result_before_commit",
        "result_after_commit",
    ] {
        let case = PortfolioCase::new().await;
        prepare(&case).await;
        let before = case.base.store.audit_events(0, 100).await.unwrap().len();
        std::fs::write(case.base.fixture.directory.path().join("release"), b"ready").unwrap();
        let mut child = spawn(&case, 0, Some(point));
        wait_file(&case.base.fixture.directory.path().join("fault"), 120).await;
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
        assert_eq!(
            count(&case).await,
            i64::from(point == "result_after_commit")
        );
        let mut retry = spawn(&case, 1, None);
        finish(&mut retry).await;
        assert_eq!(count(&case).await, 1);
        assert_eq!(
            case.base.store.audit_events(0, 100).await.unwrap().len(),
            before + 1
        );
    }
}
