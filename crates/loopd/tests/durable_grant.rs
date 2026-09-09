mod support;

use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;

use loop_core::audit::{AuditAction, verify_audit_chain};
use loop_protocol::wire::holdout::v1::RequestHoldoutGrantResponse;
use loop_protocol::wire::v1::{HoldoutGrantState, HoldoutPeriodState, RequestId};
use loopd::store::{DenyHoldout, GrantClosure, HoldoutRepository, PgJobStore, StoreError};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::Connection;
use support::*;

#[tokio::test]
async fn issue_binds_approvals_and_period() {
    let (_directory, store, _, request) = grant::setup().await;
    let result = store.issue_grant(&actor(), request).await.unwrap();
    assert!(!result.replayed);
    assert_eq!(result.grant.state, HoldoutGrantState::Issued as i32);
    assert_eq!(result.grant.revision, 1);
    assert_eq!(result.period.state, HoldoutPeriodState::GrantIssued as i32);
    assert_eq!(result.period.revision, 2);
    assert_eq!(result.grant.approval_records.len(), 2);
    let reference = result.grant.reference.as_ref().unwrap();
    assert_eq!(result.period.issued_grant_id, reference.holdout_grant_id);
    assert_eq!(reference.issued_at, Some(timestamp(NOW)));
    assert_eq!(reference.expires_at, Some(timestamp(NOW + 60_000)));
    let id = &reference.holdout_grant_id.as_ref().unwrap().value;
    assert_eq!(
        store.get_grant(&actor(), id).await.unwrap(),
        Some(result.grant)
    );
    let events = store.audit_events(0, 500).await.unwrap();
    assert_eq!(events.len(), 4);
    assert_eq!(events[3].action, AuditAction::HoldoutGrantIssued);
    verify_audit_chain(&events).unwrap();
    store.close().await;
}

#[tokio::test]
async fn personal_policy_accepts_one_human() {
    let policy = grant::Policy {
        required_approvers: 1,
        ..Default::default()
    };
    let (_directory, store, _, request) = grant::setup_with(policy, 1).await;
    assert_eq!(
        store
            .issue_grant(&actor(), request)
            .await
            .unwrap()
            .grant
            .approval_records
            .len(),
        1
    );
    store.close().await;
}

#[tokio::test]
async fn pinned_policy_enforces_approver_count() {
    let (_directory, store, _, request) = grant::setup_with(grant::Policy::default(), 1).await;
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 2);
    store.close().await;
}

#[tokio::test]
async fn aliases_do_not_count_as_distinct_humans() {
    let policy = grant::Policy {
        alias_subject: true,
        ..Default::default()
    };
    let (_directory, store, _, request) = grant::setup_with(policy, 2).await;
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::Invalid("approvers are not independent"))
    ));
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 3);
    store.close().await;
}

#[tokio::test]
async fn same_actor_cannot_supply_two_approvals() {
    let (_directory, store, _, mut request) = grant::setup().await;
    let first = store
        .record_approval(
            &approval::human(1),
            approval::command(0, "another", &approval::human(1)),
        )
        .await
        .unwrap()
        .record
        .holdout_approval_record_id
        .unwrap();
    let other = store
        .record_approval(
            &approval::human(1),
            approval::command(0, "approval.1", &approval::human(1)),
        )
        .await
        .unwrap()
        .record
        .holdout_approval_record_id
        .unwrap();
    request.approval_record_ids = vec![first, other];
    request
        .approval_record_ids
        .sort_by(|a, b| a.value.cmp(&b.value));
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::Invalid("approvers are not independent"))
    ));
    store.close().await;
}

#[tokio::test]
async fn expired_approval_cannot_issue() {
    let (_directory, store, clock, request) = grant::setup().await;
    clock.0.store(NOW + 3_600_000, Ordering::SeqCst);
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::Invalid("grant approval validity"))
    ));
    store.close().await;
}

#[tokio::test]
async fn grant_cannot_outlive_approval() {
    let policy = grant::Policy {
        grant_validity_ms: 3_600_001,
        ..Default::default()
    };
    let (_directory, store, _, request) = grant::setup_with(policy, 2).await;
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::Invalid("grant outlives approval"))
    ));
    store.close().await;
}

