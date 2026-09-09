mod support;

use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use loop_core::audit::verify_audit_chain;
use loop_protocol::provenance::{ProvenanceComponent, ProvenanceError};
use loop_protocol::wire::v1::*;
use loopd::store::{BacktestRepository, JobMutation, JobRepository, PgJobStore, StoreError};
use sqlx::Executor;
use support::{backtest, *};
use tempfile::TempDir;

async fn fixture() -> (
    TempDir,
    PgJobStore,
    Arc<FixtureClock>,
    Arc<backtest::Policy>,
) {
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let policy = Arc::new(backtest::Policy::default());
    let store = PgJobStore::open(backtest::options(
        &directory.path().join("state"),
        clock.clone(),
        policy.clone(),
    ))
    .await
    .unwrap();
    (directory, store, clock, policy)
}

async fn counts(directory: &TempDir) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM backtest_results),
        (SELECT count(*) FROM command_receipts), (SELECT count(*) FROM audit_events)",
    )
    .fetch_one(&mut connection(directory).await)
    .await
    .unwrap()
}

#[tokio::test]
async fn result_and_replay_survive_restart() {
    let (directory, store, clock, policy) = fixture().await;
    let request = backtest::seed(&store).await;
    let completed = store
        .mutate(&actor(), JobMutation::Complete(request.clone()))
        .await
        .unwrap();
    assert_eq!(completed.job.state, JobState::Succeeded as i32);
    assert_eq!(counts(&directory).await, (1, 3, 3));
    store.close().await;
    let reopened = PgJobStore::open(backtest::options(
        &directory.path().join("state"),
        clock,
        policy,
    ))
    .await
    .unwrap();
    let replay = reopened
        .mutate(&actor(), JobMutation::Complete(request))
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(completed.job, replay.job);
    assert_eq!(
        reopened
            .current_backtest(&actor(), "job.1", "context.fixture")
            .await
            .unwrap(),
        backtest::result()
    );
    assert_eq!(counts(&directory).await, (1, 3, 3));
    verify_audit_chain(&reopened.audit_events(0, 500).await.unwrap()).unwrap();
    reopened.close().await;
}

#[tokio::test]
async fn current_read_checks_all_components() {
    let (directory, store, clock, policy) = fixture().await;
    let request = backtest::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request.clone()))
        .await
        .unwrap();
    store.close().await;
    let reopened = PgJobStore::open(backtest::options(
        &directory.path().join("state"),
        clock,
        policy.clone(),
    ))
    .await
    .unwrap();
    for component in ProvenanceComponent::ALL {
        let mut current = backtest::result().provenance.unwrap();
        let changed = match component {
            ProvenanceComponent::SourceCode => &mut current.source_code_sha256,
            ProvenanceComponent::OperatorRegistry => &mut current.operator_registry_sha256,
            ProvenanceComponent::Configuration => &mut current.configuration_sha256,
            ProvenanceComponent::DataManifest => &mut current.data_manifest_sha256,
            ProvenanceComponent::TradingCalendar => &mut current.trading_calendar_sha256,
            ProvenanceComponent::Environment => &mut current.environment_sha256,
        };
        *changed = Some(digest(99));
        *policy.current.lock().unwrap() = Some(current);
        assert!(
            matches!(reopened.current_backtest(&actor(), "job.1", "context.fixture").await,
            Err(StoreError::Provenance(ProvenanceError::Stale(fields))) if fields == [component])
        );
    }
    assert!(
        reopened
            .mutate(&actor(), JobMutation::Complete(request))
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(counts(&directory).await, (1, 3, 3));
    reopened.close().await;
}

#[tokio::test]
async fn absent_context_is_not_current() {
    let (_directory, store, _clock, policy) = fixture().await;
    let request = backtest::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request))
        .await
        .unwrap();
    *policy.current.lock().unwrap() = None;
    assert!(matches!(
        store
            .current_backtest(&actor(), "job.1", "context.fixture")
            .await,
        Err(StoreError::Provenance(ProvenanceError::UnresolvedCurrent))
    ));
    store.close().await;
}

