mod identity;

use std::collections::HashSet;

use loop_core::audit::{
    AuditAction, AuditTarget, AuditTargetKind, Sha256Digest as CanonicalDigest,
    canonicalize_audit_payload,
};
use loop_protocol::artifact::validate_artifact_ref;
use loop_protocol::holdout::validate_holdout_period;
use loop_protocol::wire::holdout::v1::RecordHoldoutApprovalRequest;
use loop_protocol::wire::v1::{
    Actor, ActorKind, HoldoutApprovalRecord, HoldoutApprovalRecordId, HoldoutPeriod,
    HoldoutPeriodState, Sha256Digest,
};
use prost::Message;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Row, postgres::PgRow};

use super::lifecycle::validate_context;
use super::postgres::{
    audit_timestamp, encode_message, timestamp, timestamp_millis, verified_blob,
};
use super::{PgJobStore, StoreError, StoreResult, audit, holdout, validate_id};

const RECORD: &str = "loop.holdout.record-approval";
const READ: &str = "loop.holdout.read-approval";
const MAX_VALIDITY_MS: i64 = 604_800_000;

/// Original immutable human attestation. This response never authorizes access.
#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalResult {
    /// Server-attributed record, including its original approval and expiry times.
    pub record: HoldoutApprovalRecord,
    /// True for an identical committed retry; no new approval or audit was added.
    pub replayed: bool,
}

