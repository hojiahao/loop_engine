mod support;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI64, Ordering},
};
use std::time::Duration;

use loop_core::audit::{AuditAction, verify_audit_chain};
use loop_protocol::provenance::{ProvenanceComponent, ProvenanceError};
use loop_protocol::wire::v1::*;
use loopd::store::{
    AdmissionPolicy, BacktestPolicy, BacktestRepository, DenyBacktest, DenyHoldout,
    HoldoutRepository, JobMutation, JobRepository, PgJobStore, StoreError, StoreOptions,
    StoreResult,
};
use sqlx::Executor;
use support::{backtest, *};
use tempfile::TempDir;

#[derive(Default)]
struct Permissions(Mutex<Option<&'static str>>);

impl AdmissionPolicy for Permissions {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        backtest::Admission.validate_submission(job)
    }

    fn authorize_job_command(
        &self,
        operation: &str,
        principal: &Actor,
        job: &JobRecord,
    ) -> StoreResult<()> {
        backtest::Admission.authorize_job_command(operation, principal, job)?;
        if self
            .0
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|blocked| *blocked == operation)
        {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(())
    }
}

struct Fixture {
    directory: TempDir,
    store: PgJobStore,
    clock: Arc<FixtureClock>,
    policy: Arc<backtest::Policy>,
    permissions: Arc<Permissions>,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
        let policy = Arc::new(backtest::Policy::default());
        let permissions = Arc::new(Permissions::default());
        let mut config = backtest::options(
            &directory.path().join("state"),
            clock.clone(),
            policy.clone(),
        );
        config.admission = permissions.clone();
        let store = PgJobStore::open(config).await.unwrap();
        let request = backtest::seed(&store).await;
        store
            .mutate(&actor(), JobMutation::Complete(request))
            .await
            .unwrap();
        Self {
            directory,
            store,
            clock,
            policy,
            permissions,
        }
    }

    fn options(&self) -> StoreOptions {
        let mut config = backtest::options(
            &self.directory.path().join("state"),
            self.clock.clone(),
            self.policy.clone(),
        );
        config.admission = self.permissions.clone();
        config
    }

    async fn assert_counts(&self, exports: i64) {
        let counts: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM command_receipts),
            (SELECT count(*) FROM audit_events), (SELECT count(*) FROM backtest_results),
            (SELECT revision FROM jobs WHERE job_id = 'job.1')",
        )
        .fetch_one(&mut connection(&self.directory).await)
        .await
        .unwrap();
        assert_eq!(counts, (3 + exports, 3 + exports, 1, 3));
    }
}

fn change_component(value: &mut ResearchProvenanceFingerprint, component: ProvenanceComponent) {
    let field = match component {
        ProvenanceComponent::SourceCode => &mut value.source_code_sha256,
        ProvenanceComponent::OperatorRegistry => &mut value.operator_registry_sha256,
        ProvenanceComponent::Configuration => &mut value.configuration_sha256,
        ProvenanceComponent::DataManifest => &mut value.data_manifest_sha256,
        ProvenanceComponent::TradingCalendar => &mut value.trading_calendar_sha256,
        ProvenanceComponent::Environment => &mut value.environment_sha256,
    };
    *field = Some(digest(99));
}

#[tokio::test]
async fn export_receipt_survives_restart() {
    let mut f = Fixture::new().await;
    let first = f
        .store
        .export_current(&actor(), backtest::export("export.retry"))
        .await
        .unwrap();
    assert!(!first.replayed);
    assert_eq!(first.result, backtest::result());
    assert_eq!(first.accepted_at, timestamp(NOW));
    assert_eq!(first.job_id, "job.1");
    assert_eq!(first.context_id, "context.fixture");
    f.store.close().await;
    f.store = PgJobStore::open(f.options()).await.unwrap();
    f.clock.0.store(NOW + 1_000, Ordering::SeqCst);
    let mut retry = backtest::export("export.retry");
    let context = retry.context.as_mut().unwrap();
    context.request_id.as_mut().unwrap().value = "request.retry".to_owned();
    context.requested_at = Some(timestamp(NOW + 1_000));
    retry.deadline = Some(timestamp(NOW + 31_000));
    let replay = f.store.export_current(&actor(), retry).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.accepted_at, first.accepted_at);
    assert_eq!(replay.result, first.result);
    f.assert_counts(1).await;
    let events = f.store.audit_events(0, 500).await.unwrap();
    verify_audit_chain(&events).unwrap();
    assert_eq!(events[3].action, AuditAction::CommandAccepted);
    assert_eq!(events[3].target.value, "job.1");
    let payload: serde_json::Value =
        serde_json::from_slice(&events[3].payload.canonical_bytes).unwrap();
    assert_eq!(payload["command"], "loop.backtests.export_current");
    assert_eq!(payload["request_id"], "request.export.retry");
    assert!(
        payload["summary"]
            .as_str()
            .unwrap()
            .contains("context.fixture")
    );
    f.store.close().await;
}

