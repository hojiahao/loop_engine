mod support;

use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use loop_core::audit::verify_audit_chain;
use loop_protocol::wire::jobs::v1::{AcquireJobLeaseRequest, CancelJobRequest};
use loop_protocol::wire::v1::*;
use loopd::store::{
    AdmissionPolicy, JobMutation, JobRepository, PgJobStore, RecoveryCommand, StoreError,
    StoreResult,
};
use sqlx::Executor;
use support::*;
use tempfile::TempDir;

async fn fixture() -> (TempDir, PgJobStore, Arc<FixtureClock>) {
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let store = PgJobStore::open(backtest::options(
        &directory.path().join("state"),
        clock.clone(),
        Arc::new(backtest::Policy::default()),
    ))
    .await
    .unwrap();
    (directory, store, clock)
}

async fn counts(directory: &TempDir) -> (i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM jobs), (SELECT count(*) FROM backtest_rejections),
        (SELECT count(*) FROM command_receipts), (SELECT count(*) FROM audit_events)",
    )
    .fetch_one(&mut connection(directory).await)
    .await
    .unwrap()
}

#[tokio::test]
// Scenario: context key is pinned.
async fn context_key_pinned() {
    let (directory, store, _clock) = fixture().await;
    let request = rejection::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request))
        .await
        .unwrap();
    let key: String = sqlx::query_scalar(
        "SELECT encode(context_sha256, 'hex') FROM backtest_rejections WHERE job_id = 'job.1'",
    )
    .fetch_one(&mut connection(&directory).await)
    .await
    .unwrap();
    // Independent Node.js crypto/JSON oracle for the documented v1 profile.
    assert_eq!(
        key,
        "ef69eb561404de69fe9e95e330e8b9f81982b9dc53736fabb9160035f16a9ffa"
    );
    store.close().await;
}

#[tokio::test]
async fn rejection_survives_restart() {
    let (directory, store, clock) = fixture().await;
    let request = rejection::seed(&store).await;
    let completed = store
        .mutate(&actor(), JobMutation::Complete(request.clone()))
        .await
        .unwrap();
    assert_eq!(completed.job.state, JobState::FactorRejected as i32);
    assert_eq!(counts(&directory).await, (1, 1, 3, 3));
    store.close().await;
    let reopened = PgJobStore::open(backtest::options(
        &directory.path().join("state"),
        clock,
        Arc::new(backtest::Policy::default()),
    ))
    .await
    .unwrap();
    let replay = reopened
        .mutate(&actor(), JobMutation::Complete(request))
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.job, completed.job);
    assert!(matches!(
        reopened.submit(rejection::command(2)).await,
        Err(StoreError::PreviouslyRejected)
    ));
    assert!(matches!(
        reopened
            .submit_role(&actor(), rejection::role("new.role"), research::metadata())
            .await,
        Err(StoreError::PreviouslyRejected)
    ));
    assert_eq!(counts(&directory).await, (1, 1, 3, 3));
    verify_audit_chain(&reopened.audit_events(0, 500).await.unwrap()).unwrap();
    reopened.close().await;
}

#[tokio::test]
// Scenario: submission replay is not redispatch.
async fn submission_replay_redispatch() {
    let (directory, store, _clock) = fixture().await;
    let request = rejection::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request))
        .await
        .unwrap();
    let replay = store.submit(rejection::command(1)).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.job.state, JobState::Queued as i32);
    assert_eq!(
        store.get("job.1").await.unwrap().unwrap().state,
        JobState::FactorRejected as i32
    );
    assert_eq!(counts(&directory).await, (1, 1, 3, 3));
    store.close().await;
}

#[tokio::test]
// Scenario: queued duplicate cannot acquire.
async fn queued() {
    let (directory, store, _clock) = fixture().await;
    let request = rejection::seed(&store).await;
    store.submit(rejection::command(2)).await.unwrap();
    store
        .mutate(&actor(), JobMutation::Complete(request))
        .await
        .unwrap();
    let acquire = JobMutation::Acquire(AcquireJobLeaseRequest {
        context: Some(context("late.acquire")),
        job_id: Some(JobId {
            value: "job.2".to_owned(),
        }),
        expected_revision: 1,
        requested_duration: Some(prost_types::Duration {
            seconds: 30,
            nanos: 0,
        }),
    });
    assert!(matches!(
        store.mutate(&actor(), acquire).await,
        Err(StoreError::PreviouslyRejected)
    ));
    assert_eq!(store.get("job.2").await.unwrap().unwrap().revision, 1);
    assert_eq!(counts(&directory).await, (2, 1, 4, 4));
    store.close().await;
}

