//! Installed CLI and real Python workers cross the scoped numerical workflow.

use std::process::Output;

use loop_protocol::wire::{jobs::v1 as jobs, runs::v1 as runs, v1};
use prost::Message;
use serde_json::Value;

use super::fixture::{Case, operations::Operations, timed};
use crate::store::JobRepository;

fn document(output: Output, code: i32, operation: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{operation}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{operation}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{operation}: invalid CLI JSON: {error}"));
    assert_eq!(value["schema"], "loop.run-cli/v2", "{operation}");
    value["run"].clone()
}

async fn start(operations: &Operations) -> Value {
    document(
        operations
            .run_cli(&[
                "--workflow-version",
                "2",
                "start",
                "--plan",
                "PLAN",
                "--key",
                "numerical.start",
            ])
            .await,
        0,
        "start",
    )
}

async fn step(operations: &Operations, run: &Value, key: &str) -> Value {
    document(
        operations
            .run_cli(&[
                "--workflow-version",
                "2",
                "--timeout-seconds",
                "240",
                "step",
                "--run",
                "run.discovery",
                "--revision",
                run["revision"].as_str().unwrap(),
                "--key",
                key,
            ])
            .await,
        0,
        key,
    )
}

async fn status(operations: &Operations) -> Value {
    document(
        operations
            .run_cli(&[
                "--workflow-version",
                "2",
                "status",
                "--run",
                "run.discovery",
            ])
            .await,
        0,
        "status",
    )
}

