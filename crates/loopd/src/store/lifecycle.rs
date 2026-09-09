use loop_core::audit::{AuditAction, AuditTarget, AuditTargetKind, state_transition_payload};
use loop_protocol::job::validate_job_record;
use loop_protocol::wire::jobs::v1::{
    AcquireJobLeaseRequest, CancelJobRequest, CompleteJobRequest, HeartbeatJobLeaseRequest,
};
use loop_protocol::wire::v1::{
    Actor, BudgetExhaustion, CommandContext, ErrorCategory, InfrastructureFailure, JobCancellation,
    JobId, JobLease, JobOutcome, JobRecord, JobState, LeaseId, ServiceError, job_outcome,
};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};

use super::postgres::{
    budget, deadline_millis, decode_record, encode_message, record_from_row, timestamp,
    timestamp_millis, verified_blob,
};
use super::{CommandResult, PgJobStore, StoreError, StoreResult, audit, backtest, validate_id};

/// Scheduler-only request to terminalize an expired lease or absolute budget.
/// This is an internal storage envelope, not a remotely exposed RPC.
#[derive(Clone, PartialEq, Message)]
pub struct RecoveryCommand {
    /// Authenticated scheduler command metadata.
    #[prost(message, optional, tag = "1")]
    pub context: Option<CommandContext>,
    /// Job observed by a bounded expired-job scan.
    #[prost(message, optional, tag = "2")]
    pub job_id: Option<JobId>,
    /// Revision observed by the scan; competing recovery is fenced.
    #[prost(uint64, tag = "3")]
    pub expected_revision: u64,
}

/// Supported atomic job lifecycle operations, independent of a storage backend.
#[derive(Clone, Debug, PartialEq)]
pub enum JobMutation {
    /// Claim a queued job and increment its execution attempt.
    Acquire(AcquireJobLeaseRequest),
    /// Acknowledge execution and extend a live lease, capped at the job deadline.
    Heartbeat(HeartbeatJobLeaseRequest),
    /// Commit a typed outcome while holding the current, unexpired lease.
    Complete(CompleteJobRequest),
    /// Administrative cancellation, including a job that never acquired a lease.
    Cancel(CancelJobRequest),
    /// Mark abandoned work failed without replaying external effects.
    Recover(RecoveryCommand),
}

impl JobMutation {
    /// Stable name used for authorization, receipt scope, and audit reasons.
    pub fn operation(&self) -> &'static str {
        match self {
            Self::Acquire(_) => "loop.jobs.acquire",
            Self::Heartbeat(_) => "loop.jobs.heartbeat",
            Self::Complete(_) => "loop.jobs.complete",
            Self::Cancel(_) => "loop.jobs.cancel",
            Self::Recover(_) => "loop.jobs.recover",
        }
    }

    fn fields(&self) -> (&Option<CommandContext>, &Option<JobId>, u64) {
        match self {
            Self::Acquire(value) => (&value.context, &value.job_id, value.expected_revision),
            Self::Heartbeat(value) => (&value.context, &value.job_id, value.expected_revision),
            Self::Complete(value) => (&value.context, &value.job_id, value.expected_revision),
            Self::Cancel(value) => (&value.context, &value.job_id, value.expected_revision),
            Self::Recover(value) => (&value.context, &value.job_id, value.expected_revision),
        }
    }

    fn normalized(&self) -> Self {
        let mut command = self.clone();
        let context = match &mut command {
            Self::Acquire(value) => &mut value.context,
            Self::Heartbeat(value) => &mut value.context,
            Self::Complete(value) => &mut value.context,
            Self::Cancel(value) => &mut value.context,
            Self::Recover(value) => &mut value.context,
        };
        if let Some(context) = context {
            context.request_id = None;
            context.requested_at = None;
        }
        command
    }

    fn encode(&self) -> StoreResult<Vec<u8>> {
        match self {
            Self::Acquire(value) => encode_message(value),
            Self::Heartbeat(value) => encode_message(value),
            Self::Complete(value) => encode_message(value),
            Self::Cancel(value) => encode_message(value),
            Self::Recover(value) => encode_message(value),
        }
    }

    fn decode_like(&self, bytes: &[u8]) -> StoreResult<Self> {
        let decoded = match self {
            Self::Acquire(_) => AcquireJobLeaseRequest::decode(bytes).map(Self::Acquire),
            Self::Heartbeat(_) => HeartbeatJobLeaseRequest::decode(bytes).map(Self::Heartbeat),
            Self::Complete(_) => CompleteJobRequest::decode(bytes).map(Self::Complete),
            Self::Cancel(_) => CancelJobRequest::decode(bytes).map(Self::Cancel),
            Self::Recover(_) => RecoveryCommand::decode(bytes).map(Self::Recover),
        };
        decoded.map_err(|_| StoreError::Corrupt("lifecycle receipt request"))
    }
}

