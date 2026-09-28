use std::collections::HashSet;

use loop_protocol::wire::holdout::v1::{RequestHoldoutGrantRequest, RequestHoldoutGrantResponse};
use loop_protocol::wire::v1::{
    FreezeManifestReference, HoldoutApprovalRecord, HoldoutGrantRecord, HoldoutGrantReference,
    HoldoutGrantState, HoldoutPeriodRecord, HoldoutPeriodState,
};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use super::super::postgres::{audit_timestamp, encode_message, timestamp_millis, verified_blob};
use super::super::{approval, holdout, validate_id};
use super::{GrantResult, StoreError, StoreResult, resolve};

pub(in crate::store) struct PersistedPeriod {
    pub(in crate::store) record: HoldoutPeriodRecord,
    pub(in crate::store) canonical_bytes: Vec<u8>,
}

pub(in crate::store) struct PersistedGrant {
    pub(in crate::store) grant: HoldoutGrantRecord,
    pub(in crate::store) period: PersistedPeriod,
    pub(in crate::store) freeze: FreezeManifestReference,
}

pub(in crate::store) fn reference(
    grant: &HoldoutGrantRecord,
) -> StoreResult<&HoldoutGrantReference> {
    grant
        .reference
        .as_ref()
        .ok_or(StoreError::Corrupt("grant reference absent"))
}

pub(in crate::store) fn grant_id(grant: &HoldoutGrantRecord) -> StoreResult<&str> {
    Ok(&reference(grant)?
        .holdout_grant_id
        .as_ref()
        .ok_or(StoreError::Corrupt("grant identity absent"))?
        .value)
}

pub(in crate::store) fn period_id(grant: &HoldoutGrantRecord) -> StoreResult<&str> {
    Ok(&reference(grant)?
        .holdout_period_id
        .as_ref()
        .ok_or(StoreError::Corrupt("grant period absent"))?
        .value)
}

pub(in crate::store) fn record_time(value: Option<&prost_types::Timestamp>) -> StoreResult<i64> {
    let value = value.ok_or(StoreError::Corrupt("grant timestamp absent"))?;
    if value.nanos % 1_000_000 != 0 {
        return Err(StoreError::Corrupt("grant timestamp precision"));
    }
    let millis =
        timestamp_millis(value, false).map_err(|_| StoreError::Corrupt("grant timestamp"))?;
    audit_timestamp(millis).map_err(|_| StoreError::Corrupt("grant timestamp range"))?;
    Ok(millis)
}

pub(in crate::store) async fn load_period(
    transaction: &mut Transaction<'_, Postgres>,
    id: &str,
) -> StoreResult<PersistedPeriod> {
    let row = sqlx::query("SELECT * FROM holdout_periods WHERE period_id = $1")
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::NotFound)?;
    Ok(PersistedPeriod {
        record: holdout::record_from_row(&row)?,
        canonical_bytes: row.try_get("canonical_blob")?,
    })
}

