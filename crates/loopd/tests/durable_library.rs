mod support;

use loop_core::audit::{AuditAction, verify_audit_chain};
use loop_protocol::wire::jobs::v1::CancelJobRequest;
use loop_protocol::wire::v1::*;
use loopd::store::{
    DenyBacktest, FactorRepository, HoldoutRepository, JobMutation, JobRepository, PgJobStore,
    StoreError,
};
use sqlx::{Connection, Executor};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use support::*;
use tempfile::TempDir;

async fn fixture() -> (TempDir, PgJobStore, Arc<FixtureClock>, Arc<library::Policy>) {
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let policy = Arc::new(library::Policy::default());
    let store = PgJobStore::open(library::options(
        &directory.path().join("state"),
        clock.clone(),
        policy.clone(),
    ))
    .await
    .unwrap();
    library::seed(&store, 1, 15).await;
    (directory, store, clock, policy)
}

async fn counts(dir: &TempDir) -> (i64, i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM factor_trials), (SELECT count(*) FROM factor_states), (SELECT count(*) FROM command_receipts), (SELECT count(*) FROM audit_events)")
        .fetch_one(&mut connection(dir).await).await.unwrap()
}

#[tokio::test]
async fn admission_replays_after_restart() {
    let (dir, store, clock, policy) = fixture().await;
    let command = library::command(1, 0, "decide.1");
    let first = store
        .decide_factor(&actor(), command.clone())
        .await
        .unwrap();
    assert_eq!(first.states[0].status, "admitted");
    assert_eq!(first.states[0].admissions, 1);
    let before = counts(&dir).await;
    store.close().await;
    let reopened = PgJobStore::open(library::options(&dir.path().join("state"), clock, policy))
        .await
        .unwrap();
    let replay = reopened.decide_factor(&actor(), command).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(first.states, replay.states);
    assert_eq!(counts(&dir).await, before);
    verify_audit_chain(&reopened.audit_events(0, 500).await.unwrap()).unwrap();
    reopened.close().await;
}

#[tokio::test]
async fn readmission_rechecks_coverage() {
    let (dir, store, _, policy) = fixture().await;
    policy.evidence.lock().unwrap().valid_observations = 949;
    let rejected = store
        .decide_factor(&actor(), library::command(1, 0, "first"))
        .await
        .unwrap();
    assert_eq!(rejected.rejection_code, "insufficient_coverage");
    library::seed(&store, 2, 15).await;
    let still_rejected = store
        .decide_factor(&actor(), library::command(2, 1, "again"))
        .await
        .unwrap();
    assert_eq!(still_rejected.states[0].revision, 2);
    assert_eq!(still_rejected.rejection_code, "insufficient_coverage");
    library::seed(&store, 3, 15).await;
    policy.evidence.lock().unwrap().valid_observations = 950;
    let admitted = store
        .decide_factor(&actor(), library::command(3, 2, "last"))
        .await
        .unwrap();
    assert_eq!(admitted.states[0].admissions, 1);
    assert_eq!(admitted.states[0].revision, 3);
    assert_eq!(counts(&dir).await.0, 3);
    assert_eq!(
        store
            .audit_events(0, 500)
            .await
            .unwrap()
            .iter()
            .filter(|e| e.action == AuditAction::FactorRejected)
            .count(),
        2
    );
    store.close().await;
}