pub(super) fn validate_context<'a>(
    context: Option<&'a CommandContext>,
    principal: &Actor,
) -> StoreResult<&'a CommandContext> {
    let context = context.ok_or(StoreError::Invalid("command context"))?;
    if context.actor.as_ref() != Some(principal) || principal.authenticated_subject.is_empty() {
        return Err(StoreError::AdmissionDenied);
    }
    for id in [
        context
            .request_id
            .as_ref()
            .map(|value| value.value.as_str()),
        context
            .correlation_id
            .as_ref()
            .map(|value| value.value.as_str()),
        context
            .causation_id
            .as_ref()
            .map(|value| value.value.as_str()),
        context
            .idempotency_key
            .as_ref()
            .map(|value| value.value.as_str()),
        principal
            .actor_id
            .as_ref()
            .map(|value| value.value.as_str()),
    ] {
        validate_id(id.ok_or(StoreError::Invalid("command identity"))?)?;
    }
    timestamp_millis(
        context
            .requested_at
            .as_ref()
            .ok_or(StoreError::Invalid("command time"))?,
        true,
    )?;
    Ok(context)
}

pub(super) async fn mutate(
    store: &PgJobStore,
    principal: &Actor,
    command: JobMutation,
) -> StoreResult<CommandResult> {
    let (context, job_id, expected_revision) = command.fields();
    let context = validate_context(context.as_ref(), principal)?;
    let job_id = &job_id.as_ref().ok_or(StoreError::Invalid("job id"))?.value;
    validate_id(job_id)?;
    if expected_revision == 0 || expected_revision >= i64::MAX as u64 {
        return Err(StoreError::Invalid("expected revision"));
    }
    let actor_id = &principal.actor_id.as_ref().expect("validated actor").value;
    let key = &context
        .idempotency_key
        .as_ref()
        .expect("validated key")
        .value;
    let operation = command.operation();
    let normalized = command.normalized();
    let request_blob = normalized.encode()?;
    let mut transaction = store.pool.begin().await?;
    let now = store.observe_clock(&mut transaction).await?;
    if timestamp_millis(context.requested_at.as_ref().expect("validated time"), true)? > now {
        return Err(StoreError::Invalid("future command time"));
    }
    let row = sqlx::query("SELECT * FROM jobs WHERE job_id = $1")
        .bind(job_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StoreError::NotFound)?;
    let mut record = record_from_row(&row)?;
    store
        .admission
        .authorize_job_command(operation, principal, &record)?;
    if matches!(command, JobMutation::Acquire(_)) {
        store.admission.validate_submission(
            record
                .specification
                .as_ref()
                .expect("validated specification"),
        )?;
    }

    let receipt = sqlx::query(
        "SELECT * FROM command_receipts
         WHERE actor_id = $1 AND operation = $2 AND idempotency_key = $3",
    )
    .bind(actor_id)
    .bind(operation)
    .bind(key)
    .fetch_optional(&mut *transaction)
    .await?;
    if let Some(receipt) = receipt {
        let original = verified_blob(&receipt, "request_blob", "request_sha256")?;
        if normalized.decode_like(&original)? != normalized {
            return Err(StoreError::IdempotencyConflict);
        }
        let job = decode_record(&verified_blob(
            &receipt,
            "response_blob",
            "response_sha256",
        )?)?;
        if job.specification != record.specification || job.revision != expected_revision + 1 {
            return Err(StoreError::Corrupt("lifecycle receipt binding"));
        }
        backtest::verify_stored(&mut transaction, &job).await?;
        transaction.commit().await?;
        return Ok(CommandResult {
            job,
            replayed: true,
        });
    }
    if record.revision != expected_revision {
        return Err(StoreError::RevisionConflict);
    }
    let previous_state = state_name(record.state)?;
    apply(&mut record, &command, principal, now)?;
    record.revision += 1;
    record.updated_at = Some(timestamp(now));
    validate_job_record(&record)?;
    write_record(&mut transaction, &record, expected_revision).await?;
    backtest::record_completion(store, &mut transaction, &record, now).await?;
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
            action: AuditAction::StateTransitioned,
            target: AuditTarget {
                kind: AuditTargetKind::JobId,
                value: job_id.clone(),
            },
            payload: state_transition_payload(
                previous_state,
                state_name(record.state)?,
                operation,
            )?,
        },
    )
    .await?;
    save_receipt(
        &mut transaction,
        context,
        operation,
        job_id,
        &request_blob,
        &record,
        now,
    )
    .await?;
    #[cfg(test)]
    if backtest::is_completed_backtest(&record)? {
        super::crash_tests::fault_point("result_before_commit").await;
    }
    transaction.commit().await?;
    #[cfg(test)]
    if backtest::is_completed_backtest(&record)? {
        super::crash_tests::fault_point("result_after_commit").await;
    }
    Ok(CommandResult {
        job: record,
        replayed: false,
    })
}

