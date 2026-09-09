mod support;

use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use std::time::Duration;

use loop_core::audit::{AuditAction, verify_audit_chain};
use loop_protocol::job::validate_holdout_backtest_plan_entry_binding;
use loop_protocol::wire::jobs::v1::CancelJobRequest;
use loop_protocol::wire::v1::{
    HoldoutGrantState, HoldoutPeriodState, JobKind, RequestId, job_specification,
};
use loopd::store::{
    DenyHoldout, DenySubmission, GrantClosure, HoldoutRepository, JobMutation, JobRepository,
    PgJobStore, StoreError,
};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::Connection;
use support::*;

#[tokio::test]
async fn consumes_complete_frozen_plan() {
    let (_directory, store, _, issued, request) = batch::setup().await;
    let result = store
        .consume_grant(&actor(), request, research::metadata())
        .await
        .unwrap();
    assert!(!result.replayed);
    let handle = result.response.job_batch.unwrap();
    assert_eq!(
        (
            handle.job_count,
            handle.evaluation_plan_entry_count,
            handle.revision
        ),
        (2, 2, 1)
    );
    assert_eq!(handle.job_ids.len(), 2);
    let resolved = grant::resolved(0);
    for (index, id) in handle.job_ids.iter().enumerate() {
        let record = store.get(&id.value).await.unwrap().unwrap();
        assert_eq!((record.revision, record.attempt), (1, 0));
        let spec = record.specification.unwrap();
        assert_eq!(spec.kind, JobKind::HoldoutBacktest as i32);
        let job_specification::Input::HoldoutBacktest(input) = spec.input.unwrap() else {
            panic!("holdout input")
        };
        assert_eq!(input.job_batch_id, handle.job_batch_id);
        assert_eq!(input.evaluation_plan_entry_index, index as u32 + 1);
        assert_eq!(input.consumed_grant_revision, 2);
        validate_holdout_backtest_plan_entry_binding(
            &input,
            spec.submitted_at.as_ref().unwrap(),
            &holdout::command(0, "fixture").canonical_bytes,
            &resolved.canonical_plan,
            &resolved.backtest_schema_sha256,
            &resolved.backtest_artifacts,
        )
        .unwrap();
    }
    let id = issued.period.issued_grant_id.unwrap().value;
    let consumed = store.get_grant(&actor(), &id).await.unwrap().unwrap();
    assert_eq!(consumed.state, HoldoutGrantState::Consumed as i32);
    assert_eq!(consumed.consumed_at, Some(timestamp(NOW)));
    assert_eq!(
        result.response.period_record.unwrap().state,
        HoldoutPeriodState::Consumed as i32
    );
    let events = store.audit_events(0, 500).await.unwrap();
    assert_eq!(events.len(), 7);
    assert_eq!(events[6].action, AuditAction::HoldoutGrantConsumed);
    verify_audit_chain(&events).unwrap();
    store.close().await;
}

#[tokio::test]
async fn replay_survives_restart_and_expiry() {
    let (directory, store, clock, _, mut request) = batch::setup().await;
    let original = store
        .consume_grant(&actor(), request.clone(), research::metadata())
        .await
        .unwrap();
    store.close().await;
    clock.0.store(NOW + 3_600_000, Ordering::SeqCst);
    let mut config = options(&directory.path().join("state"), clock);
    config.holdout_policy = Arc::new(batch::Policy::default());
    config.admission = Arc::new(batch::Admission);
    let reopened = PgJobStore::open(config).await.unwrap();
    request.context.as_mut().unwrap().request_id = Some(RequestId {
        value: "request.retry".to_owned(),
    });
    request.context.as_mut().unwrap().requested_at = Some(timestamp(NOW + 3_600_000));
    let replay = reopened
        .consume_grant(&actor(), request, research::metadata())
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.response, original.response);
    assert_eq!(reopened.audit_events(0, 500).await.unwrap().len(), 7);
    reopened.close().await;
}