#[tokio::test]
async fn original_mismatch_rolls_back_completion() {
    let (directory, store, _clock, policy) = fixture().await;
    let request = backtest::seed(&store).await;
    policy
        .result
        .lock()
        .unwrap()
        .provenance
        .as_mut()
        .unwrap()
        .operator_registry_sha256 = Some(digest(99));
    assert!(
        matches!(store.mutate(&actor(), JobMutation::Complete(request.clone())).await,
        Err(StoreError::Provenance(ProvenanceError::RecordingMismatch(fields)))
            if fields == [ProvenanceComponent::OperatorRegistry])
    );
    assert_eq!(counts(&directory).await, (0, 2, 2));
    assert_eq!(store.get("job.1").await.unwrap().unwrap().revision, 2);
    *policy.result.lock().unwrap() = backtest::result();
    assert!(
        !store
            .mutate(&actor(), JobMutation::Complete(request))
            .await
            .unwrap()
            .replayed
    );
    store.close().await;
}

#[tokio::test]
async fn default_result_policy_denies_success() {
    let (directory, store, clock, _policy) = fixture().await;
    let request = backtest::seed(&store).await;
    store.close().await;
    let mut options = support::options(&directory.path().join("state"), clock);
    options.admission = Arc::new(backtest::Admission);
    let denied = PgJobStore::open(options).await.unwrap();
    assert!(matches!(
        denied
            .mutate(&actor(), JobMutation::Complete(request))
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert_eq!(counts(&directory).await, (0, 2, 2));
    denied.close().await;
}

#[tokio::test]
async fn malformed_metrics_cannot_commit() {
    let (directory, store, _clock, policy) = fixture().await;
    let request = backtest::seed(&store).await;
    for decimal in ["NaN", "Infinity", "-0", "1e3", "0.010", "+1"] {
        policy.result.lock().unwrap().metrics[0]
            .value
            .as_mut()
            .unwrap()
            .value = decimal.to_owned();
        assert!(matches!(
            store
                .mutate(&actor(), JobMutation::Complete(request.clone()))
                .await,
            Err(StoreError::Invalid("metric decimal"))
        ));
    }
    assert_eq!(counts(&directory).await, (0, 2, 2));
    store.close().await;
}

#[tokio::test]
async fn duplicate_metrics_are_rejected() {
    let (_directory, store, _clock, policy) = fixture().await;
    let request = backtest::seed(&store).await;
    let metric = policy.result.lock().unwrap().metrics[0].clone();
    policy.result.lock().unwrap().metrics.push(metric);
    assert!(matches!(
        store.mutate(&actor(), JobMutation::Complete(request)).await,
        Err(StoreError::Invalid("metric size or order"))
    ));
    store.close().await;
}

#[tokio::test]
async fn result_artifacts_must_be_committed_outputs() {
    let (directory, store, _clock, policy) = fixture().await;
    let request = backtest::seed(&store).await;
    policy
        .result
        .lock()
        .unwrap()
        .artifacts
        .as_mut()
        .unwrap()
        .nav = Some(artifact());
    assert!(matches!(
        store
            .mutate(&actor(), JobMutation::Complete(request.clone()))
            .await,
        Err(StoreError::Invalid("backtest artifact output binding"))
    ));
    *policy.result.lock().unwrap() = backtest::result();
    policy.result.lock().unwrap().result_manifest_sha256 = Some(digest(99));
    assert!(matches!(
        store.mutate(&actor(), JobMutation::Complete(request)).await,
        Err(StoreError::Invalid("result manifest output binding"))
    ));
    assert_eq!(counts(&directory).await, (0, 2, 2));
    store.close().await;
}

#[tokio::test]
async fn expired_lease_cannot_record_metrics() {
    let (directory, store, clock, _policy) = fixture().await;
    let request = backtest::seed(&store).await;
    clock.0.store(NOW + 30_000, Ordering::SeqCst);
    assert!(matches!(
        store.mutate(&actor(), JobMutation::Complete(request)).await,
        Err(StoreError::LeaseFenced)
    ));
    assert_eq!(counts(&directory).await, (0, 2, 2));
    store.close().await;
}

#[tokio::test]
async fn future_result_time_is_rejected() {
    let (_directory, store, _clock, policy) = fixture().await;
    let request = backtest::seed(&store).await;
    policy.result.lock().unwrap().completed_at = Some(timestamp(NOW + 1));
    assert!(matches!(
        store.mutate(&actor(), JobMutation::Complete(request)).await,
        Err(StoreError::Invalid("result completion time binding"))
    ));
    store.close().await;
}

#[tokio::test]
async fn result_rows_are_immutable() {
    let (directory, store, _clock, _policy) = fixture().await;
    let request = backtest::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request))
        .await
        .unwrap();
    let mut connection = connection(&directory).await;
    for sql in [
        "UPDATE backtest_results SET engine = 2",
        "DELETE FROM backtest_results",
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
    store.close().await;
}

#[tokio::test]
async fn database_rejects_success_without_result() {
    let (directory, store, _clock, _policy) = fixture().await;
    backtest::seed(&store).await;
    let error = sqlx::query(
        "UPDATE jobs SET state = 4, revision = 3,
        lease_id = NULL, lease_owner_id = NULL, lease_expires_at_ms = NULL WHERE job_id = 'job.1'",
    )
    .execute(&mut connection(&directory).await)
    .await
    .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("23514")
    );
    assert_eq!(
        store.get("job.1").await.unwrap().unwrap().state,
        JobState::Leased as i32
    );
    assert_eq!(counts(&directory).await, (0, 2, 2));
    store.close().await;
}