#[tokio::test]
async fn different_key_cannot_issue_twice() {
    let (_directory, store, _, mut request) = grant::setup().await;
    store.issue_grant(&actor(), request.clone()).await.unwrap();
    request.context = Some(context("second"));
    assert!(matches!(
        store.issue_grant(&actor(), request.clone()).await,
        Err(StoreError::RevisionConflict)
    ));
    request.expected_period_revision = 2;
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::InvalidTransition)
    ));
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 4);
    store.close().await;
}

#[tokio::test]
async fn changed_request_conflicts() {
    let (_directory, store, _, mut request) = grant::setup().await;
    store.issue_grant(&actor(), request.clone()).await.unwrap();
    request.approval_record_ids.pop();
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::IdempotencyConflict)
    ));
    store.close().await;
}

#[tokio::test]
async fn replay_survives_expiry_and_restart() {
    let (directory, store, clock, mut request) = grant::setup().await;
    let original = store.issue_grant(&actor(), request.clone()).await.unwrap();
    store.close().await;
    clock.0.store(NOW + 70_000, Ordering::SeqCst);
    let mut config = options(&directory.path().join("state"), clock);
    config.holdout_policy = Arc::new(grant::Policy::default());
    let reopened = PgJobStore::open(config).await.unwrap();
    request.context.as_mut().unwrap().request_id = Some(RequestId {
        value: "retry.new-transport".to_owned(),
    });
    request.context.as_mut().unwrap().requested_at = Some(timestamp(NOW + 70_000));
    let retry = reopened.issue_grant(&actor(), request).await.unwrap();
    assert!(retry.replayed);
    assert_eq!(retry.grant, original.grant);
    assert_eq!(retry.period, original.period);
    assert_eq!(reopened.audit_events(0, 500).await.unwrap().len(), 4);
    reopened.close().await;
}

#[tokio::test]
async fn expired_grant_permanently_closes_period() {
    let (_directory, store, clock, mut request) = grant::setup().await;
    let issued = store.issue_grant(&actor(), request.clone()).await.unwrap();
    clock.0.store(NOW + 60_000, Ordering::SeqCst);
    let command = grant::close(&issued, GrantClosure::Expire, "expire");
    let closed = store.close_grant(&actor(), command.clone()).await.unwrap();
    assert_eq!(closed.grant.state, HoldoutGrantState::Expired as i32);
    assert_eq!(closed.period.state, HoldoutPeriodState::Closed as i32);
    assert_eq!(closed.period.issued_grant_id, issued.period.issued_grant_id);
    assert!(store.close_grant(&actor(), command).await.unwrap().replayed);
    let retry = store.issue_grant(&actor(), request.clone()).await.unwrap();
    assert!(retry.replayed);
    assert_eq!(retry.grant, issued.grant);
    request.context = Some(context("replacement"));
    request.expected_period_revision = 3;
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::InvalidTransition)
    ));
    let id = closed.period.issued_grant_id.unwrap().value;
    assert_eq!(
        store.get_grant(&actor(), &id).await.unwrap(),
        Some(closed.grant)
    );
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 5);
    store.close().await;
}

#[tokio::test]
async fn revoked_grant_cannot_be_replaced() {
    let (_directory, store, _, mut request) = grant::setup().await;
    let issued = store.issue_grant(&actor(), request.clone()).await.unwrap();
    let closed = store
        .close_grant(
            &actor(),
            grant::close(&issued, GrantClosure::Revoke, "revoke"),
        )
        .await
        .unwrap();
    assert_eq!(closed.grant.state, HoldoutGrantState::Revoked as i32);
    assert_eq!(closed.grant.reference, issued.grant.reference);
    request.context = Some(context("replacement"));
    request.expected_period_revision = 3;
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::InvalidTransition)
    ));
    store.close().await;
}