#[tokio::test]
async fn human_override_is_explicit_and_replayable() {
    let (dir, store, _, policy) = fixture().await;
    policy.evidence.lock().unwrap().semantic_accepted = false;
    store
        .decide_factor(&actor(), library::command(1, 0, "reject"))
        .await
        .unwrap();
    *policy.override_allowed.lock().unwrap() = true;
    let forced = library::forced(1, 1, "override");
    let decision = store
        .decide_factor(&library::human(), forced.clone())
        .await
        .unwrap();
    assert!(decision.override_applied);
    assert_eq!(decision.states[0].status, "admitted");
    let before = counts(&dir).await;
    assert!(
        store
            .decide_factor(&library::human(), forced.clone())
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(counts(&dir).await, before);
    let events = store.audit_events(0, 500).await.unwrap();
    verify_audit_chain(&events).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.action == AuditAction::OverrideAuthorized)
            .count(),
        1
    );
    assert!(
        String::from_utf8(events.last().unwrap().payload.canonical_bytes.clone())
            .unwrap()
            .contains("override_applied=true")
    );
    *policy.override_allowed.lock().unwrap() = false;
    assert!(matches!(
        store.decide_factor(&library::human(), forced).await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}

#[tokio::test]
async fn override_cannot_bypass_machine_gates() {
    let (dir, store, _, policy) = fixture().await;
    *policy.override_allowed.lock().unwrap() = true;
    let before = counts(&dir).await;
    for code in [
        "deterministic_filter",
        "performance",
        "correlation",
        "policy",
    ] {
        policy.evidence.lock().unwrap().machine_rejection = code.to_owned();
        assert!(matches!(
            store
                .decide_factor(&library::human(), library::forced(1, 0, "force"))
                .await,
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(counts(&dir).await, before);
    }
    let mut evidence = library::evidence();
    evidence.valid_observations = 1;
    *policy.evidence.lock().unwrap() = evidence;
    assert!(matches!(
        store
            .decide_factor(&library::human(), library::forced(1, 0, "force"))
            .await,
        Err(StoreError::Invalid(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn missing_review_is_not_rejection() {
    let (dir, store, _, policy) = fixture().await;
    let before = counts(&dir).await;
    *policy.available.lock().unwrap() = false;
    assert!(matches!(
        store
            .decide_factor(&actor(), library::command(1, 0, "decide"))
            .await,
        Err(StoreError::Unavailable(_))
    ));
    assert_eq!(counts(&dir).await, before);
    store.close().await;
}

#[tokio::test]
async fn replaces_and_preserves_lifetime_retirements() {
    let (_dir, store, _, policy) = fixture().await;
    let first = store
        .decide_factor(&actor(), library::command(1, 0, "first"))
        .await
        .unwrap();
    library::seed(&store, 2, 20).await;
    {
        let mut evidence = policy.evidence.lock().unwrap();
        evidence.library_sha256 = library::snapshot(&first.states);
        evidence.replacements = vec![first.states[0].factor_spec_id.clone()];
    }
    let next = store
        .decide_factor(&actor(), library::command(2, 0, "replace"))
        .await
        .unwrap();
    assert_eq!(next.states[1].retirements, 1);
    assert_eq!(next.states[1].admissions, 1);
    assert_eq!(next.states[1].status, "retired");
    library::seed(&store, 3, 15).await;
    {
        let mut evidence = policy.evidence.lock().unwrap();
        evidence.library_sha256 = library::snapshot(&next.states);
        evidence.replacements.clear();
    }
    let readmitted = store
        .decide_factor(&actor(), library::command(3, 2, "readmit"))
        .await
        .unwrap();
    assert_eq!(readmitted.states[0].admissions, 2);
    assert_eq!(readmitted.states[0].retirements, 1);
    assert_eq!(readmitted.states[0].revision, 3);
    verify_audit_chain(&store.audit_events(0, 500).await.unwrap()).unwrap();
    store.close().await;
}

#[tokio::test]
async fn replacement_failure_rolls_back_admission() {
    let (dir, store, _, policy) = fixture().await;
    policy.evidence.lock().unwrap().replacements =
        vec![perturbation::candidate(20).factor_spec_id.unwrap().value];
    let before = counts(&dir).await;
    assert!(matches!(
        store
            .decide_factor(&actor(), library::command(1, 0, "replace"))
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(counts(&dir).await, before);
    store.close().await;
}

#[tokio::test]
async fn stale_library_blocks_decision() {
    let (dir, store, _, _) = fixture().await;
    store
        .decide_factor(&actor(), library::command(1, 0, "first"))
        .await
        .unwrap();
    library::seed(&store, 2, 20).await;
    let before = counts(&dir).await;
    assert!(matches!(
        store
            .decide_factor(&actor(), library::command(2, 0, "stale"))
            .await,
        Err(StoreError::RevisionConflict)
    ));
    assert_eq!(counts(&dir).await, before);
    store.close().await;
}

#[tokio::test]
async fn stale_provenance_blocks_retry() {
    let (dir, store, _, policy) = fixture().await;
    let request = library::command(1, 0, "decide");
    store
        .decide_factor(&actor(), request.clone())
        .await
        .unwrap();
    let before = counts(&dir).await;
    for component in 0..6 {
        let mut changed = perturbation::space().provenance.unwrap();
        let parts = [
            &mut changed.source_code_sha256,
            &mut changed.operator_registry_sha256,
            &mut changed.configuration_sha256,
            &mut changed.data_manifest_sha256,
            &mut changed.trading_calendar_sha256,
            &mut changed.environment_sha256,
        ];
        *parts.into_iter().nth(component).unwrap() = Some(digest(77));
        *policy.current.lock().unwrap() = Some(changed);
        assert!(matches!(
            store.decide_factor(&actor(), request.clone()).await,
            Err(StoreError::Provenance(_))
        ));
    }
    assert_eq!(counts(&dir).await, before);
    store.close().await;
}

#[tokio::test]
async fn authorization_and_default_deny() {
    let (dir, store, clock, _) = fixture().await;
    let mut spoofed = library::command(1, 0, "spoof");
    spoofed.context.as_mut().unwrap().actor = Some(library::human());
    assert!(matches!(
        store.decide_factor(&actor(), spoofed).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(matches!(
        store
            .decide_factor(&actor(), library::forced(1, 0, "force"))
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
    let mut options = library::options(
        &dir.path().join("state"),
        clock,
        Arc::new(library::Policy::default()),
    );
    options.backtest_policy = Arc::new(DenyBacktest);
    let denied = PgJobStore::open(options).await.unwrap();
    assert!(matches!(
        denied
            .decide_factor(&actor(), library::command(1, 0, "denied"))
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    denied.close().await;
}

#[tokio::test]
async fn revisions_and_keys_are_fenced() {
    let (dir, store, _, _) = fixture().await;
    let request = library::command(1, 0, "decide");
    store
        .decide_factor(&actor(), request.clone())
        .await
        .unwrap();
    let before = counts(&dir).await;
    let mut changed = request;
    changed.reason = "changed".to_owned();
    assert!(matches!(
        store.decide_factor(&actor(), changed).await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert!(matches!(
        store
            .decide_factor(&actor(), library::command(1, 0, "another"))
            .await,
        Err(StoreError::RevisionConflict)
    ));
    assert!(matches!(
        store
            .decide_factor(&actor(), library::command(1, 1, "another"))
            .await,
        Err(StoreError::InvalidTransition)
    ));
    assert_eq!(counts(&dir).await, before);
    store.close().await;
}

#[tokio::test]
async fn deadlines_and_clock_regression_fail_closed() {
    let (dir, store, clock, _) = fixture().await;
    let before = counts(&dir).await;
    clock.0.store(NOW - 1, Ordering::SeqCst);
    assert!(matches!(
        store
            .decide_factor(&actor(), library::command(1, 0, "old"))
            .await,
        Err(StoreError::ClockRegression)
    ));
    clock.0.store(NOW + 30_000, Ordering::SeqCst);
    assert!(matches!(
        store
            .decide_factor(&actor(), library::command(1, 0, "late"))
            .await,
        Err(StoreError::Unavailable(_))
    ));
    assert_eq!(counts(&dir).await, before);
    store.close().await;
}

#[tokio::test]
async fn trials_preserve_preexecution_cancellation() {
    let (dir, store, _, _) = fixture().await;
    let mut submission = rejection::command(2);
    if let Some(job_specification::Input::Backtest(input)) = &mut submission.specification.input {
        input.factor_spec_id = perturbation::candidate(20).factor_spec_id;
    }
    let run = submission
        .specification
        .run_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    store.submit(submission.clone()).await.unwrap();
    assert!(store.submit(submission).await.unwrap().replayed);
    store
        .mutate(
            &actor(),
            JobMutation::Cancel(CancelJobRequest {
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
    let trials = store.factor_trials(&actor(), &run, "", 500).await.unwrap();
    let cancelled = trials.iter().find(|trial| trial.job_id == "job.2").unwrap();
    assert_eq!(cancelled.state, JobState::Cancelled as i32);
    assert_eq!(cancelled.attempt, 0);
    assert_eq!(counts(&dir).await.0, 2);
    assert!(matches!(
        store
            .decide_factor(&actor(), library::command(2, 0, "cancelled"))
            .await,
        Err(StoreError::InvalidTransition)
    ));
    store.close().await;
}

#[tokio::test]
async fn audit_failure_rolls_back_decision() {
    let (dir, store, _, _) = fixture().await;
    connection(&dir).await.execute("CREATE FUNCTION fail_factor_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture audit outage'; END $$; CREATE TRIGGER fail_factor_audit BEFORE INSERT ON audit_events FOR EACH ROW EXECUTE FUNCTION fail_factor_audit();").await.unwrap();
    let before = counts(&dir).await;
    assert!(matches!(
        store
            .decide_factor(&actor(), library::command(1, 0, "fail"))
            .await,
        Err(StoreError::Database(_))
    ));
    assert_eq!(counts(&dir).await, before);
    store.close().await;
}

#[tokio::test]
async fn sql_guards_preserve_history() {
    let (dir, store, _, _) = fixture().await;
    store
        .decide_factor(&actor(), library::command(1, 0, "first"))
        .await
        .unwrap();
    for sql in [
        "DELETE FROM factor_trials",
        "UPDATE factor_trials SET factor_spec_id = factor_spec_id",
        "DELETE FROM factor_states",
        "UPDATE factor_states SET revision = revision + 2",
        "UPDATE factor_states SET retirements = 0, admissions = 0",
    ] {
        assert!(connection(&dir).await.execute(sql).await.is_err(), "{sql}");
    }
    store.close().await;
}

#[tokio::test]
async fn malformed_evidence_cannot_create_decisions() {
    let (dir, store, _, policy) = fixture().await;
    let before = counts(&dir).await;
    for mutate in [
        |e: &mut loopd::store::AdmissionEvidence| e.report = None,
        |e: &mut loopd::store::AdmissionEvidence| e.eligible_observations = 0,
        |e: &mut loopd::store::AdmissionEvidence| e.valid_observations = 1001,
        |e: &mut loopd::store::AdmissionEvidence| e.minimum_coverage_bps = 0,
        |e: &mut loopd::store::AdmissionEvidence| e.minimum_coverage_bps = 10001,
        |e: &mut loopd::store::AdmissionEvidence| e.result_manifest_sha256 = vec![7; 32],
        |e: &mut loopd::store::AdmissionEvidence| {
            e.policy.as_mut().unwrap().revision = "01".to_owned()
        },
    ] {
        let mut evidence = library::evidence();
        mutate(&mut evidence);
        *policy.evidence.lock().unwrap() = evidence;
        assert!(
            store
                .decide_factor(&actor(), library::command(1, 0, "bad"))
                .await
                .is_err()
        );
        assert_eq!(counts(&dir).await, before);
    }
    store.close().await;
}

#[tokio::test]
async fn trial_corruption_blocks_admission_and_reads() {
    let (dir, store, _, _) = fixture().await;
    connection(&dir).await.execute("ALTER TABLE factor_trials DISABLE TRIGGER factor_trials_immutable; UPDATE factor_trials SET specification_sha256 = decode(repeat('00', 32), 'hex');").await.unwrap();
    assert!(matches!(
        store
            .decide_factor(&actor(), library::command(1, 0, "bad"))
            .await,
        Err(StoreError::Corrupt(_))
    ));
    let run = command(1).specification.run_id.unwrap().value;
    assert!(matches!(
        store.factor_trials(&actor(), &run, "", 10).await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn factor_projection_corruption_is_not_a_new_factor() {
    let (dir, store, _, policy) = fixture().await;
    let admitted = store
        .decide_factor(&actor(), library::command(1, 0, "first"))
        .await
        .unwrap();
    connection(&dir).await.execute("ALTER TABLE factor_states DISABLE TRIGGER factor_states_fenced; UPDATE factor_states SET revision = revision + 1;").await.unwrap();
    library::seed(&store, 2, 20).await;
    policy.evidence.lock().unwrap().library_sha256 = library::snapshot(&admitted.states);
    assert!(matches!(
        store
            .decide_factor(&actor(), library::command(2, 0, "next"))
            .await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn receipt_corruption_blocks_replay() {
    let (dir, store, _, _) = fixture().await;
    store
        .decide_factor(&actor(), library::command(1, 0, "first"))
        .await
        .unwrap();
    let triggers: Vec<String> = sqlx::query_scalar("SELECT tgname FROM pg_trigger WHERE tgrelid = 'command_receipts'::regclass AND NOT tgisinternal").fetch_all(&mut connection(&dir).await).await.unwrap();
    for trigger in triggers {
        connection(&dir)
            .await
            .execute(format!("ALTER TABLE command_receipts DISABLE TRIGGER \"{trigger}\"").as_str())
            .await
            .unwrap();
    }
    connection(&dir).await.execute("UPDATE command_receipts SET response_sha256 = decode(repeat('00', 32), 'hex') WHERE operation = 'loop.factors.decide'").await.unwrap();
    assert!(matches!(
        store
            .decide_factor(&actor(), library::command(1, 0, "first"))
            .await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn cancelled_waiter_does_not_publish() {
    let (dir, store, _, _) = fixture().await;
    let before = counts(&dir).await;
    let mut conn = connection(&dir).await;
    let mut tx = conn.begin().await.unwrap();
    sqlx::query("SELECT singleton FROM store_metadata FOR UPDATE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let task = tokio::spawn({
        let store = store.clone();
        async move {
            store
                .decide_factor(&actor(), library::command(1, 0, "cancel"))
                .await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tx.rollback().await.unwrap();
    assert_eq!(counts(&dir).await, before);
    assert!(
        !store
            .decide_factor(&actor(), library::command(1, 0, "cancel"))
            .await
            .unwrap()
            .replayed
    );
    store.close().await;
}

#[tokio::test]
async fn factor_evaluation_submissions_are_trials() {
    let (dir, store, _, _) = fixture().await;
    let mut request = command(2);
    let (kind, input) = research::inputs().remove(1);
    request.specification.kind = kind as i32;
    request.specification.input = Some(input);
    let run = request.specification.run_id.as_ref().unwrap().value.clone();
    store.submit(request).await.unwrap();
    let page = store
        .factor_trials(&actor(), &run, "job.1", 1)
        .await
        .unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(
        (page[0].state, page[0].attempt),
        (JobState::Queued as i32, 0)
    );
    assert_eq!(counts(&dir).await.0, 2);
    assert!(matches!(
        store
            .decide_factor(&actor(), library::command(2, 0, "wrong"))
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}

#[tokio::test]
async fn rejects_and_infrastructure_failures_remain_trials() {
    let (_dir, store, clock, _) = fixture().await;
    perturbation::seed(&store, 2, 20, true).await;
    let mut submission = rejection::command(3);
    if let Some(job_specification::Input::Backtest(input)) = &mut submission.specification.input {
        input.factor_spec_id = perturbation::candidate(25).factor_spec_id;
    }
    let run = submission
        .specification
        .run_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    store.submit(submission).await.unwrap();
    store
        .mutate(
            &actor(),
            JobMutation::Acquire(loop_protocol::wire::jobs::v1::AcquireJobLeaseRequest {
                context: Some(context("lease.fail")),
                job_id: Some(JobId {
                    value: "job.3".to_owned(),
                }),
                expected_revision: 1,
                requested_duration: Some(prost_types::Duration {
                    seconds: 30,
                    nanos: 0,
                }),
            }),
        )
        .await
        .unwrap();
    clock.0.store(NOW + 30_000, Ordering::SeqCst);
    store
        .mutate(
            &actor(),
            JobMutation::Recover(loopd::store::RecoveryCommand {
                context: Some(context("recover.fail")),
                job_id: Some(JobId {
                    value: "job.3".to_owned(),
                }),
                expected_revision: 2,
            }),
        )
        .await
        .unwrap();
    let page = store.factor_trials(&actor(), &run, "", 500).await.unwrap();
    assert_eq!(page.len(), 3);
    assert_eq!(page[1].state, JobState::FactorRejected as i32);
    assert_eq!(page[2].state, JobState::InfrastructureFailed as i32);
    assert!(page.iter().all(|trial| trial.attempt == 1));
    store.close().await;
}

#[tokio::test]
async fn migration_refuses_unindexed_trials() {
    let (dir, store, _, _) = fixture().await;
    let mut conn = connection(&dir).await;
    conn.execute("CREATE TEMP TABLE jobs (kind INTEGER); INSERT INTO jobs VALUES (2);")
        .await
        .unwrap();
    let error = conn
        .execute(include_str!(
            "../../../migrations/postgres/0009_factor_admission.sql"
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("23514")
    );
    assert_eq!(
        error.as_database_error().unwrap().message(),
        "existing research jobs require verified trial migration"
    );
    store.close().await;
}

#[tokio::test]
async fn protected_jobs_are_not_admission_trials() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = options(
        &directory.path().join("state"),
        Arc::new(FixtureClock(AtomicI64::new(NOW))),
    );
    config.admission = Arc::new(batch::Admission);
    config.holdout_policy = Arc::new(batch::Policy::default());
    config.backtest_policy = Arc::new(library::Policy::default());
    let store = PgJobStore::open(config).await.unwrap();
    let (_, request) = batch::seed(&store).await;
    let result = store
        .consume_grant(&actor(), request, research::metadata())
        .await
        .unwrap();
    let mut command = library::command(1, 0, "protected");
    command.source_job_id = Some(result.response.job_batch.as_ref().unwrap().job_ids[0].clone());
    assert!(matches!(
        store.decide_factor(&actor(), command).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert_eq!(counts(&directory).await.0, 0);
    assert_eq!(counts(&directory).await.1, 0);
    store.close().await;
}

#[tokio::test]
async fn trial_pages_require_authority_and_bounds() {
    let (_dir, store, _, _) = fixture().await;
    let run = command(1).specification.run_id.unwrap().value;
    let mut stranger = actor();
    stranger.authenticated_subject = "unknown".to_owned();
    assert!(matches!(
        store.factor_trials(&stranger, &run, "", 10).await,
        Err(StoreError::AdmissionDenied)
    ));
    for limit in [0, 501] {
        assert!(matches!(
            store.factor_trials(&actor(), &run, "", limit).await,
            Err(StoreError::Invalid(_))
        ));
    }
    assert_eq!(
        store
            .factor_trials(&actor(), &run, "", 1)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .factor_trials(&actor(), &run, "job.1", 1)
            .await
            .unwrap()
            .is_empty()
    );
    store.close().await;
}