fn apply(
    record: &mut JobRecord,
    command: &JobMutation,
    principal: &Actor,
    now: i64,
) -> StoreResult<()> {
    let state = JobState::try_from(record.state).map_err(|_| StoreError::Corrupt("state"))?;
    if !matches!(
        state,
        JobState::Queued | JobState::Leased | JobState::Running
    ) {
        return Err(StoreError::InvalidTransition);
    }
    let specification = record
        .specification
        .as_ref()
        .expect("validated specification");
    let deadline = deadline_millis(specification)?;
    match command {
        JobMutation::Acquire(request) => {
            if state != JobState::Queued || now >= deadline {
                return Err(StoreError::InvalidTransition);
            }
            let expiry = lease_expiry(now, request.requested_duration.as_ref(), deadline)?;
            record.attempt = record
                .attempt
                .checked_add(1)
                .ok_or(StoreError::Invalid("attempt overflow"))?;
            record.state = JobState::Leased as i32;
            record.active_lease = Some(JobLease {
                lease_id: Some(LeaseId {
                    value: format!("lease.{}", uuid::Uuid::new_v4().simple()),
                }),
                job_id: specification.job_id.clone(),
                owner: Some(principal.clone()),
                acquired_revision: record.revision + 1,
                issued_at: Some(timestamp(now)),
                heartbeat_at: Some(timestamp(now)),
                expires_at: Some(timestamp(expiry)),
            });
        }
        JobMutation::Heartbeat(request) => {
            fence(record, request.lease_id.as_ref(), principal, now)?;
            let expiry = lease_expiry(now, request.requested_extension.as_ref(), deadline)?;
            let lease = record.active_lease.as_mut().expect("fenced lease");
            let old_expiry =
                timestamp_millis(lease.expires_at.as_ref().expect("validated expiry"), false)?;
            lease.expires_at = Some(timestamp(expiry.max(old_expiry)));
            lease.heartbeat_at = Some(timestamp(now));
            record.state = JobState::Running as i32;
        }
        JobMutation::Complete(request) => {
            fence(record, request.lease_id.as_ref(), principal, now)?;
            let outcome = request
                .outcome
                .clone()
                .ok_or(StoreError::Invalid("outcome"))?;
            record.state = match outcome.outcome.as_ref() {
                Some(job_outcome::Outcome::Success(_)) => JobState::Succeeded,
                Some(job_outcome::Outcome::FactorRejection(_)) => JobState::FactorRejected,
                Some(job_outcome::Outcome::InfrastructureFailure(_)) => {
                    JobState::InfrastructureFailed
                }
                Some(job_outcome::Outcome::BudgetExhaustion(_)) => JobState::BudgetExhausted,
                // Cancellation attribution is server-owned, via Cancel.
                _ => return Err(StoreError::Invalid("completion outcome")),
            } as i32;
            record.outcome = Some(outcome);
            record.active_lease = None;
        }
        JobMutation::Cancel(request) => {
            if request.reason.trim().is_empty() || request.reason.len() > 4_096 {
                return Err(StoreError::Invalid("cancellation reason"));
            }
            record.state = JobState::Cancelled as i32;
            record.active_lease = None;
            record.outcome = Some(JobOutcome {
                outcome: Some(job_outcome::Outcome::Cancellation(JobCancellation {
                    reason: request.reason.clone(),
                    cancelled_by: Some(principal.clone()),
                    cancelled_at: Some(timestamp(now)),
                })),
            });
        }
        JobMutation::Recover(_) => {
            let expired_lease = record
                .active_lease
                .as_ref()
                .and_then(|lease| lease.expires_at.as_ref())
                .map(|expiry| timestamp_millis(expiry, false))
                .transpose()?
                .is_some_and(|expiry| now >= expiry);
            if now < deadline && !expired_lease {
                return Err(StoreError::InvalidTransition);
            }
            let outcome = if now >= deadline {
                record.state = JobState::BudgetExhausted as i32;
                job_outcome::Outcome::BudgetExhaustion(BudgetExhaustion {
                    exhausted_limit: "maximum_wall_time".to_owned(),
                    enforced_budget: Some(budget(specification)?.clone()),
                    exhausted_at: Some(timestamp(now)),
                })
            } else {
                record.state = JobState::InfrastructureFailed as i32;
                job_outcome::Outcome::InfrastructureFailure(InfrastructureFailure {
                    error: Some(ServiceError {
                        category: ErrorCategory::Timeout as i32, code: "lease_expired".to_owned(),
                        message: "Worker lease expired; external effects require reconciliation before retry".to_owned(),
                        retryable: false, details: vec![],
                    }),
                    attempt: record.attempt, failed_at: Some(timestamp(now)),
                })
            };
            record.active_lease = None;
            record.outcome = Some(JobOutcome {
                outcome: Some(outcome),
            });
        }
    }
    Ok(())
}

