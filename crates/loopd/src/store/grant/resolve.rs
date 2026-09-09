use std::collections::HashSet;

use loop_core::audit::{AuditPayload, Sha256Digest as CanonicalDigest, canonicalize_audit_payload};
use loop_protocol::artifact::validate_artifact_ref;
use loop_protocol::holdout::{validate_holdout_evaluation_plan_reference, validate_holdout_period};
use loop_protocol::wire::holdout::v1::RequestHoldoutGrantRequest;
use loop_protocol::wire::v1::{
    FreezeManifestReference, HoldoutApprovalRecord, HoldoutApprovalRecordReference,
    HoldoutEvaluationPlanReference, HoldoutGrantRecord, PolicyReference, Sha256Digest,
    VcsObjectAlgorithm,
};
use serde::Serialize;
use sqlx::{Postgres, Transaction};

use super::super::postgres::audit_timestamp;
use super::super::{approval, validate_id};
use super::state::{PersistedPeriod, record_time};
use super::{PgJobStore, ResolvedFreeze, StoreError, StoreResult};

pub(super) fn digest(value: Option<&Sha256Digest>) -> StoreResult<&[u8]> {
    let bytes = &value.ok_or(StoreError::Invalid("grant digest"))?.value;
    if bytes.len() != 32 {
        return Err(StoreError::Invalid("grant digest length"));
    }
    Ok(bytes)
}

fn digest_id(id: &str, bytes: Option<&Sha256Digest>) -> StoreResult<()> {
    let parsed =
        CanonicalDigest::parse(id).map_err(|_| StoreError::Invalid("grant content identity"))?;
    if parsed.as_bytes().as_slice() != digest(bytes)? {
        return Err(StoreError::Invalid("grant content identity mismatch"));
    }
    Ok(())
}

pub(super) fn policy(value: Option<&PolicyReference>) -> StoreResult<()> {
    let value = value.ok_or(StoreError::Invalid("grant approval policy"))?;
    validate_id(
        &value
            .policy_id
            .as_ref()
            .ok_or(StoreError::Invalid("policy identity"))?
            .value,
    )?;
    digest(value.sha256.as_ref())?;
    if value.revision.is_empty()
        || value.revision.len() > 20
        || value.revision.starts_with('0')
        || !value.revision.bytes().all(|byte| byte.is_ascii_digit())
        || value.revision.parse::<u64>().is_err()
    {
        return Err(StoreError::Invalid("policy revision"));
    }
    Ok(())
}

pub(super) fn freeze_shape(
    freeze: &FreezeManifestReference,
) -> StoreResult<&HoldoutEvaluationPlanReference> {
    let manifest = validate_artifact_ref(
        freeze
            .manifest
            .as_ref()
            .ok_or(StoreError::Invalid("freeze manifest"))?,
    )
    .map_err(|_| StoreError::Invalid("freeze artifact"))?;
    if manifest.schema_name != "loop.freeze_manifest"
        || manifest.schema_version != 1
        || manifest.media_type != "application/json"
        || manifest.row_count.is_some()
        || manifest.manifest_sha256.is_some()
        || manifest.byte_size > 4 * 1024 * 1024
    {
        return Err(StoreError::Invalid("freeze artifact schema"));
    }
    for value in [
        &freeze.configuration_sha256,
        &freeze.model_catalog_sha256,
        &freeze.data_manifest_sha256,
        &freeze.source_tree_sha256,
    ] {
        digest(value.as_ref())?;
    }
    let commit = freeze
        .source_commit
        .as_ref()
        .ok_or(StoreError::Invalid("freeze commit"))?;
    let length = match VcsObjectAlgorithm::try_from(commit.algorithm) {
        Ok(VcsObjectAlgorithm::Sha1) => 20,
        Ok(VcsObjectAlgorithm::Sha256) => 32,
        _ => return Err(StoreError::Invalid("freeze commit algorithm")),
    };
    if commit.value.len() != length {
        return Err(StoreError::Invalid("freeze commit length"));
    }
    policy(freeze.holdout_approval_policy.as_ref())?;
    let plan = freeze
        .holdout_evaluation_plan
        .as_ref()
        .ok_or(StoreError::Invalid("freeze plan"))?;
    let artifact = validate_artifact_ref(
        plan.canonical_plan
            .as_ref()
            .ok_or(StoreError::Invalid("plan artifact"))?,
    )
    .map_err(|_| StoreError::Invalid("plan artifact"))?;
    if artifact.schema_name != "loop.holdout_evaluation_plan"
        || artifact.schema_version != 1
        || artifact.media_type != "application/json"
        || artifact.row_count.is_some()
        || artifact.manifest_sha256.is_some()
        || artifact.byte_size > 8 * 1024 * 1024
        || artifact.sha256.as_slice() != digest(plan.plan_sha256.as_ref())?
        || !(1..=4096).contains(&plan.entry_count)
    {
        return Err(StoreError::Invalid("freeze plan bounds"));
    }
    CanonicalDigest::parse(
        &plan
            .holdout_evaluation_plan_id
            .as_ref()
            .ok_or(StoreError::Invalid("plan identity"))?
            .value,
    )
    .map_err(|_| StoreError::Invalid("plan identity"))?;
    digest_id(
        &plan
            .holdout_period_id
            .as_ref()
            .ok_or(StoreError::Invalid("plan period"))?
            .value,
        plan.canonical_period_sha256.as_ref(),
    )?;
    Ok(plan)
}

