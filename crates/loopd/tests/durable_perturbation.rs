mod support;

use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use std::time::Duration;

use loop_core::audit::verify_audit_chain;
use loop_protocol::provenance::ProvenanceComponent;
use loop_protocol::wire::v1::*;
use loopd::research_worker::PerturbationWorker;
use loopd::store::{
    DenyBacktest, HoldoutRepository, JobMutation, JobRepository, PerturbationRepository,
    PgJobStore, StoreError, StoreOptions, StoreResult,
};
use sqlx::Executor;
use support::{perturbation as p, *};
use tempfile::TempDir;

struct Fixture {
    directory: TempDir,
    store: PgJobStore,
    clock: Arc<FixtureClock>,
    policy: Arc<p::Policy>,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_policy(Arc::new(p::Policy::default())).await
    }

    async fn with_policy(policy: Arc<p::Policy>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
        let store = PgJobStore::open(p::options(
            &directory.path().join("state"),
            clock.clone(),
            policy.clone(),
        ))
        .await
        .unwrap();
        p::seed(&store, 1, 15, false).await;
        Self {
            directory,
            store,
            clock,
            policy,
        }
    }

    fn options(&self) -> StoreOptions {
        p::options(
            &self.directory.path().join("state"),
            self.clock.clone(),
            self.policy.clone(),
        )
    }

    async fn assert_steps(&self, count: i64) {
        let (revision, receipts): (i64, i64) = sqlx::query_as(
            "SELECT COALESCE((SELECT revision FROM perturbation_states), 0),
             (SELECT count(*) FROM command_receipts WHERE operation = 'loop.perturbation.advance')",
        )
        .fetch_one(&mut connection(&self.directory).await)
        .await
        .unwrap();
        assert_eq!((revision, receipts), (count, count));
        let events = self.store.audit_events(0, 500).await.unwrap();
        verify_audit_chain(&events).unwrap();
        let proposals = events
            .iter()
            .filter(|event| {
                String::from_utf8_lossy(&event.payload.canonical_bytes)
                    .contains("loop.perturbation.advance")
            })
            .count();
        assert_eq!(proposals as i64, count);
    }
}

struct Unavailable;

impl PerturbationWorker for Unavailable {
    async fn advance(&self, _: PerturbationWork) -> StoreResult<PerturbationStep> {
        Err(StoreError::Unavailable("test worker unavailable"))
    }
}

#[tokio::test]
async fn restart_restores_history_and_rng() {
    let mut f = Fixture::new().await;
    let first = f
        .store
        .advance_perturbation(&actor(), p::command(1, 0, "first"), &p::worker())
        .await
        .unwrap();
    assert_eq!(first.step.candidate.as_ref().unwrap().window, 25);
    let state = first.step.state.as_ref().unwrap();
    assert_eq!(state.history.len(), 1);
    assert_eq!(state.history[0].net_sharpe, 1.5);
    assert_eq!(state.random_draws, 1);
    f.store.close().await;
    f.store = PgJobStore::open(f.options()).await.unwrap();
    let replay = f
        .store
        .advance_perturbation(&actor(), p::command(1, 0, "first"), &Unavailable)
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.step, first.step);
    p::seed(&f.store, 2, 25, false).await;
    let expected = p::worker()
        .advance(PerturbationWork {
            state: first.step.state.clone(),
            candidates: p::space().candidates,
            current_window: 25,
            observation: Some(WindowObservation {
                source_job_id: Some(JobId {
                    value: "job.2".to_owned(),
                }),
                candidate: Some(p::candidate(25)),
                net_sharpe: 2.5,
            }),
            failed_factor_ids: vec![],
        })
        .await
        .unwrap();
    let second = f
        .store
        .advance_perturbation(&actor(), p::command(2, 1, "second"), &p::worker())
        .await
        .unwrap();
    assert_eq!(second.step, expected);
    assert_eq!(second.step.reason, PerturbationReason::Gradient as i32);
    assert_eq!(second.step.state.as_ref().unwrap().history.len(), 2);
    assert!(second.step.state.as_ref().unwrap().momentum > 0.0);
    let old = f
        .store
        .advance_perturbation(&actor(), p::command(1, 0, "first"), &Unavailable)
        .await
        .unwrap();
    assert_eq!(old, replay);
    f.assert_steps(2).await;
}