fn lease_expiry(
    now: i64,
    duration: Option<&prost_types::Duration>,
    deadline: i64,
) -> StoreResult<i64> {
    let duration = duration.ok_or(StoreError::Invalid("lease duration"))?;
    if duration.seconds < 0 || !(0..1_000_000_000).contains(&duration.nanos) {
        return Err(StoreError::Invalid("lease duration"));
    }
    let millis = duration
        .seconds
        .checked_mul(1_000)
        .and_then(|value| value.checked_add(i64::from(duration.nanos) / 1_000_000))
        .ok_or(StoreError::Invalid("lease duration"))?;
    if !(1..=300_000).contains(&millis) {
        return Err(StoreError::Invalid(
            "lease duration must be 1ms through 5min",
        ));
    }
    Ok(now
        .checked_add(millis)
        .ok_or(StoreError::Invalid("lease expiry"))?
        .min(deadline))
}

fn fence(
    record: &JobRecord,
    lease_id: Option<&LeaseId>,
    actor: &Actor,
    now: i64,
) -> StoreResult<()> {
    let lease = record
        .active_lease
        .as_ref()
        .ok_or(StoreError::LeaseFenced)?;
    if lease_id.is_none()
        || lease.lease_id.as_ref() != lease_id
        || lease.owner.as_ref() != Some(actor)
        || now
            >= timestamp_millis(
                lease.expires_at.as_ref().expect("validated lease expiry"),
                false,
            )?
    {
        return Err(StoreError::LeaseFenced);
    }
    Ok(())
}