#[tokio::test]
async fn numerical_roundtrip() {
    let mut case = Case::open().await;
    let mut operations = case.install_numerical(2, false).await;
    case.deploy(&mut operations, false).await;
    let discovery = start(&operations).await;
    assert_eq!(start(&operations).await, discovery);
    assert_eq!(discovery["research"]["stage"], "discovery");
    let evaluation = step(&operations, &discovery, "numerical.discovery").await;
    assert_eq!(evaluation["research"]["stage"], "evaluation");
    assert_eq!(evaluation["completed_rounds"], "0");
    assert_eq!(case.calls(), 1);
    operations.stop(true).await;
    case.deploy(&mut operations, false).await;
    assert_eq!(status(&operations).await, evaluation);
    assert_eq!(
        step(&operations, &discovery, "numerical.discovery").await,
        evaluation
    );
    assert_eq!(case.calls(), 1);

    let portfolio = step(&operations, &evaluation, "numerical.evaluate").await;
    assert_eq!(portfolio["research"]["stage"], "portfolio");
    let evaluation_id = evaluation["research"]["current_child"]["job_id"]
        .as_str()
        .unwrap();
    let evaluated = case.store.get(evaluation_id).await.unwrap().unwrap();
    assert_eq!(evaluated.state, v1::JobState::Succeeded as i32);
    assert!(matches!(
        evaluated.outcome.unwrap().outcome,
        Some(v1::job_outcome::Outcome::Success(_))
    ));
    operations.stop(true).await;
    case.deploy(&mut operations, false).await;
    assert_eq!(
        step(&operations, &evaluation, "numerical.evaluate").await,
        portfolio
    );
    let second = step(&operations, &portfolio, "numerical.portfolio").await;
    assert_eq!(second["research"]["stage"], "discovery");
    assert_eq!(second["research"]["round"], "2");
    assert_eq!(second["completed_rounds"], "1");
    assert!(second["research"]["factor_spec_id"].is_null());
    let portfolio_id = portfolio["research"]["current_child"]["job_id"]
        .as_str()
        .unwrap();
    assert_eq!(
        case.store.get(portfolio_id).await.unwrap().unwrap().state,
        v1::JobState::Succeeded as i32
    );

    let completed = step(&operations, &second, "numerical.duplicate").await;
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["completed_rounds"], "2");
    assert_eq!(completed["research"]["disposition"], "duplicate");
    assert_eq!(
        completed["research"]["factor_spec_id"],
        evaluation["research"]["factor_spec_id"]
    );
    assert_eq!(completed["reserved_steps"], "4");
    assert_eq!(case.calls(), 2);
    assert_eq!(
        step(&operations, &second, "numerical.duplicate").await,
        completed
    );
    assert_eq!(case.calls(), 2);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn numerical_scope() {
    let mut case = Case::open().await;
    let mut operations = case.install_numerical(1, false).await;
    case.deploy(&mut operations, false).await;
    let discovery = start(&operations).await;
    let error = case
        .agent_run()
        .await
        .get_run(timed(runs::GetRunRequest {
            context: Some(case.context()),
            run_id: Some(v1::RunId {
                value: "run.discovery".into(),
            }),
            workflow_version: 2,
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    let evaluation = step(&operations, &discovery, "scope.discovery").await;
    let job_id = evaluation["research"]["current_child"]["job_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let error = case
        .agent_jobs()
        .await
        .acquire_job_lease(timed(jobs::AcquireJobLeaseRequest {
            context: Some(case.context()),
            job_id: Some(v1::JobId {
                value: job_id.clone(),
            }),
            expected_revision: 1,
            requested_duration: Some(prost_types::Duration {
                seconds: 60,
                nanos: 0,
            }),
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    assert_eq!(
        case.store.get(&job_id).await.unwrap().unwrap().state,
        v1::JobState::Queued as i32
    );
    let legacy = operations
        .run_cli(&["status", "--run", "run.discovery"])
        .await;
    assert_ne!(legacy.status.code(), Some(0));
    assert!(legacy.stdout.is_empty());
    assert_eq!(status(&operations).await, evaluation);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn numerical_corruption() {
    let mut case = Case::open().await;
    let mut operations = case.install_numerical(1, false).await;
    case.deploy(&mut operations, false).await;
    let discovery = start(&operations).await;
    operations.corrupt_run();
    let output = operations
        .run_cli(&[
            "--workflow-version",
            "2",
            "step",
            "--run",
            "run.discovery",
            "--revision",
            discovery["revision"].as_str().unwrap(),
            "--key",
            "corrupt.recipe",
        ])
        .await;
    assert_eq!(output.status.code(), Some(6));
    assert!(output.stdout.is_empty());
    assert_eq!(case.calls(), 0);
    let job_id = discovery["research"]["current_child"]["job_id"]
        .as_str()
        .unwrap();
    assert_eq!(
        case.store.get(job_id).await.unwrap().unwrap().state,
        v1::JobState::Queued as i32
    );
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn coverage_failure_memory() {
    let mut case = Case::open().await;
    let mut operations = case.install_numerical(2, true).await;
    case.deploy(&mut operations, false).await;
    let discovery = start(&operations).await;
    let evaluation = step(&operations, &discovery, "coverage.discovery").await;
    assert_eq!(evaluation["research"]["stage"], "evaluation");
    let next = step(&operations, &evaluation, "coverage.evaluate").await;
    assert_eq!(next["research"]["stage"], "discovery");
    assert_eq!(next["research"]["round"], "2");
    assert_eq!(next["completed_rounds"], "1");
    assert_eq!(next["research"]["disposition"], "pending");
    assert!(next["research"]["factor_spec_id"].is_null());

    let mut connection = case.numerical_db().await;
    let evidence: Vec<u8> =
        sqlx::query_scalar("SELECT evidence_blob FROM factor_evaluations WHERE job_id=$1")
            .bind(
                evaluation["research"]["current_child"]["job_id"]
                    .as_str()
                    .unwrap(),
            )
            .fetch_one(&mut connection)
            .await
            .unwrap();
    let trial = crate::store::EvaluationTrial::decode(evidence.as_slice()).unwrap();
    assert_eq!(trial.disposition, "insufficient_coverage");
    assert_eq!(trial.minimum_coverage_bps, 6666);
    let result = trial.result.unwrap();
    assert_eq!(result.valid_observations, 2);
    assert_eq!(result.eligible_observations, 6);
    assert_eq!(
        result.factor_spec_id.as_ref().unwrap().value,
        evaluation["research"]["factor_spec_id"].as_str().unwrap()
    );
    drop(connection);

    // A new server must recover the real rejection, without evaluating the
    // second identical proposal or manufacturing another empirical failure.
    operations.stop(true).await;
    case.deploy(&mut operations, false).await;
    let completed = step(&operations, &next, "coverage.remembered").await;
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["completed_rounds"], "2");
    assert_eq!(completed["research"]["stage"], "discovery");
    assert_eq!(completed["research"]["disposition"], "previously_rejected");
    assert!(completed["research"]["evaluation_job_id"].is_null());
    assert_eq!(
        completed["research"]["factor_spec_id"],
        evaluation["research"]["factor_spec_id"]
    );
    assert_eq!(completed["reserved_steps"], "3");
    assert_eq!(case.calls(), 2);
    assert_eq!(
        step(&operations, &next, "coverage.remembered").await,
        completed
    );
    assert_eq!(case.calls(), 2);
    let mut connection = case.numerical_db().await;
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM jobs),
                (SELECT count(*) FROM factor_trials),
                (SELECT count(*) FROM factor_evaluations),
                (SELECT count(*) FROM backtest_results)",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(counts, (3, 1, 1, 0));
    drop(connection);
    operations.stop(false).await;
    case.close().await;
}