fn validate_pair(grant: &HoldoutGrantRecord, period: &HoldoutPeriodRecord) -> StoreResult<()> {
    let reference = reference(grant)?;
    let wire = period
        .period
        .as_ref()
        .ok_or(StoreError::Corrupt("grant period envelope"))?;
    validate_id(grant_id(grant)?).map_err(|_| StoreError::Corrupt("grant identity"))?;
    resolve::digest(reference.freeze_manifest_sha256.as_ref())?;
    resolve::digest(reference.evaluation_plan_sha256.as_ref())?;
    let plan_id = &reference
        .holdout_evaluation_plan_id
        .as_ref()
        .ok_or(StoreError::Corrupt("grant plan identity"))?
        .value;
    loop_core::audit::Sha256Digest::parse(plan_id)
        .map_err(|_| StoreError::Corrupt("grant plan identity"))?;
    resolve::policy(grant.approval_policy.as_ref())
        .map_err(|_| StoreError::Corrupt("grant approval policy"))?;
    let issued = record_time(reference.issued_at.as_ref())?;
    let expires = record_time(reference.expires_at.as_ref())?;
    resolve::validate_expiry(issued, expires).map_err(|_| StoreError::Corrupt("grant validity"))?;
    if reference.holdout_period_id != wire.holdout_period_id
        || reference.canonical_period_sha256 != wire.canonical_period_sha256
        || grant.holdout_evaluation_plan_id != reference.holdout_evaluation_plan_id
        || grant.evaluation_plan_sha256 != reference.evaluation_plan_sha256
        || grant.evaluation_plan_entry_count != reference.evaluation_plan_entry_count
        || grant.canonical_period_sha256 != reference.canonical_period_sha256
        || !(1..=4096).contains(&grant.evaluation_plan_entry_count)
        || !(1..=8).contains(&grant.approval_records.len())
        || period.issued_grant_id != reference.holdout_grant_id
        || period.grant_issued_at != reference.issued_at
    {
        return Err(StoreError::Corrupt("grant period binding"));
    }
    let terminal = period
        .terminal_at
        .as_ref()
        .map(|value| record_time(Some(value)))
        .transpose()?;
    let valid_state = match HoldoutGrantState::try_from(grant.state) {
        Ok(HoldoutGrantState::Issued) => {
            grant.revision == 1
                && period.state == HoldoutPeriodState::GrantIssued as i32
                && period.revision == 2
                && terminal.is_none()
                && grant.consumed_at.is_none()
        }
        Ok(HoldoutGrantState::Consumed) => {
            grant.revision == 2
                && period.state == HoldoutPeriodState::Consumed as i32
                && period.revision == 3
                && terminal.is_some_and(|end| issued <= end && end < expires)
                && grant.consumed_at == period.terminal_at
        }
        Ok(HoldoutGrantState::Expired) => {
            grant.revision == 2
                && period.state == HoldoutPeriodState::Closed as i32
                && period.revision == 3
                && terminal.is_some_and(|end| end >= expires)
                && grant.consumed_at.is_none()
        }
        Ok(HoldoutGrantState::Revoked) => {
            grant.revision == 2
                && period.state == HoldoutPeriodState::Closed as i32
                && period.revision == 3
                && terminal.is_some_and(|end| issued <= end && end < expires)
                && grant.consumed_at.is_none()
        }
        _ => false,
    };
    if !valid_state {
        return Err(StoreError::Corrupt("grant lifecycle"));
    }
    let mut previous = None;
    let mut ids = HashSet::new();
    let mut digests = HashSet::new();
    for item in &grant.approval_records {
        let id = &item
            .holdout_approval_record_id
            .as_ref()
            .ok_or(StoreError::Corrupt("grant approval identity"))?
            .value;
        let actor = &item
            .approved_by_actor_id
            .as_ref()
            .ok_or(StoreError::Corrupt("grant approval actor"))?
            .value;
        validate_id(id).map_err(|_| StoreError::Corrupt("grant approval identity"))?;
        validate_id(actor).map_err(|_| StoreError::Corrupt("grant approval actor"))?;
        let digest = resolve::digest(item.approval_record_sha256.as_ref())?;
        if !ids.insert(id)
            || !digests.insert(digest)
            || previous.is_some_and(|value| value >= actor.as_str())
            || item.holdout_period_id != reference.holdout_period_id
            || item.freeze_manifest_sha256 != reference.freeze_manifest_sha256
            || item.holdout_evaluation_plan_id != reference.holdout_evaluation_plan_id
            || item.evaluation_plan_sha256 != reference.evaluation_plan_sha256
            || item.evaluation_plan_entry_count != reference.evaluation_plan_entry_count
            || item.canonical_period_sha256 != reference.canonical_period_sha256
            || record_time(item.approved_at.as_ref())? > issued
            || record_time(item.expires_at.as_ref())? < expires
        {
            return Err(StoreError::Corrupt("grant approval binding"));
        }
        previous = Some(actor.as_str());
    }
    Ok(())
}

