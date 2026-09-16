//! Independent PostgreSQL writers reuse genuinely computed immutable output.
use super::*;
use std::process::{Child, Command, Stdio};
use std::time::Instant;

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn wait_file(path: &Path, seconds: u64) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "evaluation process marker deadline"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn finish(worker: &mut Worker) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = worker.0.try_wait().unwrap() {
            assert!(status.success(), "evaluation writer failed: {status}");
            return;
        }
        assert!(Instant::now() < deadline, "evaluation writer deadline");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn prepare(case: &Case) {
    let request = case.request_with_lease(300).await;
    let specification = &case.fixture.job.specification;
    let lease = request.lease_id.as_ref().unwrap();
    let resolver = EvaluationResolver::open(
        &case.fixture.root,
        vec![EvaluationPin {
            job_id: specification.job_id.as_ref().unwrap().value.clone(),
            context: case.fixture.json(&case.fixture.context),
        }],
    )
    .unwrap();
    let inputs = resolver.prepare(specification, lease).await.unwrap();
    let job = case
        .store
        .get(&specification.job_id.as_ref().unwrap().value)
        .await
        .unwrap()
        .unwrap();
    let prepared = case
        .broker
        .prepare(
            &case.store,
            &actor(),
            &PrepareJobArtifactsRequest {
                context: request.context.clone(),
                job_id: request.job_id.clone(),
                lease_id: request.lease_id.clone(),
                expected_revision: request.expected_revision,
            },
            &job,
        )
        .await
        .unwrap();
    let view = case.broker.evaluation_view(&prepared).await.unwrap();
    let result = case
        .executor
        .run(&inputs.work, Some(&view), Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    let command = CompleteJobRequest {
        context: request.context,
        job_id: request.job_id,
        lease_id: request.lease_id,
        expected_revision: request.expected_revision,
        outcome: Some(JobOutcome {
            outcome: Some(job_outcome::Outcome::Success(JobSuccess {
                outputs: vec![result.values.unwrap(), result.manifest.unwrap()],
            })),
        }),
    };
    let root = case.fixture.directory.path();
    std::fs::write(
        root.join("evaluation-job.pb"),
        specification.encode_to_vec(),
    )
    .unwrap();
    std::fs::write(root.join("evaluation-command.pb"), command.encode_to_vec()).unwrap();
    std::fs::write(
        root.join("evaluation-pin.json"),
        serde_json::to_vec(&case.fixture.json(&case.fixture.context)).unwrap(),
    )
    .unwrap();
}

fn spawn(case: &Case, index: usize, point: Option<&str>) -> Worker {
    let root = case.fixture.directory.path();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "manifests::tests::runtime::evaluation::workflow::processes::evaluation_writer",
            "--ignored",
            "--nocapture",
        ])
        .env("LOOP_EVALUATION_TEST_ROOT", root)
        .env("LOOP_EVALUATION_TEST_INDEX", index.to_string())
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
#[ignore = "invoked by independent evaluation writer and kill/restart tests"]
fn evaluation_writer() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let root = PathBuf::from(std::env::var_os("LOOP_EVALUATION_TEST_ROOT").unwrap());
            let index = std::env::var("LOOP_EVALUATION_TEST_INDEX").unwrap();
            let job = JobSpecification::decode(
                std::fs::read(root.join("evaluation-job.pb"))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
            let command = CompleteJobRequest::decode(
                std::fs::read(root.join("evaluation-command.pb"))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
            let pin: ObjectRef =
                serde_json::from_slice(&std::fs::read(root.join("evaluation-pin.json")).unwrap())
                    .unwrap();
            let mut options = support::base_options(&root.join("state"));
            options.admission = Arc::new(Pinned(vec![job.clone()]));
            let store = PgJobStore::open(options).await.unwrap();
            let resolver = EvaluationResolver::open(
                &root.join("objects"),
                vec![EvaluationPin {
                    job_id: job.job_id.as_ref().unwrap().value.clone(),
                    context: pin,
                }],
            )
            .unwrap();
            let inputs = resolver
                .prepare(&job, command.lease_id.as_ref().unwrap())
                .await
                .unwrap();
            let Some(job_outcome::Outcome::Success(success)) =
                command.outcome.as_ref().unwrap().outcome.as_ref()
            else {
                unreachable!()
            };
            let proof = inputs
                .resolve(
                    &crate::manifests::LocalArtifacts::open(&root.join("output")).unwrap(),
                    success,
                )
                .await
                .unwrap();
            std::fs::write(root.join(format!("ready-{index}")), b"ready").unwrap();
            wait_file(&root.join("release"), 300).await;
            store
                .with_evaluation_evidence(proof)
                .mutate(&actor(), crate::store::JobMutation::Complete(command))
                .await
                .unwrap();
            store.close().await;
        });
}

#[tokio::test]
async fn independent_writers_commit_one_evaluation() {
    for count in [2, 4, 8] {
        let case = Case::start().await;
        prepare(&case).await;
        let mut writers = Vec::with_capacity(count);
        // Verify each independent process's real build before releasing the
        // common write barrier. The concurrency gate concerns transactional
        // commits; it does not require simultaneous cold native-file hashing.
        for index in 0..count {
            writers.push(spawn(&case, index, None));
            wait_file(
                &case.fixture.directory.path().join(format!("ready-{index}")),
                90,
            )
            .await;
        }
        std::fs::write(case.fixture.directory.path().join("release"), b"ready").unwrap();
        for writer in &mut writers {
            finish(writer).await;
        }
        let page = trials(&case).await;
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].state, JobState::Succeeded as i32);
        assert!(page[0].evaluation.is_some());
        let events = case.store.audit_events(0, 20).await.unwrap();
        loop_core::audit::verify_audit_chain(&events).unwrap();
        assert_eq!(events.len(), 4);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM factor_evaluations")
            .fetch_one(&mut support::connection(&case.fixture.directory).await)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
}

#[tokio::test]
async fn killed_completion_preserves_atomic_evidence() {
    for point in ["evaluation_after_insert", "evaluation_after_commit"] {
        let case = Case::start().await;
        prepare(&case).await;
        std::fs::write(case.fixture.directory.path().join("release"), b"ready").unwrap();
        let mut writer = spawn(&case, 0, Some(point));
        wait_file(&case.fixture.directory.path().join("fault"), 90).await;
        writer.0.kill().unwrap();
        assert!(!writer.0.wait().unwrap().success());
        let page = trials(&case).await;
        assert_eq!(
            page[0].evaluation.is_some(),
            point == "evaluation_after_commit"
        );
        let mut retry = spawn(&case, 1, None);
        finish(&mut retry).await;
        assert!(trials(&case).await[0].evaluation.is_some());
        assert_eq!(case.store.audit_events(0, 20).await.unwrap().len(), 4);
    }
}