#[tokio::test]
async fn audit_failure_rolls_back_result() {
    let (directory, store, _clock, _policy) = fixture().await;
    let request = backtest::seed(&store).await;
    let mut connection = connection(&directory).await;
    connection
        .execute(
            "CREATE TRIGGER injected_result_failure BEFORE INSERT ON audit_events
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
    assert_eq!(counts(&directory).await, (0, 2, 2));
    assert_eq!(store.get("job.1").await.unwrap().unwrap().revision, 2);
    connection
        .execute("DROP TRIGGER injected_result_failure ON audit_events")
        .await
        .unwrap();
    assert!(
        !store
            .mutate(&actor(), JobMutation::Complete(request))
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(counts(&directory).await, (1, 3, 3));
    store.close().await;
}

#[tokio::test]
async fn corrupt_result_is_not_replayed_or_read() {
    let (directory, store, _clock, _policy) = fixture().await;
    let request = backtest::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request.clone()))
        .await
        .unwrap();
    let mut connection = connection(&directory).await;
    connection
        .execute("ALTER TABLE backtest_results DISABLE TRIGGER backtest_results_no_update")
        .await
        .unwrap();
    connection
        .execute("UPDATE backtest_results SET result_sha256 = decode(repeat('00', 32), 'hex')")
        .await
        .unwrap();
    assert!(matches!(
        store
            .current_backtest(&actor(), "job.1", "context.fixture")
            .await,
        Err(StoreError::Corrupt(_))
    ));
    assert!(matches!(
        store.mutate(&actor(), JobMutation::Complete(request)).await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn current_read_requires_authenticated_access() {
    let (_directory, store, _clock, _policy) = fixture().await;
    let request = backtest::seed(&store).await;
    store
        .mutate(&actor(), JobMutation::Complete(request))
        .await
        .unwrap();
    let mut untrusted = actor();
    untrusted.authenticated_subject.clear();
    assert!(matches!(
        store
            .current_backtest(&untrusted, "job.1", "context.fixture")
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    untrusted.authenticated_subject = "other:subject".to_owned();
    assert!(matches!(
        store
            .current_backtest(&untrusted, "job.1", "context.fixture")
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(matches!(
        store
            .current_backtest(&actor(), "job.1", "context.unknown")
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}