pub(super) async fn record(
    store: &PgJobStore,
    principal: &Actor,
    command: RecordHoldoutApprovalRequest,
) -> StoreResult<ApprovalResult> {
    let context = validate_context(command.context.as_ref(), principal)?;
    validate_human(principal)?;
    let period_id = validate_input(&command)?;
    store
        .holdout_policy
        .authorize_period(RECORD, principal, period_id)?;
    let normalized = normalize(&command);
    let request_blob = encode_message(&normalized)?;
    let mut transaction = store.pool.begin().await?;
    let now = store.observe_clock(&mut transaction).await?;
    if timestamp_millis(context.requested_at.as_ref().expect("validated time"), true)? > now {
        return Err(StoreError::Invalid("future command time"));
    }
    store
        .holdout_policy
        .authorize_period(RECORD, principal, period_id)?;
    let period_row = sqlx::query("SELECT * FROM holdout_periods WHERE period_id = $1")
        .bind(period_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StoreError::NotFound)?;
    let period_record = holdout::record_from_row(&period_row)?;
    let period = period_record.period.as_ref().expect("validated period");
    bind_period(&command, period)?;
    let canonical_bytes: Vec<u8> = period_row.try_get("canonical_blob")?;
    let canonical = validate_holdout_period(period, &canonical_bytes)?;
    store.holdout_policy.validate_registration(&canonical)?;
    let expiry = store
        .holdout_policy
        .approval_expiry(&command, &canonical, now)?;
    validate_expiry(now, expiry)?;

    let receipt = sqlx::query(
        "SELECT * FROM holdout_command_receipts
         WHERE actor_id = $1 AND operation = $2 AND idempotency_key = $3",
    )
    .bind(&principal.actor_id.as_ref().expect("validated actor").value)
    .bind(RECORD)
    .bind(
        &context
            .idempotency_key
            .as_ref()
            .expect("validated key")
            .value,
    )
    .fetch_optional(&mut *transaction)
    .await?;
    if let Some(receipt) = receipt {
        let previous = verified_blob(&receipt, "request_blob", "request_sha256")?;
        let previous = RecordHoldoutApprovalRequest::decode(previous.as_slice())
            .map_err(|_| StoreError::Corrupt("approval receipt request"))?;
        if previous != normalized {
            return Err(StoreError::IdempotencyConflict);
        }
        let response = verified_blob(&receipt, "response_blob", "response_sha256")?;
        let record = decode_record(&response)?;
        let approved_at = record_time(record.approved_at.as_ref())?;
        if !matches_request(&record, &command, principal)
            || receipt.try_get::<String, _>("period_id")? != period_id
            || receipt.try_get::<i64, _>("committed_at_ms")? != approved_at
            || approved_at > now
        {
            return Err(StoreError::Corrupt("approval receipt binding"));
        }
        let row = sqlx::query("SELECT * FROM holdout_approvals WHERE approval_id = $1")
            .bind(
                &record
                    .holdout_approval_record_id
                    .as_ref()
                    .expect("validated id")
                    .value,
            )
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(StoreError::Corrupt("approval receipt target absent"))?;
        if record_from_row(&row)? != record {
            return Err(StoreError::Corrupt("approval receipt target changed"));
        }
        transaction.commit().await?;
        return Ok(ApprovalResult {
            record,
            replayed: true,
        });
    }
    if period_record.state != HoldoutPeriodState::Sealed as i32 {
        return Err(StoreError::InvalidTransition);
    }
    let mut record = HoldoutApprovalRecord {
        holdout_approval_record_id: Some(HoldoutApprovalRecordId {
            value: format!("approval.{}", uuid::Uuid::new_v4().simple()),
        }),
        holdout_period_id: command.holdout_period_id.clone(),
        freeze_manifest_sha256: command.freeze_manifest_sha256.clone(),
        approved_by: Some(principal.clone()),
        reason: command.reason.clone(),
        evidence: command.evidence.clone(),
        approved_at: Some(timestamp(now)),
        expires_at: Some(timestamp(expiry)),
        approval_record_sha256: None,
        holdout_evaluation_plan_id: command.holdout_evaluation_plan_id.clone(),
        evaluation_plan_sha256: command.evaluation_plan_sha256.clone(),
        evaluation_plan_entry_count: command.evaluation_plan_entry_count,
        canonical_period_sha256: command.canonical_period_sha256.clone(),
    };
    let canonical_blob = identity::canonical_bytes(&record)?;
    let canonical_sha256 = identity::digest(&canonical_blob);
    record.approval_record_sha256 = Some(Sha256Digest {
        value: canonical_sha256.to_vec(),
    });
    let response_blob = encode_message(&record)?;
    sqlx::query(
        "INSERT INTO holdout_approvals
         (approval_id, period_id, actor_id, authenticated_subject, freeze_sha256, plan_id,
          plan_sha256, plan_entry_count, approved_at_ms, expires_at_ms, canonical_blob,
          canonical_sha256, record_blob, record_sha256)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
    )
    .bind(
        &record
            .holdout_approval_record_id
            .as_ref()
            .expect("assigned id")
            .value,
    )
    .bind(period_id)
    .bind(&principal.actor_id.as_ref().expect("validated actor").value)
    .bind(&principal.authenticated_subject)
    .bind(digest_bytes(record.freeze_manifest_sha256.as_ref())?)
    .bind(
        &record
            .holdout_evaluation_plan_id
            .as_ref()
            .expect("validated plan")
            .value,
    )
    .bind(digest_bytes(record.evaluation_plan_sha256.as_ref())?)
    .bind(i32::try_from(record.evaluation_plan_entry_count).expect("bounded count"))
    .bind(now)
    .bind(expiry)
    .bind(&canonical_blob)
    .bind(canonical_sha256.as_slice())
    .bind(&response_blob)
    .bind(Sha256::digest(&response_blob).as_slice())
    .execute(&mut *transaction)
    .await?;
    let payload = approval_payload(&record)?;
    audit::append(
        &mut transaction,
        &store.ledger_id,
        now,
        audit::EventInput {
            actor: principal,
            correlation_id: &context
                .correlation_id
                .as_ref()
                .expect("validated correlation")
                .value,
            causation_id: &context
                .causation_id
                .as_ref()
                .expect("validated causation")
                .value,
            action: AuditAction::HoldoutApprovalRecorded,
            target: AuditTarget {
                kind: AuditTargetKind::HoldoutApprovalRecordId,
                value: record
                    .holdout_approval_record_id
                    .as_ref()
                    .expect("assigned id")
                    .value
                    .clone(),
            },
            payload,
        },
    )
    .await?;
    holdout::save_receipt(
        &mut transaction,
        context,
        RECORD,
        period_id,
        &request_blob,
        &response_blob,
        now,
    )
    .await?;
    #[cfg(test)]
    super::crash_tests::fault_point("approval_before_commit").await;
    transaction.commit().await?;
    #[cfg(test)]
    super::crash_tests::fault_point("approval_after_commit").await;
    Ok(ApprovalResult {
        record,
        replayed: false,
    })
}

pub(super) async fn get(
    store: &PgJobStore,
    principal: &Actor,
    approval_id: &str,
) -> StoreResult<Option<HoldoutApprovalRecord>> {
    validate_id(approval_id)?;
    store
        .holdout_policy
        .authorize_approval_read(principal, approval_id)?;
    let mut transaction = store.pool.begin().await?;
    let row = sqlx::query("SELECT * FROM holdout_approvals WHERE approval_id = $1")
        .bind(approval_id)
        .fetch_optional(&mut *transaction)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let record = record_from_row(&row)?;
    let period_id = &record
        .holdout_period_id
        .as_ref()
        .expect("validated period")
        .value;
    store
        .holdout_policy
        .authorize_period(READ, principal, period_id)?;
    let period_row = sqlx::query("SELECT * FROM holdout_periods WHERE period_id = $1")
        .bind(period_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StoreError::Corrupt("approval period absent"))?;
    let period = holdout::record_from_row(&period_row)?
        .period
        .expect("validated period");
    if record.canonical_period_sha256 != period.canonical_period_sha256 {
        return Err(StoreError::Corrupt("approval period binding"));
    }
    transaction.commit().await?;
    Ok(Some(record))
}