pub(super) fn validate_request(
    command: &RequestHoldoutGrantRequest,
) -> StoreResult<(&str, &FreezeManifestReference)> {
    let period_id = &command
        .holdout_period_id
        .as_ref()
        .ok_or(StoreError::Invalid("grant period"))?
        .value;
    digest_id(period_id, command.canonical_period_sha256.as_ref())?;
    if command.expected_period_revision == 0
        || command.expected_period_revision > i64::MAX as u64
        || !(1..=8).contains(&command.approval_record_ids.len())
    {
        return Err(StoreError::Invalid("grant request bounds"));
    }
    let mut previous = None;
    for id in &command.approval_record_ids {
        validate_id(&id.value)?;
        if previous.is_some_and(|value| value >= id.value.as_str()) {
            return Err(StoreError::Invalid(
                "approval identities must be sorted and unique",
            ));
        }
        previous = Some(id.value.as_str());
    }
    let freeze = command
        .freeze_manifest
        .as_ref()
        .ok_or(StoreError::Invalid("grant freeze"))?;
    let plan = freeze_shape(freeze)?;
    if plan.holdout_period_id != command.holdout_period_id
        || plan.canonical_period_sha256 != command.canonical_period_sha256
    {
        return Err(StoreError::Invalid("grant period binding"));
    }
    Ok((period_id, freeze))
}

pub(super) fn resolve_freeze(
    store: &PgJobStore,
    requested: &FreezeManifestReference,
    period: &PersistedPeriod,
    now: i64,
) -> StoreResult<ResolvedFreeze> {
    let plan = freeze_shape(requested)?;
    let manifest = requested
        .manifest
        .as_ref()
        .ok_or(StoreError::Invalid("freeze manifest"))?;
    let resolved = store.holdout_policy.resolve_freeze(
        manifest
            .sha256
            .as_ref()
            .ok_or(StoreError::Invalid("freeze digest"))?,
    )?;
    if &resolved.reference != requested
        || resolved.canonical_plan.len() > 8 * 1024 * 1024
        || resolved.backtest_artifacts.len() > 4096
        || resolved
            .backtest_artifacts
            .values()
            .try_fold(0_usize, |sum, bytes| sum.checked_add(bytes.len()))
            .is_none_or(|sum| sum > 64 * 1024 * 1024)
    {
        return Err(StoreError::AdmissionDenied);
    }
    let wire = period
        .record
        .period
        .as_ref()
        .ok_or(StoreError::Corrupt("grant period absent"))?;
    let canonical = validate_holdout_period(wire, &period.canonical_bytes)?;
    store.holdout_policy.validate_registration(&canonical)?;
    validate_holdout_evaluation_plan_reference(
        plan,
        &resolved.canonical_plan,
        &canonical,
        &resolved.plan_schema_sha256,
        &resolved.backtest_schema_sha256,
        &resolved.backtest_artifacts,
    )?;
    for artifact in [Some(manifest), plan.canonical_plan.as_ref()]
        .into_iter()
        .flatten()
    {
        let created = artifact
            .created_at
            .as_ref()
            .ok_or(StoreError::Invalid("freeze artifact time"))?;
        if super::timestamp_millis(created, true)? > now {
            return Err(StoreError::Invalid("future freeze artifact"));
        }
    }
    Ok(resolved)
}

pub(super) fn validate_expiry(start: i64, end: i64) -> StoreResult<()> {
    audit_timestamp(start)?;
    audit_timestamp(end)?;
    if !matches!(end.checked_sub(start), Some(1..=604_800_000)) {
        return Err(StoreError::Invalid("grant validity"));
    }
    Ok(())
}

pub(super) fn bind_approval(
    record: &HoldoutApprovalRecord,
    freeze: &FreezeManifestReference,
    at: i64,
) -> StoreResult<()> {
    let plan = freeze_shape(freeze)?;
    if record.holdout_period_id != plan.holdout_period_id
        || record.canonical_period_sha256 != plan.canonical_period_sha256
        || record.holdout_evaluation_plan_id != plan.holdout_evaluation_plan_id
        || record.evaluation_plan_sha256 != plan.plan_sha256
        || record.evaluation_plan_entry_count != plan.entry_count
        || record.freeze_manifest_sha256
            != freeze
                .manifest
                .as_ref()
                .and_then(|value| value.sha256.clone())
    {
        return Err(StoreError::Invalid("grant approval binding"));
    }
    if record_time(record.approved_at.as_ref())? > at
        || record_time(record.expires_at.as_ref())? <= at
    {
        return Err(StoreError::Invalid("grant approval validity"));
    }
    Ok(())
}