pub(in crate::store) async fn load_grant(
    transaction: &mut Transaction<'_, Postgres>,
    id: &str,
) -> StoreResult<PersistedGrant> {
    let row = sqlx::query("SELECT * FROM holdout_grants WHERE grant_id = $1")
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::NotFound)?;
    let grant =
        HoldoutGrantRecord::decode(verified_blob(&row, "record_blob", "record_sha256")?.as_slice())
            .map_err(|_| StoreError::Corrupt("grant envelope"))?;
    let freeze = FreezeManifestReference::decode(
        verified_blob(&row, "freeze_blob", "freeze_blob_sha256")?.as_slice(),
    )
    .map_err(|_| StoreError::Corrupt("grant freeze envelope"))?;
    let plan =
        resolve::freeze_shape(&freeze).map_err(|_| StoreError::Corrupt("grant freeze metadata"))?;
    let period = load_period(transaction, period_id(&grant)?).await?;
    validate_pair(&grant, &period.record).map_err(|_| StoreError::Corrupt("grant aggregate"))?;
    let reference = reference(&grant)?;
    let issued = record_time(reference.issued_at.as_ref())?;
    let expires = record_time(reference.expires_at.as_ref())?;
    let terminal = period
        .record
        .terminal_at
        .as_ref()
        .map(|value| record_time(Some(value)))
        .transpose()?;
    if row.try_get::<String, _>("grant_id")? != grant_id(&grant)?
        || row.try_get::<String, _>("period_id")? != period_id(&grant)?
        || row.try_get::<Vec<u8>, _>("freeze_sha256")?
            != resolve::digest(reference.freeze_manifest_sha256.as_ref())?
        || row.try_get::<i32, _>("approval_count")? != grant.approval_records.len() as i32
        || row.try_get::<i32, _>("state")? != grant.state
        || row.try_get::<i64, _>("revision")? != grant.revision as i64
        || row.try_get::<i64, _>("issued_at_ms")? != issued
        || row.try_get::<i64, _>("expires_at_ms")? != expires
        || row.try_get::<Option<i64>, _>("terminal_at_ms")? != terminal
        || freeze
            .manifest
            .as_ref()
            .and_then(|value| value.sha256.as_ref())
            != reference.freeze_manifest_sha256.as_ref()
        || freeze.holdout_approval_policy != grant.approval_policy
        || plan.holdout_period_id != reference.holdout_period_id
        || plan.canonical_period_sha256 != reference.canonical_period_sha256
        || plan.holdout_evaluation_plan_id != reference.holdout_evaluation_plan_id
        || plan.plan_sha256 != reference.evaluation_plan_sha256
        || plan.entry_count != reference.evaluation_plan_entry_count
    {
        return Err(StoreError::Corrupt("grant projection mismatch"));
    }
    let rows = sqlx::query(
        "SELECT a.*, l.actor_id AS link_actor, l.authenticated_subject AS link_subject,
        l.period_id AS link_period FROM holdout_grant_approvals l
        JOIN holdout_approvals a ON a.approval_id = l.approval_id
        WHERE l.grant_id = $1 ORDER BY l.actor_id COLLATE \"C\" LIMIT 9",
    )
    .bind(id)
    .fetch_all(&mut **transaction)
    .await?;
    if rows.len() != grant.approval_records.len() {
        return Err(StoreError::Corrupt("grant attachment count"));
    }
    let mut subjects = HashSet::new();
    for (row, expected) in rows.iter().zip(&grant.approval_records) {
        let record = approval::record_from_row(row)?;
        let human = record
            .approved_by
            .as_ref()
            .ok_or(StoreError::Corrupt("approval human"))?;
        let actor = human
            .actor_id
            .as_ref()
            .ok_or(StoreError::Corrupt("approval actor"))?;
        if &resolve::approval_reference(&record) != expected
            || !subjects.insert(human.authenticated_subject.clone())
            || row.try_get::<String, _>("link_actor")? != actor.value
            || row.try_get::<String, _>("link_subject")? != human.authenticated_subject
            || row.try_get::<String, _>("link_period")? != period_id(&grant)?
        {
            return Err(StoreError::Corrupt("grant approval attachment"));
        }
        resolve::bind_approval(&record, &freeze, issued)
            .map_err(|_| StoreError::Corrupt("grant approval validity"))?;
    }
    Ok(PersistedGrant {
        grant,
        period,
        freeze,
    })
}