#[tokio::test]
async fn changed_run_cannot_reuse_receipt() {
    let (_directory, store, _, _, request) = batch::setup().await;
    store
        .consume_grant(&actor(), request.clone(), research::metadata())
        .await
        .unwrap();
    let mut metadata = research::metadata();
    metadata.run_id.value = "run.changed".to_owned();
    assert!(matches!(
        store.consume_grant(&actor(), request, metadata).await,
        Err(StoreError::IdempotencyConflict)
    ));
    store.close().await;
}

#[tokio::test]
async fn different_key_cannot_consume_twice() {
    let (_directory, store, _, _, mut request) = batch::setup().await;
    store
        .consume_grant(&actor(), request.clone(), research::metadata())
        .await
        .unwrap();
    request.context = Some(context("another"));
    assert!(matches!(
        store
            .consume_grant(&actor(), request.clone(), research::metadata())
            .await,
        Err(StoreError::RevisionConflict)
    ));
    request.expected_grant_revision = 2;
    request.expected_period_revision = 3;
    assert!(matches!(
        store
            .consume_grant(&actor(), request, research::metadata())
            .await,
        Err(StoreError::InvalidTransition)
    ));
    store.close().await;
}

#[tokio::test]
async fn expired_grant_cannot_consume() {
    let (_directory, store, clock, _, request) = batch::setup().await;
    clock.0.store(NOW + 60_000, Ordering::SeqCst);
    assert!(matches!(
        store
            .consume_grant(&actor(), request, research::metadata())
            .await,
        Err(StoreError::Invalid("consume grant validity"))
    ));
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 4);
    store.close().await;
}

#[tokio::test]
async fn revoked_grant_cannot_consume() {
    let (_directory, store, _, issued, mut request) = batch::setup().await;
    store
        .close_grant(
            &actor(),
            grant::close(&issued, GrantClosure::Revoke, "revoke"),
        )
        .await
        .unwrap();
    request.expected_grant_revision = 2;
    request.expected_period_revision = 3;
    assert!(matches!(
        store
            .consume_grant(&actor(), request, research::metadata())
            .await,
        Err(StoreError::InvalidTransition)
    ));
    store.close().await;
}

#[tokio::test]
async fn rejects_materializer_binding_changes() {
    for field in [
        "factor",
        "sample",
        "snapshots",
        "digest",
        "configuration",
        "source",
        "data",
        "seed",
    ] {
        let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
        let policy = batch::Policy {
            wrong_field: Some(field),
            ..Default::default()
        };
        let (directory, store, issued, request) = batch::setup_with(policy, clock).await;
        let outcome = store
            .consume_grant(&actor(), request, research::metadata())
            .await;
        assert!(
            matches!(outcome, Err(StoreError::Invalid(_) | StoreError::Job(_))),
            "field {field}: {outcome:?}"
        );
        assert_rolled_back(&directory, &store, &issued).await;
        store.close().await;
    }
}

#[tokio::test]
async fn unavailable_parser_is_infrastructure_failure() {
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let policy = batch::Policy {
        wrong_field: Some("unavailable"),
        ..Default::default()
    };
    let (directory, store, issued, request) = batch::setup_with(policy, clock).await;
    assert!(matches!(
        store
            .consume_grant(&actor(), request, research::metadata())
            .await,
        Err(StoreError::Unavailable(_))
    ));
    assert_rolled_back(&directory, &store, &issued).await;
    store.close().await;
}

