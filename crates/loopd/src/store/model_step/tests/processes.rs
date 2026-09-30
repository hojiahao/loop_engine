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
    let deadline = Instant::now() + Duration::from_secs(20);
    while !path.exists() {
        assert!(Instant::now() < deadline, "model worker barrier timed out");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(super) async fn execute(
    store: &PgJobStore,
    mode: &str,
    command: ModelStepCommand,
) -> StoreResult<bool> {
    match mode {
        "reserve" => {
            let request = if command.ordinal == 0 {
                invocation()
            } else {
                let history = store.model_history(&fixture::actor(), "job.1").await?;
                next_request(&history[0])
            };
            store
                .reserve_model(&fixture::actor(), command, request)
                .await
                .map(|_| true)
        }
        "dispatch" => store
            .dispatch_model(&fixture::actor(), command)
            .await
            .map(|result| result.send),
        "finish" => {
            let step = store.model_step(&fixture::actor(), "job.1").await?.unwrap();
            store
                .finish_model(&fixture::actor(), command, response(&step), outcome())
                .await
                .map(|_| true)
        }
        "call" => {
            let step = store.model_step(&fixture::actor(), "job.1").await?.unwrap();
            store
                .finish_call(&fixture::actor(), command, call_response(&step))
                .await
                .map(|_| true)
        }
        "record" => store
            .record_tool(&fixture::actor(), command, tool_result())
            .await
            .map(|_| true),
        "pause" | "cancel" | "expire" => store
            .control_model(
                &fixture::actor(),
                command,
                match mode {
                    "pause" => ModelControl::Pause,
                    "cancel" => ModelControl::Cancel,
                    _ => ModelControl::Expire,
                },
            )
            .await
            .map(|_| true),
        "resume" => store
            .resume_model(&fixture::actor(), command, None)
            .await
            .map(|result| result.execute),
        "retry" | "tool_retry" => store
            .retry_model(
                &fixture::actor(),
                command,
                if mode == "retry" {
                    ModelRetry::Lookup
                } else {
                    ModelRetry::Tool
                },
            )
            .await
            .map(|_| true),
        "fail" => store
            .fail_model(&fixture::actor(), command, "provider_unavailable")
            .await
            .map(|_| true),
        "reconcile" => {
            let step = store.model_step(&fixture::actor(), "job.1").await?.unwrap();
            store
                .reconcile_model(&fixture::actor(), command, response(&step))
                .await
                .map(|_| true)
        }
        _ => panic!("test mode"),
    }
}

#[tokio::test]
async fn concurrent_context() {
    for mode in ["reserve", "dispatch", "record"] {
        for count in [2, 4, 8] {
            let (directory, store, _) = tool_setup().await;
            let step = if mode == "record" {
                tool_called(&store).await
            } else {
                tool_ready(&store).await
            };
            let step = if mode == "dispatch" {
                store
                    .reserve_model(
                        &fixture::actor(),
                        next_command(&step.job, "reserve.second"),
                        next_request(&step),
                    )
                    .await
                    .unwrap()
            } else {
                step
            };
            let cmd = if mode == "record" {
                command(&step.job, "context.concurrent")
            } else {
                next_command(&step.job, "context.concurrent")
            };
            std::fs::write(directory.path().join("command"), cmd.encode_to_vec()).unwrap();
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
            if mode == "dispatch" {
                let sends = (0..count)
                    .filter(|index| {
                        std::fs::read(directory.path().join(format!("result.{index}"))).unwrap()
                            == b"send"
                    })
                    .count();
                assert_eq!(sends, 1);
            }
            let history = store
                .model_history(&fixture::actor(), "job.1")
                .await
                .unwrap();
            assert_eq!(history.len(), if mode == "record" { 1 } else { 2 });
            assert!(history[0].tool_result.is_some());
            assert_eq!(history.last().unwrap().job.revision, step.job.revision + 1);
            let events = store.audit_events(0, 100).await.unwrap();
            assert_eq!(events.len(), step.job.revision as usize + 1);
            loop_core::audit::verify_audit_chain(&events).unwrap();
            store.close().await;
        }
    }
}

#[tokio::test]
async fn context_crashes() {
    for mode in ["call", "record", "reserve", "dispatch", "finish"] {
        for point in ["before", "after"] {
            let (directory, store, clock) = tool_setup().await;
            let step = if mode == "call" {
                let job = store.get("job.1").await.unwrap().unwrap();
                let step = store
                    .reserve_model(&fixture::actor(), command(&job, "reserve"), tool_request())
                    .await
                    .unwrap();
                store
                    .dispatch_model(&fixture::actor(), command(&step.job, "dispatch"))
                    .await
                    .unwrap()
                    .step
            } else if mode == "record" {
                tool_called(&store).await
            } else {
                tool_ready(&store).await
            };
            let step = if matches!(mode, "dispatch" | "finish") {
                store
                    .reserve_model(
                        &fixture::actor(),
                        next_command(&step.job, "reserve.second"),
                        next_request(&step),
                    )
                    .await
                    .unwrap()
            } else {
                step
            };
            let step = if mode == "finish" {
                store
                    .dispatch_model(
                        &fixture::actor(),
                        next_command(&step.job, "dispatch.second"),
                    )
                    .await
                    .unwrap()
                    .step
            } else {
                step
            };
            let cmd = if matches!(mode, "call" | "record") {
                command(&step.job, &format!("{mode}.context"))
            } else {
                next_command(&step.job, &format!("{mode}.context"))
            };
            std::fs::write(directory.path().join("command"), cmd.encode_to_vec()).unwrap();
            std::fs::write(directory.path().join("start"), b"start").unwrap();
            let mut worker = spawn(
                directory.path(),
                0,
                mode,
                Some(&format!("model_{mode}_{point}")),
            );
            wait_file(&directory.path().join("fault")).await;
            worker.0.kill().unwrap();
            worker.0.wait().unwrap();
            store.close().await;
            let reopened = PgJobStore::open(options(&directory.path().join("state"), clock))
                .await
                .unwrap();
            let observed = reopened.get("job.1").await.unwrap().unwrap();
            assert_eq!(
                observed.revision,
                step.job.revision + u64::from(point == "after")
            );
            let sends = execute(&reopened, mode, cmd).await.unwrap();
            if mode == "dispatch" {
                assert_eq!(sends, point == "before");
            }
            let history = reopened
                .model_history(&fixture::actor(), "job.1")
                .await
                .unwrap();
            let current = history.last().unwrap();
            assert_eq!(current.job.revision, step.job.revision + 1);
            assert_eq!(history[0].ordinal, 0);
            if !matches!(mode, "call" | "record") {
                assert_eq!(history.len(), 2);
                assert!(history[0].tool_result.is_some());
            }
            let events = reopened.audit_events(0, 100).await.unwrap();
            assert_eq!(events.len(), current.job.revision as usize);
            loop_core::audit::verify_audit_chain(&events).unwrap();
            reopened.close().await;
        }
    }
}

#[test]
#[ignore = "called only by independent OS-process tests"]
fn process_worker() {
    let root = std::path::PathBuf::from(std::env::var_os("LOOP_MODEL_TEST_ROOT").unwrap());
    let index = std::env::var("LOOP_MODEL_TEST_INDEX").unwrap();
    let mode = std::env::var("LOOP_MODEL_TEST_MODE").unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let now = std::fs::read_to_string(root.join("time"))
                .ok()
                .map(|value| value.parse::<i64>().unwrap())
                .unwrap_or(fixture::NOW);
            let store = PgJobStore::open(options(
                &root.join("state"),
                Arc::new(fixture::FixtureClock(AtomicI64::new(now))),
            ))
            .await
            .unwrap();
            let mut command =
                ModelStepCommand::decode(std::fs::read(root.join("command")).unwrap().as_slice())
                    .unwrap();
            if std::env::var_os("LOOP_MODEL_UNIQUE").is_some() {
                command
                    .context
                    .as_mut()
                    .unwrap()
                    .idempotency_key
                    .as_mut()
                    .unwrap()
                    .value
                    .push_str(&format!(".{index}"));
            }
            std::fs::write(root.join(format!("ready.{index}")), b"ready").unwrap();
            wait_file(&root.join("start")).await;
            let result = match execute(&store, &mode, command).await {
                Ok(value) => value,
                Err(StoreError::RevisionConflict | StoreError::Invalid("model retry replay")) => {
                    false
                }
                Err(error) => panic!("model process operation failed: {error}"),
            };
            std::fs::write(
                root.join(format!("result.{index}")),
                if result {
                    b"send".as_slice()
                } else {
                    b"lookup".as_slice()
                },
            )
            .unwrap();
            store.close().await;
        });
}

