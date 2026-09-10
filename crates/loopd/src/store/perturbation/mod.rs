//! Version-fenced numerical proposals from registered IS evidence.

mod evidence;
mod validation;

use std::future::Future;
use std::time::Duration;

use loop_core::audit::{AuditAction, AuditTarget, AuditTargetKind, canonicalize_audit_payload};
use loop_protocol::wire::v1::{Actor, CommandContext, JobId, PerturbationStep};
use prost::Message;
use prost_types::Timestamp;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};

use super::lifecycle::validate_context;
use super::postgres::{encode_message, timestamp, timestamp_millis};
use super::{PgJobStore, StoreError, StoreResult, audit, validate_id};
use crate::research_worker::PerturbationWorker;
use evidence::{Prepared, Receipt};

const OPERATION: &str = "loop.perturbation.advance";

/// Internal IS-only command. It accepts no caller-provided Sharpe or RNG state.
#[derive(Clone, PartialEq, Message)]
pub struct AdvancePerturbation {
    /// Attribution bound to a separately authenticated principal.
    #[prost(message, optional, tag = "1")]
    pub context: Option<CommandContext>,
    /// Completed primary IS backtest, or reusable deterministic rejection.
    #[prost(message, optional, tag = "2")]
    pub source_job_id: Option<JobId>,
    /// Immutable, server-resolved single-window parameter family and IS context.
    #[prost(string, tag = "3")]
    pub context_id: String,
    /// Last accepted state revision, or zero to initialize a new family.
    #[prost(uint64, tag = "4")]
    pub expected_revision: u64,
    /// Required absolute deadline, at most 30 seconds after request time.
    #[prost(message, optional, tag = "5")]
    pub deadline: Option<Timestamp>,
}

/// A committed proposal, not factor admission or authority to dispatch work.
#[derive(Clone, Debug, PartialEq)]
pub struct PerturbationResult {
    /// Family revision at the original acceptance.
    pub revision: u64,
    /// Persisted numerical history, random position and selected candidate.
    pub step: PerturbationStep,
    /// Original acceptance time, retained on replay.
    pub accepted_at: Timestamp,
    /// True for a revalidated receipt; never authorizes duplicate dispatch.
    pub replayed: bool,
}

/// Persistence boundary for the single-window perturbation workflow.
pub trait PerturbationRepository: Send + Sync {
    /// Resolve authorized IS evidence and failure memory, calculate outside the
    /// database lock, then atomically commit state, receipt and audit using CAS.
    /// `principal` is transport-authenticated; the server chooses `worker`.
    /// Replays revalidate references but do not call the worker or advance RNG.
    /// Deadlines, stale/denied references, invalid workers, clock regression and
    /// storage faults fail closed. Cancellation before commit rolls back; retry
    /// with the same key resolves an uncertain commit. No jobs are dispatched.
    fn advance_perturbation(
        &self,
        principal: &Actor,
        command: AdvancePerturbation,
        worker: &impl PerturbationWorker,
    ) -> impl Future<Output = StoreResult<PerturbationResult>> + Send;
}

impl PerturbationRepository for PgJobStore {
    async fn advance_perturbation(
        &self,
        principal: &Actor,
        command: AdvancePerturbation,
        worker: &impl PerturbationWorker,
    ) -> StoreResult<PerturbationResult> {
        tokio::time::timeout(
            Duration::from_secs(30),
            execute(self, principal, command, worker),
        )
        .await
        .map_err(|_| StoreError::Unavailable("perturbation timeout"))?
    }
}

impl AdvancePerturbation {
    fn normalized(&self) -> Self {
        let mut command = self.clone();
        if let Some(context) = &mut command.context {
            context.request_id = None;
            context.requested_at = None;
        }
        command.deadline = None;
        command
    }
}

