use loop_protocol::artifact::validate_artifact_ref;
use loop_protocol::wire::holdout::v1::RecordHoldoutApprovalRequest;
use loop_protocol::wire::v1::{ArtifactRef, HoldoutApprovalRecord, Sha256Digest};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{
    CanonicalDigest, StoreError, StoreResult, audit_timestamp, digest_bytes, record_time,
    timestamp_millis, validate_expiry, validate_human, validate_id, validate_input,
};

const DOMAIN: &[u8] = b"loop.holdout-approval-record/v1\0";

#[derive(Serialize)]
struct CanonicalApproval<'a> {
    schema: &'static str,
    holdout_approval_record_id: &'a str,
    holdout_period_id: &'a str,
    freeze_manifest_sha256: String,
    approved_by: CanonicalActor<'a>,
    reason: &'a str,
    evidence: Vec<CanonicalArtifact>,
    approved_at: String,
    expires_at: String,
    holdout_evaluation_plan_id: &'a str,
    evaluation_plan_sha256: String,
    evaluation_plan_entry_count: String,
    canonical_period_sha256: String,
}

#[derive(Serialize)]
struct CanonicalActor<'a> {
    actor_id: &'a str,
    kind: &'static str,
    display_name: &'a str,
    authenticated_subject: &'a str,
}

#[derive(Serialize)]
struct CanonicalArtifact {
    artifact_id: String,
    uri: String,
    sha256: String,
    schema_name: String,
    schema_version: String,
    schema_sha256: String,
    media_type: String,
    byte_size: String,
    row_count: Option<String>,
    created_at: String,
    manifest_sha256: Option<String>,
}

pub(super) fn canonical_bytes(record: &HoldoutApprovalRecord) -> StoreResult<Vec<u8>> {
    let id = &record
        .holdout_approval_record_id
        .as_ref()
        .ok_or(StoreError::Invalid("approval identity"))?
        .value;
    validate_id(id)?;
    let actor = record
        .approved_by
        .as_ref()
        .ok_or(StoreError::Invalid("approval actor"))?;
    validate_human(actor)?;
    let request = RecordHoldoutApprovalRequest {
        holdout_period_id: record.holdout_period_id.clone(),
        freeze_manifest_sha256: record.freeze_manifest_sha256.clone(),
        reason: record.reason.clone(),
        evidence: record.evidence.clone(),
        canonical_period_sha256: record.canonical_period_sha256.clone(),
        holdout_evaluation_plan_id: record.holdout_evaluation_plan_id.clone(),
        evaluation_plan_sha256: record.evaluation_plan_sha256.clone(),
        evaluation_plan_entry_count: record.evaluation_plan_entry_count,
        context: None,
    };
    let period_id = validate_input(&request)?;
    let approved = record_time(record.approved_at.as_ref())?;
    let expiry = record_time(record.expires_at.as_ref())?;
    validate_expiry(approved, expiry)?;
    let value = CanonicalApproval {
        schema: "loop.holdout-approval-record/v1",
        holdout_approval_record_id: id,
        holdout_period_id: period_id,
        freeze_manifest_sha256: digest_text(record.freeze_manifest_sha256.as_ref())?,
        approved_by: CanonicalActor {
            actor_id: &actor.actor_id.as_ref().expect("validated actor").value,
            kind: "human",
            display_name: &actor.display_name,
            authenticated_subject: &actor.authenticated_subject,
        },
        reason: &record.reason,
        evidence: record
            .evidence
            .iter()
            .map(|artifact| artifact_projection(artifact, approved))
            .collect::<StoreResult<_>>()?,
        approved_at: audit_timestamp(approved)?,
        expires_at: audit_timestamp(expiry)?,
        holdout_evaluation_plan_id: &record
            .holdout_evaluation_plan_id
            .as_ref()
            .expect("validated plan")
            .value,
        evaluation_plan_sha256: digest_text(record.evaluation_plan_sha256.as_ref())?,
        evaluation_plan_entry_count: record.evaluation_plan_entry_count.to_string(),
        canonical_period_sha256: digest_text(record.canonical_period_sha256.as_ref())?,
    };
    let bytes =
        serde_json::to_vec(&value).map_err(|_| StoreError::Invalid("approval serialization"))?;
    if bytes.len() > 256 * 1024 {
        return Err(StoreError::Invalid("approval canonical size"));
    }
    Ok(bytes)
}

fn artifact_projection(reference: &ArtifactRef, approved: i64) -> StoreResult<CanonicalArtifact> {
    let value =
        validate_artifact_ref(reference).map_err(|_| StoreError::Invalid("approval evidence"))?;
    let created = reference
        .created_at
        .as_ref()
        .expect("validated artifact time");
    if timestamp_millis(created, true)? > approved {
        return Err(StoreError::Invalid("future approval evidence"));
    }
    // Artifact timestamps retain nanoseconds; approval timestamps are server milliseconds.
    let created_at =
        chrono::DateTime::from_timestamp(value.created_at_seconds, value.created_at_nanos as u32)
            .ok_or(StoreError::Invalid("approval artifact time"))?
            .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    Ok(CanonicalArtifact {
        artifact_id: value.artifact_id,
        uri: value.uri,
        sha256: CanonicalDigest::from_bytes(value.sha256).to_string(),
        schema_name: value.schema_name,
        schema_version: value.schema_version.to_string(),
        schema_sha256: CanonicalDigest::from_bytes(value.schema_sha256).to_string(),
        media_type: value.media_type,
        byte_size: value.byte_size.to_string(),
        row_count: value.row_count.map(|value| value.to_string()),
        created_at,
        manifest_sha256: value
            .manifest_sha256
            .map(|value| CanonicalDigest::from_bytes(value).to_string()),
    })
}

pub(super) fn digest(bytes: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(DOMAIN);
    hash.update(bytes);
    hash.finalize().into()
}

pub(super) fn digest_text(value: Option<&Sha256Digest>) -> StoreResult<String> {
    let bytes: [u8; 32] = digest_bytes(value)?
        .try_into()
        .map_err(|_| StoreError::Invalid("approval digest"))?;
    Ok(CanonicalDigest::from_bytes(bytes).to_string())
}