fn spawn(root: &Path, index: usize, mode: &str, fault: Option<&str>) -> Worker {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "store::model_step::tests::processes::process_worker",
            "--ignored",
            "--nocapture",
        ])
        .env("LOOP_MODEL_TEST_ROOT", root)
        .env("LOOP_MODEL_TEST_INDEX", index.to_string())
        .env("LOOP_MODEL_TEST_MODE", mode)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if let Some(fault) = fault {
        command
            .env("LOOP_TEST_FAULT_POINT", fault)
            .env("LOOP_TEST_FAULT_READY", root.join("fault"));
    } else if matches!(
        mode,
        "pause" | "cancel" | "expire" | "resume" | "retry" | "tool_retry" | "fail" | "reconcile"
    ) {
        command.env("LOOP_MODEL_UNIQUE", "1");
    }
    Worker(command.spawn().unwrap())
}

pub(super) struct LifecycleCase {
    pub(super) directory: tempfile::TempDir,
    pub(super) store: PgJobStore,
    pub(super) clock: Arc<fixture::FixtureClock>,
    pub(super) job: JobRecord,
    pub(super) command: ModelStepCommand,
}

pub(super) async fn lifecycle_case(mode: &str) -> LifecycleCase {
    let (directory, store, clock) = if mode == "tool_retry" {
        tool_setup().await
    } else {
        setup().await
    };
    let step = if mode == "tool_retry" {
        tool_called(&store).await
    } else {
        let step = reserve(&store).await;
        store
            .dispatch_model(&fixture::actor(), command(&step.job, "dispatch"))
            .await
            .unwrap()
            .step
    };
    let mut job = step.job;
    if mode == "resume" || mode == "reconcile" {
        job = store
            .control_model(
                &fixture::actor(),
                super::lifecycle::unleased(&job, "control.setup"),
                if mode == "resume" {
                    ModelControl::Pause
                } else {
                    ModelControl::Cancel
                },
            )
            .await
            .unwrap();
    }
    let mut cmd = if matches!(mode, "pause" | "cancel" | "expire" | "resume" | "reconcile") {
        super::lifecycle::unleased(&job, &format!("lifecycle.{mode}"))
    } else {
        command(&job, &format!("lifecycle.{mode}"))
    };
    if mode == "expire" {
        clock.0.store(fixture::NOW + 120_000, Ordering::SeqCst);
        cmd.context.as_mut().unwrap().requested_at =
            Some(fixture::timestamp(fixture::NOW + 120_000));
    }
    std::fs::write(
        directory.path().join("time"),
        clock.0.load(Ordering::SeqCst).to_string(),
    )
    .unwrap();
    LifecycleCase {
        directory,
        store,
        clock,
        job,
        command: cmd,
    }
}