fn normalize(command: &RecordHoldoutApprovalRequest) -> RecordHoldoutApprovalRequest {
    let mut normalized = command.clone();
    if let Some(context) = &mut normalized.context {
        context.request_id = None;
        context.requested_at = None;
    }
    normalized
}

fn validate_human(principal: &Actor) -> StoreResult<()> {
    if principal.kind != ActorKind::Human as i32 {
        return Err(StoreError::AdmissionDenied);
    }
    validate_id(
        &principal
            .actor_id
            .as_ref()
            .ok_or(StoreError::Invalid("approval actor"))?
            .value,
    )?;
    if principal.authenticated_subject.is_empty()
        || principal.authenticated_subject.len() > 4096
        || principal
            .authenticated_subject
            .chars()
            .any(char::is_control)
        || principal.display_name.len() > 4096
        || principal.display_name.chars().any(char::is_control)
    {
        return Err(StoreError::Invalid("approval actor metadata"));
    }
    Ok(())
}

fn validate_input(command: &RecordHoldoutApprovalRequest) -> StoreResult<&str> {
    let period_id = &command
        .holdout_period_id
        .as_ref()
        .ok_or(StoreError::Invalid("approval period"))?
        .value;
    let period_digest = CanonicalDigest::parse(period_id)
        .map_err(|_| StoreError::Invalid("approval period identity"))?;
    if period_digest.as_bytes().as_slice()
        != digest_bytes(command.canonical_period_sha256.as_ref())?
    {
        return Err(StoreError::Invalid("approval period digest"));
    }
    digest_bytes(command.freeze_manifest_sha256.as_ref())?;
    digest_bytes(command.evaluation_plan_sha256.as_ref())?;
    CanonicalDigest::parse(
        &command
            .holdout_evaluation_plan_id
            .as_ref()
            .ok_or(StoreError::Invalid("approval plan"))?
            .value,
    )
    .map_err(|_| StoreError::Invalid("approval plan identity"))?;
    if !(1..=4096).contains(&command.evaluation_plan_entry_count)
        || command.reason.trim().is_empty()
        || command.reason.len() > 4096
        || command
            .reason
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        || command.evidence.len() > 64
    {
        return Err(StoreError::Invalid("approval bounds"));
    }
    let mut ids = HashSet::new();
    for artifact in &command.evidence {
        let validated = validate_artifact_ref(artifact)
            .map_err(|_| StoreError::Invalid("approval evidence"))?;
        if !ids.insert(validated.artifact_id) {
            return Err(StoreError::Invalid("duplicate approval evidence"));
        }
    }
    Ok(period_id)
}

fn bind_period(command: &RecordHoldoutApprovalRequest, period: &HoldoutPeriod) -> StoreResult<()> {
    if command.holdout_period_id != period.holdout_period_id
        || command.canonical_period_sha256 != period.canonical_period_sha256
    {
        return Err(StoreError::Invalid("approval period binding"));
    }
    Ok(())
}

fn matches_request(
    record: &HoldoutApprovalRecord,
    command: &RecordHoldoutApprovalRequest,
    principal: &Actor,
) -> bool {
    record.holdout_period_id == command.holdout_period_id
        && record.freeze_manifest_sha256 == command.freeze_manifest_sha256
        && record.approved_by.as_ref() == Some(principal)
        && record.reason == command.reason
        && record.evidence == command.evidence
        && record.holdout_evaluation_plan_id == command.holdout_evaluation_plan_id
        && record.evaluation_plan_sha256 == command.evaluation_plan_sha256
        && record.evaluation_plan_entry_count == command.evaluation_plan_entry_count
        && record.canonical_period_sha256 == command.canonical_period_sha256
}

fn validate_expiry(approved: i64, expiry: i64) -> StoreResult<()> {
    audit_timestamp(approved)?;
    audit_timestamp(expiry)?;
    if !matches!(expiry.checked_sub(approved), Some(1..=MAX_VALIDITY_MS)) {
        return Err(StoreError::Invalid("approval validity"));
    }
    Ok(())
}

