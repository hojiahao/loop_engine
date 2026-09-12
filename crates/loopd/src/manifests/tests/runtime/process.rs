//! Exercise the actual file publisher and transactional receipt in OS processes.
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, atomic::AtomicI64};
use std::time::{Duration, Instant};

use loop_protocol::wire::v1::JobSpecification;
use prost::Message;

use super::{ArtifactBroker, DataPin, Fixture, JobRepository, NOW, Role, Running, actor, support};

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn worker(running: &Running, key: &str, index: usize, fault: Option<&str>) -> Worker {
    let root = running.fixture.directory.path();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "manifests::tests::runtime::process::data_worker",
            "--ignored",
            "--nocapture",
        ])
        .env("LOOP_RUNTIME_TEST_ROOT", root)
        .env("LOOP_RUNTIME_TEST_KEY", key)
        .env("LOOP_RUNTIME_TEST_INDEX", index.to_string())
        .env("TMPDIR", root)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if let Some(point) = fault {
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

async fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !path.exists() {
        assert!(Instant::now() < deadline, "data process marker deadline");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn finish(worker: &mut Worker) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = worker.0.try_wait().unwrap() {
            assert!(status.success(), "data publisher failed: {status}");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "data process completion deadline"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[test]
#[ignore = "invoked by data publication process and kill/restart tests"]
fn data_worker() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let root = PathBuf::from(std::env::var_os("LOOP_RUNTIME_TEST_ROOT").unwrap());
            let mut fixture = Fixture::new();
            fixture.root = root.join("objects");
            fixture.job.specification =
                JobSpecification::decode(std::fs::read(root.join("job.pb")).unwrap().as_slice())
                    .unwrap();
            let store = fixture
                .open_at(
                    &root.join("state"),
                    Arc::new(support::FixtureClock(AtomicI64::new(NOW))),
                    fixture.policy().await,
                )
                .await;
            let job = store.get("job.1").await.unwrap().unwrap();
            let mut request = loop_protocol::wire::jobs::v1::PrepareJobArtifactsRequest {
                context: Some(support::context(
                    &std::env::var("LOOP_RUNTIME_TEST_KEY").unwrap(),
                )),
                job_id: fixture.job.specification.job_id.clone(),
                lease_id: job.active_lease.as_ref().unwrap().lease_id.clone(),
                expected_revision: job.revision,
            };
            request
                .context
                .as_mut()
                .unwrap()
                .request_id
                .as_mut()
                .unwrap()
                .value = format!(
                "data-process-{}",
                std::env::var("LOOP_RUNTIME_TEST_INDEX").unwrap()
            );
            let broker = ArtifactBroker::open(
                &fixture.root,
                &root.join("protected"),
                &root.join("views"),
                vec![DataPin {
                    job_id: "job.1".to_owned(),
                    manifest: fixture.context.data.clone(),
                    protected: false,
                }],
            )
            .unwrap();
            std::fs::write(
                root.join(format!(
                    "ready-{}",
                    std::env::var("LOOP_RUNTIME_TEST_INDEX").unwrap()
                )),
                b"ready",
            )
            .unwrap();
            wait_file(&root.join("release")).await;
            let response = broker
                .prepare(&store, &actor(), &request, &job)
                .await
                .unwrap();
            assert_eq!(
                std::fs::read_dir(root.join("views").join(response.view_id))
                    .unwrap()
                    .count(),
                response.artifacts.len()
            );
            store.close().await;
        });
}

#[tokio::test]
async fn independent_publishers_preserve_view_identity() {
    for count in [2, 4, 8] {
        for shared in [true, false] {
            let running = Running::start(Role::Research).await;
            running.acquire().await;
            let root = running.fixture.directory.path();
            std::fs::write(
                root.join("job.pb"),
                running.fixture.job.specification.encode_to_vec(),
            )
            .unwrap();
            let mut workers = (0..count)
                .map(|index| {
                    worker(
                        &running,
                        &if shared {
                            "data-shared".to_owned()
                        } else {
                            format!("data-{index}")
                        },
                        index,
                        None,
                    )
                })
                .collect::<Vec<_>>();
            for index in 0..count {
                wait_file(&root.join(format!("ready-{index}"))).await;
            }
            std::fs::write(root.join("release"), b"ready").unwrap();
            for child in &mut workers {
                finish(child).await;
            }
            let events = running.store.audit_events(0, 100).await.unwrap();
            loop_core::audit::verify_audit_chain(&events).unwrap();
            assert_eq!(events.len(), 2 + if shared { 1 } else { count });
            assert_eq!(std::fs::read_dir(&running.views).unwrap().count(), 1);
        }
    }
}

#[tokio::test]
async fn killed_publisher_replays_without_duplicate_acceptance() {
    for point in ["data_before_commit", "data_after_commit"] {
        let running = Running::start(Role::Research).await;
        let job = running.acquire().await;
        let root = running.fixture.directory.path();
        std::fs::write(
            root.join("job.pb"),
            running.fixture.job.specification.encode_to_vec(),
        )
        .unwrap();
        std::fs::write(root.join("release"), b"ready").unwrap();
        let mut child = worker(&running, "tls-data", 0, Some(point));
        wait_file(&root.join("fault")).await;
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
        assert_eq!(
            running.store.audit_events(0, 20).await.unwrap().len(),
            if point == "data_after_commit" { 3 } else { 2 }
        );
        let first = running
            .client()
            .await
            .prepare_job_artifacts(running.data_request(&job))
            .await
            .unwrap()
            .into_inner();
        let repeated = running
            .client()
            .await
            .prepare_job_artifacts(running.data_request(&job))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(first, repeated);
        assert_eq!(running.store.audit_events(0, 20).await.unwrap().len(), 3);
    }
}