async fn execute(
    store: &PgJobStore,
    principal: &Actor,
    command: AdvancePerturbation,
    worker: &impl PerturbationWorker,
) -> StoreResult<PerturbationResult> {
    let context = validate_context(command.context.as_ref(), principal)?;
    validate_id(&command.context_id)?;
    validate_id(
        &command
            .source_job_id
            .as_ref()
            .ok_or(StoreError::Invalid("source job"))?
            .value,
    )?;
    if command.expected_revision >= i64::MAX as u64 {
        return Err(StoreError::Invalid("perturbation revision"));
    }
    let requested = timestamp_millis(
        context
            .requested_at
            .as_ref()
            .ok_or(StoreError::Invalid("request time"))?,
        true,
    )?;
    let deadline = timestamp_millis(
        command
            .deadline
            .as_ref()
            .ok_or(StoreError::Invalid("perturbation deadline"))?,
        true,
    )?;
    if deadline <= requested || deadline - requested > 30_000 {
        return Err(StoreError::Invalid("perturbation deadline"));
    }
    let mut transaction = store.pool.begin().await?;
    let now = store.observe_clock(&mut transaction).await?;
    check_time(now, requested, deadline)?;
    let prepared = evidence::prepare(store, &mut transaction, principal, &command).await?;
    if let Some(result) = evidence::replay(&mut transaction, &command, &prepared, now).await? {
        final_time(store, now, deadline)?;
        transaction.commit().await?;
        return Ok(result);
    }
    if prepared.revision != command.expected_revision {
        return Err(StoreError::RevisionConflict);
    }
    transaction.commit().await?;
    let step = worker.advance(prepared.work.clone()).await?;
    validation::transition(&prepared.work, &prepared.space, &step)?;

    // Re-resolve authority/evidence after computation; no long-held SQL lock.
    let mut transaction = store.pool.begin().await?;
    let commit_start = store.observe_clock(&mut transaction).await?;
    check_time(commit_start, now, deadline)?;
    let current = evidence::prepare(store, &mut transaction, principal, &command).await?;
    if let Some(result) =
        evidence::replay(&mut transaction, &command, &current, commit_start).await?
    {
        final_time(store, commit_start, deadline)?;
        transaction.commit().await?;
        return Ok(result);
    }
    if current.revision != prepared.revision
        || current.space != prepared.space
        || current.record != prepared.record
        || current.work.state != prepared.work.state
        || current.work.observation != prepared.work.observation
    {
        return Err(StoreError::RevisionConflict);
    }
    if step
        .candidate
        .as_ref()
        .and_then(|candidate| candidate.factor_spec_id.as_ref())
        .is_some_and(|id| current.work.failed_factor_ids.contains(id))
    {
        return Err(StoreError::PreviouslyRejected);
    }
    validation::transition(&current.work, &current.space, &step)?;
    let accepted_ms = final_time(store, commit_start, deadline)?;
    let receipt = Receipt {
        record: Some(current.record.clone()),
        space: Some(current.space.clone()),
        context_id: command.context_id.clone(),
        revision: current.revision + 1,
        step: Some(step.clone()),
        accepted_at: Some(timestamp(accepted_ms)),
        request_id: context
            .request_id
            .as_ref()
            .ok_or(StoreError::Invalid("request id"))?
            .value
            .clone(),
    };
    persist(&mut transaction, &command, &current, &receipt).await?;
    #[cfg(test)]
    super::crash_tests::fault_point("perturbation_after_state").await;
    audit::append(
        &mut transaction,
        &store.ledger_id,
        accepted_ms,
        audit::EventInput {
            actor: principal,
            correlation_id: &context
                .correlation_id
                .as_ref()
                .ok_or(StoreError::Invalid("correlation id"))?
                .value,
            causation_id: &context
                .causation_id
                .as_ref()
                .ok_or(StoreError::Invalid("causation id"))?
                .value,
            action: AuditAction::CommandAccepted,
            target: AuditTarget {
                kind: AuditTargetKind::JobId,
                value: command
                    .source_job_id
                    .as_ref()
                    .ok_or(StoreError::Invalid("source job"))?
                    .value
                    .clone(),
            },
            payload: canonicalize_audit_payload(
                "loop.audit.command_accepted",
                1,
                &serde_json::to_vec(&serde_json::json!({
                    "command": OPERATION, "request_id": receipt.request_id,
                    "summary": format!("IS window proposal; context={}; revision={}", command.context_id, receipt.revision),
                }))
                .map_err(|_| StoreError::Invalid("perturbation audit"))?,
            )?,
        },
    )
    .await?;
    #[cfg(test)]
    super::crash_tests::fault_point("perturbation_before_commit").await;
    final_time(store, accepted_ms, deadline)?;
    transaction.commit().await?;
    #[cfg(test)]
    super::crash_tests::fault_point("perturbation_after_commit").await;
    receipt.result(false)
}

