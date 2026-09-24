mod plan;
mod replay;

use std::time::Duration;

use loop_core::audit::{AuditAction, AuditTarget, AuditTargetKind, canonicalize_audit_payload};
use loop_protocol::job::validate_job_specification;
use loop_protocol::wire::holdout::v1::{
    ConsumeGrantAndEnqueueBacktestRequest, ConsumeGrantAndEnqueueBacktestResponse, JobBatchHandle,
};
use loop_protocol::wire::v1::{
    Actor, CommandContext, HoldoutBacktestJobInput, HoldoutGrantState, HoldoutPeriodState,
    JobBatchId, JobId, JobKind, JobSpecification, job_specification,
};
use prost::Message;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};

use super::grant::{self, resolve, state};
use super::lifecycle::validate_context;
use super::postgres::{deadline_millis, encode_message, insert_job, timestamp};
use super::{
    PgJobStore, StoreError, StoreResult, SubmissionMetadata, SubmitJob, audit, holdout, validate_id,
};

const CONSUME: &str = "loop.holdout.consume_grant";

/// Original narrow batch receipt; replay is not authority to execute again.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchResult {
    /// Frozen grant reference, complete ordered batch handle, and consumed period.
    pub response: ConsumeGrantAndEnqueueBacktestResponse,
    /// True if no new job, state transition or audit event was committed.
    pub replayed: bool,
}

pub(super) async fn consume(
    store: &PgJobStore,
    principal: &Actor,
    command: ConsumeGrantAndEnqueueBacktestRequest,
    metadata: SubmissionMetadata,
) -> StoreResult<BatchResult> {
    tokio::time::timeout(
        Duration::from_secs(30),
        consume_inner(store, principal, command, metadata),
    )
    .await
    .map_err(|_| StoreError::Unavailable("batch command deadline"))?
}

