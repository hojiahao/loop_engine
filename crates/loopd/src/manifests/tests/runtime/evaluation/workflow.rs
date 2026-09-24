//! Real installed-worker output connected to durable research bookkeeping.

use super::*;
use crate::store::{AdmissionPolicy, FactorRepository, StoreError, StoreResult, SubmitJob};

mod admission;
mod processes;

#[derive(Clone)]
struct Pinned(Vec<JobSpecification>);

impl AdmissionPolicy for Pinned {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        if self.0.contains(job) {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
    fn authorize_job_command(
        &self,
        _: &str,
        principal: &Actor,
        record: &JobRecord,
    ) -> StoreResult<()> {
        if principal != &actor() {
            return Err(StoreError::AdmissionDenied);
        }
        self.validate_submission(
            record
                .specification
                .as_ref()
                .ok_or(StoreError::AdmissionDenied)?,
        )
    }
}

async fn open(case: &Case, jobs: Vec<JobSpecification>) -> PgJobStore {
    let mut options = support::base_options(&case.fixture.directory.path().join("state"));
    options.clock = case.clock.clone();
    options.admission = Arc::new(Pinned(jobs));
    PgJobStore::open(options).await.unwrap()
}

fn candidate(case: &Case, id: &str) -> SubmitJob {
    let mut command = case.fixture.job.clone();
    command.specification.job_id.as_mut().unwrap().value = id.to_owned();
    command
        .specification
        .idempotency_key
        .as_mut()
        .unwrap()
        .value = format!("submit.{id}");
    command.specification.run_id.as_mut().unwrap().value = "run.other".to_owned();
    command
}

async fn trials(case: &Case) -> Vec<crate::store::FactorTrial> {
    case.store
        .factor_trials(
            &actor(),
            &case
                .fixture
                .job
                .specification
                .run_id
                .as_ref()
                .unwrap()
                .value,
            "",
            10,
        )
        .await
        .unwrap()
}

#[tokio::test]
// Scenario: coverage failure filters future trials.
async fn coverage_failure_filters() {
    let case = Case::start().await;
    let duplicate = candidate(&case, "job.duplicate");
    let queued = candidate(&case, "job.queued");
    let mut changed = candidate(&case, "job.changed");
    let Some(job_specification::Input::FactorEvaluation(input)) = &mut changed.specification.input
    else {
        unreachable!()
    };
    input.deterministic_seed.as_mut().unwrap().value[0] ^= 1;
    let store = open(
        &case,
        vec![
            case.fixture.job.specification.clone(),
            duplicate.specification.clone(),
            queued.specification.clone(),
            changed.specification.clone(),
        ],
    )
    .await;
    store.submit(queued.clone()).await.unwrap();
    let request = case.request().await;
    case.client().await.evaluate_factor(request).await.unwrap();
    let page = trials(&case).await;
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].attempt, 1);
    let evaluation = page[0].evaluation.as_ref().unwrap();
    assert_eq!(evaluation.disposition, "insufficient_coverage");
    assert_eq!(evaluation.minimum_coverage_bps, 9500);
    assert_eq!(evaluation.result.as_ref().unwrap().valid_observations, 4);
    assert!(matches!(
        store.submit(duplicate).await,
        Err(StoreError::PreviouslyRejected)
    ));
    assert!(matches!(
        store
            .mutate(
                &actor(),
                crate::store::JobMutation::Acquire(AcquireJobLeaseRequest {
                    context: Some(context("blocked.acquire")),
                    job_id: queued.specification.job_id.clone(),
                    expected_revision: 1,
                    requested_duration: Some(prost_types::Duration {
                        seconds: 60,
                        nanos: 0
                    }),
                })
            )
            .await,
        Err(StoreError::PreviouslyRejected)
    ));
    // Accepted-but-not-executed work stays countable and queued; a skip is not
    // a second empirical failure. A different frozen seed is independent work.
    assert_eq!(store.get("job.queued").await.unwrap().unwrap().attempt, 0);
    assert_eq!(
        store.submit(changed).await.unwrap().job.state,
        JobState::Queued as i32
    );
    assert!(store.get("job.duplicate").await.unwrap().is_none());
    store.close().await;
}