fn check_time(now: i64, previous: i64, deadline: i64) -> StoreResult<()> {
    if now < previous {
        return Err(StoreError::ClockRegression);
    }
    if now >= deadline {
        return Err(StoreError::Unavailable("perturbation deadline exceeded"));
    }
    Ok(())
}

fn final_time(store: &PgJobStore, previous: i64, deadline: i64) -> StoreResult<i64> {
    let now = store.clock.now_millis()?;
    check_time(now, previous, deadline)?;
    Ok(now)
}

async fn persist(
    transaction: &mut Transaction<'_, Postgres>,
    command: &AdvancePerturbation,
    prepared: &Prepared,
    receipt: &Receipt,
) -> StoreResult<()> {
    let context = command
        .context
        .as_ref()
        .ok_or(StoreError::Invalid("context"))?;
    let actor = &context
        .actor
        .as_ref()
        .and_then(|actor| actor.actor_id.as_ref())
        .ok_or(StoreError::AdmissionDenied)?
        .value;
    let key = &context
        .idempotency_key
        .as_ref()
        .ok_or(StoreError::Invalid("key"))?
        .value;
    let job = &command
        .source_job_id
        .as_ref()
        .ok_or(StoreError::Invalid("source job"))?
        .value;
    let step = receipt.step.as_ref().ok_or(StoreError::Invalid("step"))?;
    let state = encode_message(step.state.as_ref().ok_or(StoreError::Invalid("state"))?)?;
    let space = encode_message(&prepared.space)?;
    if state.len() > 1_048_576 || space.len() > 1_048_576 {
        return Err(StoreError::Invalid("perturbation state size"));
    }
    let accepted_ms = timestamp_millis(
        receipt
            .accepted_at
            .as_ref()
            .ok_or(StoreError::Invalid("time"))?,
        false,
    )?;
    let changed = sqlx::query(
        "INSERT INTO perturbation_states
         (context_id, revision, space_blob, space_sha256, state_blob, state_sha256, updated_at_ms, actor_id, operation, idempotency_key)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (context_id) DO UPDATE SET revision = EXCLUDED.revision,
         state_blob = EXCLUDED.state_blob, state_sha256 = EXCLUDED.state_sha256,
         updated_at_ms = EXCLUDED.updated_at_ms, actor_id = EXCLUDED.actor_id,
         idempotency_key = EXCLUDED.idempotency_key
         WHERE perturbation_states.revision = $11",
    ).bind(&command.context_id).bind(receipt.revision as i64)
        .bind(&space).bind(Sha256::digest(&space).to_vec())
        .bind(&state).bind(Sha256::digest(&state).to_vec()).bind(accepted_ms)
        .bind(actor).bind(OPERATION).bind(key).bind(prepared.revision as i64)
        .execute(&mut **transaction).await?;
    if changed.rows_affected() != 1 {
        return Err(StoreError::RevisionConflict);
    }
    let request = encode_message(&command.normalized())?;
    let response = encode_message(receipt)?;
    sqlx::query(
        "INSERT INTO command_receipts (actor_id, operation, idempotency_key, request_id, job_id,
         request_blob, request_sha256, response_blob, response_sha256, committed_at_ms)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(actor)
    .bind(OPERATION)
    .bind(key)
    .bind(&receipt.request_id)
    .bind(job)
    .bind(&request)
    .bind(Sha256::digest(&request).to_vec())
    .bind(&response)
    .bind(Sha256::digest(&response).to_vec())
    .bind(accepted_ms)
    .execute(&mut **transaction)
    .await?;
    sqlx::query("UPDATE store_metadata SET last_observed_at_ms = $1 WHERE singleton = 1")
        .bind(accepted_ms)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}
