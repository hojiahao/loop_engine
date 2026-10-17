//! Installed boundary cases; statistics and selection come from real workers.

use std::process::Output;
use std::time::Duration;

use loop_protocol::wire::v1;
use serde_json::Value;

use super::fixture::{Case, operations::Operations};
use crate::store::{Clock, JobRepository, SystemClock};

fn document(output: Output, operation: &str) -> Value {
    assert!(
        matches!(output.status.code(), Some(0 | 7)),
        "{operation}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "{operation}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema"], "loop.run-cli/v3", "{operation}");
    let expected = match value["run"]["status"].as_str().unwrap() {
        "budget_exhausted" | "infrastructure_failed" | "deadline_exceeded" => 7,
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
                "limits.start",
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

async fn first_round(operations: &Operations) -> Value {
    let discovery = start(operations).await;
    let evaluation = step(operations, &discovery, "limits.discovery").await;
    assert_eq!(evaluation["research"]["stage"], "evaluation");
    let portfolio = step(operations, &evaluation, "limits.evaluation").await;
    assert_eq!(portfolio["research"]["stage"], "portfolio");
    step(operations, &portfolio, "limits.portfolio").await
}

fn child(run: &Value) -> &str {
    run["research"]["current_child"]["job_id"].as_str().unwrap()
}

fn feedback(body: &Value) -> Value {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|message| {
            message["content"]
                .as_str()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
        })
        .find(|value| value["schema"] == "loop.run-feedback-sources/v1")
        .expect("frozen feedback message")
}

#[tokio::test]
async fn search_exhausted() {
    let mut case = Case::open().await;
    let mut operations = case.install_adaptive().await;
    operations.limit_windows();
    case.deploy(&mut operations, false).await;
    let selection = first_round(&operations).await;
    assert_eq!(selection["research"]["stage"], "perturbation");
    let evaluation = step(&operations, &selection, "limits.second.selection").await;
    assert_eq!(evaluation["research"]["stage"], "evaluation");
    let portfolio = step(&operations, &evaluation, "limits.second.evaluation").await;
    let selection = step(&operations, &portfolio, "limits.second.portfolio").await;
    assert_eq!(selection["research"]["stage"], "perturbation");
    assert_eq!(selection["research"]["round"], "3");
    let draws = selection["adaptive"]["random_draws"].clone();
    assert_ne!(draws, "0");
    let exhausted = step(&operations, &selection, "limits.exhausted").await;
    assert_eq!(exhausted["status"], "search_exhausted");
    assert_eq!(exhausted["completed_rounds"], "3");
    assert_eq!(exhausted["maximum_rounds"], "4");
    assert_eq!(exhausted["research"]["round"], "3");
    assert_eq!(exhausted["research"]["disposition"], "exhausted");
    assert_eq!(exhausted["adaptive"]["random_draws"], draws);
    assert_eq!(exhausted["reserved_steps"], "7");
    assert_eq!(case.calls(), 1);
    assert_eq!(operations.perturbation_starts(), 2);
    assert_eq!(
        step(&operations, &selection, "limits.exhausted").await,
        exhausted
    );
    assert_eq!(operations.perturbation_starts(), 2);
    let job = case.store.get(child(&selection)).await.unwrap().unwrap();
    assert_eq!(job.state, v1::JobState::Succeeded as i32);
    let mut connection = case.numerical_db().await;
    let counts: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM jobs), (SELECT count(*) FROM model_steps),
                (SELECT count(*) FROM factor_trials), (SELECT count(*) FROM factor_evaluations),
                (SELECT count(*) FROM backtest_results), (SELECT count(*) FROM run_transitions)",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    // Successful exhaustion is not an Evaluation submission or factor trial.
    assert_eq!(counts, (7, 1, 4, 2, 2, 8));
    drop(connection);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn unavailable_fallback() {
    let mut case = Case::open().await;
    let mut operations = case.install_feedback(false).await;
    case.deploy(&mut operations, false).await;
    let fallback = first_round(&operations).await;
    assert_eq!(fallback["research"]["stage"], "discovery");
    assert_eq!(fallback["research"]["round"], "2");
    assert_eq!(fallback["completed_rounds"], "1");
    assert_eq!(fallback["adaptive"]["source_count"], "1");
    assert_eq!(fallback["adaptive"]["random_draws"], "0");
    assert!(fallback["research"]["perturbation_job_id"].is_null());
    let evaluation = step(&operations, &fallback, "limits.fallback").await;
    assert_eq!(evaluation["research"]["stage"], "evaluation");
    assert_eq!(evaluation["adaptive"]["random_draws"], "0");
    assert_eq!(operations.perturbation_starts(), 0);
    assert_eq!(case.calls(), 2);
    let bodies = case.supplier_bodies();
    let context = feedback(&bodies[1]);
    let sources = context["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["sharpe"]["status"], "unavailable");
    assert!(sources[0]["sharpe"]["value"].is_null());
    assert!(!sources[0]["sharpe"]["reason"].as_str().unwrap().is_empty());
    let mut connection = case.numerical_db().await;
    let selections: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind=8")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(selections, 0);
    drop(connection);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn selection_deadline() {
    let mut case = Case::open().await;
    // Freeze a short total budget before registration; never rewrite an
    // existing deadline or wait out the normal 180s recovery allowance.
    let mut operations = case.install_timed(20).await;
    case.deploy(&mut operations, false).await;
    let selection = first_round(&operations).await;
    assert_eq!(selection["research"]["stage"], "perturbation");
    let job = case.store.get(child(&selection)).await.unwrap().unwrap();
    assert_eq!(job.state, v1::JobState::Queued as i32);
    let specification = job.specification.as_ref().unwrap();
    let submitted = specification.submitted_at.as_ref().unwrap();
    let Some(v1::job_specification::Input::Perturbation(input)) = &specification.input else {
        panic!("actual perturbation input")
    };
    let wall = input
        .budget
        .as_ref()
        .unwrap()
        .maximum_wall_time
        .as_ref()
        .unwrap();
    let deadline = (submitted.seconds + wall.seconds) * 1000
        + i64::from(submitted.nanos + wall.nanos) / 1_000_000;
    let remaining = (deadline - SystemClock.now_millis().unwrap()).max(0) as u64;
    assert!(remaining <= 20_000);
    // The installed daemon uses its own real clock. Do not mutate fixture time
    // or database deadlines to simulate a condition it has not observed.
    tokio::time::sleep(Duration::from_millis(remaining)).await;
    let stopped = step(&operations, &selection, "limits.deadline").await;
    assert_eq!(stopped["status"], "budget_exhausted");
    assert_eq!(stopped["completed_rounds"], "1");
    assert_eq!(stopped["research"]["round"], "2");
    assert_eq!(stopped["adaptive"], selection["adaptive"]);
    assert_eq!(stopped["reserved_steps"], "4");
    assert_eq!(operations.perturbation_starts(), 0);
    assert_eq!(case.calls(), 1);
    let job = case.store.get(child(&selection)).await.unwrap().unwrap();
    assert_eq!(job.state, v1::JobState::BudgetExhausted as i32);
    assert_eq!(job.attempt, 0);
    let Some(v1::job_outcome::Outcome::BudgetExhaustion(failure)) =
        job.outcome.as_ref().unwrap().outcome.as_ref()
    else {
        panic!("queued deadline must retain its budget outcome")
    };
    assert_eq!(failure.exhausted_limit, "maximum_wall_time");
    assert_eq!(failure.enforced_budget, input.budget);
    let mut connection = case.numerical_db().await;
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM jobs), (SELECT count(*) FROM factor_trials),
                (SELECT count(*) FROM run_transitions)",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(counts, (4, 2, 5));
    drop(connection);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn selection_cancelled() {
    let mut case = Case::open().await;
    let mut operations = case.install_adaptive().await;
    case.deploy(&mut operations, false).await;
    let selection = first_round(&operations).await;
    assert_eq!(selection["research"]["stage"], "perturbation");
    let job = case.store.get(child(&selection)).await.unwrap().unwrap();
    assert_eq!(job.state, v1::JobState::Queued as i32);
    let cancelled = case.cancel_selection(&job).await;
    assert_eq!(cancelled.state, v1::JobState::Cancelled as i32);
    assert_eq!(cancelled.attempt, 0);
    let stopped = step(&operations, &selection, "limits.cancelled").await;
    assert_eq!(stopped["status"], "infrastructure_failed");
    assert_eq!(stopped["completed_rounds"], "1");
    assert_eq!(stopped["research"]["round"], "2");
    assert_eq!(stopped["adaptive"], selection["adaptive"]);
    assert_eq!(stopped["research"]["disposition"], "pending");
    assert_eq!(stopped["reserved_steps"], "4");
    assert_eq!(operations.perturbation_starts(), 0);
    assert_eq!(case.calls(), 1);
    assert_eq!(
        case.store.get(child(&selection)).await.unwrap().unwrap(),
        cancelled
    );
    let mut connection = case.numerical_db().await;
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM jobs), (SELECT count(*) FROM factor_trials),
                (SELECT count(*) FROM run_transitions)",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(counts, (4, 2, 5));
    drop(connection);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn running_cancelled() {
    let mut case = Case::open().await;
    let mut operations = case.install_adaptive().await;
    operations.hold_selection();
    case.deploy(&mut operations, false).await;
    let selection = first_round(&operations).await;
    assert_eq!(selection["research"]["stage"], "perturbation");
    let command = operations.spawn_run(&[
        "--workflow-version",
        "3",
        "--timeout-seconds",
        "240",
        "step",
        "--run",
        "run.discovery",
        "--revision",
        selection["revision"].as_str().unwrap(),
        "--key",
        "limits.running",
    ]);
    let worker = operations.held_worker().await;
    let leased = case.store.get(child(&selection)).await.unwrap().unwrap();
    assert_eq!(leased.state, v1::JobState::Leased as i32);
    assert_eq!(leased.attempt, 1);
    assert_eq!(operations.perturbation_starts(), 1);
    let cancelled = case.cancel_selection(&leased).await;
    assert_eq!(cancelled.state, v1::JobState::Cancelled as i32);
    let output = tokio::time::timeout(Duration::from_secs(5), command.wait_with_output())
        .await
        .expect("cancelled worker handler must finish promptly")
        .unwrap();
    // The running request loses its exact lease. A later explicit parent step
    // observes cancellation; no successful worker receipt may be published.
    assert_eq!(output.status.code(), Some(4));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["category"], "conflict");
    worker.wait_exit().await;
    let stopped = step(&operations, &selection, "limits.running.stopped").await;
    assert_eq!(stopped["status"], "infrastructure_failed");
    assert_eq!(stopped["completed_rounds"], "1");
    assert_eq!(stopped["adaptive"], selection["adaptive"]);
    assert_eq!(stopped["reserved_steps"], "4");
    assert_eq!(operations.perturbation_starts(), 1);
    assert_eq!(case.calls(), 1);
    assert_eq!(
        case.store.get(child(&selection)).await.unwrap().unwrap(),
        cancelled
    );
    let mut connection = case.numerical_db().await;
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM jobs), (SELECT count(*) FROM factor_trials),
                (SELECT count(*) FROM run_transitions)",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(counts, (4, 2, 5));
    drop(connection);
    operations.stop(false).await;
    case.close().await;
}