// Fixture-only reference resolution permits changed frozen inputs for isolation
// tests. The production default still denies every unresolved submission.
struct ContextAdmission;

impl AdmissionPolicy for ContextAdmission {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        if !matches!(job.input, Some(job_specification::Input::Backtest(_))) {
            return Err(StoreError::AdmissionDenied);
        }
        let mut resolved = job.clone();
        resolved.input = rejection::command(1).specification.input;
        backtest::Admission.validate_submission(&resolved)
    }
    fn authorize_job_command(
        &self,
        operation: &str,
        principal: &Actor,
        job: &JobRecord,
    ) -> StoreResult<()> {
        backtest::Admission.authorize_job_command(operation, principal, job)
    }
}

#[tokio::test]
// Scenario: changed context is not blacklisted.
async fn changed_context() {
    let (directory, store, clock) = fixture().await;
    let request = rejection::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request))
        .await
        .unwrap();
    store.close().await;
    let mut config = options(&directory.path().join("state"), clock);
    config.admission = Arc::new(ContextAdmission);
    let store = PgJobStore::open(config).await.unwrap();
    for component in 0..9 {
        let mut request = rejection::command(component + 2);
        let Some(job_specification::Input::Backtest(input)) = &mut request.specification.input
        else {
            unreachable!()
        };
        let provenance = input.provenance.as_mut().unwrap();
        match component {
            0 => provenance.source_code_sha256 = Some(digest(99)),
            1 => provenance.operator_registry_sha256 = Some(digest(99)),
            2 => provenance.configuration_sha256 = Some(digest(99)),
            3 => {
                provenance.data_manifest_sha256 = Some(digest(99));
                input.dataset.as_mut().unwrap().manifest_sha256 = Some(digest(99));
            }
            4 => provenance.trading_calendar_sha256 = Some(digest(99)),
            5 => provenance.environment_sha256 = Some(digest(99)),
            6 => input.deterministic_seed = Some(digest(99)),
            7 => {
                input.factor_spec_id.as_mut().unwrap().value = format!("sha256:{}", "ab".repeat(32))
            }
            8 => {
                input.dataset.as_mut().unwrap().snapshot_ids[0].value = "snapshot.other".to_owned()
            }
            _ => unreachable!(),
        }
        assert!(
            !store.submit(request).await.unwrap().replayed,
            "component {component}"
        );
    }
    assert_eq!(counts(&directory).await, (10, 1, 12, 12));
    store.close().await;
}

#[tokio::test]
// Scenario: new run or budget does not bypass memory.
async fn budget_memory() {
    let (directory, store, clock) = fixture().await;
    let request = rejection::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request))
        .await
        .unwrap();
    store.close().await;
    let mut config = options(&directory.path().join("state"), clock);
    config.admission = Arc::new(ContextAdmission);
    let store = PgJobStore::open(config).await.unwrap();
    let mut request = rejection::command(2);
    request.specification.run_id.as_mut().unwrap().value = "run.other".to_owned();
    let Some(job_specification::Input::Backtest(input)) = &mut request.specification.input else {
        unreachable!()
    };
    input.budget.as_mut().unwrap().maximum_steps += 1;
    assert!(matches!(
        store.submit(request).await,
        Err(StoreError::PreviouslyRejected)
    ));
    assert_eq!(counts(&directory).await, (1, 1, 3, 3));
    store.close().await;
}

#[tokio::test]
// Scenario: only deterministic codes block resubmission.
async fn deterministic_codes_resubmission() {
    for code in 2..=9 {
        let (directory, store, _clock) = fixture().await;
        let mut request = rejection::seed(&store).await;
        let Some(job_outcome::Outcome::FactorRejection(rejection)) =
            &mut request.outcome.as_mut().unwrap().outcome
        else {
            unreachable!()
        };
        rejection.code = code;
        store
            .mutate(&actor(), JobMutation::Complete(request))
            .await
            .unwrap();
        let result = store.submit(rejection::command(2)).await;
        if (4..=6).contains(&code) {
            assert!(matches!(result, Err(StoreError::PreviouslyRejected)));
        } else {
            assert!(!result.unwrap().replayed);
        }
        assert_eq!(counts(&directory).await.1, 1);
        store.close().await;
    }
}