#[tokio::test]
async fn failures_filter_real_worker_proposals() {
    let f = Fixture::new().await;
    p::seed(&f.store, 2, 25, true).await;
    let result = f
        .store
        .advance_perturbation(&actor(), p::command(1, 0, "first"), &p::worker())
        .await
        .unwrap();
    assert_ne!(result.step.candidate.as_ref().unwrap().window, 25);
    assert_eq!(result.step.state.as_ref().unwrap().history.len(), 1);
    let rejected = f
        .store
        .advance_perturbation(&actor(), p::command(2, 1, "failed"), &p::worker())
        .await
        .unwrap();
    assert_eq!(rejected.step.state.as_ref().unwrap().history.len(), 1);
    assert_ne!(rejected.step.candidate.as_ref().unwrap().window, 25);
    f.assert_steps(2).await;
}

#[tokio::test]
async fn full_failure_memory_returns_exhausted() {
    let f = Fixture::new().await;
    for (index, window) in [5, 10, 20, 25].into_iter().enumerate() {
        p::seed(&f.store, index as u32 + 2, window, true).await;
    }
    let result = f
        .store
        .advance_perturbation(&actor(), p::command(1, 0, "full"), &p::worker())
        .await
        .unwrap();
    assert_eq!(result.step.reason, PerturbationReason::Exhausted as i32);
    assert!(result.step.candidate.is_none());
    assert_eq!(result.step.state.as_ref().unwrap().random_draws, 0);
    f.assert_steps(1).await;
}

#[tokio::test]
async fn worker_failure_does_not_advance_state() {
    let f = Fixture::new().await;
    assert!(matches!(
        f.store
            .advance_perturbation(&actor(), p::command(1, 0, "failed"), &Unavailable)
            .await,
        Err(StoreError::Unavailable(_))
    ));
    f.assert_steps(0).await;
}

#[tokio::test]
async fn source_replay_does_not_duplicate_sharpe() {
    let f = Fixture::new().await;
    let first = f
        .store
        .advance_perturbation(&actor(), p::command(1, 0, "first"), &p::worker())
        .await
        .unwrap();
    let second = f
        .store
        .advance_perturbation(&actor(), p::command(1, 1, "second"), &p::worker())
        .await
        .unwrap();
    assert_eq!(
        first.step.state.as_ref().unwrap().history,
        second.step.state.as_ref().unwrap().history
    );
    assert_ne!(first.step.candidate, second.step.candidate);
    f.assert_steps(2).await;
}

