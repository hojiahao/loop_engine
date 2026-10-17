//! Installed three-round feedback and leased perturbation; no numerical mocks.

use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use loop_protocol::wire::v1;
use serde_json::Value;
use tokio::sync::Barrier;
use tokio::task::JoinSet;

use super::fixture::{Case, operations::Operations};
use crate::store::{Clock, JobRepository, PgJobStore, StoreOptions, SystemClock};

fn document(output: Output, operation: &str) -> Value {
    assert!(
        output.status.success(),
        "{operation}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{operation}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema"], "loop.run-cli/v3", "{operation}");
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
                "adaptive.start",
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
async fn adaptive_roundtrip() {
    let mut case = Case::open().await;
    let mut operations = case.install_adaptive().await;
    case.deploy(&mut operations, false).await;
    let discovery = start(&operations).await;
    assert_eq!(start(&operations).await, discovery);
    assert_eq!(discovery["adaptive"]["source_count"], "0");
    let first_evaluation = step(&operations, &discovery, "adaptive.first.discovery").await;
    assert_eq!(first_evaluation["research"]["stage"], "evaluation");
    let first_portfolio = step(&operations, &first_evaluation, "adaptive.first.evaluation").await;
    assert_eq!(first_portfolio["research"]["stage"], "portfolio");
    let perturbation = step(&operations, &first_portfolio, "adaptive.first.portfolio").await;
    assert_eq!(perturbation["research"]["stage"], "perturbation");
    assert_eq!(perturbation["research"]["round"], "2");
    assert_eq!(perturbation["adaptive"]["source_count"], "1");
    assert!(perturbation["research"]["discovery_job_id"].is_null());
    assert_eq!(case.calls(), 1);
    assert_eq!(operations.perturbation_starts(), 0);
    operations.stop(true).await;
    case.deploy(&mut operations, false).await;
    assert_eq!(status(&operations).await, perturbation);
    assert_eq!(
        step(&operations, &first_portfolio, "adaptive.first.portfolio").await,
        perturbation
    );

    let second_evaluation = step(&operations, &perturbation, "adaptive.perturbation").await;
    assert_eq!(second_evaluation["research"]["stage"], "evaluation");
    assert_ne!(
        second_evaluation["research"]["factor_spec_id"],
        first_evaluation["research"]["factor_spec_id"]
    );
    assert!(
        second_evaluation["adaptive"]["random_draws"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    assert_eq!(case.calls(), 1);
    let selection = case.store.get(child(&perturbation)).await.unwrap().unwrap();
    assert_eq!(selection.state, v1::JobState::Succeeded as i32);
    assert_eq!(
        selection.specification.as_ref().unwrap().kind,
        v1::JobKind::Perturbation as i32
    );
    let Some(v1::job_outcome::Outcome::Success(receipt)) =
        selection.outcome.as_ref().unwrap().outcome.as_ref()
    else {
        panic!("perturbation success")
    };
    assert_eq!(receipt.outputs.len(), 2);
    let state = second_evaluation["adaptive"]["state_sha256"].clone();
    operations.stop(true).await;
    case.deploy(&mut operations, false).await;
    assert_eq!(status(&operations).await, second_evaluation);
    assert_eq!(
        step(&operations, &perturbation, "adaptive.perturbation").await,
        second_evaluation
    );
    assert_eq!(status(&operations).await["adaptive"]["state_sha256"], state);
    assert_eq!(operations.perturbation_starts(), 1);

    let second_portfolio = step(
        &operations,
        &second_evaluation,
        "adaptive.second.evaluation",
    )
    .await;
    let third_discovery = step(&operations, &second_portfolio, "adaptive.second.portfolio").await;
    assert_eq!(third_discovery["research"]["stage"], "discovery");
    assert_eq!(third_discovery["research"]["round"], "3");
    assert_eq!(third_discovery["adaptive"]["source_count"], "2");
    let third_evaluation = step(&operations, &third_discovery, "adaptive.third.discovery").await;
    assert_eq!(third_evaluation["research"]["stage"], "evaluation");
    assert_eq!(case.calls(), 2);
    let bodies = case.supplier_bodies();
    assert_eq!(feedback(&bodies[0])["sources"].as_array().unwrap().len(), 0);
    let context = feedback(&bodies[1]);
    let sources = context["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0]["job_id"], child(&first_portfolio));
    assert_eq!(sources[1]["job_id"], child(&second_portfolio));
    assert!(
        sources
            .iter()
            .all(|source| source["sharpe"]["value"].is_number())
    );
    let third_portfolio = step(&operations, &third_evaluation, "adaptive.third.evaluation").await;
    let completed = step(&operations, &third_portfolio, "adaptive.third.portfolio").await;
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["completed_rounds"], "3");
    assert_eq!(completed["reserved_steps"], "9");
    assert_eq!(completed["reserved_input_tokens"], "8192");
    assert_eq!(case.calls(), 2);
    let mut connection = case.numerical_db().await;
    let counts: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM jobs), (SELECT count(*) FROM model_steps),
                (SELECT count(*) FROM factor_trials), (SELECT count(*) FROM factor_evaluations),
                (SELECT count(*) FROM backtest_results), (SELECT count(*) FROM run_transitions)",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(counts, (9, 2, 6, 3, 3, 10));
    let perturbation_steps: i64 =
        sqlx::query_scalar("SELECT count(*) FROM model_steps WHERE job_id=$1")
            .bind(child(&perturbation))
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!(perturbation_steps, 0);
    drop(connection);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn corrupt_feedback() {
    let mut case = Case::open().await;
    let mut operations = case.install_adaptive().await;
    case.deploy(&mut operations, false).await;
    let discovery = start(&operations).await;
    case.corrupt_evidence(discovery["adaptive"]["feedback_sha256"].as_str().unwrap());
    let output = operations
        .run_cli(&[
            "--workflow-version",
            "3",
            "step",
            "--run",
            "run.discovery",
            "--revision",
            "1",
            "--key",
            "corrupt.feedback",
        ])
        .await;
    assert_eq!(output.status.code(), Some(6));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["schema"], "loop.run-cli/v3");
    assert_eq!(error["error"]["category"], "transport");
    assert_eq!(case.calls(), 0);
    assert_eq!(operations.perturbation_starts(), 0);
    let child = case.store.get(child(&discovery)).await.unwrap().unwrap();
    assert_eq!(child.state, v1::JobState::Queued as i32);
    assert_eq!(child.revision, 1);
    let mut connection = case.numerical_db().await;
    let counts: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM jobs), (SELECT count(*) FROM model_steps),
                (SELECT count(*) FROM factor_trials), (SELECT count(*) FROM backtest_results),
                (SELECT count(*) FROM run_transitions)",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(counts, (1, 0, 0, 0, 1));
    drop(connection);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn generic_bypass() {
    // The controlled plan would first ask the Provider for a tool call. Both
    // direct Execute and queued-Pause/Resume must be denied before that call.
    let mut case = Case::controlled().await;
    let mut operations = case.install_adaptive().await;
    case.deploy(&mut operations, false).await;
    let discovery = start(&operations).await;
    let id = child(&discovery);
    let denied = operations
        .cli(&[
            "execute",
            "--job",
            id,
            "--revision",
            "1",
            "--key",
            "bypass.execute",
        ])
        .await;
    assert_denied(denied);
    let paused = operations
        .cli(&[
            "pause",
            "--job",
            id,
            "--revision",
            "1",
            "--key",
            "bypass.pause",
        ])
        .await;
    assert!(paused.status.success());
    let paused: Value = serde_json::from_slice(&paused.stdout).unwrap();
    assert_eq!(paused["job"]["status"], "paused");
    let revision = paused["job"]["revision"].as_str().unwrap();
    for action in ["resume", "reconcile"] {
        let key = format!("bypass.{action}");
        assert_denied(
            operations
                .cli(&[action, "--job", id, "--revision", revision, "--key", &key])
                .await,
        );
    }
    let observed = operations.cli(&["status", "--job", id]).await;
    assert!(observed.status.success());
    let observed: Value = serde_json::from_slice(&observed.stdout).unwrap();
    assert_eq!(observed["job"]["status"], "paused");
    assert_eq!(observed["job"]["revision"], revision);
    assert!(observed["step"].get("candidate").is_none());
    assert_eq!(case.calls(), 0);
    assert_eq!(operations.perturbation_starts(), 0);
    let mut connection = case.numerical_db().await;
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM model_steps), (SELECT count(*) FROM factor_trials),
                (SELECT count(*) FROM run_transitions)",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0, 1));
    drop(connection);
    operations.stop(false).await;
    case.close().await;
}