#[tokio::test]
async fn default_authority_denies_consumption() {
    let (directory, store, clock, _, request) = batch::setup().await;
    store.close().await;
    let mut config = options(&directory.path().join("state"), clock);
    config.holdout_policy = Arc::new(DenyHoldout);
    config.admission = Arc::new(batch::Admission);
    let reopened = PgJobStore::open(config).await.unwrap();
    assert!(matches!(
        reopened
            .consume_grant(&actor(), request, research::metadata())
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    reopened.close().await;
}

#[tokio::test]
async fn job_admission_remains_independent() {
    let (directory, store, clock, issued, request) = batch::setup().await;
    store.close().await;
    let mut config = options(&directory.path().join("state"), clock);
    config.holdout_policy = Arc::new(batch::Policy::default());
    config.admission = Arc::new(DenySubmission);
    let reopened = PgJobStore::open(config).await.unwrap();
    assert!(matches!(
        reopened
            .consume_grant(&actor(), request, research::metadata())
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert_rolled_back(&directory, &reopened, &issued).await;
    reopened.close().await;
}

#[tokio::test]
async fn changed_grant_reference_is_rejected() {
    let (_directory, store, _, _, mut request) = batch::setup().await;
    request
        .grant_reference
        .as_mut()
        .unwrap()
        .evaluation_plan_entry_count = 1;
    assert!(matches!(
        store
            .consume_grant(&actor(), request, research::metadata())
            .await,
        Err(StoreError::Invalid("consume grant binding"))
    ));
    store.close().await;
}

#[tokio::test]
async fn actor_spoofing_is_rejected() {
    let (_directory, store, _, _, mut request) = batch::setup().await;
    request
        .context
        .as_mut()
        .unwrap()
        .actor
        .as_mut()
        .unwrap()
        .authenticated_subject = "forged".to_owned();
    assert!(matches!(
        store
            .consume_grant(&actor(), request, research::metadata())
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}

#[tokio::test]
async fn second_job_failure_rolls_back_batch() {
    let (directory, store, _, issued, request) = batch::setup().await;
    let mut database = connection(&directory).await;
    sqlx::query("CREATE FUNCTION reject_second_job() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN
        IF EXISTS(SELECT FROM jobs) THEN RAISE EXCEPTION 'injected second job failure'; END IF; RETURN NEW; END; $$")
        .execute(&mut database).await.unwrap();
    sqlx::query("CREATE TRIGGER injected_failure BEFORE INSERT ON jobs FOR EACH ROW EXECUTE FUNCTION reject_second_job()")
        .execute(&mut database).await.unwrap();
    assert!(matches!(
        store
            .consume_grant(&actor(), request, research::metadata())
            .await,
        Err(StoreError::Database(_))
    ));
    assert_rolled_back(&directory, &store, &issued).await;
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn receipt_failure_rolls_back_all_jobs() {
    let (directory, store, _, issued, request) = batch::setup().await;
    let mut database = connection(&directory).await;
    sqlx::query("CREATE TRIGGER injected_failure BEFORE INSERT ON holdout_command_receipts FOR EACH ROW EXECUTE FUNCTION reject_immutable_change()")
        .execute(&mut database).await.unwrap();
    assert!(matches!(
        store
            .consume_grant(&actor(), request, research::metadata())
            .await,
        Err(StoreError::Database(_))
    ));
    assert_rolled_back(&directory, &store, &issued).await;
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn grant_expiry_during_materialization_rolls_back() {
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let policy = batch::Policy {
        advance_clock: Some((clock.clone(), NOW + 60_000)),
        ..Default::default()
    };
    let (directory, store, issued, request) = batch::setup_with(policy, clock).await;
    assert!(matches!(
        store
            .consume_grant(&actor(), request, research::metadata())
            .await,
        Err(StoreError::Invalid("grant expired before commit"))
    ));
    assert_rolled_back(&directory, &store, &issued).await;
    store.close().await;
}

#[tokio::test]
async fn clock_regression_during_command_rolls_back() {
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let policy = batch::Policy {
        advance_clock: Some((clock.clone(), NOW - 1)),
        ..Default::default()
    };
    let (directory, store, issued, request) = batch::setup_with(policy, clock).await;
    assert!(matches!(
        store
            .consume_grant(&actor(), request, research::metadata())
            .await,
        Err(StoreError::ClockRegression)
    ));
    assert_rolled_back(&directory, &store, &issued).await;
    store.close().await;
}

#[tokio::test]
async fn cancellation_preserves_issued_grant() {
    let (directory, store, _, issued, request) = batch::setup().await;
    let mut database = connection(&directory).await;
    let mut transaction = database.begin().await.unwrap();
    sqlx::query("SELECT singleton FROM store_metadata WHERE singleton=1 FOR UPDATE")
        .fetch_one(&mut *transaction)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            store.consume_grant(&actor(), request.clone(), research::metadata())
        )
        .await
        .is_err()
    );
    transaction.rollback().await.unwrap();
    assert_rolled_back(&directory, &store, &issued).await;
    assert!(
        !store
            .consume_grant(&actor(), request, research::metadata())
            .await
            .unwrap()
            .replayed
    );
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn replay_tolerates_job_lifecycle_changes() {
    let (_directory, store, _, _, request) = batch::setup().await;
    let original = store
        .consume_grant(&actor(), request.clone(), research::metadata())
        .await
        .unwrap();
    let job_id = original.response.job_batch.as_ref().unwrap().job_ids[0].clone();
    store
        .mutate(
            &actor(),
            JobMutation::Cancel(CancelJobRequest {
                context: Some(context("cancel.job")),
                job_id: Some(job_id),
                expected_revision: 1,
                reason: "Fixture cancellation".to_owned(),
            }),
        )
        .await
        .unwrap();
    let retry = store
        .consume_grant(&actor(), request, research::metadata())
        .await
        .unwrap();
    assert!(retry.replayed);
    assert_eq!(retry.response, original.response);
    store.close().await;
}

#[tokio::test]
async fn rehashed_batch_receipt_cannot_change_order() {
    let (directory, store, _, _, request) = batch::setup().await;
    let mut response = store
        .consume_grant(&actor(), request.clone(), research::metadata())
        .await
        .unwrap()
        .response;
    response.job_batch.as_mut().unwrap().job_ids.reverse();
    let bytes = response.encode_to_vec();
    let mut database = connection(&directory).await;
    sqlx::query("DROP TRIGGER holdout_receipts_no_update ON holdout_command_receipts")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("UPDATE holdout_command_receipts SET response_blob=$1,response_sha256=$2 WHERE operation='loop.holdout.consume_grant'")
        .bind(&bytes).bind(Sha256::digest(&bytes).as_slice()).execute(&mut database).await.unwrap();
    assert!(matches!(
        store
            .consume_grant(&actor(), request, research::metadata())
            .await,
        Err(StoreError::Corrupt(_))
    ));
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn database_protects_batch_membership() {
    let (directory, store, _, _, request) = batch::setup().await;
    store
        .consume_grant(&actor(), request, research::metadata())
        .await
        .unwrap();
    let mut database = connection(&directory).await;
    for query in [
        "DELETE FROM holdout_batches",
        "DELETE FROM holdout_batch_jobs",
        "UPDATE holdout_batch_jobs SET entry_index=3",
        "UPDATE holdout_batches SET job_count=1",
        "UPDATE jobs SET kind=5",
        "UPDATE jobs SET run_id='other.run'",
    ] {
        assert!(
            sqlx::query(query).execute(&mut database).await.is_err(),
            "accepted {query}"
        );
    }
    database.close().await.unwrap();
    store.close().await;
}

async fn assert_rolled_back(
    directory: &tempfile::TempDir,
    store: &PgJobStore,
    issued: &loopd::store::GrantResult,
) {
    let mut database = connection(directory).await;
    for table in ["jobs", "holdout_batches", "holdout_batch_jobs"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&mut database)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    let watermark: i64 = sqlx::query_scalar("SELECT last_observed_at_ms FROM store_metadata")
        .fetch_one(&mut database)
        .await
        .unwrap();
    assert_eq!(watermark, NOW);
    let id = &issued.period.issued_grant_id.as_ref().unwrap().value;
    assert_eq!(
        store.get_grant(&actor(), id).await.unwrap(),
        Some(issued.grant.clone())
    );
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 4);
    database.close().await.unwrap();
}