pub(super) async fn write_record(
    transaction: &mut Transaction<'_, Postgres>,
    record: &JobRecord,
    previous: u64,
) -> StoreResult<()> {
    let blob = encode_message(record)?;
    let lease = record.active_lease.as_ref();
    let result = sqlx::query(
        "UPDATE jobs SET state = $1, revision = $2, attempt = $3, updated_at_ms = $4,
            lease_id = $5, lease_owner_id = $6, lease_expires_at_ms = $7, record_blob = $8, record_sha256 = $9
         WHERE job_id = $10 AND revision = $11",
    ).bind(record.state).bind(record.revision as i64).bind(i64::from(record.attempt))
        .bind(timestamp_millis(record.updated_at.as_ref().expect("validated time"), false)?)
        .bind(lease.and_then(|value| value.lease_id.as_ref()).map(|id| &id.value))
        .bind(lease.and_then(|value| value.owner.as_ref()).and_then(|actor| actor.actor_id.as_ref()).map(|id| &id.value))
        .bind(lease.and_then(|value| value.expires_at.as_ref()).map(|time| timestamp_millis(time, false)).transpose()?)
        .bind(&blob).bind(Sha256::digest(&blob).to_vec())
        .bind(&record.specification.as_ref().expect("validated spec").job_id.as_ref().expect("validated id").value)
        .bind(previous as i64).execute(&mut **transaction).await?;
    if result.rows_affected() != 1 {
        return Err(StoreError::RevisionConflict);
    }
    Ok(())
}

pub(super) async fn save_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    context: &CommandContext,
    operation: &str,
    job_id: &str,
    request: &[u8],
    record: &JobRecord,
    now: i64,
) -> StoreResult<()> {
    let response = encode_message(record)?;
    sqlx::query(
        "INSERT INTO command_receipts (actor_id, operation, idempotency_key, request_id, job_id,
            request_blob, request_sha256, response_blob, response_sha256, committed_at_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(
        &context
            .actor
            .as_ref()
            .expect("validated actor")
            .actor_id
            .as_ref()
            .expect("validated actor id")
            .value,
    )
    .bind(operation)
    .bind(
        &context
            .idempotency_key
            .as_ref()
            .expect("validated key")
            .value,
    )
    .bind(
        &context
            .request_id
            .as_ref()
            .expect("validated request id")
            .value,
    )
    .bind(job_id)
    .bind(request)
    .bind(Sha256::digest(request).to_vec())
    .bind(&response)
    .bind(Sha256::digest(&response).to_vec())
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    sqlx::query("UPDATE store_metadata SET last_observed_at_ms = $1 WHERE singleton = 1")
        .bind(now)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn state_name(state: i32) -> StoreResult<&'static str> {
    match JobState::try_from(state) {
        Ok(JobState::Queued) => Ok("queued"),
        Ok(JobState::Leased) => Ok("leased"),
        Ok(JobState::Running) => Ok("running"),
        Ok(JobState::Succeeded) => Ok("succeeded"),
        Ok(JobState::FactorRejected) => Ok("factor_rejected"),
        Ok(JobState::InfrastructureFailed) => Ok("infrastructure_failed"),
        Ok(JobState::Cancelled) => Ok("cancelled"),
        Ok(JobState::BudgetExhausted) => Ok("budget_exhausted"),
        _ => Err(StoreError::Corrupt("job state")),
    }
}