fn assert_denied(output: Output) {
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(value["schema"], "loop.discovery-cli/v1");
    assert_eq!(value["error"]["category"], "authorization");
}

async fn competing_steps(operations: &Arc<Operations>, writers: usize) -> Vec<Output> {
    let barrier = Arc::new(Barrier::new(writers + 1));
    let mut tasks = JoinSet::new();
    for writer in 0..writers {
        let operations = Arc::clone(operations);
        let barrier = Arc::clone(&barrier);
        tasks.spawn(async move {
            let key = format!("adaptive.race.{writer}");
            barrier.wait().await;
            // Each task launches an independent installed loopctl process with
            // its own idempotency key against the exact same parent revision.
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
                    "4",
                    "--key",
                    &key,
                ])
                .await
        });
    }
    barrier.wait().await;
    let mut outputs = Vec::with_capacity(writers);
    while let Some(output) = tasks.join_next().await {
        outputs.push(output.unwrap());
    }
    outputs
}

#[tokio::test]
async fn adaptive_writers() {
    for writers in [2, 4, 8] {
        let mut case = Case::open().await;
        let mut operations = case.install_adaptive().await;
        case.deploy(&mut operations, false).await;
        let discovery = start(&operations).await;
        let evaluation = step(&operations, &discovery, "race.discovery").await;
        let portfolio = step(&operations, &evaluation, "race.evaluation").await;
        let perturbation = step(&operations, &portfolio, "race.portfolio").await;
        assert_eq!(perturbation["research"]["stage"], "perturbation");
        assert_eq!(perturbation["revision"], "4");
        assert_eq!(operations.perturbation_starts(), 0);
        let operations = Arc::new(operations);
        let outputs = competing_steps(&operations, writers).await;
        assert_eq!(outputs.len(), writers);
        let mut successes = Vec::new();
        let mut failures = Vec::new();
        for output in outputs {
            if output.status.success() {
                successes.push(document(output, "concurrent perturbation"));
            } else {
                assert!(matches!(output.status.code(), Some(4 | 6)));
                assert!(output.stdout.is_empty());
                let error: Value = serde_json::from_slice(&output.stderr).unwrap();
                assert_eq!(error["schema"], "loop.run-cli/v3");
                assert!(matches!(
                    error["error"]["category"].as_str(),
                    Some("conflict" | "transport" | "remote_cancelled")
                ));
                failures.push(error);
            }
        }
        assert_eq!(
            successes.len(),
            1,
            "{writers} writers; worker starts={}; errors={failures:?}; runtime={}",
            operations.perturbation_starts(),
            operations.runtime_log()
        );
        let candidate = successes.pop().unwrap();
        assert_eq!(candidate["revision"], "5");
        assert_eq!(candidate["research"]["stage"], "evaluation");
        assert_eq!(candidate["research"]["round"], "2");
        assert_eq!(candidate["completed_rounds"], "1");
        assert_eq!(candidate["reserved_steps"], "5");
        assert_eq!(candidate["reserved_input_tokens"], "4096");
        assert_eq!(candidate["adaptive"]["source_count"], "1");
        assert_eq!(status(&operations).await, candidate);
        assert_eq!(operations.perturbation_starts(), 1);
        assert_eq!(case.calls(), 1);
        let next = case.store.get(child(&candidate)).await.unwrap().unwrap();
        assert_eq!(next.state, v1::JobState::Queued as i32);
        assert_eq!(next.revision, 1);
        let mut connection = case.numerical_db().await;
        let counts: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM jobs), (SELECT count(*) FROM model_steps),
                    (SELECT count(*) FROM factor_trials), (SELECT count(*) FROM factor_evaluations),
                    (SELECT count(*) FROM backtest_results), (SELECT count(*) FROM run_transitions)",
        )
        .fetch_one(&mut connection)
        .await
        .unwrap();
        assert_eq!(counts, (5, 1, 3, 1, 1, 5), "{writers} writers");
        let selection_steps: i64 =
            sqlx::query_scalar("SELECT count(*) FROM model_steps WHERE job_id=$1")
                .bind(child(&perturbation))
                .fetch_one(&mut connection)
                .await
                .unwrap();
        assert_eq!(selection_steps, 0);
        let trial_job: String =
            sqlx::query_scalar("SELECT job_id FROM factor_trials WHERE job_id=$1")
                .bind(child(&candidate))
                .fetch_one(&mut connection)
                .await
                .unwrap();
        assert_eq!(trial_job, child(&candidate));
        drop(connection);
        let mut operations = Arc::try_unwrap(operations)
            .unwrap_or_else(|_| panic!("all competing CLI processes must be joined"));
        operations.stop(false).await;
        case.close().await;
    }
}