#[tokio::test]
async fn stale_components_block_new_exports() {
    let f = Fixture::new().await;
    for component in ProvenanceComponent::ALL {
        let mut current = backtest::result().provenance.unwrap();
        change_component(&mut current, component);
        *f.policy.current.lock().unwrap() = Some(current);
        assert!(
            matches!(f.store.export_current(&actor(), backtest::export("stale")).await,
            Err(StoreError::Provenance(ProvenanceError::Stale(fields))) if fields == [component])
        );
    }
    f.assert_counts(0).await;
    f.store.close().await;
}

#[tokio::test]
async fn replay_cannot_bypass_stale_context() {
    let f = Fixture::new().await;
    f.store
        .export_current(&actor(), backtest::export("retry"))
        .await
        .unwrap();
    for component in ProvenanceComponent::ALL {
        let mut current = backtest::result().provenance.unwrap();
        change_component(&mut current, component);
        *f.policy.current.lock().unwrap() = Some(current);
        assert!(
            matches!(f.store.export_current(&actor(), backtest::export("retry")).await,
            Err(StoreError::Provenance(ProvenanceError::Stale(fields))) if fields == [component])
        );
    }
    f.assert_counts(1).await;
    f.store.close().await;
}

#[tokio::test]
async fn unavailable_context_blocks_replay() {
    let f = Fixture::new().await;
    f.store
        .export_current(&actor(), backtest::export("retry"))
        .await
        .unwrap();
    *f.policy.current.lock().unwrap() = None;
    assert!(matches!(
        f.store
            .export_current(&actor(), backtest::export("retry"))
            .await,
        Err(StoreError::Provenance(ProvenanceError::UnresolvedCurrent))
    ));
    f.assert_counts(1).await;
    f.store.close().await;
}

