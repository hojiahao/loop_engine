use std::collections::HashSet;

use loop_protocol::wire::holdout::v1::{
    ConsumeGrantAndEnqueueBacktestRequest, ConsumeGrantAndEnqueueBacktestResponse, JobBatchHandle,
};
use loop_protocol::wire::v1::{
    HoldoutBacktestJobInput, HoldoutGrantState, JobKind, job_specification,
};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use super::super::grant::{resolve, state};
use super::super::postgres::{encode_message, record_from_row, verified_blob};
use super::{
    BatchResult, PgJobStore, StoreError, StoreResult, SubmissionMetadata, plan, validate_id,
};

pub(super) async fn verify(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    current: &state::PersistedGrant,
    command: &ConsumeGrantAndEnqueueBacktestRequest,
    metadata: &SubmissionMetadata,
    receipt: &PgRow,
    now: i64,
) -> StoreResult<BatchResult> {
    let previous = ConsumeGrantAndEnqueueBacktestRequest::decode(
        verified_blob(receipt, "request_blob", "request_sha256")?.as_slice(),
    )
    .map_err(|_| StoreError::Corrupt("batch receipt request"))?;
    if &previous != command {
        return Err(StoreError::IdempotencyConflict);
    }
    let response = ConsumeGrantAndEnqueueBacktestResponse::decode(
        verified_blob(receipt, "response_blob", "response_sha256")?.as_slice(),
    )
    .map_err(|_| StoreError::Corrupt("batch receipt response"))?;
    let handle = response
        .job_batch
        .as_ref()
        .ok_or(StoreError::Corrupt("batch receipt handle"))?;
    let reference = state::reference(&current.grant)?;
    let consumed = state::record_time(current.grant.consumed_at.as_ref())?;
    if current.grant.state != HoldoutGrantState::Consumed as i32
        || current.grant.revision != 2
        || response.consumed_grant.as_ref() != Some(reference)
        || response.period_record.as_ref() != Some(&current.period.record)
        || command.expected_grant_revision != 1
        || command.expected_period_revision != 2
        || receipt.try_get::<String, _>("period_id")? != state::period_id(&current.grant)?
        || receipt.try_get::<i64, _>("committed_at_ms")? != consumed
        || consumed > now
        || state::record_time(handle.created_at.as_ref())? != consumed
        || handle.revision != 1
        || handle.holdout_grant_id != reference.holdout_grant_id
        || handle.holdout_evaluation_plan_id != reference.holdout_evaluation_plan_id
        || handle.evaluation_plan_sha256 != reference.evaluation_plan_sha256
        || handle.evaluation_plan_entry_count != reference.evaluation_plan_entry_count
        || handle.job_count != reference.evaluation_plan_entry_count
        || !(1..=4096).contains(&handle.job_count)
        || handle.job_ids.len() != handle.job_count as usize
    {
        return Err(StoreError::Corrupt("batch receipt binding"));
    }
    let batch_id = &handle
        .job_batch_id
        .as_ref()
        .ok_or(StoreError::Corrupt("batch identity"))?
        .value;
    validate_id(batch_id).map_err(|_| StoreError::Corrupt("batch identity"))?;
    if batch_id == state::grant_id(&current.grant)? {
        return Err(StoreError::Corrupt("batch identity alias"));
    }
    let context = command
        .context
        .as_ref()
        .ok_or(StoreError::Corrupt("batch context"))?;
    let actor = context
        .actor
        .as_ref()
        .ok_or(StoreError::Corrupt("batch actor"))?;
    let row = sqlx::query("SELECT * FROM holdout_batches WHERE batch_id=$1")
        .bind(batch_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::Corrupt("batch absent"))?;
    let stored =
        JobBatchHandle::decode(verified_blob(&row, "handle_blob", "handle_sha256")?.as_slice())
            .map_err(|_| StoreError::Corrupt("batch handle envelope"))?;
    if row.try_get::<String, _>("run_id")? != metadata.run_id.value {
        return Err(StoreError::IdempotencyConflict);
    }
    if &stored != handle
        || row.try_get::<String, _>("grant_id")? != state::grant_id(&current.grant)?
        || row.try_get::<String, _>("period_id")? != state::period_id(&current.grant)?
        || row.try_get::<String, _>("actor_id")?
            != actor
                .actor_id
                .as_ref()
                .ok_or(StoreError::Corrupt("batch actor id"))?
                .value
        || row.try_get::<String, _>("plan_id")?
            != reference
                .holdout_evaluation_plan_id
                .as_ref()
                .expect("validated plan")
                .value
        || row.try_get::<Vec<u8>, _>("plan_sha256")?
            != resolve::digest(reference.evaluation_plan_sha256.as_ref())?
        || row.try_get::<i32, _>("job_count")? != handle.job_count as i32
        || row.try_get::<i64, _>("created_at_ms")? != consumed
    {
        return Err(StoreError::Corrupt("batch projection mismatch"));
    }
    let entries = plan::materialize(store, current, now).await?;
    if entries.len() != handle.job_count as usize {
        return Err(StoreError::Corrupt("batch plan count"));
    }
    let links = sqlx::query("SELECT entry_index,job_id,specification_sha256 FROM holdout_batch_jobs WHERE batch_id=$1 ORDER BY entry_index LIMIT 4097")
        .bind(batch_id).fetch_all(&mut **transaction).await?;
    if links.len() != entries.len() {
        return Err(StoreError::Corrupt("incomplete batch jobs"));
    }
    let mut ids = HashSet::new();
    let mut selection = None;
    let mut total = 0_usize;
    for (index, (link, (spec, budget))) in links.iter().zip(entries).enumerate() {
        let job_id: String = link.try_get("job_id")?;
        if link.try_get::<i32, _>("entry_index")? != index as i32 + 1
            || handle.job_ids[index].value != job_id
            || !ids.insert(job_id.clone())
        {
            return Err(StoreError::Corrupt("batch job order"));
        }
        let row = sqlx::query("SELECT * FROM jobs WHERE job_id=$1")
            .bind(&job_id)
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or(StoreError::Corrupt("batch job absent"))?;
        let record = record_from_row(&row)?;
        let job = record
            .specification
            .ok_or(StoreError::Corrupt("batch job specification"))?;
        let blob = encode_message(&job)?;
        total = total
            .checked_add(blob.len())
            .ok_or(StoreError::Corrupt("batch job size"))?;
        if total > 64 * 1024 * 1024 {
            return Err(StoreError::Corrupt("batch job size"));
        }
        let expected = HoldoutBacktestJobInput {
            consumed_grant: Some(reference.clone()),
            consumed_grant_revision: 2,
            frozen_backtest_spec: Some(spec),
            budget: Some(budget),
            job_batch_id: handle.job_batch_id.clone(),
            holdout_evaluation_plan_id: reference.holdout_evaluation_plan_id.clone(),
            evaluation_plan_sha256: reference.evaluation_plan_sha256.clone(),
            evaluation_plan_entry_index: index as u32 + 1,
        };
        if link.try_get::<Vec<u8>, _>("specification_sha256")? != Sha256::digest(&blob).as_slice()
            || job.kind != JobKind::HoldoutBacktest as i32
            || job.input != Some(job_specification::Input::HoldoutBacktest(expected))
            || job.run_id.as_ref() != Some(&metadata.run_id)
            || job.submitted_by.as_ref() != Some(actor)
            || job.submitted_at != handle.created_at
            || job.idempotency_key != context.idempotency_key
            || job.correlation_id != context.correlation_id
            || job.causation_id != context.causation_id
            || selection
                .as_ref()
                .is_some_and(|value| Some(value) != job.protocol_selection.as_ref())
        {
            return Err(StoreError::Corrupt("batch job binding"));
        }
        selection = job.protocol_selection.clone();
        super::validate_admission(store, &job)?;
    }
    Ok(BatchResult {
        response,
        replayed: true,
    })
}
