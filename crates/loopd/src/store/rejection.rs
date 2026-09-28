//! Transactional memory of development backtest rejections, never holdout results.

use loop_protocol::wire::v1::{
    FactorRejection, JobRecord, JobSpecification, Sha256Digest, job_outcome, job_specification,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use super::postgres::{record_from_row, timestamp_millis};
use super::{StoreError, StoreResult};

// This versioned lookup key is internal; it never replaces a canonical factor ID.
#[derive(Serialize)]
struct Context<'a> {
    factor_spec_id: &'a str,
    snapshot_ids: Vec<&'a str>,
    data_manifest: &'a [u8],
    return_definition: i32,
    provenance: [&'a [u8]; 6],
    seed: &'a [u8],
}

fn digest(value: Option<&Sha256Digest>) -> StoreResult<&[u8]> {
    value
        .filter(|value| value.value.len() == 32)
        .map(|value| value.value.as_slice())
        .ok_or(StoreError::Invalid("rejection context digest"))
}

fn context_key(job: &JobSpecification) -> StoreResult<Option<[u8; 32]>> {
    let Some(job_specification::Input::Backtest(input)) = &job.input else {
        return Ok(None);
    };
    let data = input
        .dataset
        .as_ref()
        .ok_or(StoreError::Invalid("dataset"))?;
    let provenance = input
        .provenance
        .as_ref()
        .ok_or(StoreError::Invalid("provenance"))?;
    let context = Context {
        factor_spec_id: &input
            .factor_spec_id
            .as_ref()
            .ok_or(StoreError::Invalid("factor id"))?
            .value,
        snapshot_ids: data
            .snapshot_ids
            .iter()
            .map(|id| id.value.as_str())
            .collect(),
        data_manifest: digest(data.manifest_sha256.as_ref())?,
        return_definition: input.return_definition,
        provenance: [
            digest(provenance.source_code_sha256.as_ref())?,
            digest(provenance.operator_registry_sha256.as_ref())?,
            digest(provenance.configuration_sha256.as_ref())?,
            digest(provenance.data_manifest_sha256.as_ref())?,
            digest(provenance.trading_calendar_sha256.as_ref())?,
            digest(provenance.environment_sha256.as_ref())?,
        ],
        seed: digest(input.deterministic_seed.as_ref())?,
    };
    let bytes = serde_json::to_vec(&context)
        .map_err(|_| StoreError::Invalid("rejection context encoding"))?;
    let mut hash = Sha256::new();
    hash.update(b"loop.backtest-rejection-context/v1\0");
    hash.update(bytes);
    Ok(Some(hash.finalize().into()))
}

fn rejection(record: &JobRecord) -> Option<&FactorRejection> {
    if !matches!(
        record.specification.as_ref()?.input,
        Some(job_specification::Input::Backtest(_))
    ) {
        return None;
    }
    match record.outcome.as_ref()?.outcome.as_ref()? {
        job_outcome::Outcome::FactorRejection(value) => Some(value),
        _ => None,
    }
}

pub(super) fn is_rejected_backtest(record: &JobRecord) -> bool {
    rejection(record).is_some()
}

pub(super) async fn record_completion(
    transaction: &mut Transaction<'_, Postgres>,
    record: &JobRecord,
    now: i64,
) -> StoreResult<()> {
    let Some(rejected) = rejection(record) else {
        return Ok(());
    };
    let job = record
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("job specification"))?;
    let key = context_key(job)?.ok_or(StoreError::Corrupt("rejection job kind"))?;
    sqlx::query(
        "INSERT INTO backtest_rejections
         (job_id, job_revision, context_sha256, rejection_code, committed_at_ms)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(
        &job.job_id
            .as_ref()
            .ok_or(StoreError::Corrupt("job id"))?
            .value,
    )
    .bind(record.revision as i64)
    .bind(key.as_slice())
    .bind(rejected.code)
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    #[cfg(test)]
    super::crash_tests::fault_point("rejection_after_insert").await;
    Ok(())
}

fn verify_projection(row: &PgRow, record: &JobRecord) -> StoreResult<[u8; 32]> {
    let rejected = rejection(record).ok_or(StoreError::Corrupt("rejection outcome"))?;
    let job = record
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("job specification"))?;
    let key = context_key(job)?.ok_or(StoreError::Corrupt("rejection context"))?;
    if row.try_get::<Vec<u8>, _>("context_sha256")? != key
        || row.try_get::<i64, _>("job_revision")? != record.revision as i64
        || row.try_get::<i32, _>("rejection_code")? != rejected.code
        || row.try_get::<i64, _>("committed_at_ms")?
            != timestamp_millis(
                record
                    .updated_at
                    .as_ref()
                    .ok_or(StoreError::Corrupt("job time"))?,
                false,
            )?
    {
        return Err(StoreError::Corrupt("rejection projection"));
    }
    Ok(key)
}

pub(super) async fn verify_stored(
    transaction: &mut Transaction<'_, Postgres>,
    record: &JobRecord,
) -> StoreResult<()> {
    if !is_rejected_backtest(record) {
        return Ok(());
    }
    let id = &record
        .specification
        .as_ref()
        .and_then(|job| job.job_id.as_ref())
        .ok_or(StoreError::Corrupt("job id"))?
        .value;
    let row = sqlx::query("SELECT * FROM backtest_rejections WHERE job_id = $1")
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::Corrupt("missing rejection record"))?;
    verify_projection(&row, record)?;
    Ok(())
}

pub(super) async fn check_previous(
    transaction: &mut Transaction<'_, Postgres>,
    job: &JobSpecification,
) -> StoreResult<()> {
    let Some(key) = context_key(job)? else {
        return Ok(());
    };
    // Only deterministic coverage/filter/performance outcomes are reusable.
    // Semantic review and correlation depend on evidence absent from this context.
    let row = sqlx::query(
        "SELECT j.*, r.job_revision, r.context_sha256, r.rejection_code, r.committed_at_ms
         FROM backtest_rejections r JOIN jobs j USING (job_id)
         WHERE r.context_sha256 = $1 AND r.rejection_code IN (4, 5, 6)
         ORDER BY r.job_id LIMIT 1",
    )
    .bind(key.as_slice())
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some(row) = row {
        let previous = record_from_row(&row)?;
        if verify_projection(&row, &previous)? != key {
            return Err(StoreError::Corrupt("rejection lookup binding"));
        }
        return Err(StoreError::PreviouslyRejected);
    }
    Ok(())
}