#[tokio::test]
async fn unavailable_manifest_resolver_blocks_replay() {
    let mut f = Fixture::new().await;
    f.store
        .export_current(&actor(), backtest::export("retry"))
        .await
        .unwrap();
    f.store.close().await;
    let mut config = f.options();
    config.backtest_policy = Arc::new(DenyBacktest);
    f.store = PgJobStore::open(config).await.unwrap();
    assert!(matches!(
        f.store
            .export_current(&actor(), backtest::export("retry"))
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    f.assert_counts(1).await;
    f.store.close().await;
}

#[tokio::test]
async fn protected_jobs_keep_the_holdout_gate() {
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let mut config = options(&directory.path().join("state"), clock.clone());
    config.holdout_policy = Arc::new(batch::Policy::default());
    config.admission = Arc::new(batch::Admission);
    let store = PgJobStore::open(config).await.unwrap();
    let (_, request) = batch::seed(&store).await;
    let consumed = store
        .consume_grant(&actor(), request, research::metadata())
        .await
        .unwrap();
    let job_id = consumed.response.job_batch.unwrap().job_ids[0].clone();
    let count = store.audit_events(0, 500).await.unwrap().len();
    store.close().await;
    let mut config = options(&directory.path().join("state"), clock);
    config.admission = Arc::new(batch::Admission);
    config.holdout_policy = Arc::new(DenyHoldout);
    let reopened = PgJobStore::open(config).await.unwrap();
    let mut export = backtest::export("protected");
    export.job_id = Some(job_id);
    assert!(matches!(
        reopened.export_current(&actor(), export).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert_eq!(reopened.audit_events(0, 500).await.unwrap().len(), count);
    reopened.close().await;
}

#[tokio::test]
async fn both_read_and_export_permissions_are_required() {
    let f = Fixture::new().await;
    for operation in [
        "loop.backtests.read_current",
        "loop.backtests.export_current",
    ] {
        *f.permissions.0.lock().unwrap() = Some(operation);
        assert!(matches!(
            f.store
                .export_current(&actor(), backtest::export("denied"))
                .await,
            Err(StoreError::AdmissionDenied)
        ));
    }
    f.assert_counts(0).await;
    f.store.close().await;
}

#[tokio::test]
async fn revoked_permission_blocks_replay() {
    let f = Fixture::new().await;
    f.store
        .export_current(&actor(), backtest::export("retry"))
        .await
        .unwrap();
    *f.permissions.0.lock().unwrap() = Some("loop.backtests.export_current");
    assert!(matches!(
        f.store
            .export_current(&actor(), backtest::export("retry"))
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    f.assert_counts(1).await;
    f.store.close().await;
}

#[tokio::test]
async fn metadata_cannot_spoof_transport_identity() {
    let f = Fixture::new().await;
    let mut impostor = actor();
    impostor.authenticated_subject = "different:subject".to_owned();
    assert!(matches!(
        f.store
            .export_current(&impostor, backtest::export("spoof"))
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    let mut unsigned = actor();
    unsigned.authenticated_subject.clear();
    let mut command = backtest::export("spoof");
    command.context.as_mut().unwrap().actor = Some(unsigned.clone());
    assert!(matches!(
        f.store.export_current(&unsigned, command).await,
        Err(StoreError::AdmissionDenied)
    ));
    f.assert_counts(0).await;
    f.store.close().await;
}

#[tokio::test]
async fn changed_retry_content_conflicts() {
    let f = Fixture::new().await;
    f.store
        .export_current(&actor(), backtest::export("retry"))
        .await
        .unwrap();
    let mut changed = backtest::export("retry");
    changed
        .context
        .as_mut()
        .unwrap()
        .correlation_id
        .as_mut()
        .unwrap()
        .value = "corr.changed".to_owned();
    assert!(matches!(
        f.store.export_current(&actor(), changed).await,
        Err(StoreError::IdempotencyConflict)
    ));
    f.assert_counts(1).await;
    f.store.close().await;
}

#[tokio::test]
async fn deadline_is_required_and_bounded() {
    let f = Fixture::new().await;
    for deadline in [None, Some(timestamp(NOW)), Some(timestamp(NOW + 30_001))] {
        let mut command = backtest::export("deadline");
        command.deadline = deadline;
        assert!(matches!(
            f.store.export_current(&actor(), command).await,
            Err(StoreError::Invalid("export deadline"))
        ));
    }
    f.assert_counts(0).await;
    f.store.close().await;
}

#[tokio::test]
async fn expired_request_cannot_export() {
    let f = Fixture::new().await;
    f.clock.0.store(NOW + 30_000, Ordering::SeqCst);
    assert!(matches!(
        f.store
            .export_current(&actor(), backtest::export("expired"))
            .await,
        Err(StoreError::Unavailable("export deadline exceeded"))
    ));
    f.assert_counts(0).await;
    f.store.close().await;
}

#[tokio::test]
async fn future_request_time_is_rejected() {
    let f = Fixture::new().await;
    let mut request = backtest::export("future");
    request.context.as_mut().unwrap().requested_at = Some(timestamp(NOW + 1));
    assert!(matches!(
        f.store.export_current(&actor(), request).await,
        Err(StoreError::Invalid("future command time"))
    ));
    f.assert_counts(0).await;
    f.store.close().await;
}

#[tokio::test]
async fn clock_regression_cannot_export() {
    let f = Fixture::new().await;
    f.clock.0.store(NOW - 1, Ordering::SeqCst);
    assert!(matches!(
        f.store
            .export_current(&actor(), backtest::export("regression"))
            .await,
        Err(StoreError::ClockRegression)
    ));
    f.assert_counts(0).await;
    f.store.close().await;
}

struct AdvancingPolicy {
    policy: Arc<backtest::Policy>,
    clock: Arc<FixtureClock>,
}

impl BacktestPolicy for AdvancingPolicy {
    fn resolve_result(
        &self,
        job: &JobSpecification,
        success: &JobSuccess,
    ) -> StoreResult<BacktestResult> {
        self.policy.resolve_result(job, success)
    }

    fn resolve_current(
        &self,
        principal: &Actor,
        job: &JobSpecification,
        context: &str,
    ) -> StoreResult<Option<ResearchProvenanceFingerprint>> {
        let current = self.policy.resolve_current(principal, job, context)?;
        self.clock.0.store(NOW + 30_000, Ordering::SeqCst);
        Ok(current)
    }
}

#[tokio::test]
async fn deadline_is_rechecked_after_resolution() {
    let mut f = Fixture::new().await;
    f.store.close().await;
    let mut config = f.options();
    config.backtest_policy = Arc::new(AdvancingPolicy {
        policy: f.policy.clone(),
        clock: f.clock.clone(),
    });
    f.store = PgJobStore::open(config).await.unwrap();
    assert!(matches!(
        f.store
            .export_current(&actor(), backtest::export("slow"))
            .await,
        Err(StoreError::Unavailable("export deadline exceeded"))
    ));
    f.assert_counts(0).await;
    f.store.close().await;
}

#[tokio::test]
async fn audit_failure_rolls_back_receipt() {
    let f = Fixture::new().await;
    let mut db = connection(&f.directory).await;
    db.execute("CREATE TRIGGER injected_export_failure BEFORE INSERT ON audit_events FOR EACH ROW EXECUTE FUNCTION reject_immutable_change()")
        .await.unwrap();
    f.clock.0.store(NOW + 1, Ordering::SeqCst);
    assert!(matches!(
        f.store
            .export_current(&actor(), backtest::export("audit"))
            .await,
        Err(StoreError::Database(_))
    ));
    f.assert_counts(0).await;
    let observed: i64 = sqlx::query_scalar("SELECT last_observed_at_ms FROM store_metadata")
        .fetch_one(&mut db)
        .await
        .unwrap();
    assert_eq!(observed, NOW);
    db.execute("DROP TRIGGER injected_export_failure ON audit_events")
        .await
        .unwrap();
    assert!(
        !f.store
            .export_current(&actor(), backtest::export("audit"))
            .await
            .unwrap()
            .replayed
    );
    f.assert_counts(1).await;
    f.store.close().await;
}

#[tokio::test]
async fn corrupt_receipt_is_not_released() {
    let f = Fixture::new().await;
    f.store
        .export_current(&actor(), backtest::export("retry"))
        .await
        .unwrap();
    let mut db = connection(&f.directory).await;
    db.execute("ALTER TABLE command_receipts DISABLE TRIGGER command_receipts_no_update")
        .await
        .unwrap();
    db.execute("UPDATE command_receipts SET response_sha256 = decode(repeat('00', 32), 'hex') WHERE operation = 'loop.backtests.export_current'").await.unwrap();
    assert!(matches!(
        f.store
            .export_current(&actor(), backtest::export("retry"))
            .await,
        Err(StoreError::Corrupt(_))
    ));
    f.assert_counts(1).await;
    f.store.close().await;
}

#[tokio::test]
async fn receipt_projection_is_verified() {
    let f = Fixture::new().await;
    f.store
        .export_current(&actor(), backtest::export("retry"))
        .await
        .unwrap();
    let mut db = connection(&f.directory).await;
    db.execute("ALTER TABLE command_receipts DISABLE TRIGGER command_receipts_no_update")
        .await
        .unwrap();
    db.execute("UPDATE command_receipts SET request_id = 'request.tampered' WHERE operation = 'loop.backtests.export_current'").await.unwrap();
    assert!(matches!(
        f.store
            .export_current(&actor(), backtest::export("retry"))
            .await,
        Err(StoreError::Corrupt("export receipt binding"))
    ));
    f.assert_counts(1).await;
    f.store.close().await;
}

#[tokio::test]
async fn cancellation_rolls_back_pending_export() {
    let f = Fixture::new().await;
    let mut db = connection(&f.directory).await;
    db.execute("CREATE FUNCTION delay_export() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(hashtextextended(TG_TABLE_SCHEMA || '.export-test', 0)); RETURN NEW; END $$").await.unwrap();
    db.execute("CREATE TRIGGER delay_export BEFORE INSERT ON audit_events FOR EACH ROW EXECUTE FUNCTION delay_export()").await.unwrap();
    db.execute("SELECT pg_advisory_lock(hashtextextended(current_schema() || '.export-test', 0))")
        .await
        .unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut db)
        .await
        .unwrap();
    let principal = actor();
    let mut exporting = Box::pin(
        f.store
            .export_current(&principal, backtest::export("cancel")),
    );
    let blocked_on_audit = async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))",
            )
            .bind(blocker)
            .fetch_one(&mut db)
            .await
            .unwrap();
            if blocked {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::select! {
        result = &mut exporting => panic!("export did not block at audit insertion: {result:?}"),
        result = tokio::time::timeout(Duration::from_secs(3), blocked_on_audit) => result.expect("export did not reach the audit barrier"),
    }
    drop(exporting);
    db.execute(
        "SELECT pg_advisory_unlock(hashtextextended(current_schema() || '.export-test', 0))",
    )
    .await
    .unwrap();
    db.execute("DROP TRIGGER delay_export ON audit_events")
        .await
        .unwrap();
    f.assert_counts(0).await;
    assert!(
        !f.store
            .export_current(&actor(), backtest::export("cancel"))
            .await
            .unwrap()
            .replayed
    );
    f.assert_counts(1).await;
    f.store.close().await;
}