#[tokio::test]
async fn expiry_is_half_open() {
    let (_directory, store, clock, request) = grant::setup().await;
    let issued = store.issue_grant(&actor(), request).await.unwrap();
    clock.0.store(NOW + 59_999, Ordering::SeqCst);
    assert!(matches!(
        store
            .close_grant(
                &actor(),
                grant::close(&issued, GrantClosure::Expire, "early")
            )
            .await,
        Err(StoreError::Invalid("grant closing time"))
    ));
    clock.0.store(NOW + 60_000, Ordering::SeqCst);
    assert!(matches!(
        store
            .close_grant(
                &actor(),
                grant::close(&issued, GrantClosure::Revoke, "late")
            )
            .await,
        Err(StoreError::Invalid("grant closing time"))
    ));
    store.close().await;
}

#[tokio::test]
async fn close_replay_preserves_reason() {
    let (_directory, store, _, request) = grant::setup().await;
    let issued = store.issue_grant(&actor(), request).await.unwrap();
    let mut command = grant::close(&issued, GrantClosure::Revoke, "revoke");
    let original = store.close_grant(&actor(), command.clone()).await.unwrap();
    let replay = store.close_grant(&actor(), command.clone()).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.grant, original.grant);
    command.reason = "A different decision".to_owned();
    assert!(matches!(
        store.close_grant(&actor(), command).await,
        Err(StoreError::IdempotencyConflict)
    ));
    store.close().await;
}