#[tokio::test]
async fn revision_and_retry_conflicts_fail_closed() {
    let f = Fixture::new().await;
    f.store
        .advance_perturbation(&actor(), p::command(1, 0, "first"), &p::worker())
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .advance_perturbation(&actor(), p::command(1, 0, "other"), &Unavailable)
            .await,
        Err(StoreError::RevisionConflict)
    ));
    assert!(matches!(
        f.store
            .advance_perturbation(&actor(), p::command(1, 1, "first"), &Unavailable)
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    f.assert_steps(1).await;
}

#[tokio::test]
async fn unavailable_or_stale_context_blocks_replay() {
    let f = Fixture::new().await;
    f.store
        .advance_perturbation(&actor(), p::command(1, 0, "first"), &p::worker())
        .await
        .unwrap();
    for component in ProvenanceComponent::ALL {
        let mut current = p::space().provenance.unwrap();
        let field = match component {
            ProvenanceComponent::SourceCode => &mut current.source_code_sha256,
            ProvenanceComponent::OperatorRegistry => &mut current.operator_registry_sha256,
            ProvenanceComponent::Configuration => &mut current.configuration_sha256,
            ProvenanceComponent::DataManifest => &mut current.data_manifest_sha256,
            ProvenanceComponent::TradingCalendar => &mut current.trading_calendar_sha256,
            ProvenanceComponent::Environment => &mut current.environment_sha256,
        };
        *field = Some(digest(99));
        *f.policy.current.lock().unwrap() = Some(current);
        assert!(matches!(
            f.store
                .advance_perturbation(&actor(), p::command(1, 0, "first"), &Unavailable)
                .await,
            Err(StoreError::Provenance(_))
        ));
    }
    *f.policy.current.lock().unwrap() = None;
    assert!(matches!(
        f.store
            .advance_perturbation(&actor(), p::command(1, 0, "first"), &Unavailable)
            .await,
        Err(StoreError::Provenance(_))
    ));
    f.assert_steps(1).await;
}

#[tokio::test]
async fn default_resolver_denies_optimization() {
    let f = Fixture::new().await;
    let mut options = f.options();
    options.backtest_policy = Arc::new(DenyBacktest);
    let denied = PgJobStore::open(options).await.unwrap();
    assert!(matches!(
        denied
            .advance_perturbation(&actor(), p::command(1, 0, "denied"), &Unavailable)
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    f.assert_steps(0).await;
}

#[tokio::test]
async fn revoked_authority_blocks_replay() {
    let f = Fixture::new().await;
    f.store
        .advance_perturbation(&actor(), p::command(1, 0, "first"), &p::worker())
        .await
        .unwrap();
    *f.policy.denied.lock().unwrap() = true;
    assert!(matches!(
        f.store
            .advance_perturbation(&actor(), p::command(1, 0, "first"), &Unavailable)
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    f.assert_steps(1).await;
}

#[tokio::test]
async fn cancelled_source_is_not_a_sharpe_observation() {
    let f = Fixture::new().await;
    let mut submission = rejection::command(2);
    let Some(job_specification::Input::Backtest(input)) = &mut submission.specification.input
    else {
        unreachable!()
    };
    input.factor_spec_id = p::candidate(5).factor_spec_id;
    f.store.submit(submission).await.unwrap();
    f.store
        .mutate(
            &actor(),
            JobMutation::Cancel(loop_protocol::wire::jobs::v1::CancelJobRequest {
                context: Some(context("cancel")),
                job_id: Some(JobId {
                    value: "job.2".to_owned(),
                }),
                expected_revision: 1,
                reason: "fixture cancellation".to_owned(),
            }),
        )
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .advance_perturbation(&actor(), p::command(2, 0, "cancelled"), &Unavailable)
            .await,
        Err(StoreError::InvalidTransition)
    ));
    f.assert_steps(0).await;
}

#[tokio::test]
async fn changing_family_cannot_reset_state() {
    let f = Fixture::new().await;
    f.store
        .advance_perturbation(&actor(), p::command(1, 0, "first"), &p::worker())
        .await
        .unwrap();
    f.policy.space.lock().unwrap().random_seed = Some(digest(9));
    assert!(matches!(
        f.store
            .advance_perturbation(&actor(), p::command(1, 1, "reset"), &Unavailable)
            .await,
        Err(StoreError::Invalid("immutable perturbation space changed"))
    ));
    f.assert_steps(1).await;
}

#[tokio::test]
async fn ambiguous_sharpe_estimator_is_rejected() {
    let policy = Arc::new(p::Policy::default());
    *policy.metric_override.lock().unwrap() = Some(("1.5".to_owned(), "undefined.v1".to_owned()));
    let f = Fixture::with_policy(policy).await;
    assert!(matches!(
        f.store
            .advance_perturbation(&actor(), p::command(1, 0, "metric"), &Unavailable)
            .await,
        Err(StoreError::Invalid("IS net Sharpe metric"))
    ));
    f.assert_steps(0).await;
}

#[tokio::test]
async fn audit_failure_rolls_back_state_and_receipt() {
    let f = Fixture::new().await;
    let mut db = connection(&f.directory).await;
    db.execute("CREATE FUNCTION reject_proposal_audit() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture audit outage'; END; $$").await.unwrap();
    db.execute("CREATE TRIGGER reject_proposal_audit BEFORE INSERT ON audit_events FOR EACH ROW EXECUTE FUNCTION reject_proposal_audit()").await.unwrap();
    assert!(matches!(
        f.store
            .advance_perturbation(&actor(), p::command(1, 0, "rollback"), &p::worker())
            .await,
        Err(StoreError::Database(_))
    ));
    f.assert_steps(0).await;
    db.execute("DROP TRIGGER reject_proposal_audit ON audit_events")
        .await
        .unwrap();
    f.store
        .advance_perturbation(&actor(), p::command(1, 0, "rollback"), &p::worker())
        .await
        .unwrap();
    f.assert_steps(1).await;
}

#[tokio::test]
async fn sql_guards_preserve_history() {
    let f = Fixture::new().await;
    f.store
        .advance_perturbation(&actor(), p::command(1, 0, "first"), &p::worker())
        .await
        .unwrap();
    let mut db = connection(&f.directory).await;
    for sql in [
        "DELETE FROM perturbation_states",
        "UPDATE perturbation_states SET revision = revision + 2",
        "UPDATE perturbation_states SET revision = revision + 1, updated_at_ms = updated_at_ms - 1",
        "UPDATE perturbation_states SET revision = revision + 1, idempotency_key = 'missing'",
        "DELETE FROM command_receipts WHERE operation = 'loop.perturbation.advance'",
    ] {
        assert!(db.execute(sql).await.is_err(), "{sql}");
    }
    f.assert_steps(1).await;
}

#[tokio::test]
async fn corrupt_state_fails_before_worker() {
    let f = Fixture::new().await;
    f.store
        .advance_perturbation(&actor(), p::command(1, 0, "first"), &p::worker())
        .await
        .unwrap();
    let mut db = connection(&f.directory).await;
    db.execute("ALTER TABLE perturbation_states DISABLE TRIGGER USER")
        .await
        .unwrap();
    db.execute("UPDATE perturbation_states SET state_blob = decode('00', 'hex')")
        .await
        .unwrap();
    db.execute("ALTER TABLE perturbation_states ENABLE TRIGGER USER")
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .advance_perturbation(&actor(), p::command(1, 1, "corrupt"), &Unavailable)
            .await,
        Err(StoreError::Corrupt(_))
    ));
}

#[tokio::test]
async fn deadlines_and_clock_regression_fail_closed() {
    let f = Fixture::new().await;
    f.clock.0.store(NOW - 1, Ordering::SeqCst);
    assert!(matches!(
        f.store
            .advance_perturbation(&actor(), p::command(1, 0, "clock"), &Unavailable)
            .await,
        Err(StoreError::ClockRegression)
    ));
    f.clock.0.store(NOW + 30_000, Ordering::SeqCst);
    assert!(matches!(
        f.store
            .advance_perturbation(&actor(), p::command(1, 0, "expired"), &Unavailable)
            .await,
        Err(StoreError::Unavailable(_))
    ));
    f.assert_steps(0).await;
}

struct Pending;

impl PerturbationWorker for Pending {
    async fn advance(&self, _: PerturbationWork) -> StoreResult<PerturbationStep> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn cancelled_calculation_leaves_no_write_lock() {
    let f = Fixture::new().await;
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            f.store
                .advance_perturbation(&actor(), p::command(1, 0, "cancel"), &Pending)
        )
        .await
        .is_err()
    );
    f.assert_steps(0).await;
    f.store
        .advance_perturbation(&actor(), p::command(1, 0, "cancel"), &p::worker())
        .await
        .unwrap();
    f.assert_steps(1).await;
}