pub(in crate::store) async fn insert_grant(
    transaction: &mut Transaction<'_, Postgres>,
    grant: &HoldoutGrantRecord,
    freeze: &FreezeManifestReference,
    approvals: &[HoldoutApprovalRecord],
) -> StoreResult<()> {
    let reference = reference(grant)?;
    let record_blob = encode_message(grant)?;
    let freeze_blob = encode_message(freeze)?;
    sqlx::query(
        "INSERT INTO holdout_grants
        (grant_id, period_id, freeze_sha256, freeze_blob, freeze_blob_sha256, approval_count,
         state, revision, issued_at_ms, expires_at_ms, record_blob, record_sha256)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    )
    .bind(grant_id(grant)?)
    .bind(period_id(grant)?)
    .bind(resolve::digest(reference.freeze_manifest_sha256.as_ref())?)
    .bind(&freeze_blob)
    .bind(Sha256::digest(&freeze_blob).as_slice())
    .bind(approvals.len() as i32)
    .bind(grant.state)
    .bind(grant.revision as i64)
    .bind(record_time(reference.issued_at.as_ref())?)
    .bind(record_time(reference.expires_at.as_ref())?)
    .bind(&record_blob)
    .bind(Sha256::digest(&record_blob).as_slice())
    .execute(&mut **transaction)
    .await?;
    for record in approvals {
        let human = record
            .approved_by
            .as_ref()
            .ok_or(StoreError::Corrupt("approval human"))?;
        sqlx::query("INSERT INTO holdout_grant_approvals (grant_id, period_id, approval_id, actor_id, authenticated_subject)
            VALUES ($1,$2,$3,$4,$5)")
            .bind(grant_id(grant)?).bind(period_id(grant)?)
            .bind(&record.holdout_approval_record_id.as_ref().ok_or(StoreError::Corrupt("approval id"))?.value)
            .bind(&human.actor_id.as_ref().ok_or(StoreError::Corrupt("approval actor"))?.value)
            .bind(&human.authenticated_subject).execute(&mut **transaction).await?;
    }
    Ok(())
}

pub(in crate::store) async fn update_period(
    transaction: &mut Transaction<'_, Postgres>,
    record: &HoldoutPeriodRecord,
    expected: u64,
) -> StoreResult<()> {
    let blob = encode_message(record)?;
    let id = record
        .period
        .as_ref()
        .and_then(|value| value.holdout_period_id.as_ref())
        .ok_or(StoreError::Corrupt("period identity"))?;
    let updated = sqlx::query(
        "UPDATE holdout_periods SET state=$1, revision=$2, issued_grant_id=$3,
        grant_issued_at_ms=$4, terminal_at_ms=$5, record_blob=$6, record_sha256=$7
        WHERE period_id=$8 AND revision=$9",
    )
    .bind(record.state)
    .bind(record.revision as i64)
    .bind(record.issued_grant_id.as_ref().map(|value| &value.value))
    .bind(
        record
            .grant_issued_at
            .as_ref()
            .map(|value| record_time(Some(value)))
            .transpose()?,
    )
    .bind(
        record
            .terminal_at
            .as_ref()
            .map(|value| record_time(Some(value)))
            .transpose()?,
    )
    .bind(&blob)
    .bind(Sha256::digest(&blob).as_slice())
    .bind(&id.value)
    .bind(expected as i64)
    .execute(&mut **transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(StoreError::RevisionConflict);
    }
    Ok(())
}

pub(in crate::store) async fn update_grant(
    transaction: &mut Transaction<'_, Postgres>,
    grant: &HoldoutGrantRecord,
    expected: u64,
    terminal: i64,
) -> StoreResult<()> {
    let blob = encode_message(grant)?;
    let updated = sqlx::query(
        "UPDATE holdout_grants SET state=$1, revision=$2, terminal_at_ms=$3,
        record_blob=$4, record_sha256=$5 WHERE grant_id=$6 AND revision=$7",
    )
    .bind(grant.state)
    .bind(grant.revision as i64)
    .bind(terminal)
    .bind(&blob)
    .bind(Sha256::digest(&blob).as_slice())
    .bind(grant_id(grant)?)
    .bind(expected as i64)
    .execute(&mut **transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(StoreError::RevisionConflict);
    }
    Ok(())
}

pub(in crate::store) fn decode_response(
    bytes: &[u8],
    canonical: &[u8],
) -> StoreResult<GrantResult> {
    let response = RequestHoldoutGrantResponse::decode(bytes)
        .map_err(|_| StoreError::Corrupt("grant response receipt"))?;
    let grant = response
        .grant
        .ok_or(StoreError::Corrupt("grant response absent"))?;
    let period = response
        .period_record
        .ok_or(StoreError::Corrupt("grant period response absent"))?;
    let period = holdout::decode_record(&encode_message(&period)?, canonical)?;
    validate_pair(&grant, &period).map_err(|_| StoreError::Corrupt("grant response aggregate"))?;
    Ok(GrantResult {
        grant,
        period,
        replayed: false,
    })
}

pub(in crate::store) fn verify_issued_receipt(
    original: &GrantResult,
    current: &PersistedGrant,
    command: &RequestHoldoutGrantRequest,
    receipt: &PgRow,
    now: i64,
) -> StoreResult<()> {
    let issued = record_time(reference(&original.grant)?.issued_at.as_ref())?;
    let mut ids = original
        .grant
        .approval_records
        .iter()
        .map(|value| value.holdout_approval_record_id.clone())
        .collect::<Vec<_>>();
    ids.sort_by(|a, b| {
        a.as_ref()
            .map(|v| &v.value)
            .cmp(&b.as_ref().map(|v| &v.value))
    });
    let requested_ids = command
        .approval_record_ids
        .iter()
        .cloned()
        .map(Some)
        .collect::<Vec<_>>();
    if original.grant.state != HoldoutGrantState::Issued as i32
        || original.grant.reference != current.grant.reference
        || original.grant.approval_records != current.grant.approval_records
        || original.grant.approval_policy != current.grant.approval_policy
        || original.period.period != current.period.record.period
        || command.freeze_manifest.as_ref() != Some(&current.freeze)
        || command.holdout_period_id != reference(&original.grant)?.holdout_period_id
        || command.expected_period_revision != 1
        || ids != requested_ids
        || receipt.try_get::<String, _>("period_id")? != period_id(&original.grant)?
        || receipt.try_get::<i64, _>("committed_at_ms")? != issued
        || issued > now
    {
        return Err(StoreError::Corrupt("grant receipt binding"));
    }
    Ok(())
}
