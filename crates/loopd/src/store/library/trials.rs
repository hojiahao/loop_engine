use loop_protocol::wire::v1::{Actor, JobRecord, JobSpecification, job_specification};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};

use super::FactorTrial;
use crate::store::postgres::{encode_message, record_from_row};
use crate::store::{PgJobStore, StoreError, StoreResult, validate_id};

pub(crate) fn factor_id(job: &JobSpecification) -> StoreResult<Option<&str>> {
    let id = match &job.input {
        Some(job_specification::Input::Backtest(input)) => input.factor_spec_id.as_ref(),
        Some(job_specification::Input::FactorEvaluation(input)) => input
            .factor
            .as_ref()
            .and_then(|factor| factor.factor_spec_id.as_ref()),
        _ => return Ok(None),
    };
    Ok(Some(&id.ok_or(StoreError::Invalid("trial factor"))?.value))
}

pub(crate) async fn register(
    transaction: &mut Transaction<'_, Postgres>,
    job: &JobSpecification,
) -> StoreResult<()> {
    let Some(factor) = factor_id(job)? else {
        return Ok(());
    };
    sqlx::query("INSERT INTO factor_trials (job_id, factor_spec_id, specification_sha256, run_id) VALUES ($1, $2, $3, $4)")
        .bind(&job.job_id.as_ref().ok_or(StoreError::Invalid("trial job"))?.value)
        .bind(factor)
        .bind(Sha256::digest(encode_message(job)?).to_vec())
        .bind(&job.run_id.as_ref().ok_or(StoreError::Invalid("trial run"))?.value)
        .execute(&mut **transaction).await?;
    Ok(())
}

pub(super) async fn verify(
    transaction: &mut Transaction<'_, Postgres>,
    record: &JobRecord,
) -> StoreResult<FactorTrial> {
    let job = record
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("trial job"))?;
    let id = &job
        .job_id
        .as_ref()
        .ok_or(StoreError::Corrupt("trial job id"))?
        .value;
    let factor = factor_id(job)?.ok_or(StoreError::Invalid("research trial required"))?;
    let row = sqlx::query("SELECT * FROM factor_trials WHERE job_id = $1")
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::Corrupt("trial missing"))?;
    if row.try_get::<String, _>("factor_spec_id")? != factor
        || row.try_get::<String, _>("run_id")?
            != job
                .run_id
                .as_ref()
                .ok_or(StoreError::Corrupt("trial run"))?
                .value
        || row.try_get::<Vec<u8>, _>("specification_sha256")?
            != Sha256::digest(encode_message(job)?).as_slice()
    {
        return Err(StoreError::Corrupt("trial binding"));
    }
    Ok(FactorTrial {
        job_id: id.clone(),
        factor_spec_id: factor.to_owned(),
        state: record.state,
        attempt: record.attempt,
    })
}

pub(super) async fn page(
    store: &PgJobStore,
    principal: &Actor,
    run_id: &str,
    after: &str,
    limit: u32,
) -> StoreResult<Vec<FactorTrial>> {
    validate_id(run_id)?;
    if !after.is_empty() {
        validate_id(after)?;
    }
    if !(1..=500).contains(&limit) || principal.authenticated_subject.is_empty() {
        return Err(StoreError::Invalid("trial page or principal"));
    }
    let mut transaction = store.pool.begin().await?;
    let rows = sqlx::query("SELECT j.* FROM factor_trials t JOIN jobs j ON t.job_id = j.job_id WHERE t.run_id = $1 AND t.job_id > $2 ORDER BY t.job_id LIMIT $3")
        .bind(run_id).bind(after).bind(i64::from(limit)).fetch_all(&mut *transaction).await?;
    let mut trials = Vec::with_capacity(rows.len());
    for row in rows {
        let record = record_from_row(&row)?;
        store
            .admission
            .authorize_job_command("loop.factors.read_trials", principal, &record)?;
        trials.push(verify(&mut transaction, &record).await?);
    }
    transaction.commit().await?;
    Ok(trials)
}
