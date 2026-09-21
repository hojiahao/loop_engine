//! Real global-report evidence through independent replay and shared admission.

use super::*;
use crate::manifests::reconciliation::{RegisteredStatistics, ValidationEvidence};
use crate::runtime::{ReconciliationExecutor, StatisticsExecutor, StatisticsTask, ValidationTask};
use crate::store::{JobMutation, StoreError};

async fn bound_case(multiple: bool, complete: bool) -> ValidationCase {
    let report = super::super::statistics::ReportCase::new(multiple).await;
    report.run_portfolios().await;
    if complete {
        report
            .client()
            .await
            .execute_statistics(report.request().await)
            .await
            .unwrap();
    }
    ValidationCase::attach(report.portfolio, Some("job.statistics".to_owned())).await
}

async fn complete(case: &ValidationCase) -> JobRecord {
    case.client()
        .await
        .execute_reconciliation(case.request().await)
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap()
}

#[tokio::test]
async fn bound_replay() {
    let mut case = bound_case(true, true).await;
    let first = complete(&case).await;
    let document = case.document(&first);
    assert_eq!(document.schema, "loop.authorized-reconciliation/v2");
    let binding = document.global_statistics.as_ref().unwrap();
    assert!(binding.available, "{binding:?}");
    assert_eq!(binding.job_id, "job.statistics");
    assert!(binding.strategy_binding.is_some());
    assert!(!document.production_eligible);
    assert!(
        !document
            .admission_prerequisites
            .iter()
            .any(|reason| reason == "global_synchronous_trial_returns")
    );
    let events = case
        .portfolio
        .base
        .store
        .audit_events(0, 100)
        .await
        .unwrap();
    let files = std::fs::read_dir(&case.output).unwrap().count();
    case.portfolio.restart().await;
    let current = case
        .portfolio
        .operator()
        .await
        .read_reconciliation(ReadReconciliationRequest {
            job_id: case.job.job_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(current.job, Some(first));
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), files);
    assert_eq!(
        case.portfolio
            .base
            .store
            .audit_events(0, 100)
            .await
            .unwrap(),
        events
    );
}

#[tokio::test]
async fn bound_admission_denied() {
    let case = bound_case(true, true).await;
    let record = complete(&case).await;
    assert!(case.document(&record).global_statistics.unwrap().available);
    let events = case
        .portfolio
        .base
        .store
        .audit_events(0, 100)
        .await
        .unwrap();
    for force in [false, true] {
        let mut request = case.decision(0);
        if force {
            request.context = Some(case.portfolio.operator_context("statistics.force"));
            request.override_reason =
                "Statistical availability does not certify production data".to_owned();
            request.override_approval_id = "approval.nonexistent".to_owned();
        }
        let error = case
            .portfolio
            .operator()
            .await
            .decide_factor(request)
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::FailedPrecondition);
        assert!(
            error.message().contains("production admission requires"),
            "{error}"
        );
    }
    assert_eq!(
        case.portfolio
            .base
            .store
            .audit_events(0, 100)
            .await
            .unwrap(),
        events
    );
}

#[tokio::test]
async fn unavailable_statistics_denied() {
    let case = bound_case(false, true).await;
    let record = complete(&case).await;
    let document = case.document(&record);
    assert!(document.disposition == crate::manifests::reconciliation::Disposition::Accepted);
    assert!(!document.global_statistics.unwrap().available);
    let error = case
        .portfolio
        .operator()
        .await
        .decide_factor(case.decision(0))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert_eq!(error.message(), "global statistical evidence unavailable");
}

#[tokio::test]
async fn pending_report_denied() {
    let case = bound_case(false, false).await;
    let error = case
        .client()
        .await
        .execute_reconciliation(case.request().await)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert_eq!(error.message(), "global statistical evidence pending");
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), 0);
}

#[tokio::test]
async fn stale_report_denied() {
    let mut case = bound_case(false, true).await;
    case.portfolio.add_trial().await;
    let error = case
        .client()
        .await
        .execute_reconciliation(case.request().await)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Aborted, "{error}");
    assert_eq!(error.message(), "global research trial accounting changed");
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), 0);
}