#[tokio::test]
async fn concurrent_lifecycle() {
    for mode in [
        "pause",
        "cancel",
        "expire",
        "resume",
        "retry",
        "tool_retry",
        "fail",
        "reconcile",
    ] {
        for count in [2, 4, 8] {
            let case = lifecycle_case(mode).await;
            std::fs::write(
                case.directory.path().join("command"),
                case.command.encode_to_vec(),
            )
            .unwrap();
            let mut workers: Vec<_> = (0..count)
                .map(|index| spawn(case.directory.path(), index, mode, None))
                .collect();
            for index in 0..count {
                wait_file(&case.directory.path().join(format!("ready.{index}"))).await;
            }
            std::fs::write(case.directory.path().join("start"), b"start").unwrap();
            for worker in &mut workers {
                join(worker).await;
            }
            let winners = (0..count)
                .filter(|index| {
                    std::fs::read(case.directory.path().join(format!("result.{index}"))).unwrap()
                        == b"send"
                })
                .count();
            assert_eq!(winners, 1, "{mode}/{count}");
            let job = case.store.get("job.1").await.unwrap().unwrap();
            assert_eq!(job.revision, case.job.revision + 1, "{mode}/{count}");
            let history = case
                .store
                .model_history(&fixture::actor(), "job.1")
                .await
                .unwrap();
            assert_eq!(history.len(), 1);
            if mode == "retry" {
                assert_eq!(history[0].lookup_attempts, 1);
            }
            if mode == "tool_retry" {
                assert_eq!(history[0].tool_attempts, 1);
            }
            let events = case.store.audit_events(0, 100).await.unwrap();
            assert_eq!(events.len(), job.revision as usize);
            loop_core::audit::verify_audit_chain(&events).unwrap();
            case.store.close().await;
        }
    }
}

