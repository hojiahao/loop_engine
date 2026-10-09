//! Installed operator CLI, actual Provider and PostgreSQL share one run budget.

use std::process::Output;

use serde_json::Value;

use super::fixture::{Case, operations::Operations};

fn document(output: Output, code: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema"], "loop.run-cli/v1");
    value
}

fn denied(output: Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(value["schema"], "loop.run-cli/v1");
}

async fn start(operations: &Operations) -> Value {
    document(
        operations
            .run_cli(&["start", "--plan", "PLAN", "--key", "run.start"])
            .await,
        0,
    )
}

async fn step(operations: &Operations, run: &Value, key: &str, code: i32) -> Value {
    document(
        operations
            .run_cli(&[
                "step",
                "--run",
                run["run_id"].as_str().unwrap(),
                "--revision",
                run["revision"].as_str().unwrap(),
                "--key",
                key,
            ])
            .await,
        code,
    )
}

async fn status(operations: &Operations, code: i32) -> Value {
    document(
        operations
            .run_cli(&["status", "--run", "run.discovery"])
            .await,
        code,
    )
}

#[tokio::test]
async fn two_rounds() {
    let mut case = Case::controlled().await;
    let mut operations = case.install_run(2, 2);
    case.deploy(&mut operations, false).await;
    let first = start(&operations).await;
    assert_eq!(start(&operations).await, first);
    assert_eq!(first["run"]["reserved_steps"], "3");
    let second = step(&operations, &first["run"], "run.first", 0).await;
    assert_eq!(second["run"]["completed_rounds"], "1");
    assert_eq!(second["run"]["reserved_steps"], "6");
    assert_eq!(case.calls(), 2);
    assert_ne!(
        first["run"]["current_job"]["job_id"],
        second["run"]["current_job"]["job_id"]
    );
    operations.stop(true).await;
    case.deploy(&mut operations, false).await;
    assert_eq!(status(&operations, 0).await["run"], second["run"]);
    assert_eq!(
        step(&operations, &first["run"], "run.first", 0).await,
        second
    );
    assert_eq!(case.calls(), 2);
    let completed = step(&operations, &second["run"], "run.second", 0).await;
    assert_eq!(completed["run"]["status"], "completed");
    assert_eq!(completed["run"]["completed_rounds"], "2");
    assert_eq!(completed["run"]["reserved_steps"], "6");
    assert_eq!(completed["run"]["reserved_cost"]["amount"], "4");
    assert_eq!(case.calls(), 4);
    assert_eq!(
        step(&operations, &second["run"], "run.second", 0).await,
        completed
    );
    assert_eq!(case.calls(), 4);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn cumulative_exhaustion() {
    let mut case = Case::open().await;
    let mut operations = case.install_run(3, 1);
    case.deploy(&mut operations, false).await;
    let first = start(&operations).await;
    let exhausted = step(&operations, &first["run"], "run.exhaust", 7).await;
    assert_eq!(exhausted["run"]["status"], "budget_exhausted");
    assert_eq!(exhausted["run"]["completed_rounds"], "1");
    assert_eq!(exhausted["run"]["reserved_cost"]["amount"], "1");
    assert_eq!(
        first["run"]["current_job"]["job_id"],
        exhausted["run"]["current_job"]["job_id"]
    );
    assert_eq!(case.calls(), 1);
    operations.stop(false).await;
    case.deploy(&mut operations, false).await;
    assert_eq!(status(&operations, 7).await["run"], exhausted["run"]);
    let replay = step(&operations, &first["run"], "run.exhaust", 7).await;
    assert_eq!(replay, exhausted);
    assert_eq!(case.calls(), 1);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn stop_only_status() {
    let mut case = Case::open().await;
    let mut operations = case.install_run(2, 2);
    case.deploy(&mut operations, false).await;
    let original = start(&operations).await;
    operations.stop(false).await;
    operations.corrupt_run();
    case.corrupt_plan();
    case.deploy(&mut operations, true).await;
    let restored = status(&operations, 0).await;
    assert_eq!(restored["run"]["plan_verified"], false);
    assert_eq!(
        restored["run"]["reserved_steps"],
        original["run"]["reserved_steps"]
    );
    denied(
        operations
            .run_cli(&[
                "step",
                "--run",
                "run.discovery",
                "--revision",
                "1",
                "--key",
                "disabled",
            ])
            .await,
        3,
    );
    denied(
        operations
            .run_cli(&["start", "--plan", "PLAN", "--key", "new-start"])
            .await,
        3,
    );
    assert_eq!(case.calls(), 0);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn changed_plan() {
    let mut case = Case::open().await;
    let mut operations = case.install_run(2, 2);
    case.deploy(&mut operations, false).await;
    start(&operations).await;
    operations.corrupt_run();
    denied(
        operations
            .run_cli(&[
                "step",
                "--run",
                "run.discovery",
                "--revision",
                "1",
                "--key",
                "changed.plan",
            ])
            .await,
        // Server-side evidence corruption uses the existing dependency-unavailable
        // transport category; CLI code 8 is for malformed responses/client faults.
        6,
    );
    assert_eq!(case.calls(), 0);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn failed_child() {
    let mut case = Case::open().await;
    let mut operations = case.install_run(2, 2);
    case.deploy(&mut operations, false).await;
    let first = start(&operations).await;
    case.invalid_ast();
    let failed = step(&operations, &first["run"], "run.invalid", 7).await;
    assert_eq!(failed["run"]["status"], "infrastructure_failed");
    assert_eq!(failed["run"]["completed_rounds"], "0");
    assert_eq!(failed["run"]["reserved_cost"]["amount"], "1");
    assert_eq!(case.calls(), 1);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn legacy_bypass() {
    let mut case = Case::open().await;
    let mut operations = case.install_run(2, 2);
    case.deploy(&mut operations, false).await;
    let first = start(&operations).await;
    let output = operations
        .cli(&["start", "--input", "INPUT", "--key", "outside-run"])
        .await;
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(status(&operations, 0).await["run"], first["run"]);
    assert_eq!(case.calls(), 0);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn operator_scope() {
    let mut case = Case::open().await;
    let mut operations = case.install_run(2, 2);
    case.deploy(&mut operations, false).await;
    let first = start(&operations).await;
    let error = case
        .agent_run()
        .await
        .get_run(super::fixture::timed(
            loop_protocol::wire::runs::v1::GetRunRequest {
                context: Some(case.context()),
                run_id: Some(super::v1::RunId {
                    value: "run.discovery".into(),
                }),
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    denied(
        operations.run_cli(&["status", "--run", "run.other"]).await,
        3,
    );
    assert_eq!(status(&operations, 0).await["run"], first["run"]);
    assert_eq!(case.calls(), 0);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn stale_step() {
    let mut case = Case::open().await;
    let mut operations = case.install_run(2, 2);
    case.deploy(&mut operations, false).await;
    let first = start(&operations).await;
    let second = step(&operations, &first["run"], "run.first", 0).await;
    denied(
        operations
            .run_cli(&[
                "step",
                "--run",
                "run.discovery",
                "--revision",
                "1",
                "--key",
                "different.key",
            ])
            .await,
        4,
    );
    denied(
        operations
            .run_cli(&[
                "step",
                "--run",
                "run.discovery",
                "--revision",
                "2",
                "--key",
                "run.first",
            ])
            .await,
        4,
    );
    assert_eq!(status(&operations, 0).await["run"], second["run"]);
    assert_eq!(case.calls(), 1);
    operations.stop(false).await;
    case.close().await;
}