async fn consume_inner(
    store: &PgJobStore,
    principal: &Actor,
    command: ConsumeGrantAndEnqueueBacktestRequest,
    metadata: SubmissionMetadata,
) -> StoreResult<BatchResult> {
    let context = validate_context(command.context.as_ref(), principal)?;
    let reference = command
        .grant_reference
        .as_ref()
        .ok_or(StoreError::Invalid("consume grant reference"))?;
    let grant_id = &reference
        .holdout_grant_id
        .as_ref()
        .ok_or(StoreError::Invalid("consume grant identity"))?
        .value;
    let period_id = &reference
        .holdout_period_id
        .as_ref()
        .ok_or(StoreError::Invalid("consume period identity"))?
        .value;
    validate_id(grant_id)?;
    validate_id(period_id)?;
    validate_id(&metadata.run_id.value)?;
    for revision in [
        command.expected_grant_revision,
        command.expected_period_revision,
    ] {
        if revision == 0 || revision > i64::MAX as u64 {
            return Err(StoreError::Invalid("consume revision"));
        }
    }
    store
        .holdout_policy
        .authorize_period(CONSUME, principal, period_id)?;
    let mut normalized = command.clone();
    grant::normalize_context(&mut normalized.context);
    let request_blob = encode_message(&normalized)?;
    let mut transaction = store.pool.begin().await?;
    let now = grant::command_time(store, &mut transaction, context).await?;
    store
        .holdout_policy
        .authorize_period(CONSUME, principal, period_id)?;
    let current = state::load_grant(&mut transaction, grant_id).await?;
    if current.grant.reference.as_ref() != Some(reference) {
        return Err(StoreError::Invalid("consume grant binding"));
    }
    if let Some(receipt) = grant::receipt(&mut transaction, context, CONSUME).await? {
        let result = replay::verify(
            store,
            &mut transaction,
            &current,
            &normalized,
            &metadata,
            &receipt,
            now,
        )
        .await?;
        transaction.commit().await?;
        return Ok(result);
    }
    if current.grant.revision != command.expected_grant_revision
        || current.period.record.revision != command.expected_period_revision
    {
        return Err(StoreError::RevisionConflict);
    }
    if current.grant.state != HoldoutGrantState::Issued as i32 {
        return Err(StoreError::InvalidTransition);
    }
    let expires = state::record_time(reference.expires_at.as_ref())?;
    if now < state::record_time(reference.issued_at.as_ref())? || now >= expires {
        return Err(StoreError::Invalid("consume grant validity"));
    }
    let entries = plan::materialize(store, &current, now).await?;
    let batch_id = JobBatchId {
        value: format!("batch.{}", uuid::Uuid::new_v4().simple()),
    };
    let mut jobs = Vec::with_capacity(entries.len());
    let mut total = 0_usize;
    for (index, (spec, budget)) in entries.into_iter().enumerate() {
        let job = JobSpecification {
            job_id: Some(JobId {
                value: format!("job.{}", uuid::Uuid::new_v4().simple()),
            }),
            run_id: Some(metadata.run_id.clone()),
            kind: JobKind::HoldoutBacktest as i32,
            input: Some(job_specification::Input::HoldoutBacktest(
                HoldoutBacktestJobInput {
                    consumed_grant: Some(reference.clone()),
                    consumed_grant_revision: 2,
                    frozen_backtest_spec: Some(spec),
                    budget: Some(budget),
                    job_batch_id: Some(batch_id.clone()),
                    holdout_evaluation_plan_id: reference.holdout_evaluation_plan_id.clone(),
                    evaluation_plan_sha256: reference.evaluation_plan_sha256.clone(),
                    evaluation_plan_entry_index: index as u32 + 1,
                },
            )),
            submitted_at: Some(timestamp(now)),
            submitted_by: Some(principal.clone()),
            idempotency_key: context.idempotency_key.clone(),
            correlation_id: context.correlation_id.clone(),
            causation_id: context.causation_id.clone(),
            protocol_selection: Some(metadata.protocol_selection.clone()),
        };
        validate_admission(store, &job)?;
        total = total
            .checked_add(job.encoded_len())
            .ok_or(StoreError::Invalid("batch envelope size"))?;
        if total > 64 * 1024 * 1024 {
            return Err(StoreError::Invalid("batch envelope size"));
        }
        jobs.push(job);
        tokio::task::yield_now().await;
    }
    let handle = JobBatchHandle {
        job_batch_id: Some(batch_id),
        holdout_grant_id: reference.holdout_grant_id.clone(),
        holdout_evaluation_plan_id: reference.holdout_evaluation_plan_id.clone(),
        evaluation_plan_sha256: reference.evaluation_plan_sha256.clone(),
        evaluation_plan_entry_count: reference.evaluation_plan_entry_count,
        job_count: jobs.len() as u32,
        job_ids: jobs.iter().filter_map(|job| job.job_id.clone()).collect(),
        revision: 1,
        created_at: Some(timestamp(now)),
    };
    if handle.job_count != reference.evaluation_plan_entry_count {
        return Err(StoreError::Invalid("incomplete frozen batch"));
    }
    insert_batch(
        &mut transaction,
        &handle,
        context,
        &metadata,
        period_id,
        now,
    )
    .await?;
    for (index, job) in jobs.iter().enumerate() {
        insert_job(&mut transaction, job, now).await?;
        let spec_blob = encode_message(job)?;
        sqlx::query("INSERT INTO holdout_batch_jobs (batch_id,entry_index,job_id,specification_sha256) VALUES ($1,$2,$3,$4)")
            .bind(&handle.job_batch_id.as_ref().expect("assigned batch").value).bind(index as i32 + 1)
            .bind(&job.job_id.as_ref().expect("assigned job").value).bind(Sha256::digest(&spec_blob).as_slice())
            .execute(&mut *transaction).await?;
        audit::append_command(
            &mut transaction,
            &store.ledger_id,
            &SubmitJob {
                request_id: context
                    .request_id
                    .as_ref()
                    .expect("validated request")
                    .value
                    .clone(),
                specification: job.clone(),
            },
            CONSUME,
            now,
        )
        .await?;
        #[cfg(test)]
        super::crash_tests::fault_point("batch_mid_insert").await;
    }
    let mut consumed = current.grant;
    consumed.state = HoldoutGrantState::Consumed as i32;
    consumed.revision = 2;
    consumed.consumed_at = Some(timestamp(now));
    let mut period = current.period.record;
    period.state = HoldoutPeriodState::Consumed as i32;
    period.revision = 3;
    period.terminal_at = Some(timestamp(now));
    state::update_grant(
        &mut transaction,
        &consumed,
        command.expected_grant_revision,
        now,
    )
    .await?;
    state::update_period(&mut transaction, &period, command.expected_period_revision).await?;
    audit::append(
        &mut transaction,
        &store.ledger_id,
        now,
        grant::event(
            context,
            principal,
            AuditAction::HoldoutGrantConsumed,
            AuditTarget {
                kind: AuditTargetKind::HoldoutGrantId,
                value: grant_id.clone(),
            },
            consumed_payload(reference, &handle)?,
        ),
    )
    .await?;
    let response = ConsumeGrantAndEnqueueBacktestResponse {
        consumed_grant: Some(reference.clone()),
        job_batch: Some(handle),
        period_record: Some(period),
    };
    holdout::save_receipt(
        &mut transaction,
        context,
        CONSUME,
        period_id,
        &request_blob,
        &encode_message(&response)?,
        now,
    )
    .await?;
    let final_time = store.clock.now_millis()?;
    if final_time < now {
        return Err(StoreError::ClockRegression);
    }
    if final_time >= expires {
        return Err(StoreError::Invalid("grant expired before commit"));
    }
    for job in &jobs {
        if deadline_millis(job)? <= final_time {
            return Err(StoreError::Invalid("batch budget expired before commit"));
        }
    }
    sqlx::query("UPDATE store_metadata SET last_observed_at_ms=$1 WHERE singleton=1")
        .bind(final_time)
        .execute(&mut *transaction)
        .await?;
    #[cfg(test)]
    super::crash_tests::fault_point("batch_before_commit").await;
    transaction.commit().await?;
    #[cfg(test)]
    super::crash_tests::fault_point("batch_after_commit").await;
    Ok(BatchResult {
        response,
        replayed: false,
    })
}

