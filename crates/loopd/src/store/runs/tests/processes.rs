use super::*;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !path.exists() {
        assert!(Instant::now() < deadline, "run worker barrier timed out");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn join(worker: &mut Worker) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(status) = worker.0.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        assert!(Instant::now() < deadline, "run worker timed out");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn spawn(root: &Path, index: usize, mode: &str, fault: Option<&str>) -> Worker {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "store::runs::tests::processes::process_worker",
            "--ignored",
            "--nocapture",
        ])
        .env("LOOP_RUN_TEST_ROOT", root)
        .env("LOOP_RUN_TEST_INDEX", index.to_string())
        .env("LOOP_RUN_TEST_MODE", mode)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if let Some(fault) = fault {
        command
            .env("LOOP_TEST_FAULT_POINT", fault)
            .env("LOOP_TEST_FAULT_READY", root.join("fault"));
    }
    Worker(command.spawn().unwrap())
}

#[test]
#[ignore = "invoked only by independent process and kill/restart gates"]
fn process_worker() {
    let root = std::path::PathBuf::from(std::env::var_os("LOOP_RUN_TEST_ROOT").unwrap());
    let index = std::env::var("LOOP_RUN_TEST_INDEX").unwrap();
    let mode = std::env::var("LOOP_RUN_TEST_MODE").unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let store = PgJobStore::open(options(
                &root.join("state"),
                Arc::new(fixture::FixtureClock(AtomicI64::new(fixture::NOW))),
            ))
            .await
            .unwrap();
            std::fs::write(root.join(format!("ready.{index}")), b"ready").unwrap();
            wait_file(&root.join("start")).await;
            let result = if mode == "start" {
                let mut request = start();
                request
                    .context
                    .as_mut()
                    .unwrap()
                    .idempotency_key
                    .as_mut()
                    .unwrap()
                    .value
                    .push_str(&format!(".{index}"));
                store.start_run(&owner(), &request, &specification()).await
            } else {
                let mut request = step(1);
                if mode != "replay" {
                    request
                        .context
                        .as_mut()
                        .unwrap()
                        .idempotency_key
                        .as_mut()
                        .unwrap()
                        .value
                        .push_str(&format!(".{index}"));
                }
                store
                    .advance_run(&owner(), &request, &specification())
                    .await
            };
            let result = match result {
                Ok(_) => "accepted",
                Err(StoreError::DuplicateJob | StoreError::RevisionConflict) => "conflict",
                Err(error) => panic!("run process failed: {error}"),
            };
            std::fs::write(root.join(format!("result.{index}")), result).unwrap();
            store.close().await;
        });
}

#[tokio::test]
async fn concurrent_writers() {
    for mode in ["start", "step", "replay"] {
        for count in [2, 4, 8] {
            let (directory, store, _) = setup().await;
            if mode != "start" {
                store
                    .start_run(&owner(), &start(), &specification())
                    .await
                    .unwrap();
                finish(&store).await;
            }
            let mut workers: Vec<_> = (0..count)
                .map(|index| spawn(directory.path(), index, mode, None))
                .collect();
            for index in 0..count {
                wait_file(&directory.path().join(format!("ready.{index}"))).await;
            }
            std::fs::write(directory.path().join("start"), b"start").unwrap();
            for worker in &mut workers {
                join(worker).await;
            }
            let accepted = (0..count)
                .filter(|index| {
                    std::fs::read(directory.path().join(format!("result.{index}"))).unwrap()
                        == b"accepted"
                })
                .count();
            assert_eq!(
                accepted,
                if mode == "replay" { count } else { 1 },
                "{mode}/{count}"
            );
            let current = store.read_run(&owner(), "run.managed").await.unwrap();
            assert_eq!(
                current.view.reserved_steps,
                if mode == "start" { 1 } else { 2 }
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM jobs")
                    .fetch_one(&store.pool)
                    .await
                    .unwrap(),
                if mode == "start" { 1 } else { 2 }
            );
            loop_core::audit::verify_audit_chain(&store.audit_events(0, 100).await.unwrap())
                .unwrap();
            store.close().await;
        }
    }
}

#[tokio::test]
async fn reservation_crashes() {
    for mode in ["start", "step"] {
        for timing in ["before", "after"] {
            let (directory, store, _) = setup().await;
            if mode == "step" {
                store
                    .start_run(&owner(), &start(), &specification())
                    .await
                    .unwrap();
                finish(&store).await;
            }
            std::fs::write(directory.path().join("start"), b"start").unwrap();
            let point = format!("run_{mode}_{timing}");
            let mut worker = spawn(directory.path(), 0, mode, Some(&point));
            wait_file(&directory.path().join("fault")).await;
            worker.0.kill().unwrap();
            worker.0.wait().unwrap();
            let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs")
                .fetch_one(&store.pool)
                .await
                .unwrap();
            assert_eq!(
                jobs,
                i64::from(mode == "step") + i64::from(timing == "after"),
                "{point}"
            );
            store.close().await;
            let reopened = PgJobStore::open(options(
                &directory.path().join("state"),
                Arc::new(fixture::FixtureClock(AtomicI64::new(fixture::NOW))),
            ))
            .await
            .unwrap();
            let result = if mode == "start" {
                let mut request = start();
                request
                    .context
                    .as_mut()
                    .unwrap()
                    .idempotency_key
                    .as_mut()
                    .unwrap()
                    .value
                    .push_str(".0");
                reopened
                    .start_run(&owner(), &request, &specification())
                    .await
                    .unwrap()
            } else {
                let mut request = step(1);
                request
                    .context
                    .as_mut()
                    .unwrap()
                    .idempotency_key
                    .as_mut()
                    .unwrap()
                    .value
                    .push_str(".0");
                reopened
                    .advance_run(&owner(), &request, &specification())
                    .await
                    .unwrap()
            };
            assert_eq!(result.reserved_steps, if mode == "start" { 1 } else { 2 });
            let receipts: i64 =
                sqlx::query_scalar("SELECT count(*) FROM command_receipts WHERE operation=$1")
                    .bind(format!("loop.runs.{mode}"))
                    .fetch_one(&reopened.pool)
                    .await
                    .unwrap();
            assert_eq!(receipts, 1);
            loop_core::audit::verify_audit_chain(&reopened.audit_events(0, 100).await.unwrap())
                .unwrap();
            reopened.close().await;
        }
    }
}