#[test]
#[ignore = "launched only by independent adaptive commit/crash acceptance"]
fn fault_server() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let config =
                std::path::PathBuf::from(std::env::var_os("LOOP_ADAPTIVE_RUNTIME").unwrap());
            let database =
                std::path::PathBuf::from(std::env::var_os("LOOP_ADAPTIVE_DATABASE").unwrap());
            let runtime = crate::runtime::RuntimeDeployment::load(&config).unwrap();
            let url = std::fs::read_to_string(database).unwrap();
            let mut options = StoreOptions::new(url.trim()).unwrap();
            options.schema = std::env::var("LOOP_ADAPTIVE_SCHEMA").unwrap();
            options.admission = runtime.authority.clone();
            options.backtest_policy = runtime.portfolio.clone().unwrap();
            let store = PgJobStore::open(options).await.unwrap();
            store.verify_configuration().await.unwrap();
            let tls = runtime.tls();
            let listener = tokio::net::TcpListener::bind(runtime.bind).await.unwrap();
            let service = crate::runtime::RuntimeService::new(
                store.clone(),
                runtime.authority,
                runtime.artifacts,
            )
            .with_factor_executor(runtime.evaluator.unwrap())
            .with_portfolio_executor(runtime.portfolio.unwrap())
            .with_discoverer(runtime.discovery.unwrap())
            .with_runs(runtime.runs.unwrap());
            crate::runtime::serve(service, listener, tls, std::future::pending())
                .await
                .unwrap();
            store.close().await;
        });
}