#[tokio::test]
async fn default_policy_denies_all_grant_operations() {
    let (directory, store, clock, request) = grant::setup().await;
    let issued = store.issue_grant(&actor(), request.clone()).await.unwrap();
    store.close().await;
    let mut config = options(&directory.path().join("state"), clock);
    config.holdout_policy = Arc::new(DenyHoldout);
    let denied = PgJobStore::open(config).await.unwrap();
    assert!(matches!(
        denied.issue_grant(&actor(), request).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(matches!(
        denied
            .get_grant(
                &actor(),
                &issued.period.issued_grant_id.as_ref().unwrap().value
            )
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(matches!(
        denied
            .close_grant(
                &actor(),
                grant::close(&issued, GrantClosure::Revoke, "denied")
            )
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    denied.close().await;
}

#[tokio::test]
async fn unresolved_freeze_fails_closed() {
    let policy = grant::Policy {
        resolved: None,
        ..Default::default()
    };
    let (_directory, store, _, request) = grant::setup_with(policy, 2).await;
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}

#[tokio::test]
async fn changed_plan_bytes_fail_closed() {
    let mut resolved = grant::resolved(0);
    resolved.canonical_plan.push(b' ');
    let policy = grant::Policy {
        resolved: Some(resolved),
        ..Default::default()
    };
    let (_directory, store, _, request) = grant::setup_with(policy, 2).await;
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::Holdout(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn caller_cannot_change_frozen_policy() {
    let (_directory, store, _, mut request) = grant::setup().await;
    request
        .freeze_manifest
        .as_mut()
        .unwrap()
        .holdout_approval_policy
        .as_mut()
        .unwrap()
        .revision = "2".to_owned();
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}

#[tokio::test]
async fn clock_regression_blocks_mutation() {
    let (_directory, store, clock, request) = grant::setup().await;
    clock.0.store(NOW - 1, Ordering::SeqCst);
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::ClockRegression)
    ));
    store.close().await;
}

#[tokio::test]
async fn future_command_fails_before_issue() {
    let (_directory, store, _, mut request) = grant::setup().await;
    request.context.as_mut().unwrap().requested_at = Some(timestamp(NOW + 1));
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::Invalid("future command time"))
    ));
    store.close().await;
}

#[tokio::test]
async fn actor_metadata_is_not_authority() {
    let (_directory, store, _, mut request) = grant::setup().await;
    request
        .context
        .as_mut()
        .unwrap()
        .actor
        .as_mut()
        .unwrap()
        .authenticated_subject = "forged".to_owned();
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}

#[tokio::test]
async fn audit_failure_rolls_back_issue() {
    let (directory, store, _, request) = grant::setup().await;
    let mut database = connection(&directory).await;
    sqlx::query("CREATE TRIGGER injected_failure BEFORE INSERT ON audit_events FOR EACH ROW EXECUTE FUNCTION reject_immutable_change()")
        .execute(&mut database).await.unwrap();
    assert!(matches!(
        store.issue_grant(&actor(), request.clone()).await,
        Err(StoreError::Database(_))
    ));
    for table in ["holdout_grants", "holdout_grant_approvals"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&mut database)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    let id = request.holdout_period_id.unwrap().value;
    assert_eq!(
        store
            .get_period(&actor(), &id)
            .await
            .unwrap()
            .unwrap()
            .state,
        HoldoutPeriodState::Sealed as i32
    );
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 3);
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn receipt_failure_rolls_back_close() {
    let (directory, store, _, request) = grant::setup().await;
    let issued = store.issue_grant(&actor(), request).await.unwrap();
    let mut database = connection(&directory).await;
    sqlx::query("CREATE TRIGGER injected_failure BEFORE INSERT ON holdout_command_receipts FOR EACH ROW EXECUTE FUNCTION reject_immutable_change()")
        .execute(&mut database).await.unwrap();
    assert!(matches!(
        store
            .close_grant(
                &actor(),
                grant::close(&issued, GrantClosure::Revoke, "rollback")
            )
            .await,
        Err(StoreError::Database(_))
    ));
    let id = issued
        .period
        .issued_grant_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    assert_eq!(
        store.get_grant(&actor(), &id).await.unwrap(),
        Some(issued.grant)
    );
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 4);
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn cancellation_leaves_no_grant() {
    let (directory, store, _, request) = grant::setup().await;
    let mut database = connection(&directory).await;
    let mut transaction = database.begin().await.unwrap();
    sqlx::query("SELECT singleton FROM store_metadata WHERE singleton = 1 FOR UPDATE")
        .fetch_one(&mut *transaction)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            store.issue_grant(&actor(), request.clone())
        )
        .await
        .is_err()
    );
    transaction.rollback().await.unwrap();
    assert!(!store.issue_grant(&actor(), request).await.unwrap().replayed);
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 4);
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn rehashed_receipt_cannot_rewrite_grant() {
    let (directory, store, _, request) = grant::setup().await;
    let issued = store.issue_grant(&actor(), request.clone()).await.unwrap();
    let mut response = RequestHoldoutGrantResponse {
        grant: Some(issued.grant),
        period_record: Some(issued.period),
    };
    response.grant.as_mut().unwrap().revision = 2;
    let bytes = response.encode_to_vec();
    let mut database = connection(&directory).await;
    sqlx::query("DROP TRIGGER holdout_receipts_no_update ON holdout_command_receipts")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("UPDATE holdout_command_receipts SET response_blob=$1,response_sha256=$2 WHERE operation='loop.holdout.issue-grant'")
        .bind(&bytes).bind(Sha256::digest(&bytes).as_slice()).execute(&mut database).await.unwrap();
    assert!(matches!(
        store.issue_grant(&actor(), request).await,
        Err(StoreError::Corrupt(_))
    ));
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn database_rejects_partial_and_mutable_aggregates() {
    let (directory, store, _, request) = grant::setup().await;
    let issued = store.issue_grant(&actor(), request).await.unwrap();
    let mut database = connection(&directory).await;
    for query in [
        "DELETE FROM holdout_grants",
        "DELETE FROM holdout_grant_approvals",
        "UPDATE holdout_grant_approvals SET actor_id='changed'",
        "UPDATE holdout_grants SET expires_at_ms=expires_at_ms+1",
        "UPDATE holdout_grants SET state=4,revision=2,terminal_at_ms=NULL",
        "UPDATE holdout_grants SET state=4,revision=2,terminal_at_ms=issued_at_ms",
        "UPDATE holdout_periods SET state=4,revision=3,terminal_at_ms=grant_issued_at_ms",
    ] {
        assert!(
            sqlx::query(query).execute(&mut database).await.is_err(),
            "accepted {query}"
        );
    }
    let id = issued.period.issued_grant_id.unwrap().value;
    assert_eq!(
        store.get_grant(&actor(), &id).await.unwrap(),
        Some(issued.grant)
    );
    database.close().await.unwrap();
    store.close().await;
}
