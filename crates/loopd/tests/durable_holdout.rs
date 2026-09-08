mod support;

use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use loop_core::audit::{AuditAction, AuditTargetKind, verify_audit_chain};
use loop_protocol::wire::v1::{HoldoutGrantId, HoldoutPeriodRecord, HoldoutPeriodState, RequestId};
use loopd::store::{HoldoutRepository, PgJobStore, StoreError};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::Connection;
use support::*;

async fn setup() -> (tempfile::TempDir, PgJobStore, Arc<FixtureClock>) {
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let mut config = options(&directory.path().join("state"), clock.clone());
    config.holdout_policy = Arc::new(holdout::Policy);
    let store = PgJobStore::open(config).await.unwrap();
    (directory, store, clock)
}

#[tokio::test]
async fn registers_sealed_periods() {
    let (_directory, store, _) = setup().await;
    for index in [0, 1] {
        let request = holdout::command(index, &format!("period.{index}"));
        let result = store
            .register_period(&actor(), request.clone())
            .await
            .unwrap();
        assert!(!result.replayed);
        assert_eq!(result.record.period, request.period);
        assert_eq!(result.record.state, HoldoutPeriodState::Sealed as i32);
        assert_eq!(result.record.revision, 1);
        assert!(result.record.issued_grant_id.is_none());
        let id = &request.period.unwrap().holdout_period_id.unwrap().value;
        assert_eq!(
            store.get_period(&actor(), id).await.unwrap(),
            Some(result.record)
        );
    }
    let events = store.audit_events(0, 500).await.unwrap();
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|event| event.action == AuditAction::StateTransitioned
                && event.target.kind == AuditTargetKind::HoldoutPeriodId)
    );
    verify_audit_chain(&events).unwrap();
    store.close().await;
}

#[tokio::test]
async fn replay_survives_reopen() {
    let (directory, store, clock) = setup().await;
    let original = store
        .register_period(&actor(), holdout::command(0, "register"))
        .await
        .unwrap();
    store.close().await;
    clock.0.store(NOW + 1_000, Ordering::SeqCst);
    let mut config = options(&directory.path().join("state"), clock);
    config.holdout_policy = Arc::new(holdout::Policy);
    let reopened = PgJobStore::open(config).await.unwrap();
    let mut retry = holdout::command(0, "register");
    let context = retry.context.as_mut().unwrap();
    context.request_id = Some(RequestId {
        value: "request.retry".to_owned(),
    });
    context.requested_at = Some(timestamp(NOW + 1_000));
    let result = reopened.register_period(&actor(), retry).await.unwrap();
    assert!(result.replayed);
    assert_eq!(result.record, original.record);
    assert_eq!(reopened.audit_events(0, 500).await.unwrap().len(), 1);
    reopened.close().await;
}

