//! Installed binaries, real mTLS and PostgreSQL, and a local synthetic supplier.
mod negative;

use std::process::Output;
use std::time::Duration;

use rustix::process::{Pid, Signal, kill_process};
use serde_json::Value;

use super::fixture::Case;
use super::fixture::operations::Operations;
use super::{provider, v1};

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
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["schema"], "loop.discovery-cli/v1");
    document
}

fn failure(output: Output, code: i32, category: &str) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(
        error,
        serde_json::json!({"schema":"loop.discovery-cli/v1","error":{"category":category}})
    );
}

async fn start(ops: &Operations) -> Value {
    document(
        ops.cli(&["start", "--input", "INPUT", "--key", "cli.start"])
            .await,
        0,
    )
}

async fn status(ops: &Operations, job: &Value, code: i32) -> Value {
    document(
        ops.cli(&["status", "--job", job["job_id"].as_str().unwrap()])
            .await,
        code,
    )
}

async fn change(ops: &Operations, command: &str, job: &Value, key: &str) -> Output {
    ops.cli(&[
        command,
        "--job",
        job["job_id"].as_str().unwrap(),
        "--revision",
        job["revision"].as_str().unwrap(),
        "--key",
        key,
    ])
    .await
}

async fn called(case: &Case) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while case.calls() == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("supplier must receive the original invocation");
}

async fn invocation_state(case: &Case, job: &Value) -> i32 {
    let history = case
        .store
        .model_history(&case.actor(), job["job_id"].as_str().unwrap())
        .await
        .unwrap();
    let step = history.last().unwrap();
    let original = step.request.context.as_ref().unwrap();
    let mut client = case.provider().await;
    let mut request = tonic::Request::new(provider::LookupInvocationRequest {
        context: Some(case.context()),
        original_request_id: original.request_id.clone(),
        original_idempotency_key: original.idempotency_key.clone(),
        request_sha256: Some(v1::Sha256Digest {
            value: step.request_sha256.to_vec(),
        }),
    });
    request.set_timeout(Duration::from_secs(5));
    client
        .lookup_invocation(request)
        .await
        .unwrap()
        .into_inner()
        .state
}