struct RejectDuringCompute<'a>(&'a PgJobStore);

impl PerturbationWorker for RejectDuringCompute<'_> {
    async fn advance(&self, work: PerturbationWork) -> StoreResult<PerturbationStep> {
        let step = p::worker().advance(work).await?;
        p::seed(self.0, 2, step.candidate.as_ref().unwrap().window, true).await;
        Ok(step)
    }
}

#[tokio::test]
async fn rejection_during_compute_blocks_commit() {
    let f = Fixture::new().await;
    assert!(matches!(
        f.store
            .advance_perturbation(
                &actor(),
                p::command(1, 0, "race"),
                &RejectDuringCompute(&f.store)
            )
            .await,
        Err(StoreError::PreviouslyRejected)
    ));
    f.assert_steps(0).await;
    let retry = f
        .store
        .advance_perturbation(&actor(), p::command(1, 0, "race"), &p::worker())
        .await
        .unwrap();
    assert_ne!(retry.step.candidate.as_ref().unwrap().window, 25);
    assert_eq!(retry.step.state.as_ref().unwrap().random_draws, 1);
    f.assert_steps(1).await;
}

struct AlterHistory;

impl PerturbationWorker for AlterHistory {
    async fn advance(&self, work: PerturbationWork) -> StoreResult<PerturbationStep> {
        let mut step = p::worker().advance(work).await?;
        step.state.as_mut().unwrap().history[0].net_sharpe = 999.0;
        Ok(step)
    }
}