#[tokio::test]
async fn lifecycle_crashes() {
    for mode in [
        "pause",
        "cancel",
        "expire",
        "resume",
        "retry",
        "tool_retry",
        "fail",
        "reconcile",
    ] {
        for point in ["before", "after"] {
            let case = lifecycle_case(mode).await;
            let root = case.directory.path();
            std::fs::write(root.join("command"), case.command.encode_to_vec()).unwrap();
            std::fs::write(root.join("start"), b"start").unwrap();
            let operation = if mode == "tool_retry" { "retry" } else { mode };
            let mut worker = spawn(root, 0, mode, Some(&format!("model_{operation}_{point}")));
            wait_file(&root.join("fault")).await;
            worker.0.kill().unwrap();
            worker.0.wait().unwrap();
            case.store.close().await;
            let reopened = PgJobStore::open(options(&root.join("state"), case.clock))
                .await
                .unwrap();
            let observed = reopened.get("job.1").await.unwrap().unwrap();
            assert_eq!(
                observed.revision,
                case.job.revision + u64::from(point == "after"),
                "{mode}/{point}"
            );
            let replay = execute(&reopened, mode, case.command).await;
            if matches!(mode, "retry" | "tool_retry") && point == "after" {
                assert!(matches!(
                    replay,
                    Err(StoreError::Invalid("model retry replay"))
                ));
            } else {
                let executed = replay.unwrap();
                if mode == "resume" {
                    assert_eq!(executed, point == "before");
                }
            }
            let current = reopened.get("job.1").await.unwrap().unwrap();
            assert_eq!(current.revision, case.job.revision + 1);
            let history = reopened
                .model_history(&fixture::actor(), "job.1")
                .await
                .unwrap();
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].lookup_attempts, u32::from(mode == "retry"));
            assert_eq!(history[0].tool_attempts, u32::from(mode == "tool_retry"));
            let events = reopened.audit_events(0, 100).await.unwrap();
            assert_eq!(events.len(), current.revision as usize);
            loop_core::audit::verify_audit_chain(&events).unwrap();
            reopened.close().await;
        }
    }
}

async fn join(worker: &mut Worker) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = worker.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "model worker failed to exit");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn concurrent_dispatch() {
    for count in [2, 4, 8] {
        let (directory, store, _) = setup().await;
        let step = reserve(&store).await;
        let cmd = command(&step.job, "dispatch.concurrent");
        std::fs::write(directory.path().join("command"), cmd.encode_to_vec()).unwrap();
        let mut workers: Vec<_> = (0..count)
            .map(|index| spawn(directory.path(), index, "dispatch", None))
            .collect();
        for index in 0..count {
            wait_file(&directory.path().join(format!("ready.{index}"))).await;
        }
        std::fs::write(directory.path().join("start"), b"start").unwrap();
        for worker in &mut workers {
            join(worker).await;
        }
        let sends = (0..count)
            .filter(|index| {
                std::fs::read(directory.path().join(format!("result.{index}"))).unwrap() == b"send"
            })
            .count();
        assert_eq!(sends, 1);
        let events = store.audit_events(0, 100).await.unwrap();
        assert_eq!(events.len(), 3);
        loop_core::audit::verify_audit_chain(&events).unwrap();
        assert_eq!(
            store
                .model_step(&fixture::actor(), "job.1")
                .await
                .unwrap()
                .unwrap()
                .state,
            ModelStepState::Dispatched
        );
        store.close().await;
    }
}

#[tokio::test]
async fn crash_boundaries() {
    for mode in ["reserve", "dispatch", "finish"] {
        for point in ["before", "after"] {
            let (directory, store, clock) = setup().await;
            let mut job = store.get("job.1").await.unwrap().unwrap();
            if mode != "reserve" {
                job = reserve(&store).await.job;
            }
            if mode == "finish" {
                job = store
                    .dispatch_model(&fixture::actor(), command(&job, "dispatch"))
                    .await
                    .unwrap()
                    .step
                    .job;
            }
            let cmd = command(&job, &format!("{mode}.crash"));
            std::fs::write(directory.path().join("command"), cmd.encode_to_vec()).unwrap();
            std::fs::write(directory.path().join("start"), b"start").unwrap();
            let mut worker = spawn(
                directory.path(),
                0,
                mode,
                Some(&format!("model_{mode}_{point}")),
            );
            wait_file(&directory.path().join("fault")).await;
            worker.0.kill().unwrap();
            worker.0.wait().unwrap();
            store.close().await;
            let reopened = PgJobStore::open(options(&directory.path().join("state"), clock))
                .await
                .unwrap();
            let observed = reopened.get("job.1").await.unwrap().unwrap();
            assert_eq!(
                observed.revision,
                job.revision + u64::from(point == "after")
            );
            let result = execute(&reopened, mode, cmd).await.unwrap();
            if mode == "dispatch" {
                assert_eq!(result, point == "before");
            }
            let completed = reopened.get("job.1").await.unwrap().unwrap();
            assert_eq!(completed.revision, job.revision + 1);
            let events = reopened.audit_events(0, 100).await.unwrap();
            assert_eq!(events.len(), completed.revision as usize);
            loop_core::audit::verify_audit_chain(&events).unwrap();
            reopened.close().await;
        }
    }
}
