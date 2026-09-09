mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use loop_core::audit::{AuditAction, verify_audit_chain};
use loop_protocol::wire::v1::{ActorKind, HoldoutApprovalRecord, RequestId};
use loopd::store::{HoldoutRepository, PgJobStore, StoreError};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::{Connection, Row};
use support::{approval::*, *};

#[tokio::test]
async fn canonical_record_matches_golden() {
    let (directory, store, _) = setup().await;
    let result = store
        .record_approval(&human(1), approval::command(0, "golden", &human(1)))
        .await
        .unwrap();
    let id = &result
        .record
        .holdout_approval_record_id
        .as_ref()
        .unwrap()
        .value;
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/holdout/approval_record_v1.json"
    ))
    .unwrap();
    let template = fixture["canonical_json"].as_str().unwrap();
    let mut golden_hash = Sha256::new();
    golden_hash.update(b"loop.holdout-approval-record/v1\0");
    golden_hash.update(template.as_bytes());
    assert_eq!(
        format!("sha256:{:x}", golden_hash.finalize()),
        fixture["sha256"].as_str().unwrap()
    );
    // Only the server-assigned UUID varies; every other byte is a fixed golden.
    assert_eq!(template.matches("approval.fixture").count(), 1);
    let expected = template.replacen("approval.fixture", id, 1);
    let mut database = connection(&directory).await;
    let row = sqlx::query(
        "SELECT canonical_blob, canonical_sha256 FROM holdout_approvals WHERE approval_id = $1",
    )
    .bind(id)
    .fetch_one(&mut database)
    .await
    .unwrap();
    assert_eq!(row.get::<Vec<u8>, _>("canonical_blob"), expected.as_bytes());
    let mut hash = Sha256::new();
    hash.update(b"loop.holdout-approval-record/v1\0");
    hash.update(expected.as_bytes());
    let digest = hash.finalize().to_vec();
    assert_eq!(row.get::<Vec<u8>, _>("canonical_sha256"), digest);
    assert_eq!(result.record.approval_record_sha256.unwrap().value, digest);
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn records_authenticated_approval() {
    let (_directory, store, _) = setup().await;
    let request = approval::command(0, "approve", &human(1));
    let result = store
        .record_approval(&human(1), request.clone())
        .await
        .unwrap();
    assert!(!result.replayed);
    let record = result.record;
    assert_eq!(record.approved_by, Some(human(1)));
    assert_eq!(record.approved_at, Some(timestamp(NOW)));
    assert_eq!(record.expires_at, Some(timestamp(NOW + 3_600_000)));
    assert_eq!(record.holdout_period_id, request.holdout_period_id);
    assert_eq!(
        record.holdout_evaluation_plan_id,
        request.holdout_evaluation_plan_id
    );
    assert_eq!(
        record.evaluation_plan_sha256,
        request.evaluation_plan_sha256
    );
    assert_eq!(record.evidence, request.evidence);
    let id = &record.holdout_approval_record_id.as_ref().unwrap().value;
    assert_eq!(
        store.get_approval(&human(1), id).await.unwrap(),
        Some(record)
    );
    let events = store.audit_events(0, 500).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].action, AuditAction::HoldoutApprovalRecorded);
    verify_audit_chain(&events).unwrap();
    store.close().await;
}

#[tokio::test]
async fn expired_retry_preserves_original_record() {
    let (directory, store, clock) = setup().await;
    let result = store
        .record_approval(&human(1), approval::command(0, "approve", &human(1)))
        .await
        .unwrap();
    store.close().await;
    clock.0.store(NOW + 3_600_000, Ordering::SeqCst);
    let mut config = options(&directory.path().join("state"), clock);
    config.holdout_policy = Arc::new(Policy::default());
    let store = PgJobStore::open(config).await.unwrap();
    let mut request = approval::command(0, "approve", &human(1));
    let context = request.context.as_mut().unwrap();
    context.request_id = Some(RequestId {
        value: "request.retry".to_owned(),
    });
    context.requested_at = Some(timestamp(NOW + 3_600_000));
    let retry = store.record_approval(&human(1), request).await.unwrap();
    assert!(retry.replayed);
    assert_eq!(retry.record, result.record);
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 2);
    store.close().await;
}

#[tokio::test]
async fn changed_reason_conflicts() {
    let (_directory, store, _) = setup().await;
    let mut request = approval::command(0, "approve", &human(1));
    store
        .record_approval(&human(1), request.clone())
        .await
        .unwrap();
    request.reason = "A different attestation".to_owned();
    assert!(matches!(
        store.record_approval(&human(1), request).await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 2);
    store.close().await;
}

#[tokio::test]
async fn principals_have_independent_receipts() {
    let (_directory, store, _) = setup().await;
    let mut ids = vec![];
    for principal in [human(1), human(2)] {
        let result = store
            .record_approval(&principal, approval::command(0, "same-key", &principal))
            .await
            .unwrap();
        assert!(!result.replayed);
        ids.push(result.record.holdout_approval_record_id.unwrap().value);
    }
    assert_ne!(ids[0], ids[1]);
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 3);
    store.close().await;
}

