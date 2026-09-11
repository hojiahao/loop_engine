//! Separate processes resolve the same real objects and write one test ledger.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, atomic::AtomicI64};
use std::time::{Duration, Instant};

use crate::store::{BacktestRepository, JobRepository};

use super::{Fixture, NOW, actor, seed, support};

struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn worker(fixture: &Fixture, key: &str, index: usize, fault: Option<&str>) -> Worker {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "manifests::tests::process::manifest_worker",
            "--ignored",
            "--nocapture",
        ])
        .env("LOOP_MANIFEST_ROOT", &fixture.root)
        .env(
            "LOOP_MANIFEST_STATE",
            fixture.directory.path().join("state"),
        )
        .env("LOOP_MANIFEST_KEY", key)
        .env("LOOP_MANIFEST_INDEX", index.to_string())
        .env("TMPDIR", fixture.directory.path())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if let Some(point) = fault {
        command.env("LOOP_TEST_FAULT_POINT", point).env(
            "LOOP_TEST_FAULT_READY",
            fixture.directory.path().join("fault"),
        );
    } else {
        command.env_remove("LOOP_TEST_FAULT_POINT");
        command.env_remove("LOOP_TEST_FAULT_READY");
    }
    Worker(command.spawn().unwrap())
}

async fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !path.exists() {
        assert!(Instant::now() < deadline, "process marker deadline");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[test]
#[ignore = "invoked by manifest process and kill/restart tests"]
fn manifest_worker() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut fixture = Fixture::new();
            fixture.root = PathBuf::from(std::env::var_os("LOOP_MANIFEST_ROOT").unwrap());
            let state = PathBuf::from(std::env::var_os("LOOP_MANIFEST_STATE").unwrap());
            let parent = state.parent().unwrap();
            let index = std::env::var("LOOP_MANIFEST_INDEX").unwrap();
            let policy = fixture.policy().await;
            let store = fixture
                .open_at(
                    &state,
                    Arc::new(support::FixtureClock(AtomicI64::new(NOW))),
                    policy.clone(),
                )
                .await;
            std::fs::write(parent.join(format!("ready-{index}")), b"ready").unwrap();
            wait_file(&parent.join("release")).await;
            let mut command =
                support::backtest::export(&std::env::var("LOOP_MANIFEST_KEY").unwrap());
            command.context_id = fixture.context_id();
            let mut output = Vec::new();
            policy
                .write_current(&store, &actor(), command, &mut output)
                .await
                .unwrap();
            let document: serde_json::Value = serde_json::from_slice(&output).unwrap();
            assert_eq!(document["result_manifest"]["metrics"][0]["value"], "1.25");
            store.close().await;
        });
}

#[tokio::test]
async fn independent_writers_preserve_export_identity() {
    for count in [2, 4, 8] {
        for shared in [true, false] {
            let fixture = Fixture::new();
            let policy = fixture.policy().await;
            let store = fixture
                .open(Arc::new(support::FixtureClock(AtomicI64::new(NOW))), policy)
                .await;
            seed(&fixture, &store).await;
            let before = store.get("job.1").await.unwrap().unwrap();
            let mut workers = (0..count)
                .map(|index| {
                    let key = if shared {
                        "manifest.shared".to_owned()
                    } else {
                        format!("manifest.writer.{index}")
                    };
                    worker(&fixture, &key, index, None)
                })
                .collect::<Vec<_>>();
            for index in 0..count {
                wait_file(&fixture.directory.path().join(format!("ready-{index}"))).await;
            }
            std::fs::write(fixture.directory.path().join("release"), b"ready").unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            for worker in &mut workers {
                loop {
                    if let Some(status) = worker.0.try_wait().unwrap() {
                        assert!(status.success(), "export worker: {status}");
                        break;
                    }
                    assert!(Instant::now() < deadline, "export writer deadline");
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
            let events = store.audit_events(0, 100).await.unwrap();
            loop_core::audit::verify_audit_chain(&events).unwrap();
            assert_eq!(events.len(), 3 + if shared { 1 } else { count });
            assert_eq!(store.get("job.1").await.unwrap().unwrap(), before);
            store.close().await;
        }
    }
}

#[tokio::test]
async fn killed_exporter_recovers_from_actual_files() {
    for point in [
        "export_after_receipt",
        "export_before_commit",
        "export_after_commit",
    ] {
        let fixture = Fixture::new();
        let clock = Arc::new(support::FixtureClock(AtomicI64::new(NOW)));
        let store = fixture.open(clock.clone(), fixture.policy().await).await;
        seed(&fixture, &store).await;
        let before = store.get("job.1").await.unwrap().unwrap();
        store.close().await;
        std::fs::write(fixture.directory.path().join("release"), b"ready").unwrap();
        let mut child = worker(&fixture, "manifest.kill-retry", 0, Some(point));
        wait_file(&fixture.directory.path().join("fault")).await;
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
        let policy = fixture.policy().await;
        let store = fixture.open(clock, policy.clone()).await;
        assert_eq!(
            store.audit_events(0, 100).await.unwrap().len(),
            if point == "export_after_commit" { 4 } else { 3 }
        );
        let mut command = support::backtest::export("manifest.kill-retry");
        command.context_id = fixture.context_id();
        let accepted = policy
            .write_current(&store, &actor(), command.clone(), &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(accepted.replayed, point == "export_after_commit");
        assert!(
            store
                .export_current(&actor(), command)
                .await
                .unwrap()
                .replayed
        );
        let events = store.audit_events(0, 100).await.unwrap();
        loop_core::audit::verify_audit_chain(&events).unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(store.get("job.1").await.unwrap().unwrap(), before);
        store.close().await;
    }
}