#[tokio::test]
// Scenario: infrastructure failure is not factor memory.
async fn infrastructure_failure_factor() {
    let (directory, store, clock) = fixture().await;
    rejection::seed(&store).await;
    clock.0.store(NOW + 30_000, Ordering::SeqCst);
    let recovered = store
        .mutate(
            &actor(),
            JobMutation::Recover(RecoveryCommand {
                context: Some(context("recover")),
                job_id: Some(JobId {
                    value: "job.1".to_owned(),
                }),
                expected_revision: 2,
            }),
        )
        .await
        .unwrap();
    assert_eq!(recovered.job.state, JobState::InfrastructureFailed as i32);
    store.submit(rejection::command(2)).await.unwrap();
    assert_eq!(counts(&directory).await, (2, 0, 4, 4));
    store.close().await;
}

#[tokio::test]
// Scenario: cancellation is not factor memory.
async fn cancellation_factor_memory() {
    let (directory, store, _clock) = fixture().await;
    store.submit(rejection::command(1)).await.unwrap();
    store
        .mutate(
            &actor(),
            JobMutation::Cancel(CancelJobRequest {
                context: Some(context("cancel")),
                job_id: Some(JobId {
                    value: "job.1".to_owned(),
                }),
                expected_revision: 1,
                reason: "operator cancelled".to_owned(),
            }),
        )
        .await
        .unwrap();
    store.submit(rejection::command(2)).await.unwrap();
    assert_eq!(counts(&directory).await, (2, 0, 3, 3));
    store.close().await;
}

#[tokio::test]
// Scenario: expired lease cannot record rejection.
async fn expired_lease_rejection() {
    let (directory, store, clock) = fixture().await;
    let request = rejection::seed(&store).await;
    clock.0.store(NOW + 30_000, Ordering::SeqCst);
    assert!(matches!(
        store.mutate(&actor(), JobMutation::Complete(request)).await,
        Err(StoreError::LeaseFenced)
    ));
    assert_eq!(counts(&directory).await, (1, 0, 2, 2));
    store.close().await;
}

#[tokio::test]
// Scenario: clock regression cannot record rejection.
async fn clock_regression_rejection() {
    let (directory, store, clock) = fixture().await;
    let request = rejection::seed(&store).await;
    clock.0.store(NOW - 1, Ordering::SeqCst);
    assert!(matches!(
        store.mutate(&actor(), JobMutation::Complete(request)).await,
        Err(StoreError::ClockRegression)
    ));
    assert_eq!(counts(&directory).await, (1, 0, 2, 2));
    store.close().await;
}

#[tokio::test]
// Scenario: authorization precedes history lookup.
async fn authorization_precedes_history() {
    let (directory, store, clock) = fixture().await;
    let request = rejection::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request))
        .await
        .unwrap();
    store.close().await;
    let mut config = options(&directory.path().join("state"), clock);
    config.admission = Arc::new(loopd::store::DenySubmission);
    let denied = PgJobStore::open(config).await.unwrap();
    assert!(matches!(
        denied.submit(rejection::command(2)).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(matches!(
        denied
            .submit_role(&actor(), rejection::role("denied"), research::metadata())
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert_eq!(counts(&directory).await, (1, 1, 3, 3));
    denied.close().await;
}

#[tokio::test]
// Scenario: audit failure rolls back rejection.
async fn audit_failure_rejection() {
    let (directory, store, _clock) = fixture().await;
    let request = rejection::seed(&store).await;
    let mut connection = connection(&directory).await;
    connection
        .execute(
            "CREATE TRIGGER fail_rejection_audit BEFORE INSERT ON audit_events
        FOR EACH ROW EXECUTE FUNCTION reject_immutable_change()",
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .mutate(&actor(), JobMutation::Complete(request.clone()))
            .await,
        Err(StoreError::Database(_))
    ));
    assert_eq!(counts(&directory).await, (1, 0, 2, 2));
    assert_eq!(store.get("job.1").await.unwrap().unwrap().revision, 2);
    connection
        .execute("DROP TRIGGER fail_rejection_audit ON audit_events")
        .await
        .unwrap();
    assert!(
        !store
            .mutate(&actor(), JobMutation::Complete(request))
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(counts(&directory).await, (1, 1, 3, 3));
    store.close().await;
}

#[tokio::test]
// Scenario: corrupt source fails closed.
async fn corrupt_source_closed() {
    let (directory, store, _clock) = fixture().await;
    let request = rejection::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request))
        .await
        .unwrap();
    connection(&directory)
        .await
        .execute("UPDATE jobs SET record_sha256 = decode(repeat('00', 32), 'hex')")
        .await
        .unwrap();
    assert!(matches!(
        store.submit(rejection::command(2)).await,
        Err(StoreError::Corrupt("envelope checksum"))
    ));
    assert_eq!(counts(&directory).await, (1, 1, 3, 3));
    store.close().await;
}

#[tokio::test]
// Scenario: corrupt projection blocks replay.
async fn corrupt_projection_replay() {
    let (directory, store, _clock) = fixture().await;
    let request = rejection::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request.clone()))
        .await
        .unwrap();
    let mut connection = connection(&directory).await;
    connection
        .execute("ALTER TABLE backtest_rejections DISABLE TRIGGER backtest_rejections_no_update")
        .await
        .unwrap();
    connection
        .execute("UPDATE backtest_rejections SET rejection_code = 4")
        .await
        .unwrap();
    assert!(matches!(
        store.mutate(&actor(), JobMutation::Complete(request)).await,
        Err(StoreError::Corrupt("rejection projection"))
    ));
    assert!(matches!(
        store.submit(rejection::command(2)).await,
        Err(StoreError::Corrupt("rejection projection"))
    ));
    store.close().await;
}

