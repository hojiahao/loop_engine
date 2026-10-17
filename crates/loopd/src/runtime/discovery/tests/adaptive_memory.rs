//! Actual v3 numerical failures and denied reservations survive process restart.

use std::process::Output;

use loop_protocol::wire::v1;
use prost::Message;
use serde_json::Value;

use super::fixture::{Case, operations::Operations};
use crate::store::JobRepository;

fn document(output: Output, operation: &str) -> Value {
    assert!(
        matches!(output.status.code(), Some(0 | 7)),
        "{operation}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "{operation}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema"], "loop.run-cli/v3", "{operation}");
    let expected = match value["run"]["status"].as_str().unwrap() {
        "budget_exhausted" => 7,
        "active" | "completed" | "search_exhausted" => 0,
        status => panic!("unexpected run status {status}"),
    };
    assert_eq!(output.status.code(), Some(expected), "{operation}");
    value["run"].clone()
}

async fn start(operations: &Operations) -> Value {
    document(
        operations
            .run_cli(&[
                "--workflow-version",
                "3",
                "start",
                "--plan",
                "PLAN",
                "--key",
                "memory.start",
            ])
            .await,
        "start",
    )
}

async fn step(operations: &Operations, run: &Value, key: &str) -> Value {
    document(
        operations
            .run_cli(&[
                "--workflow-version",
                "3",
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
        key,
    )
}

async fn status(operations: &Operations) -> Value {
    document(
        operations
            .run_cli(&[
                "--workflow-version",
                "3",
                "status",
                "--run",
                "run.discovery",
            ])
            .await,
        "status",
    )
}

fn child(run: &Value) -> &str {
    run["research"]["current_child"]["job_id"].as_str().unwrap()
}

async fn first_round(operations: &Operations) -> Value {
    let discovery = start(operations).await;
    let evaluation = step(operations, &discovery, "memory.first.discovery").await;
    assert_eq!(evaluation["research"]["stage"], "evaluation");
    let portfolio = step(operations, &evaluation, "memory.first.evaluation").await;
    assert_eq!(portfolio["research"]["stage"], "portfolio");
    step(operations, &portfolio, "memory.first.portfolio").await
}

async fn restart_replay(
    case: &mut Case,
    operations: &mut Operations,
    source: &Value,
    key: &str,
    expected: &Value,
) {
    let record = case.store.get(child(source)).await.unwrap().unwrap();
    assert_eq!(record.state, v1::JobState::Succeeded as i32);
    operations.stop(true).await;
    case.deploy(operations, false).await;
    assert_eq!(&status(operations).await, expected);
    assert_eq!(&step(operations, source, key).await, expected);
    assert_eq!(&status(operations).await, expected);
    assert_eq!(
        case.store.get(child(source)).await.unwrap().unwrap(),
        record
    );
}

async fn counts(case: &Case) -> (i64, i64, i64, i64, i64, i64) {
    let mut connection = case.numerical_db().await;
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM jobs), (SELECT count(*) FROM model_steps),
                (SELECT count(*) FROM factor_trials), (SELECT count(*) FROM factor_evaluations),
                (SELECT count(*) FROM backtest_results), (SELECT count(*) FROM run_transitions)",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap()
}

#[tokio::test]
async fn discovery_budget() {
    let mut case = Case::open().await;
    let mut operations = case.install_adaptive().await;
    operations.limit_parent(1);
    case.deploy(&mut operations, false).await;
    let discovery = start(&operations).await;
    let stopped = step(&operations, &discovery, "memory.discovery.budget").await;
    assert_eq!(stopped["status"], "budget_exhausted");
    assert_eq!(stopped["revision"], "2");
    assert_eq!(stopped["completed_rounds"], "0");
    assert_eq!(stopped["reserved_steps"], "1");
    assert_eq!(stopped["research"]["stage"], "discovery");
    assert!(stopped["research"]["evaluation_job_id"].is_null());
    assert!(stopped["adaptive"]["family_sha256"].is_string());
    assert_eq!(stopped["adaptive"]["random_draws"], "0");
    restart_replay(
        &mut case,
        &mut operations,
        &discovery,
        "memory.discovery.budget",
        &stopped,
    )
    .await;
    assert_eq!(case.calls(), 1);
    assert_eq!(operations.perturbation_starts(), 0);
    assert_eq!(counts(&case).await, (1, 1, 0, 0, 0, 2));
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn selection_budget() {
    let mut case = Case::open().await;
    let mut operations = case.install_adaptive().await;
    operations.limit_parent(4);
    case.deploy(&mut operations, false).await;
    let selection = first_round(&operations).await;
    assert_eq!(selection["research"]["stage"], "perturbation");
    let stopped = step(&operations, &selection, "memory.selection.budget").await;
    assert_eq!(stopped["status"], "budget_exhausted");
    assert_eq!(stopped["revision"], "5");
    assert_eq!(stopped["completed_rounds"], "1");
    assert_eq!(stopped["reserved_steps"], "4");
    assert_eq!(stopped["research"]["stage"], "perturbation");
    assert!(stopped["research"]["evaluation_job_id"].is_null());
    assert_ne!(stopped["adaptive"]["random_draws"], "0");
    assert_ne!(
        stopped["adaptive"]["state_sha256"],
        selection["adaptive"]["state_sha256"]
    );
    restart_replay(
        &mut case,
        &mut operations,
        &selection,
        "memory.selection.budget",
        &stopped,
    )
    .await;
    assert_eq!(case.calls(), 1);
    assert_eq!(operations.perturbation_starts(), 1);
    assert_eq!(counts(&case).await, (4, 1, 2, 1, 1, 5));
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn coverage_exclusion() {
    let mut case = Case::open().await;
    let mut operations = case.install_memory().await;
    case.window_candidates();
    operations.family_schedule();
    case.deploy(&mut operations, false).await;
    let discovery = first_round(&operations).await;
    assert_eq!(discovery["research"]["stage"], "discovery");
    assert_eq!(discovery["research"]["round"], "2");
    assert_eq!(discovery["adaptive"]["source_count"], "1");
    let mut connection = case.numerical_db().await;
    let bytes: Vec<u8> = sqlx::query_scalar("SELECT evidence_blob FROM factor_evaluations")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    let accepted = crate::store::EvaluationTrial::decode(bytes.as_slice()).unwrap();
    assert_eq!(accepted.disposition, "ready_for_backtest");
    assert_eq!(accepted.minimum_coverage_bps, 6666);
    let result = accepted.result.unwrap();
    assert_eq!(result.valid_observations, 96);
    assert_eq!(result.eligible_observations, 102);
    drop(connection);
    let evaluation = step(&operations, &discovery, "memory.failed.discovery").await;
    assert_eq!(evaluation["research"]["stage"], "evaluation");
    let selection = step(&operations, &evaluation, "memory.failed.evaluation").await;
    assert_eq!(selection["research"]["stage"], "perturbation");
    assert_eq!(selection["research"]["round"], "3");
    assert_eq!(selection["completed_rounds"], "2");
    assert_eq!(selection["adaptive"]["source_count"], "1");
    assert_eq!(selection["adaptive"]["random_draws"], "0");
    let mut connection = case.numerical_db().await;
    let bytes: Vec<u8> =
        sqlx::query_scalar("SELECT evidence_blob FROM factor_evaluations WHERE job_id=$1")
            .bind(child(&evaluation))
            .fetch_one(&mut connection)
            .await
            .unwrap();
    let trial = crate::store::EvaluationTrial::decode(bytes.as_slice()).unwrap();
    assert_eq!(trial.disposition, "insufficient_coverage");
    let result = trial.result.unwrap();
    assert_eq!(result.valid_observations, 0);
    assert!(result.eligible_observations > 0);
    let rejected = result.factor_spec_id.unwrap();
    assert_eq!(
        rejected.value.as_str(),
        evaluation["research"]["factor_spec_id"].as_str().unwrap()
    );
    drop(connection);

    let job = case.store.get(child(&selection)).await.unwrap().unwrap();
    let work = case.selection_work(&job);
    assert_eq!(
        work.failed_factor_ids.as_slice(),
        std::slice::from_ref(&rejected)
    );
    assert_eq!(
        work.observed_factor_ids.as_slice(),
        std::slice::from_ref(
            work.observation
                .as_ref()
                .unwrap()
                .candidate
                .as_ref()
                .unwrap()
                .factor_spec_id
                .as_ref()
                .unwrap()
        )
    );
    let before = work.state.as_ref().unwrap();
    assert!(before.history.is_empty());
    assert!(before.proposed_factor_ids.is_empty());
    assert_eq!(before.random_draws, 0);
    let observation = work.observation.as_ref().unwrap();
    assert_eq!(observation.candidate.as_ref().unwrap().window, 5);
    assert!(observation.net_sharpe.is_finite());
    assert!(work.candidates.iter().any(|candidate| {
        candidate.window == 2 && candidate.factor_spec_id.as_ref() == Some(&rejected)
    }));

    operations.stop(true).await;
    case.deploy(&mut operations, false).await;
    assert_eq!(status(&operations).await, selection);
    let exhausted = step(&operations, &selection, "memory.failed.excluded").await;
    assert_eq!(exhausted["status"], "search_exhausted");
    assert_eq!(exhausted["completed_rounds"], "3");
    assert_eq!(exhausted["maximum_rounds"], "4");
    assert_eq!(exhausted["adaptive"]["random_draws"], "0");
    let record = case.store.get(child(&selection)).await.unwrap().unwrap();
    assert_eq!(record.state, v1::JobState::Succeeded as i32);
    // With no prior proposed candidate and only window five observed, excluding
    // the genuinely rejected window two is the sole reason search is exhausted.
    assert_eq!(case.selection_work(&record), work);
    assert_eq!(counts(&case).await, (6, 2, 3, 2, 1, 7));
    assert_eq!(case.calls(), 2);
    assert_eq!(operations.perturbation_starts(), 1);
    assert_eq!(
        step(&operations, &selection, "memory.failed.excluded").await,
        exhausted
    );
    assert_eq!(operations.perturbation_starts(), 1);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn observed_exclusion() {
    let mut case = Case::open().await;
    let mut operations = case.install_adaptive().await;
    case.window_candidates();
    operations.family_schedule();
    case.deploy(&mut operations, false).await;
    let discovery = first_round(&operations).await;
    assert_eq!(discovery["research"]["stage"], "discovery");
    let evaluation = step(&operations, &discovery, "memory.observed.discovery").await;
    assert_eq!(evaluation["research"]["stage"], "evaluation");
    let portfolio = step(&operations, &evaluation, "memory.observed.evaluation").await;
    assert_eq!(portfolio["research"]["stage"], "portfolio");
    let selection = step(&operations, &portfolio, "memory.observed.portfolio").await;
    assert_eq!(selection["research"]["stage"], "perturbation");
    assert_eq!(selection["adaptive"]["source_count"], "2");
    let record = case.store.get(child(&selection)).await.unwrap().unwrap();
    let work = case.selection_work(&record);
    assert_eq!(
        work.observed_factor_ids,
        work.candidates
            .iter()
            .map(|candidate| candidate.factor_spec_id.clone().unwrap())
            .collect::<Vec<_>>()
    );
    assert!(work.failed_factor_ids.is_empty());
    let before = work.state.as_ref().unwrap();
    assert!(before.history.is_empty());
    assert!(before.proposed_factor_ids.is_empty());
    assert_eq!(
        work.observation
            .as_ref()
            .unwrap()
            .candidate
            .as_ref()
            .unwrap()
            .window,
        2
    );
    operations.stop(true).await;
    case.deploy(&mut operations, false).await;
    assert_eq!(status(&operations).await, selection);
    let exhausted = step(&operations, &selection, "memory.observed.exhausted").await;
    assert_eq!(exhausted["status"], "search_exhausted");
    assert_eq!(exhausted["completed_rounds"], "3");
    assert_eq!(exhausted["maximum_rounds"], "4");
    assert_eq!(exhausted["adaptive"]["random_draws"], "0");
    restart_replay(
        &mut case,
        &mut operations,
        &selection,
        "memory.observed.exhausted",
        &exhausted,
    )
    .await;
    assert_eq!(counts(&case).await, (7, 2, 4, 2, 2, 8));
    assert_eq!(case.calls(), 2);
    assert_eq!(operations.perturbation_starts(), 1);
    operations.stop(false).await;
    case.close().await;
}