fn validate_admission(store: &PgJobStore, job: &JobSpecification) -> StoreResult<()> {
    validate_job_specification(job)?;
    store.admission.validate_submission(job)?;
    if !job
        .protocol_selection
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?
        .enabled_features
        .iter()
        .any(|feature| feature == "jobs.prelease-terminal.v1")
    {
        return Err(StoreError::AdmissionDenied);
    }
    Ok(())
}

async fn insert_batch(
    transaction: &mut Transaction<'_, Postgres>,
    handle: &JobBatchHandle,
    context: &CommandContext,
    metadata: &SubmissionMetadata,
    period_id: &str,
    now: i64,
) -> StoreResult<()> {
    let blob = encode_message(handle)?;
    sqlx::query("INSERT INTO holdout_batches (batch_id,grant_id,period_id,run_id,actor_id,plan_id,plan_sha256,
        job_count,created_at_ms,handle_blob,handle_sha256) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(&handle.job_batch_id.as_ref().expect("assigned batch").value)
        .bind(&handle.holdout_grant_id.as_ref().expect("validated grant").value).bind(period_id).bind(&metadata.run_id.value)
        .bind(&context.actor.as_ref().expect("validated actor").actor_id.as_ref().expect("validated actor id").value)
        .bind(&handle.holdout_evaluation_plan_id.as_ref().expect("validated plan").value)
        .bind(resolve::digest(handle.evaluation_plan_sha256.as_ref())?).bind(handle.job_count as i32).bind(now)
        .bind(&blob).bind(Sha256::digest(&blob).as_slice()).execute(&mut **transaction).await?;
    Ok(())
}

fn consumed_payload(
    reference: &loop_protocol::wire::v1::HoldoutGrantReference,
    handle: &JobBatchHandle,
) -> StoreResult<loop_core::audit::AuditPayload> {
    #[derive(Serialize)]
    struct Payload<'a> {
        holdout_grant_id: &'a str,
        holdout_period_id: &'a str,
        holdout_evaluation_plan_id: &'a str,
        job_batch_id: &'a str,
        capability_class: &'static str,
        authorization_decision: &'static str,
    }
    let payload = Payload {
        holdout_grant_id: &reference
            .holdout_grant_id
            .as_ref()
            .expect("validated grant")
            .value,
        holdout_period_id: &reference
            .holdout_period_id
            .as_ref()
            .expect("validated period")
            .value,
        holdout_evaluation_plan_id: &reference
            .holdout_evaluation_plan_id
            .as_ref()
            .expect("validated plan")
            .value,
        job_batch_id: &handle.job_batch_id.as_ref().expect("assigned batch").value,
        capability_class: "holdout_evaluation",
        authorization_decision: "authorized",
    };
    Ok(canonicalize_audit_payload(
        "loop.audit.holdout_grant_consumed",
        1,
        &serde_json::to_vec(&payload).map_err(|_| StoreError::Invalid("batch audit encoding"))?,
    )?)
}