#[tokio::test]
// Scenario: rejection rows are immutable.
async fn rejection_rows_immutable() {
    let (directory, store, _clock) = fixture().await;
    let request = rejection::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request))
        .await
        .unwrap();
    let mut connection = connection(&directory).await;
    for sql in [
        "UPDATE backtest_rejections SET rejection_code = 4",
        "DELETE FROM backtest_rejections",
    ] {
        assert_eq!(
            connection
                .execute(sql)
                .await
                .unwrap_err()
                .as_database_error()
                .unwrap()
                .code()
                .as_deref(),
            Some("23514")
        );
    }
    assert_eq!(counts(&directory).await, (1, 1, 3, 3));
    store.close().await;
}

#[tokio::test]
// Scenario: sql cannot skip rejection record.
async fn sql_rejection() {
    let (directory, store, _clock) = fixture().await;
    rejection::seed(&store).await;
    let error = connection(&directory)
        .await
        .execute(
            "UPDATE jobs SET state = 5, revision = 3, lease_id = NULL,
        lease_owner_id = NULL, lease_expires_at_ms = NULL WHERE job_id = 'job.1'",
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("23514")
    );
    assert_eq!(counts(&directory).await, (1, 0, 2, 2));
    store.close().await;
}

#[tokio::test]
// Scenario: migration refuses unindexed history.
async fn migration_refuses_unindexed() {
    let (directory, store, _clock) = fixture().await;
    let mut connection = connection(&directory).await;
    connection
        .execute("CREATE TEMP TABLE jobs (kind INTEGER, state INTEGER)")
        .await
        .unwrap();
    connection
        .execute("INSERT INTO jobs VALUES (3, 5)")
        .await
        .unwrap();
    let error = connection
        .execute(include_str!(
            "../../../migrations/postgres/0007_backtest_rejections.sql"
        ))
        .await
        .unwrap_err();
    let database = error.as_database_error().unwrap();
    assert_eq!(database.code().as_deref(), Some("23514"));
    assert_eq!(
        database.message(),
        "existing development rejections require evidence migration"
    );
    assert_eq!(counts(&directory).await, (0, 0, 0, 0));
    store.close().await;
}

#[tokio::test]
// Scenario: missing projection blocks replay.
async fn missing_projection_replay() {
    let (directory, store, _clock) = fixture().await;
    let request = rejection::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request.clone()))
        .await
        .unwrap();
    let mut connection = connection(&directory).await;
    connection
        .execute("ALTER TABLE backtest_rejections DISABLE TRIGGER backtest_rejections_no_update")
        .await
        .unwrap();
    connection
        .execute("DELETE FROM backtest_rejections")
        .await
        .unwrap();
    assert!(matches!(
        store.mutate(&actor(), JobMutation::Complete(request)).await,
        Err(StoreError::Corrupt("missing rejection record"))
    ));
    store.close().await;
}