#[tokio::test]
// Scenario: passing coverage waits for backtest.
async fn passing_coverage_waits() {
    let mut case = Case::open(fixture_with_minimum(false, 6666).await).await;
    let request = case.request().await;
    case.client()
        .await
        .evaluate_factor(request.clone())
        .await
        .unwrap();
    let first = trials(&case).await;
    assert_eq!(
        first[0].evaluation.as_ref().unwrap().disposition,
        "ready_for_backtest"
    );
    let duplicate = candidate(&case, "job.duplicate");
    let store = open(
        &case,
        vec![
            case.fixture.job.specification.clone(),
            duplicate.specification.clone(),
        ],
    )
    .await;
    assert!(matches!(
        store.submit(duplicate).await,
        Err(StoreError::AlreadyEvaluated)
    ));
    let mut decision = support::library::command(1, 0, "no.backtest");
    decision.context = Some(context("no.backtest"));
    decision.deadline = Some(support::timestamp(
        SystemClock.now_millis().unwrap() + 30_000,
    ));
    assert!(matches!(
        store.decide_factor(&actor(), decision).await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
    case.restart().await;
    case.client().await.evaluate_factor(request).await.unwrap();
    assert_eq!(trials(&case).await, first);
}

#[tokio::test]
// Scenario: corrupted projection cannot supply trial evidence.
async fn corrupted_projection_trial() {
    let case = Case::start().await;
    case.client()
        .await
        .evaluate_factor(case.request().await)
        .await
        .unwrap();
    let mut connection = support::connection(&case.fixture.directory).await;
    for statement in [
        "DELETE FROM factor_evaluations",
        "UPDATE factor_evaluations SET job_revision = job_revision",
    ] {
        assert!(
            sqlx::query(statement)
                .execute(&mut connection)
                .await
                .is_err()
        );
    }
    sqlx::query("ALTER TABLE factor_evaluations DISABLE TRIGGER factor_evaluations_immutable")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("UPDATE factor_evaluations SET evidence_sha256 = decode(repeat('00', 32), 'hex')")
        .execute(&mut connection)
        .await
        .unwrap();
    assert!(matches!(
        case.store
            .factor_trials(
                &actor(),
                &case
                    .fixture
                    .job
                    .specification
                    .run_id
                    .as_ref()
                    .unwrap()
                    .value,
                "",
                10
            )
            .await,
        Err(StoreError::Corrupt(_))
    ));
}

#[tokio::test]
// Scenario: unevaluated rejection cannot create failure memory.
async fn unevaluated_rejection_create() {
    let case = Case::start().await;
    let request = case.request().await;
    let error = case
        .client()
        .await
        .complete_job(CompleteJobRequest {
            context: request.context,
            job_id: request.job_id,
            lease_id: request.lease_id,
            expected_revision: request.expected_revision,
            outcome: Some(JobOutcome {
                outcome: Some(job_outcome::Outcome::FactorRejection(FactorRejection {
                    code: FactorRejectionCode::InsufficientCoverage as i32,
                    ..Default::default()
                })),
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    assert!(trials(&case).await[0].evaluation.is_none());
}

#[tokio::test]
// Scenario: migration preserves unverified history.
async fn migration_unverified_history() {
    use sqlx::Executor;
    let case = Case::start().await;
    case.client()
        .await
        .evaluate_factor(case.request().await)
        .await
        .unwrap();
    let before = case.store.get("job.1").await.unwrap().unwrap();
    let events = case.store.audit_events(0, 20).await.unwrap();
    let mut connection = support::connection(&case.fixture.directory).await;
    // Simulate the old schema's completed opaque job. Only this isolated test
    // namespace is changed; the upgrade must preserve the original job/audit.
    connection
        .execute("DROP FUNCTION verify_factor_evaluation() CASCADE; DROP TABLE factor_evaluations;")
        .await
        .unwrap();
    connection
        .execute(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../migrations/postgres/0010_factor_evaluations.sql"
        )))
        .await
        .unwrap();
    assert_eq!(case.store.get("job.1").await.unwrap().unwrap(), before);
    assert_eq!(case.store.audit_events(0, 20).await.unwrap(), events);
    assert!(trials(&case).await[0].evaluation.is_none());
    let fresh = candidate(&case, "job.fresh");
    let store = open(
        &case,
        vec![
            case.fixture.job.specification.clone(),
            fresh.specification.clone(),
        ],
    )
    .await;
    store.submit(fresh).await.unwrap();
    store.close().await;
}