pub(super) async fn load_approvals(
    transaction: &mut Transaction<'_, Postgres>,
    command: &RequestHoldoutGrantRequest,
    freeze: &FreezeManifestReference,
    now: i64,
) -> StoreResult<Vec<HoldoutApprovalRecord>> {
    let mut records = Vec::with_capacity(command.approval_record_ids.len());
    let mut actors = HashSet::new();
    let mut subjects = HashSet::new();
    for id in &command.approval_record_ids {
        let row = sqlx::query("SELECT * FROM holdout_approvals WHERE approval_id = $1")
            .bind(&id.value)
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or(StoreError::NotFound)?;
        let record = approval::record_from_row(&row)?;
        bind_approval(&record, freeze, now)?;
        let human = record
            .approved_by
            .as_ref()
            .ok_or(StoreError::Corrupt("approval human"))?;
        let actor_id = &human
            .actor_id
            .as_ref()
            .ok_or(StoreError::Corrupt("approval actor"))?
            .value;
        if !actors.insert(actor_id.clone()) || !subjects.insert(human.authenticated_subject.clone())
        {
            return Err(StoreError::Invalid("approvers are not independent"));
        }
        let attached: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT FROM holdout_grant_approvals WHERE approval_id = $1)",
        )
        .bind(&id.value)
        .fetch_one(&mut **transaction)
        .await?;
        if attached {
            return Err(StoreError::InvalidTransition);
        }
        records.push(record);
    }
    records.sort_by(|a, b| {
        a.approved_by
            .as_ref()
            .and_then(|v| v.actor_id.as_ref())
            .map(|v| &v.value)
            .cmp(
                &b.approved_by
                    .as_ref()
                    .and_then(|v| v.actor_id.as_ref())
                    .map(|v| &v.value),
            )
    });
    Ok(records)
}

pub(super) fn approval_reference(record: &HoldoutApprovalRecord) -> HoldoutApprovalRecordReference {
    HoldoutApprovalRecordReference {
        holdout_approval_record_id: record.holdout_approval_record_id.clone(),
        holdout_period_id: record.holdout_period_id.clone(),
        freeze_manifest_sha256: record.freeze_manifest_sha256.clone(),
        approved_by_actor_id: record
            .approved_by
            .as_ref()
            .and_then(|value| value.actor_id.clone()),
        approved_at: record.approved_at,
        expires_at: record.expires_at,
        approval_record_sha256: record.approval_record_sha256.clone(),
        holdout_evaluation_plan_id: record.holdout_evaluation_plan_id.clone(),
        evaluation_plan_sha256: record.evaluation_plan_sha256.clone(),
        evaluation_plan_entry_count: record.evaluation_plan_entry_count,
        canonical_period_sha256: record.canonical_period_sha256.clone(),
    }
}

pub(super) fn issued_payload(grant: &HoldoutGrantRecord) -> StoreResult<AuditPayload> {
    #[derive(Serialize)]
    struct Approval<'a> {
        holdout_approval_record_id: &'a str,
        approval_record_sha256: String,
        approved_by_actor_id: &'a str,
    }
    #[derive(Serialize)]
    struct Payload<'a> {
        holdout_grant_id: &'a str,
        holdout_period_id: &'a str,
        freeze_manifest_sha256: String,
        holdout_evaluation_plan_id: &'a str,
        approval_records: Vec<Approval<'a>>,
        capability_class: &'static str,
        authorization_decision: &'static str,
    }
    let reference = super::state::reference(grant)?;
    let text = |value: Option<&Sha256Digest>| -> StoreResult<String> {
        Ok(CanonicalDigest::from_bytes(
            digest(value)?
                .try_into()
                .map_err(|_| StoreError::Invalid("grant digest"))?,
        )
        .to_string())
    };
    let approvals = grant
        .approval_records
        .iter()
        .map(|record| {
            Ok(Approval {
                holdout_approval_record_id: &record
                    .holdout_approval_record_id
                    .as_ref()
                    .ok_or(StoreError::Invalid("approval id"))?
                    .value,
                approval_record_sha256: text(record.approval_record_sha256.as_ref())?,
                approved_by_actor_id: &record
                    .approved_by_actor_id
                    .as_ref()
                    .ok_or(StoreError::Invalid("approval actor"))?
                    .value,
            })
        })
        .collect::<StoreResult<Vec<_>>>()?;
    let payload = Payload {
        holdout_grant_id: super::state::grant_id(grant)?,
        holdout_period_id: super::state::period_id(grant)?,
        freeze_manifest_sha256: text(reference.freeze_manifest_sha256.as_ref())?,
        holdout_evaluation_plan_id: &reference
            .holdout_evaluation_plan_id
            .as_ref()
            .ok_or(StoreError::Invalid("plan id"))?
            .value,
        approval_records: approvals,
        capability_class: "holdout_evaluation",
        authorization_decision: "authorized",
    };
    Ok(canonicalize_audit_payload(
        "loop.audit.holdout_grant_issued",
        1,
        &serde_json::to_vec(&payload).map_err(|_| StoreError::Invalid("grant audit encoding"))?,
    )?)
}