#[tokio::test]
async fn installed_workflow() {
    let mut case = Case::controlled().await;
    let mut ops = case.install();
    case.deploy(&mut ops, false).await;
    let submitted = start(&ops).await;
    assert_eq!(start(&ops).await, submitted);
    let queued = status(&ops, &submitted["job"], 0).await;
    assert_eq!(queued["job"]["status"], "queued");
    assert!(queued["step"]["reserved_cost"].is_null());
    assert_eq!(queued["step"]["plan_verified"], true);
    let completed = document(
        change(&ops, "execute", &submitted["job"], "cli.execute").await,
        0,
    );
    assert_eq!(completed["job"]["status"], "succeeded");
    assert!(
        completed["step"]["candidate"]["expression_id"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(case.calls(), 2);
    failure(
        change(&ops, "execute", &submitted["job"], "cli.execute").await,
        4,
        "conflict",
    );
    let repeated = document(
        change(&ops, "execute", &completed["job"], "cli.observe").await,
        0,
    );
    assert_eq!(completed, repeated);
    assert_eq!(case.calls(), 2);
    ops.stop(false).await;
    case.deploy(&mut ops, false).await;
    let restored = status(&ops, &completed["job"], 0).await;
    assert_eq!(restored["job"], completed["job"]);
    assert_eq!(restored["step"], completed["step"]);
    assert_eq!(case.calls(), 2);
    let job_id = submitted["job"]["job_id"].as_str().unwrap();
    let first = document(
        ops.cli(&["events", "--job", job_id, "--limit", "1"]).await,
        0,
    );
    assert_eq!(first["events"][0]["operation"], "start");
    assert_eq!(first["has_more"], true);
    let rest = document(
        ops.cli(&[
            "events",
            "--job",
            job_id,
            "--after",
            first["next_after_sequence"].as_str().unwrap(),
        ])
        .await,
        0,
    );
    assert_eq!(rest["has_more"], false);
    assert!(
        rest["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["operation"] == "tool_record")
    );
    ops.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn restarted_pause() {
    let mut case = Case::open().await;
    let mut ops = case.install();
    case.deploy(&mut ops, false).await;
    let submitted = start(&ops).await;
    failure(
        change(&ops, "expire", &submitted["job"], "cli.early-expiry").await,
        4,
        "conflict",
    );
    let paused = document(
        change(&ops, "pause", &submitted["job"], "cli.pause").await,
        0,
    );
    assert_eq!(paused["job"]["status"], "paused");
    failure(
        change(&ops, "cancel", &submitted["job"], "cli.stale").await,
        4,
        "conflict",
    );
    // Reusing a command key with a different revision is a conflict, not replay.
    failure(
        change(&ops, "pause", &paused["job"], "cli.pause").await,
        4,
        "conflict",
    );
    ops.stop(false).await;
    case.deploy(&mut ops, false).await;
    let restored = status(&ops, &paused["job"], 0).await;
    assert_eq!(restored["job"], paused["job"]);
    let result = document(
        change(&ops, "resume", &paused["job"], "cli.resume").await,
        0,
    );
    assert_eq!(result["job"]["status"], "succeeded");
    assert_eq!(case.calls(), 1);
    ops.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn disabled_observation() {
    let mut case = Case::open().await;
    let mut ops = case.install();
    case.deploy(&mut ops, false).await;
    let submitted = start(&ops).await;
    let completed = document(
        change(&ops, "execute", &submitted["job"], "cli.execute").await,
        0,
    );
    case.corrupt_plan();
    failure(
        ops.cli(&[
            "status",
            "--job",
            completed["job"]["job_id"].as_str().unwrap(),
        ])
        .await,
        6,
        "transport",
    );
    ops.stop(false).await;
    case.deploy(&mut ops, true).await;
    let observed = status(&ops, &completed["job"], 0).await;
    assert_eq!(observed["job"], completed["job"]);
    assert_eq!(observed["step"]["plan_verified"], false);
    assert!(observed["step"].get("candidate").is_none());
    assert_eq!(
        observed["step"]["reserved_cost"],
        completed["step"]["reserved_cost"]
    );
    let events = document(
        ops.cli(&[
            "events",
            "--job",
            completed["job"]["job_id"].as_str().unwrap(),
        ])
        .await,
        0,
    );
    assert!(!events["events"].as_array().unwrap().is_empty());
    failure(
        change(&ops, "execute", &completed["job"], "cli.disabled").await,
        3,
        "authorization",
    );
    assert_eq!(case.calls(), 1);
    ops.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn disabled_cancellation() {
    let mut case = Case::open().await;
    let mut ops = case.install();
    case.deploy(&mut ops, false).await;
    let submitted = start(&ops).await;
    ops.stop(false).await;
    case.corrupt_plan();
    case.deploy(&mut ops, true).await;
    assert_eq!(
        status(&ops, &submitted["job"], 0).await["step"]["plan_verified"],
        false
    );
    let paused = document(
        change(&ops, "pause", &submitted["job"], "cli.pause").await,
        0,
    );
    let cancelled = document(
        change(&ops, "cancel", &paused["job"], "cli.cancel").await,
        7,
    );
    assert_eq!(cancelled["job"]["status"], "cancelled");
    assert_eq!(
        status(&ops, &cancelled["job"], 7).await["job"],
        cancelled["job"]
    );
    assert_eq!(case.calls(), 0);
    ops.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn denied_actor() {
    let mut case = Case::open().await;
    let mut ops = case.install();
    case.deploy(&mut ops, false).await;
    let submitted = start(&ops).await;
    let mut config: Value = serde_json::from_slice(&std::fs::read(&ops.config).unwrap()).unwrap();
    config["actor"]["actor_id"] = "agent.impersonator".into();
    std::fs::write(&ops.config, serde_json::to_vec(&config).unwrap()).unwrap();
    failure(
        ops.cli(&[
            "status",
            "--job",
            submitted["job"]["job_id"].as_str().unwrap(),
        ])
        .await,
        3,
        "authorization",
    );
    assert_eq!(case.calls(), 0);
    ops.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn interrupted_client() {
    let mut case = Case::open().await;
    let mut ops = case.install();
    case.deploy(&mut ops, false).await;
    let submitted = start(&ops).await;
    case.delay_supplier();
    let child = ops.spawn_cli(&[
        "execute",
        "--job",
        submitted["job"]["job_id"].as_str().unwrap(),
        "--revision",
        submitted["job"]["revision"].as_str().unwrap(),
        "--key",
        "cli.execute",
    ]);
    called(&case).await;
    let pid = Pid::from_raw(child.id().unwrap().try_into().unwrap()).unwrap();
    kill_process(pid, Signal::INT).unwrap();
    let output = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    failure(output, 130, "interrupted");
    let observed = status(&ops, &submitted["job"], 0).await;
    assert_ne!(observed["job"]["status"], "cancelled");
    let cancelled = document(
        change(&ops, "cancel", &observed["job"], "cli.cancel").await,
        7,
    );
    assert_eq!(cancelled["job"]["status"], "cancelled");
    assert_eq!(case.calls(), 1);
    ops.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn killed_recovery() {
    let mut case = Case::open().await;
    let mut ops = case.install();
    case.deploy(&mut ops, false).await;
    let submitted = start(&ops).await;
    case.stall_supplier();
    let child = ops.spawn_cli(&[
        "execute",
        "--job",
        submitted["job"]["job_id"].as_str().unwrap(),
        "--revision",
        submitted["job"]["revision"].as_str().unwrap(),
        "--key",
        "cli.execute",
    ]);
    called(&case).await;
    ops.stop(true).await;
    let output = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    // Killing the caller cannot prove the paid request did not execute. The
    // disconnected Provider preserves an ambiguous claim, not a fake result.
    assert_eq!(
        invocation_state(&case, &submitted["job"]).await,
        provider::InvocationState::Ambiguous as i32
    );
    case.deploy(&mut ops, false).await;
    let restored = status(&ops, &submitted["job"], 0).await;
    assert_eq!(restored["step"]["state"], "dispatched");
    let paused = document(
        change(&ops, "pause", &restored["job"], "cli.pause").await,
        0,
    );
    let receipt = document(
        change(&ops, "reconcile", &paused["job"], "cli.reconcile").await,
        0,
    );
    assert_eq!(receipt["step"]["state"], "dispatched");
    assert!(receipt["step"].get("candidate").is_none());
    let completed = document(
        change(&ops, "resume", &receipt["job"], "cli.resume").await,
        7,
    );
    assert_eq!(completed["job"]["status"], "infrastructure_failed");
    assert_eq!(
        completed["step"]["reserved_cost"],
        restored["step"]["reserved_cost"]
    );
    assert!(completed["step"].get("candidate").is_none());
    assert_eq!(case.calls(), 1);
    ops.stop(false).await;
    case.close().await;
}