#[tokio::test]
async fn different_key_cannot_replace_period() {
    let (_directory, store, _) = setup().await;
    store
        .register_period(&actor(), holdout::command(0, "first"))
        .await
        .unwrap();
    assert!(matches!(
        store
            .register_period(&actor(), holdout::command(0, "second"))
            .await,
        Err(StoreError::DuplicatePeriod)
    ));
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
async fn changed_period_conflicts() {
    let (_directory, store, _) = setup().await;
    store
        .register_period(&actor(), holdout::command(0, "same"))
        .await
        .unwrap();
    assert!(matches!(
        store
            .register_period(&actor(), holdout::command(1, "same"))
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    store.close().await;
}

#[tokio::test]
async fn default_policy_denies_access() {
    let (_directory, store, _) = fixture().await;
    let request = holdout::command(0, "denied");
    let id = request
        .period
        .as_ref()
        .unwrap()
        .holdout_period_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    assert!(matches!(
        store.register_period(&actor(), request).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(matches!(
        store.get_period(&actor(), &id).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(store.audit_events(0, 500).await.unwrap().is_empty());
    store.close().await;
}

#[tokio::test]
async fn rejects_period_alias() {
    let (_directory, store, _) = setup().await;
    let mut request = holdout::command(0, "alias");
    request
        .period
        .as_mut()
        .unwrap()
        .holdout_period_id
        .as_mut()
        .unwrap()
        .value = "holdout.alias".to_owned();
    assert!(matches!(
        store.register_period(&actor(), request).await,
        Err(StoreError::Holdout(_))
    ));
    assert!(store.audit_events(0, 500).await.unwrap().is_empty());
    store.close().await;
}

#[tokio::test]
async fn rejects_actor_spoofing() {
    let (_directory, store, _) = setup().await;
    let mut request = holdout::command(0, "spoof");
    request
        .context
        .as_mut()
        .unwrap()
        .actor
        .as_mut()
        .unwrap()
        .authenticated_subject = "another:subject".to_owned();
    assert!(matches!(
        store.register_period(&actor(), request).await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}

#[tokio::test]
async fn audit_failure_rolls_back_period() {
    let (directory, store, _) = setup().await;
    let mut database = connection(&directory).await;
    sqlx::query("CREATE TRIGGER injected_failure BEFORE INSERT ON audit_events FOR EACH ROW EXECUTE FUNCTION reject_immutable_change()")
        .execute(&mut database).await.unwrap();
    let request = holdout::command(0, "rollback");
    let id = request
        .period
        .as_ref()
        .unwrap()
        .holdout_period_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    assert!(matches!(
        store.register_period(&actor(), request).await,
        Err(StoreError::Database(_))
    ));
    assert!(store.get_period(&actor(), &id).await.unwrap().is_none());
    let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM holdout_command_receipts")
        .fetch_one(&mut database)
        .await
        .unwrap();
    let watermark: i64 = sqlx::query_scalar("SELECT last_observed_at_ms FROM store_metadata")
        .fetch_one(&mut database)
        .await
        .unwrap();
    assert_eq!((receipts, watermark), (0, 0));
    assert!(store.audit_events(0, 500).await.unwrap().is_empty());
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn rejects_rehashed_receipt_state() {
    let (directory, store, _) = setup().await;
    let result = store
        .register_period(&actor(), holdout::command(0, "receipt"))
        .await
        .unwrap();
    let mut invalid: HoldoutPeriodRecord = result.record;
    invalid.revision = 2;
    let bytes = invalid.encode_to_vec();
    let mut database = connection(&directory).await;
    sqlx::query("DROP TRIGGER holdout_receipts_no_update ON holdout_command_receipts")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("UPDATE holdout_command_receipts SET response_blob = $1, response_sha256 = $2")
        .bind(&bytes)
        .bind(Sha256::digest(&bytes).to_vec())
        .execute(&mut database)
        .await
        .unwrap();
    assert!(matches!(
        store
            .register_period(&actor(), holdout::command(0, "receipt"))
            .await,
        Err(StoreError::Corrupt(_))
    ));
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn clock_regression_blocks_retry() {
    let (_directory, store, clock) = setup().await;
    store
        .register_period(&actor(), holdout::command(0, "clock"))
        .await
        .unwrap();
    clock.0.store(NOW - 1, Ordering::SeqCst);
    assert!(matches!(
        store
            .register_period(&actor(), holdout::command(0, "clock"))
            .await,
        Err(StoreError::ClockRegression)
    ));
    store.close().await;
}

#[tokio::test]
async fn receipt_failure_rolls_back_registration() {
    let (directory, store, _) = setup().await;
    let mut database = connection(&directory).await;
    sqlx::query("CREATE TRIGGER injected_failure BEFORE INSERT ON holdout_command_receipts FOR EACH ROW EXECUTE FUNCTION reject_immutable_change()")
        .execute(&mut database).await.unwrap();
    assert!(matches!(
        store
            .register_period(&actor(), holdout::command(0, "rollback"))
            .await,
        Err(StoreError::Database(_))
    ));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM holdout_periods")
        .fetch_one(&mut database)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(store.audit_events(0, 500).await.unwrap().is_empty());
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn period_and_receipt_history_is_immutable() {
    let (directory, store, _) = setup().await;
    store
        .register_period(&actor(), holdout::command(0, "immutable"))
        .await
        .unwrap();
    let mut database = connection(&directory).await;
    for query in [
        "DELETE FROM holdout_periods",
        "UPDATE holdout_periods SET canonical_blob = decode('00', 'hex')",
        "UPDATE holdout_periods SET state = 1, revision = 1",
        "DELETE FROM holdout_command_receipts",
        "UPDATE holdout_command_receipts SET request_id = 'changed'",
    ] {
        assert!(sqlx::query(query).execute(&mut database).await.is_err());
    }
    assert!(
        store
            .register_period(&actor(), holdout::command(0, "immutable"))
            .await
            .unwrap()
            .replayed
    );
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn terminal_period_cannot_be_reset() {
    let (directory, store, _) = setup().await;
    let original = store
        .register_period(&actor(), holdout::command(0, "original"))
        .await
        .unwrap()
        .record;
    let mut advanced = original.clone();
    advanced.state = HoldoutPeriodState::GrantIssued as i32;
    advanced.revision = 2;
    advanced.issued_grant_id = Some(HoldoutGrantId {
        value: "grant.fixture".to_owned(),
    });
    advanced.grant_issued_at = Some(timestamp(NOW));
    let mut database = connection(&directory).await;
    // Simulate later lifecycle writers in this isolated schema; no grant API exists yet.
    let bytes = advanced.encode_to_vec();
    sqlx::query("UPDATE holdout_periods SET state = 2, revision = 2, issued_grant_id = 'grant.fixture', grant_issued_at_ms = $1, record_blob = $2, record_sha256 = $3")
        .bind(NOW).bind(&bytes).bind(Sha256::digest(&bytes).to_vec())
        .execute(&mut database).await.unwrap();
    advanced.state = HoldoutPeriodState::Closed as i32;
    advanced.revision = 3;
    advanced.terminal_at = Some(timestamp(NOW));
    let bytes = advanced.encode_to_vec();
    sqlx::query("UPDATE holdout_periods SET state = 4, revision = 3, terminal_at_ms = $1, record_blob = $2, record_sha256 = $3")
        .bind(NOW).bind(&bytes).bind(Sha256::digest(&bytes).to_vec())
        .execute(&mut database).await.unwrap();
    assert!(matches!(
        store
            .register_period(&actor(), holdout::command(0, "new-key"))
            .await,
        Err(StoreError::DuplicatePeriod)
    ));
    let replay = store
        .register_period(&actor(), holdout::command(0, "original"))
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.record, original);
    let id = &advanced
        .period
        .as_ref()
        .unwrap()
        .holdout_period_id
        .as_ref()
        .unwrap()
        .value;
    assert_eq!(
        store.get_period(&actor(), id).await.unwrap(),
        Some(advanced)
    );
    assert!(sqlx::query("UPDATE holdout_periods SET state = 1, revision = 1, issued_grant_id = NULL, grant_issued_at_ms = NULL, terminal_at_ms = NULL")
        .execute(&mut database).await.is_err());
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn altered_canonical_content_fails_before_write() {
    let (_directory, store, _) = setup().await;
    let mut request = holdout::command(0, "altered");
    request.canonical_bytes.push(b' ');
    assert!(matches!(
        store.register_period(&actor(), request).await,
        Err(StoreError::Holdout(_))
    ));
    assert!(store.audit_events(0, 500).await.unwrap().is_empty());
    store.close().await;
}

#[tokio::test]
async fn future_request_time_is_rejected() {
    let (_directory, store, _) = setup().await;
    let mut request = holdout::command(0, "future");
    request.context.as_mut().unwrap().requested_at = Some(timestamp(NOW + 1));
    assert!(matches!(
        store.register_period(&actor(), request).await,
        Err(StoreError::Invalid(_))
    ));
    assert!(store.audit_events(0, 500).await.unwrap().is_empty());
    store.close().await;
}