async fn reconstructed(
    case: &ValidationCase,
    request: &ExecuteReconciliationRequest,
) -> Arc<ValidationEvidence> {
    let portfolio = &case.portfolio;
    let mut pins = vec![portfolio.pin.clone()];
    pins.extend(portfolio.extra_pins.clone());
    let primary = Arc::new(
        PortfolioExecutor::open(
            &python(),
            &portfolio.base.fixture.root,
            &portfolio.output,
            pins,
            portfolio.base.broker.clone(),
        )
        .unwrap(),
    );
    let snapshot = portfolio.base.store.trial_snapshot(&actor()).await.unwrap();
    let mut sources = Vec::new();
    for record in &snapshot.records {
        if record.specification.as_ref().unwrap().kind != JobKind::Backtest as i32 {
            continue;
        }
        let Some(job_outcome::Outcome::Success(success)) =
            record.outcome.as_ref().unwrap().outcome.as_ref()
        else {
            panic!("registered primary required")
        };
        let inputs = primary
            .inputs(record.specification.as_ref().unwrap())
            .await
            .unwrap();
        sources.push((
            record.clone(),
            Arc::new(
                primary
                    .replay(inputs, record.specification.as_ref().unwrap(), success)
                    .await
                    .unwrap(),
            ),
        ));
    }
    let (source, prepared) = sources
        .iter()
        .find(|(record, _)| record.specification.as_ref().unwrap().job_id == portfolio.job.job_id)
        .unwrap()
        .clone();
    let report = portfolio
        .base
        .store
        .get("job.statistics")
        .await
        .unwrap()
        .unwrap();
    let Some(job_outcome::Outcome::Success(success)) =
        report.outcome.as_ref().unwrap().outcome.as_ref()
    else {
        panic!("registered report required")
    };
    let executor =
        StatisticsExecutor::open(portfolio.statistics.clone().unwrap(), primary.clone()).unwrap();
    let original = executor.identity(success).await.unwrap();
    let proof = Arc::new(
        executor
            .execute(StatisticsTask {
                job: report.specification.as_ref().unwrap(),
                lease: &original.lease_id,
                started_ms: original.started_at_ms,
                snapshot,
                portfolios: sources,
                prior: Some(success),
            })
            .await
            .unwrap(),
    );
    let record = portfolio
        .base
        .store
        .get("job.validation")
        .await
        .unwrap()
        .unwrap();
    let started_ms = crate::store::timestamp_millis(
        record
            .active_lease
            .as_ref()
            .unwrap()
            .issued_at
            .as_ref()
            .unwrap(),
        false,
    )
    .unwrap();
    Arc::new(
        ReconciliationExecutor::open(portfolio.validation.clone().unwrap(), primary)
            .unwrap()
            .execute(
                ValidationTask {
                    job: &case.job,
                    lease: &request.lease_id.as_ref().unwrap().value,
                    record: source,
                    primary: prepared,
                    prior: None,
                    started_ms,
                    statistics: Some(RegisteredStatistics {
                        record: report,
                        proof,
                    }),
                },
                Duration::from_secs(180),
            )
            .await
            .unwrap(),
    )
}

#[tokio::test]
async fn snapshot_fences_validation() {
    let mut case = bound_case(false, true).await;
    let request = case.request().await;
    let proof = reconstructed(&case, &request).await;
    let success = proof.success().unwrap();
    let before = case
        .portfolio
        .base
        .store
        .get("job.validation")
        .await
        .unwrap();
    case.portfolio.add_trial().await;
    let events = case
        .portfolio
        .base
        .store
        .audit_events(0, 100)
        .await
        .unwrap();
    let error = case
        .portfolio
        .base
        .store
        .with_validation_evidence(proof)
        .mutate(
            &actor(),
            JobMutation::Complete(CompleteJobRequest {
                context: request.context,
                job_id: request.job_id,
                lease_id: request.lease_id,
                expected_revision: request.expected_revision,
                outcome: Some(JobOutcome {
                    outcome: Some(job_outcome::Outcome::Success(success)),
                }),
            }),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, StoreError::StaleTrials), "{error}");
    assert_eq!(
        case.portfolio
            .base
            .store
            .get("job.validation")
            .await
            .unwrap(),
        before
    );
    assert_eq!(
        case.portfolio
            .base
            .store
            .audit_events(0, 100)
            .await
            .unwrap(),
        events
    );
}