#[tokio::test]
async fn rejects_spoofed_actor() {
    let (_directory, store, _) = setup().await;
    let request = approval::command(0, "spoof", &human(2));
    assert!(matches!(
        store.record_approval(&human(1), request).await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}

#[tokio::test]
async fn rejects_non_human_principals() {
    let (_directory, store, _) = setup().await;
    for kind in [
        ActorKind::Agent,
        ActorKind::Service,
        ActorKind::Scheduler,
        ActorKind::Unspecified,
    ] {
        let mut principal = human(1);
        principal.kind = kind as i32;
        assert!(matches!(
            store
                .record_approval(&principal, approval::command(0, "nonhuman", &principal))
                .await,
            Err(StoreError::AdmissionDenied)
        ));
    }
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
async fn unresolved_references_fail_closed() {
    let (_directory, store, _) = setup().await;
    for field in ["freeze", "plan", "count", "evidence"] {
        let mut request = approval::command(0, "unresolved", &human(1));
        match field {
            "freeze" => request.freeze_manifest_sha256 = Some(digest(99)),
            "plan" => request.evaluation_plan_sha256 = Some(digest(99)),
            "count" => request.evaluation_plan_entry_count += 1,
            "evidence" => request.evidence.clear(),
            _ => unreachable!(),
        }
        assert!(matches!(
            store.record_approval(&human(1), request).await,
            Err(StoreError::AdmissionDenied)
        ));
    }
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
async fn rejects_invalid_envelopes() {
    let (_directory, store, _) = setup().await;
    for field in [
        "empty_reason",
        "large_reason",
        "duplicate_evidence",
        "oversized_evidence",
        "alias",
        "digest",
        "count",
        "future",
    ] {
        let mut request = approval::command(0, "invalid", &human(1));
        match field {
            "empty_reason" => request.reason = " \n".to_owned(),
            "large_reason" => request.reason = "x".repeat(4097),
            "duplicate_evidence" => request.evidence.push(artifact()),
            "oversized_evidence" => request.evidence = vec![artifact(); 65],
            "alias" => {
                request.holdout_period_id.as_mut().unwrap().value = "period.alias".to_owned()
            }
            "digest" => request.canonical_period_sha256 = Some(digest(99)),
            "count" => request.evaluation_plan_entry_count = 4097,
            "future" => request.context.as_mut().unwrap().requested_at = Some(timestamp(NOW + 1)),
            _ => unreachable!(),
        }
        assert!(
            matches!(
                store.record_approval(&human(1), request).await,
                Err(StoreError::Invalid(_))
            ),
            "{field}"
        );
    }
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
async fn default_policy_denies_approval() {
    let (_directory, store, _) = fixture().await;
    assert!(matches!(
        store
            .record_approval(&human(1), approval::command(0, "denied", &human(1)))
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(matches!(
        store.get_approval(&human(1), "approval.absent").await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}

#[tokio::test]
async fn read_requires_protected_authority() {
    let (_directory, store, _) = setup().await;
    let result = store
        .record_approval(&human(1), approval::command(0, "approve", &human(1)))
        .await
        .unwrap();
    let id = result.record.holdout_approval_record_id.unwrap().value;
    assert!(matches!(
        store.get_approval(&actor(), &id).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(
        store
            .get_approval(&human(1), "approval.absent")
            .await
            .unwrap()
            .is_none()
    );
    store.close().await;
}

#[tokio::test]
async fn clock_regression_blocks_approval() {
    let (_directory, store, clock) = setup().await;
    clock.0.store(NOW - 1, Ordering::SeqCst);
    assert!(matches!(
        store
            .record_approval(&human(1), approval::command(0, "clock", &human(1)))
            .await,
        Err(StoreError::ClockRegression)
    ));
    store.close().await;
}

#[tokio::test]
async fn expiry_is_bounded_by_store() {
    for validity_ms in [-1, 0, 604_800_001] {
        let (directory, store, clock) = setup().await;
        store.close().await;
        let mut config = options(&directory.path().join("state"), clock);
        config.holdout_policy = Arc::new(Policy { validity_ms });
        let store = PgJobStore::open(config).await.unwrap();
        assert!(matches!(
            store
                .record_approval(&human(1), approval::command(0, "expiry", &human(1)))
                .await,
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 1);
        store.close().await;
    }
}

#[tokio::test]
async fn dependent_failure_rolls_back_approval() {
    for table in ["audit_events", "holdout_command_receipts"] {
        let (directory, store, _) = setup().await;
        let mut database = connection(&directory).await;
        sqlx::query(&format!("CREATE TRIGGER injected_failure BEFORE INSERT ON {table} FOR EACH ROW EXECUTE FUNCTION reject_immutable_change()"))
            .execute(&mut database).await.unwrap();
        assert!(matches!(
            store
                .record_approval(&human(1), approval::command(0, "rollback", &human(1)))
                .await,
            Err(StoreError::Database(_))
        ));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM holdout_approvals")
            .fetch_one(&mut database)
            .await
            .unwrap();
        let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM holdout_command_receipts")
            .fetch_one(&mut database)
            .await
            .unwrap();
        assert_eq!((count, receipts), (0, 1));
        assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 1);
        database.close().await.unwrap();
        store.close().await;
    }
}

#[tokio::test]
async fn approval_history_is_immutable() {
    let (directory, store, _) = setup().await;
    store
        .record_approval(&human(1), approval::command(0, "immutable", &human(1)))
        .await
        .unwrap();
    let mut database = connection(&directory).await;
    for query in [
        "DELETE FROM holdout_approvals",
        "UPDATE holdout_approvals SET expires_at_ms = expires_at_ms + 1",
        "UPDATE holdout_approvals SET actor_id = 'human.forged'",
    ] {
        assert!(sqlx::query(query).execute(&mut database).await.is_err());
    }
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn rehashed_record_cannot_change_attestation() {
    let (directory, store, _) = setup().await;
    let result = store
        .record_approval(&human(1), approval::command(0, "corrupt", &human(1)))
        .await
        .unwrap();
    let mut record: HoldoutApprovalRecord = result.record;
    let id = record
        .holdout_approval_record_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    record.reason = "Modified after approval".to_owned();
    let blob = record.encode_to_vec();
    let mut database = connection(&directory).await;
    sqlx::query("DROP TRIGGER holdout_approvals_no_update ON holdout_approvals")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("UPDATE holdout_approvals SET record_blob = $1, record_sha256 = $2")
        .bind(&blob)
        .bind(Sha256::digest(&blob).as_slice())
        .execute(&mut database)
        .await
        .unwrap();
    assert!(matches!(
        store.get_approval(&human(1), &id).await,
        Err(StoreError::Corrupt(_))
    ));
    assert!(matches!(
        store
            .record_approval(&human(1), approval::command(0, "corrupt", &human(1)))
            .await,
        Err(StoreError::Corrupt(_))
    ));
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn rehashed_receipt_cannot_change_actor() {
    let (directory, store, _) = setup().await;
    let result = store
        .record_approval(&human(1), approval::command(0, "corrupt", &human(1)))
        .await
        .unwrap();
    let mut record = result.record;
    record.approved_by = Some(human(2));
    let blob = record.encode_to_vec();
    let mut database = connection(&directory).await;
    sqlx::query("DROP TRIGGER holdout_receipts_no_update ON holdout_command_receipts")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("UPDATE holdout_command_receipts SET response_blob = $1, response_sha256 = $2 WHERE operation = 'loop.holdout.record-approval'")
        .bind(&blob).bind(Sha256::digest(&blob).as_slice()).execute(&mut database).await.unwrap();
    assert!(matches!(
        store
            .record_approval(&human(1), approval::command(0, "corrupt", &human(1)))
            .await,
        Err(StoreError::Corrupt(_))
    ));
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn issued_period_rejects_new_approval() {
    let (_directory, store, _, grant_request) = grant::setup().await;
    let request = approval::command(0, "approval.1", &human(1));
    let original = store
        .record_approval(&human(1), request.clone())
        .await
        .unwrap()
        .record;
    store.issue_grant(&actor(), grant_request).await.unwrap();
    let retry = store.record_approval(&human(1), request).await.unwrap();
    assert!(retry.replayed);
    assert_eq!(retry.record, original);
    assert!(matches!(
        store
            .record_approval(&human(1), approval::command(0, "new", &human(1)))
            .await,
        Err(StoreError::InvalidTransition)
    ));
    store.close().await;
}

#[tokio::test]
async fn cancellation_does_not_leave_an_approval() {
    let (directory, store, _) = setup().await;
    let mut database = connection(&directory).await;
    let mut transaction = database.begin().await.unwrap();
    sqlx::query("SELECT singleton FROM store_metadata WHERE singleton = 1 FOR UPDATE")
        .fetch_one(&mut *transaction)
        .await
        .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        store.record_approval(&human(1), approval::command(0, "cancel", &human(1))),
    )
    .await;
    assert!(result.is_err());
    transaction.rollback().await.unwrap();
    let result = store
        .record_approval(&human(1), approval::command(0, "cancel", &human(1)))
        .await
        .unwrap();
    assert!(!result.replayed);
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 2);
    database.close().await.unwrap();
    store.close().await;
}