fn record_time(value: Option<&prost_types::Timestamp>) -> StoreResult<i64> {
    let value = value.ok_or(StoreError::Invalid("approval timestamp"))?;
    if value.nanos % 1_000_000 != 0 {
        return Err(StoreError::Invalid("approval timestamp precision"));
    }
    timestamp_millis(value, false)
}

fn digest_bytes(value: Option<&Sha256Digest>) -> StoreResult<&[u8]> {
    let bytes = &value.ok_or(StoreError::Invalid("approval digest"))?.value;
    if bytes.len() != 32 {
        return Err(StoreError::Invalid("approval digest length"));
    }
    Ok(bytes)
}

fn decode_record(bytes: &[u8]) -> StoreResult<HoldoutApprovalRecord> {
    let record = HoldoutApprovalRecord::decode(bytes)
        .map_err(|_| StoreError::Corrupt("approval envelope"))?;
    let canonical = identity::canonical_bytes(&record)
        .map_err(|_| StoreError::Corrupt("approval canonical identity"))?;
    if digest_bytes(record.approval_record_sha256.as_ref())
        .map_err(|_| StoreError::Corrupt("approval digest"))?
        != identity::digest(&canonical)
    {
        return Err(StoreError::Corrupt("approval identity mismatch"));
    }
    Ok(record)
}

pub(super) fn record_from_row(row: &PgRow) -> StoreResult<HoldoutApprovalRecord> {
    let bytes = verified_blob(row, "record_blob", "record_sha256")?;
    let record = decode_record(&bytes)?;
    let actor = record.approved_by.as_ref().expect("validated human");
    if row.try_get::<Vec<u8>, _>("canonical_blob")? != identity::canonical_bytes(&record)?
        || row.try_get::<Vec<u8>, _>("canonical_sha256")?
            != digest_bytes(record.approval_record_sha256.as_ref())?
        || row.try_get::<String, _>("approval_id")?
            != record
                .holdout_approval_record_id
                .as_ref()
                .expect("validated id")
                .value
        || row.try_get::<String, _>("period_id")?
            != record
                .holdout_period_id
                .as_ref()
                .expect("validated period")
                .value
        || row.try_get::<String, _>("actor_id")?
            != actor.actor_id.as_ref().expect("validated actor").value
        || row.try_get::<String, _>("authenticated_subject")? != actor.authenticated_subject
        || row.try_get::<Vec<u8>, _>("freeze_sha256")?
            != digest_bytes(record.freeze_manifest_sha256.as_ref())?
        || row.try_get::<String, _>("plan_id")?
            != record
                .holdout_evaluation_plan_id
                .as_ref()
                .expect("validated plan")
                .value
        || row.try_get::<Vec<u8>, _>("plan_sha256")?
            != digest_bytes(record.evaluation_plan_sha256.as_ref())?
        || row.try_get::<i32, _>("plan_entry_count")? != record.evaluation_plan_entry_count as i32
        || row.try_get::<i64, _>("approved_at_ms")? != record_time(record.approved_at.as_ref())?
        || row.try_get::<i64, _>("expires_at_ms")? != record_time(record.expires_at.as_ref())?
    {
        return Err(StoreError::Corrupt("approval projection mismatch"));
    }
    Ok(record)
}

fn approval_payload(record: &HoldoutApprovalRecord) -> StoreResult<loop_core::audit::AuditPayload> {
    #[derive(Serialize)]
    struct Payload<'a> {
        holdout_approval_record_id: &'a str,
        holdout_period_id: &'a str,
        freeze_manifest_sha256: String,
        approved_by_actor_id: &'a str,
        expires_at: String,
    }
    let payload = Payload {
        holdout_approval_record_id: &record
            .holdout_approval_record_id
            .as_ref()
            .expect("validated id")
            .value,
        holdout_period_id: &record
            .holdout_period_id
            .as_ref()
            .expect("validated period")
            .value,
        freeze_manifest_sha256: identity::digest_text(record.freeze_manifest_sha256.as_ref())?,
        approved_by_actor_id: &record
            .approved_by
            .as_ref()
            .expect("validated human")
            .actor_id
            .as_ref()
            .expect("validated actor")
            .value,
        expires_at: audit_timestamp(record_time(record.expires_at.as_ref())?)?,
    };
    Ok(canonicalize_audit_payload(
        "loop.audit.holdout_approval_recorded",
        1,
        &serde_json::to_vec(&payload)
            .map_err(|_| StoreError::Invalid("approval audit serialization"))?,
    )?)
}