#[tokio::test]
async fn worker_cannot_rewrite_registered_sharpe() {
    let f = Fixture::new().await;
    assert!(matches!(
        f.store
            .advance_perturbation(&actor(), p::command(1, 0, "tamper"), &AlterHistory)
            .await,
        Err(StoreError::Invalid("worker history transition"))
    ));
    f.assert_steps(0).await;
}

struct AdvanceClock(Arc<FixtureClock>);

impl PerturbationWorker for AdvanceClock {
    async fn advance(&self, work: PerturbationWork) -> StoreResult<PerturbationStep> {
        let step = p::worker().advance(work).await?;
        self.0.0.store(NOW + 30_000, Ordering::SeqCst);
        Ok(step)
    }
}

#[tokio::test]
async fn deadline_expiring_during_compute_rolls_back() {
    let f = Fixture::new().await;
    assert!(matches!(
        f.store
            .advance_perturbation(
                &actor(),
                p::command(1, 0, "deadline"),
                &AdvanceClock(f.clock.clone())
            )
            .await,
        Err(StoreError::Unavailable(_))
    ));
    f.assert_steps(0).await;
}

#[tokio::test]
async fn holdout_jobs_cannot_feed_optimization() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = options(
        &directory.path().join("state"),
        Arc::new(FixtureClock(AtomicI64::new(NOW))),
    );
    config.admission = Arc::new(batch::Admission);
    config.holdout_policy = Arc::new(batch::Policy::default());
    config.backtest_policy = Arc::new(p::Policy::default());
    let store = PgJobStore::open(config).await.unwrap();
    let (_, request) = batch::seed(&store).await;
    let result = store
        .consume_grant(&actor(), request, research::metadata())
        .await
        .unwrap();
    let mut command = p::command(1, 0, "holdout");
    command.source_job_id = Some(result.response.job_batch.as_ref().unwrap().job_ids[0].clone());
    assert!(matches!(
        store
            .advance_perturbation(&actor(), command, &Unavailable)
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    let states: i64 = sqlx::query_scalar("SELECT count(*) FROM perturbation_states")
        .fetch_one(&mut connection(&directory).await)
        .await
        .unwrap();
    assert_eq!(states, 0);
}