async fn selection_fixture() -> (Case, Operations, Value) {
    let mut case = Case::open().await;
    let mut operations = case.install_adaptive().await;
    case.deploy(&mut operations, false).await;
    let discovery = start(&operations).await;
    let evaluation = step(&operations, &discovery, "fault.discovery").await;
    let portfolio = step(&operations, &evaluation, "fault.evaluation").await;
    let selection = step(&operations, &portfolio, "fault.portfolio").await;
    assert_eq!(selection["research"]["stage"], "perturbation");
    assert_eq!(selection["revision"], "4");
    assert_eq!(selection["adaptive"]["random_draws"], "0");
    (case, operations, selection)
}

async fn wait_fault(operations: &Operations) {
    let marker = operations.fault_marker();
    tokio::time::timeout(Duration::from_secs(120), async {
        while !marker.is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("adaptive commit fault marker deadline");
}

async fn crash_step(case: &mut Case, operations: &mut Operations, point: &str) {
    case.deploy_fault(operations, point).await;
    let command = operations.spawn_run(&[
        "--workflow-version",
        "3",
        "--timeout-seconds",
        "240",
        "step",
        "--run",
        "run.discovery",
        "--revision",
        "4",
        "--key",
        "fault.perturbation",
    ]);
    wait_fault(operations).await;
    assert_eq!(operations.perturbation_starts(), 1);
    operations.stop(true).await;
    let output = tokio::time::timeout(Duration::from_secs(10), command.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[tokio::test]
async fn selection_commit_recovery() {
    for point in [
        "perturbation_after_commit",
        "run_step_before",
        "run_step_after",
    ] {
        let (mut case, mut operations, selection) = selection_fixture().await;
        crash_step(&mut case, &mut operations, point).await;
        let saved = case.store.get(child(&selection)).await.unwrap().unwrap();
        assert_eq!(saved.state, v1::JobState::Succeeded as i32);
        case.deploy(&mut operations, false).await;
        let current = status(&operations).await;
        if point == "run_step_after" {
            assert_eq!(current["revision"], "5");
        } else {
            assert_eq!(current["revision"], "4");
            assert_eq!(current["adaptive"]["random_draws"], "0");
        }
        let evaluation = step(&operations, &selection, "fault.perturbation").await;
        assert_eq!(evaluation["revision"], "5");
        assert_eq!(evaluation["research"]["stage"], "evaluation");
        assert_eq!(status(&operations).await, evaluation);
        assert_eq!(
            step(&operations, &selection, "fault.perturbation").await,
            evaluation
        );
        assert_ne!(
            evaluation["adaptive"]["state_sha256"],
            selection["adaptive"]["state_sha256"]
        );
        assert_eq!(operations.perturbation_starts(), 1);
        assert_eq!(case.calls(), 1);
        assert_eq!(
            case.store.get(child(&selection)).await.unwrap().unwrap(),
            saved
        );
        let mut connection = case.numerical_db().await;
        let counts: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM jobs), (SELECT count(*) FROM factor_trials),
                    (SELECT count(*) FROM run_transitions), (SELECT count(*) FROM model_steps)",
        )
        .fetch_one(&mut connection)
        .await
        .unwrap();
        assert_eq!(counts, (5, 3, 5, 1), "{point}");
        drop(connection);
        operations.stop(false).await;
        case.close().await;
    }
}

#[tokio::test]
async fn selection_commit_rollback() {
    let (mut case, mut operations, selection) = selection_fixture().await;
    crash_step(&mut case, &mut operations, "perturbation_before_commit").await;
    let record = case.store.get(child(&selection)).await.unwrap().unwrap();
    assert_eq!(record.state, v1::JobState::Leased as i32);
    assert!(record.outcome.is_none());
    let lease = record
        .active_lease
        .as_ref()
        .unwrap()
        .expires_at
        .as_ref()
        .unwrap();
    let remaining = (lease.seconds * 1000 + i64::from(lease.nanos) / 1_000_000
        - SystemClock.now_millis().unwrap())
    .max(0) as u64;
    assert!(remaining <= 20_000);
    tokio::time::sleep(Duration::from_millis(remaining + 20)).await;
    case.deploy(&mut operations, false).await;
    let specification = record.specification.as_ref().unwrap();
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
    assert!(SystemClock.now_millis().unwrap() < deadline);
    let output = operations
        .run_cli(&[
            "--workflow-version",
            "3",
            "--timeout-seconds",
            "240",
            "step",
            "--run",
            "run.discovery",
            "--revision",
            "4",
            "--key",
            "fault.perturbation",
        ])
        .await;
    assert_eq!(output.status.code(), Some(7));
    assert!(output.stderr.is_empty());
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    let current = &document["run"];
    assert_eq!(current["status"], "infrastructure_failed");
    assert_eq!(current["revision"], "5");
    assert_eq!(current["completed_rounds"], "1");
    assert_eq!(
        current["adaptive"]["state_sha256"],
        selection["adaptive"]["state_sha256"]
    );
    assert_eq!(current["adaptive"]["random_draws"], "0");
    assert_eq!(operations.perturbation_starts(), 1);
    assert_eq!(case.calls(), 1);
    let recovered = case.store.get(child(&selection)).await.unwrap().unwrap();
    assert_eq!(recovered.state, v1::JobState::InfrastructureFailed as i32);
    let Some(v1::job_outcome::Outcome::InfrastructureFailure(failure)) =
        recovered.outcome.as_ref().unwrap().outcome.as_ref()
    else {
        panic!("expired lease must retain its infrastructure outcome")
    };
    assert_eq!(failure.error.as_ref().unwrap().code, "lease_expired");
    assert_eq!(failure.attempt, 1);
    let mut connection = case.numerical_db().await;
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM jobs), (SELECT count(*) FROM factor_trials),
                (SELECT count(*) FROM run_transitions), (SELECT count(*) FROM backtest_results)",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(counts, (4, 2, 5, 1));
    drop(connection);
    operations.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn selection_history_corruption() {
    let (mut case, mut operations, selection) = selection_fixture().await;
    let evaluation = step(&operations, &selection, "history.perturbation").await;
    let record = case.store.get(child(&selection)).await.unwrap().unwrap();
    let Some(v1::job_outcome::Outcome::Success(success)) = record.outcome.unwrap().outcome else {
        panic!("successful selection receipt")
    };
    let receipt = success
        .outputs
        .iter()
        .find(|output| {
            output
                .schema
                .as_ref()
                .is_some_and(|schema| schema.name == "loop.perturbation_receipt")
        })
        .unwrap();
    case.corrupt_evidence(&receipt.artifact_id.as_ref().unwrap().value);
    operations.stop(true).await;
    case.deploy(&mut operations, false).await;
    let output = operations
        .run_cli(&[
            "--workflow-version",
            "3",
            "--timeout-seconds",
            "240",
            "step",
            "--run",
            "run.discovery",
            "--revision",
            "5",
            "--key",
            "history.evaluation",
        ])
        .await;
    assert_eq!(output.status.code(), Some(6));
    assert!(output.stdout.is_empty());
    let record = case.store.get(child(&evaluation)).await.unwrap().unwrap();
    assert_eq!(record.state, v1::JobState::Queued as i32);
    assert_eq!(record.revision, 1);
    assert_eq!(operations.perturbation_starts(), 1);
    assert_eq!(case.calls(), 1);
    let mut connection = case.numerical_db().await;
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM jobs), (SELECT count(*) FROM factor_trials),
                (SELECT count(*) FROM run_transitions), (SELECT count(*) FROM factor_evaluations)",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(counts, (5, 3, 5, 1));
    drop(connection);
    operations.stop(false).await;
    case.close().await;
}
