//! Installed primary portfolios, authenticated commands and durable evidence.

use super::*;
use crate::manifests::PortfolioPin;
use crate::runtime::PortfolioExecutor;
use crate::store::FactorRepository;

mod fixture;
mod processes;
mod reconciliation;
use fixture::PortfolioCase;

#[tokio::test]
async fn protected_portfolio_denied() {
    for role in [Role::Research, Role::HoldoutWorker] {
        let running = super::super::Running::start_with(role, true).await;
        let error = running
            .client()
            .await
            .execute_backtest(ExecuteBacktestRequest {
                context: Some(context("protected.portfolio")),
                job_id: running.id(),
                lease_id: Some(LeaseId {
                    value: "lease.unavailable".to_owned(),
                }),
                expected_revision: 1,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::PermissionDenied);
        assert_eq!(std::fs::read_dir(&running.views).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn unconfigured_portfolio_denied() {
    let running = super::super::Running::start(Role::Research).await;
    let job = running.acquire().await;
    assert_eq!(
        running
            .client()
            .await
            .execute_backtest(ExecuteBacktestRequest {
                context: Some(context("unconfigured.portfolio")),
                job_id: running.id(),
                lease_id: job.active_lease.unwrap().lease_id,
                expected_revision: job.revision,
            })
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(std::fs::read_dir(&running.views).unwrap().count(), 0);
}

#[tokio::test]
async fn execute_and_replay() {
    let mut case = PortfolioCase::new().await;
    let request = case.request().await;
    let first = case
        .base
        .client()
        .await
        .execute_backtest(request.clone())
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    assert_eq!(first.state, JobState::Succeeded as i32);
    assert_eq!(first.revision, 3);
    let before = case.base.store.audit_events(0, 100).await.unwrap().len();
    case.restart().await;
    let replay = case
        .base
        .client()
        .await
        .execute_backtest(request)
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(
        case.base.store.audit_events(0, 100).await.unwrap().len(),
        before
    );
    let ledger = case.base.store.trial_ledger(&actor()).await.unwrap();
    assert_eq!(ledger.entries.len(), 2);
    assert!(ledger.entries.iter().all(|entry| entry.attempts == 1));
}

#[tokio::test]
async fn current_export_admission() {
    let case = PortfolioCase::new().await;
    case.base
        .client()
        .await
        .execute_backtest(case.request().await)
        .await
        .unwrap();
    let mut client = case.operator().await;
    let result = client
        .read_backtest(ReadBacktestRequest {
            job_id: case.job.job_id.clone(),
            context_id: case.context_id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .result
        .unwrap();
    assert_eq!(result.engine_version, "authorized-portfolio.1");
    assert!(
        result
            .metrics
            .iter()
            .any(|metric| metric.name == "global_attempts"
                && metric.value.as_ref().unwrap().value == "2")
    );
    let command = case.export_request("portfolio.export");
    let first = client
        .export_backtest(command.clone())
        .await
        .unwrap()
        .into_inner();
    assert!(!first.replayed);
    assert_eq!(first.result.as_ref(), Some(&result));
    let second = client.export_backtest(command).await.unwrap().into_inner();
    assert!(second.replayed);
    for revision in [0, 1] {
        let error = client
            .decide_factor(case.decide_request(revision))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::FailedPrecondition);
        assert_eq!(
            error.message(),
            "independent portfolio reconciliation pending"
        );
    }
    let mut connection = support::connection(&case.base.fixture.directory).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM factor_states")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn imported_success_denied() {
    let case = PortfolioCase::new().await;
    let request = case.request().await;
    let error = case
        .base
        .client()
        .await
        .complete_job(CompleteJobRequest {
            context: Some(context("imported.portfolio")),
            job_id: request.job_id,
            lease_id: request.lease_id,
            expected_revision: request.expected_revision,
            outcome: Some(JobOutcome {
                outcome: Some(job_outcome::Outcome::Success(case.base.fixture.success())),
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    assert_eq!(case.record().await.state, JobState::Leased as i32);
}

#[tokio::test]
async fn spoofed_actor_denied() {
    let case = PortfolioCase::new().await;
    let mut request = case.request().await;
    request.context.as_mut().unwrap().actor = Some(case.operator_actor());
    assert_eq!(
        case.base
            .client()
            .await
            .execute_backtest(request)
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(case.record().await.state, JobState::Leased as i32);
}

#[tokio::test]
async fn cancelled_execution_denied() {
    let case = PortfolioCase::new().await;
    let request = case.request().await;
    case.operator()
        .await
        .cancel_job(CancelJobRequest {
            context: Some(case.operator_context("cancel.portfolio")),
            job_id: case.job.job_id.clone(),
            expected_revision: request.expected_revision,
            reason: "Cancellation must fence the producer".to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(
        case.base
            .client()
            .await
            .execute_backtest(request)
            .await
            .unwrap_err()
            .code(),
        Code::Aborted
    );
    assert_eq!(case.record().await.state, JobState::Cancelled as i32);
}

#[tokio::test]
async fn changed_trials_stale() {
    let mut case = PortfolioCase::new().await;
    let request = case.request().await;
    case.base
        .client()
        .await
        .execute_backtest(request.clone())
        .await
        .unwrap();
    case.add_trial().await;
    assert_eq!(
        case.base
            .store
            .trial_ledger(&actor())
            .await
            .unwrap()
            .entries
            .len(),
        3
    );
    assert_eq!(
        case.base
            .client()
            .await
            .execute_backtest(request)
            .await
            .unwrap_err()
            .code(),
        Code::Aborted
    );
    let error = case
        .operator()
        .await
        .read_backtest(ReadBacktestRequest {
            job_id: case.job.job_id.clone(),
            context_id: case.context_id.clone(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Aborted);
    assert_eq!(error.message(), "global research trial accounting changed");
    let before = case.base.store.audit_events(0, 100).await.unwrap().len();
    let mut operator = case.operator().await;
    assert_eq!(
        operator
            .export_backtest(case.export_request("stale.export"))
            .await
            .unwrap_err()
            .code(),
        Code::Aborted
    );
    assert_eq!(
        operator
            .decide_factor(case.decide_request(0))
            .await
            .unwrap_err()
            .code(),
        Code::Aborted
    );
    assert_eq!(
        case.base.store.audit_events(0, 100).await.unwrap().len(),
        before
    );
    assert_eq!(case.record().await.state, JobState::Succeeded as i32);
}

#[tokio::test]
async fn expired_lease_denied() {
    let case = PortfolioCase::new().await;
    let request = case.request().await;
    case.base.clock.0.store(180_001, Ordering::SeqCst);
    assert_eq!(
        case.base
            .client()
            .await
            .execute_backtest(request)
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    assert!(case.record().await.outcome.is_none());
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), 0);
}

#[tokio::test]
async fn operator_execution_denied() {
    let case = PortfolioCase::new().await;
    let mut request = case.request().await;
    request.context = Some(case.operator_context("operator.execute"));
    assert_eq!(
        case.operator()
            .await
            .execute_backtest(request)
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert!(case.record().await.outcome.is_none());
}

#[tokio::test]
async fn ledger_scope_denied() {
    let mut case = PortfolioCase::new().await;
    case.add_trial().await;
    // Keep the registered trial, revoke its deployment pin. A smaller visible
    // subset must never be returned as a complete global denominator.
    case.jobs.pop();
    case.restart().await;
    assert!(matches!(
        case.base.store.trial_ledger(&actor()).await,
        Err(crate::store::StoreError::AdmissionDenied)
    ));
}

#[tokio::test]
async fn ledger_index_missing() {
    let case = PortfolioCase::new().await;
    let mut connection = support::connection(&case.base.fixture.directory).await;
    // Deliberate corruption in the disposable test schema only.
    sqlx::query("ALTER TABLE factor_trials DISABLE TRIGGER factor_trials_immutable")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("DELETE FROM factor_trials WHERE job_id = 'job.portfolio'")
        .execute(&mut connection)
        .await
        .unwrap();
    assert!(matches!(
        case.base.store.trial_ledger(&actor()).await,
        Err(crate::store::StoreError::Corrupt(
            "global trial index binding"
        ))
    ));
}

#[tokio::test]
async fn cancelled_trial_counted() {
    let mut case = PortfolioCase::new().await;
    case.add_trial().await;
    case.operator()
        .await
        .cancel_job(CancelJobRequest {
            context: Some(case.operator_context("cancel.unstarted")),
            job_id: Some(JobId {
                value: "job.other".to_owned(),
            }),
            expected_revision: 1,
            reason: "Count cancelled work without inventing an empirical loss".to_owned(),
        })
        .await
        .unwrap();
    let ledger = case.base.store.trial_ledger(&actor()).await.unwrap();
    assert_eq!(ledger.entries.len(), 3);
    assert_eq!(
        ledger
            .entries
            .iter()
            .find(|trial| trial.job_id == "job.other")
            .unwrap()
            .attempts,
        1
    );
}

#[tokio::test]
async fn corrupt_result_denied() {
    let case = PortfolioCase::new().await;
    let completed = case
        .base
        .client()
        .await
        .execute_backtest(case.request().await)
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    let Some(job_outcome::Outcome::Success(success)) = completed.outcome.unwrap().outcome else {
        unreachable!()
    };
    let nav = success
        .outputs
        .iter()
        .find(|artifact| {
            artifact
                .schema
                .as_ref()
                .is_some_and(|schema| schema.name == "loop.portfolio_nav")
        })
        .unwrap();
    let path = case
        .output
        .join(&nav.artifact_id.as_ref().unwrap().value[7..]);
    std::fs::write(&path, b"corrupt registered NAV").unwrap();
    let before = case.base.store.audit_events(0, 100).await.unwrap().len();
    assert_eq!(
        case.operator()
            .await
            .read_backtest(ReadBacktestRequest {
                job_id: case.job.job_id.clone(),
                context_id: case.context_id.clone(),
            })
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    assert_eq!(std::fs::read(path).unwrap(), b"corrupt registered NAV");
    assert_eq!(
        case.base.store.audit_events(0, 100).await.unwrap().len(),
        before
    );
    assert_eq!(case.record().await.state, JobState::Succeeded as i32);
}
