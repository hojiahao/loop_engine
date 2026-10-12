use loop_protocol::wire::provider::v1::InvokeModelRequest;
use loop_protocol::wire::v1::{
    Actor, BudgetExhaustion, ErrorCategory, InfrastructureFailure, JobCancellation, JobOutcome,
    JobRecord, JobState, ModelResponse, ServiceError, job_outcome,
};
use prost::Message;

use super::super::{PgJobStore, StoreError, StoreResult, postgres};
use super::{ModelStep, ModelStepCommand, ModelStepState, ReceiptRequest, storage, validation};

/// Administrative commands that only reduce Discovery execution authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ModelControl {
    /// Remove the current lease without releasing reservations or evidence.
    Pause,
    /// Irrevocably cancel execution while preserving possible paid effects.
    Cancel,
    /// Terminalize only after the original absolute deadline has elapsed.
    Expire,
}

/// Safe operations whose attempts are bounded across crashes and resumptions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ModelRetry {
    /// Read the original Provider receipt; never resend a paid invocation.
    Lookup,
    /// Verify the registered read-only tool before its first result commit.
    Tool,
}

/// A resumed step and a process-local right to continue its existing workflow.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModelResume {
    /// Current job, including an expired paused job that never reserved a step.
    pub job: JobRecord,
    /// Current verified immutable step and newly observed job state.
    pub step: Option<ModelStep>,
    /// True only for the newly committed claimant, never a receipt replay.
    pub execute: bool,
}

impl ModelControl {
    fn operation(self) -> &'static str {
        match self {
            Self::Pause => "loop.discovery.pause",
            Self::Cancel => "loop.discovery.cancel",
            Self::Expire => "loop.discovery.expire",
        }
    }
}

impl PgJobStore {
    /// Pause, cancel or expire a valid Discovery job without reading plan or
    /// model evidence. Authority comes from the authenticated control policy;
    /// no lease is accepted. CAS, receipt and audit commit together. Replay
    /// returns the current job, and cancellation rolls back uncommitted work.
    pub(crate) async fn control_model(
        &self,
        actor: &Actor,
        command: ModelStepCommand,
        control: ModelControl,
    ) -> StoreResult<JobRecord> {
        if command.lease_id.is_some() || command.ordinal != 0 {
            return Err(StoreError::Invalid("model control envelope"));
        }
        let operation = control.operation();
        let receipt = ReceiptRequest {
            command: Some(command.clone()),
            ..Default::default()
        };
        let (mut transaction, now, mut job) =
            storage::begin(self, actor, &command, operation).await?;
        if storage::replay(&mut transaction, &receipt, operation, &job).await? {
            transaction.commit().await?;
            return Ok(job);
        }
        storage::revision(&job, &command)?;
        if !matches!(
            JobState::try_from(job.state),
            Ok(JobState::Queued | JobState::Leased | JobState::Running | JobState::Paused)
        ) {
            return Err(StoreError::InvalidTransition);
        }
        let expired = now >= storage::deadline(&job)?;
        if control == ModelControl::Expire && !expired {
            return Err(StoreError::InvalidTransition);
        }
        let original = job.clone();
        job.active_lease = None;
        if control != ModelControl::Cancel && expired {
            exhaust(&mut job, now)?;
        } else if control == ModelControl::Cancel {
            job.state = JobState::Cancelled as i32;
            job.outcome = Some(JobOutcome {
                outcome: Some(job_outcome::Outcome::Cancellation(JobCancellation {
                    reason: "Discovery execution cancelled".into(),
                    cancelled_by: Some(actor.clone()),
                    cancelled_at: Some(postgres::timestamp(now)),
                })),
            });
        } else {
            job.state = JobState::Paused as i32;
            job.outcome = None;
        }
        storage::advance(&mut job, now)?;
        storage::writer(&mut transaction).await?;
        storage::commit(
            self,
            transaction,
            actor,
            storage::Mutation {
                request: receipt,
                operation,
                original,
                job: job.clone(),
                now,
            },
        )
        .await?;
        Ok(job)
    }

    /// Resume a Paused job before its original deadline, or atomically expire it
    /// under this same command identity. A derived initial invocation is required
    /// only for unreserved, unexpired work and never changes replay identity.
    /// Existing evidence and retry counts stay unchanged. Only the new claimant
    /// grants local execution; a cancelled transaction grants no execution right.
    pub(crate) async fn resume_model(
        &self,
        actor: &Actor,
        command: ModelStepCommand,
        initial: Option<InvokeModelRequest>,
    ) -> StoreResult<ModelResume> {
        if command.lease_id.is_some() || command.ordinal != 0 {
            return Err(StoreError::Invalid("model resume envelope"));
        }
        let operation = "loop.model.resume";
        let receipt = ReceiptRequest {
            command: Some(command.clone()),
            ..Default::default()
        };
        let (mut transaction, now, mut job) =
            storage::begin(self, actor, &command, operation).await?;
        if storage::replay(&mut transaction, &receipt, operation, &job).await? {
            let step = storage::history(&mut transaction, &job).await?.pop();
            transaction.commit().await?;
            return Ok(ModelResume {
                job,
                step,
                execute: false,
            });
        }
        storage::revision(&job, &command)?;
        if job.state != JobState::Paused as i32 {
            return Err(StoreError::InvalidTransition);
        }
        if now >= storage::deadline(&job)? {
            let original = job.clone();
            exhaust(&mut job, now)?;
            storage::advance(&mut job, now)?;
            storage::writer(&mut transaction).await?;
            storage::commit(
                self,
                transaction,
                actor,
                storage::Mutation {
                    request: receipt,
                    operation,
                    original,
                    job: job.clone(),
                    now,
                },
            )
            .await?;
            return Ok(ModelResume {
                job,
                step: None,
                execute: false,
            });
        }
        self.admission.validate_submission(
            job.specification
                .as_ref()
                .ok_or(StoreError::Corrupt("model specification"))?,
        )?;
        let mut history = storage::history(&mut transaction, &job).await?;
        let original = job.clone();
        let mut step = if let Some(step) = history.pop() {
            if initial.is_some() {
                return Err(StoreError::Invalid("model resume history"));
            }
            let expiry = now
                .checked_add(120_000)
                .ok_or(StoreError::Invalid("model resume expiry"))?
                .min(storage::deadline(&job)?);
            storage::lease(&mut job, actor, now, expiry)?;
            storage::advance(&mut job, now)?;
            step
        } else {
            if command.ordinal != 0 {
                return Err(StoreError::Invalid("model resume ordinal"));
            }
            let request = initial.ok_or(StoreError::Invalid("model resume invocation"))?;
            let (input, output, cost, wall) = validation::invocation(&job, actor, &request, 0)?;
            let requested = postgres::timestamp_millis(
                request
                    .context
                    .as_ref()
                    .and_then(|context| context.requested_at.as_ref())
                    .ok_or(StoreError::Invalid("model requested time"))?,
                true,
            )?;
            if requested > now || now - requested >= 30_000 {
                return Err(StoreError::Unavailable("model request deadline"));
            }
            let expiry = now
                .checked_add(wall + 5_000)
                .ok_or(StoreError::Invalid("model resume expiry"))?;
            if expiry > storage::deadline(&job)? {
                return Err(StoreError::Invalid("model invocation exceeds deadline"));
            }
            let digest = crate::runtime::model_codec::request_digest(&request)
                .map_err(|_| StoreError::Invalid("model invocation digest"))?;
            let bytes = postgres::encode_message(&request)?;
            if bytes.len() > 1_048_576 {
                return Err(StoreError::Invalid("model invocation size"));
            }
            storage::lease(&mut job, actor, now, expiry)?;
            storage::advance(&mut job, now)?;
            let step = ModelStep {
                ordinal: 0,
                job: job.clone(),
                request,
                request_sha256: digest,
                state: ModelStepState::Reserved,
                response: None,
                reserved_input: input,
                reserved_output: output,
                reserved_nano_usd: cost,
                tool_result: None,
                lookup_attempts: 0,
                tool_attempts: 0,
                retry_after_ms: 0,
            };
            validation::totals(&job, std::slice::from_ref(&step))?;
            storage::writer(&mut transaction).await?;
            storage::insert(&mut transaction, &step, &bytes, now).await?;
            step
        };
        step.job = job.clone();
        storage::writer(&mut transaction).await?;
        storage::commit(
            self,
            transaction,
            actor,
            storage::Mutation {
                request: receipt,
                operation,
                original,
                job,
                now,
            },
        )
        .await?;
        Ok(ModelResume {
            job: step.job.clone(),
            step: Some(step),
            execute: true,
        })
    }

    /// Consume one safe lookup/tool attempt before performing it. At most three
    /// attempts of each kind survive restarts, with a 250 ms absolute backoff.
    /// Existing responses/results, stale leases, early or excess retries deny.
    /// A receipt replay is rejected rather than granting another attempt.
    /// Cancellation before commit consumes no attempt.
    pub(crate) async fn retry_model(
        &self,
        actor: &Actor,
        command: ModelStepCommand,
        kind: ModelRetry,
    ) -> StoreResult<ModelStep> {
        let operation = "loop.model.retry";
        let receipt = ReceiptRequest {
            command: Some(command.clone()),
            retry_kind: match kind {
                ModelRetry::Lookup => 1,
                ModelRetry::Tool => 2,
            },
            ..Default::default()
        };
        let (mut transaction, now, mut job) =
            storage::begin(self, actor, &command, operation).await?;
        if storage::replay(&mut transaction, &receipt, operation, &job).await? {
            return Err(StoreError::Invalid("model retry replay"));
        }
        storage::revision(&job, &command)?;
        storage::fence(&job, actor, &command, now)?;
        let mut step = storage::current(&mut transaction, &job, command.ordinal).await?;
        let attempts = match kind {
            ModelRetry::Lookup => {
                if !matches!(
                    step.state,
                    ModelStepState::Dispatched | ModelStepState::Ambiguous
                ) {
                    return Err(StoreError::InvalidTransition);
                }
                &mut step.lookup_attempts
            }
            ModelRetry::Tool => {
                if step.state != ModelStepState::Completed
                    || step.ordinal != 0
                    || step.tool_result.is_some()
                {
                    return Err(StoreError::InvalidTransition);
                }
                validation::call(
                    &step,
                    step.response
                        .as_ref()
                        .ok_or(StoreError::Corrupt("tool response"))?,
                )?;
                &mut step.tool_attempts
            }
        };
        if *attempts >= 3 {
            return Err(StoreError::Invalid("model retries exhausted"));
        }
        if now < step.retry_after_ms {
            return Err(StoreError::Invalid("model retry backoff"));
        }
        *attempts += 1;
        step.retry_after_ms = now
            .checked_add(250)
            .ok_or(StoreError::Invalid("model retry time"))?;
        let original = job.clone();
        storage::advance(&mut job, now)?;
        step.job = job.clone();
        storage::writer(&mut transaction).await?;
        let changed = sqlx::query("UPDATE model_steps SET lookup_attempts=$1,tool_attempts=$2,retry_after_ms=$3,updated_revision=$4,updated_at_ms=$5 WHERE job_id=$6 AND ordinal=$7")
            .bind(step.lookup_attempts as i32).bind(step.tool_attempts as i32)
            .bind(step.retry_after_ms).bind(job.revision as i64).bind(now)
            .bind(command.job_id.as_ref().ok_or(StoreError::Invalid("model job ID"))?.value.as_str())
            .bind(command.ordinal as i32).execute(&mut *transaction).await?;
        if changed.rows_affected() != 1 {
            return Err(StoreError::Corrupt("model retry disappeared"));
        }
        storage::commit(
            self,
            transaction,
            actor,
            storage::Mutation {
                request: receipt,
                operation,
                original,
                job,
                now,
            },
        )
        .await?;
        Ok(step)
    }

    /// Terminalize a fenced running attempt using only static, allowlisted
    /// infrastructure labels. No step or reservation is removed. An elapsed job
    /// deadline takes priority over failure; the original lease identity must
    /// still match even when that deadline has expired it. Replay is read-only.
    pub(crate) async fn fail_model(
        &self,
        actor: &Actor,
        command: ModelStepCommand,
        code: &'static str,
    ) -> StoreResult<JobRecord> {
        if !matches!(
            code,
            "provider_unavailable"
                | "model_deadline"
                | "retry_exhausted"
                | "model_candidate_invalid"
                | "tool_unavailable"
                | "tool_contract_invalid"
                | "model_contract_invalid"
        ) {
            return Err(StoreError::Invalid("model failure code"));
        }
        let operation = "loop.model.fail";
        let receipt = ReceiptRequest {
            command: Some(command.clone()),
            failure_code: code.into(),
            ..Default::default()
        };
        let (mut transaction, now, mut job) =
            storage::begin(self, actor, &command, operation).await?;
        if storage::replay(&mut transaction, &receipt, operation, &job).await? {
            transaction.commit().await?;
            return Ok(job);
        }
        storage::revision(&job, &command)?;
        fence_failure(&job, actor, &command, now)?;
        storage::current(&mut transaction, &job, command.ordinal).await?;
        let original = job.clone();
        job.active_lease = None;
        if now >= storage::deadline(&job)? {
            exhaust(&mut job, now)?;
        } else {
            job.state = JobState::InfrastructureFailed as i32;
            job.outcome = Some(JobOutcome {
                outcome: Some(job_outcome::Outcome::InfrastructureFailure(InfrastructureFailure {
                    error: Some(ServiceError {
                        category: if code == "model_deadline" { ErrorCategory::Timeout } else { ErrorCategory::Dependency } as i32,
                        code: code.into(),
                        message: "Discovery execution stopped; invocation evidence and reservations are retained".into(),
                        retryable: false,
                        details: vec![],
                    }),
                    attempt: job.attempt,
                    failed_at: Some(postgres::timestamp(now)),
                })),
            });
        }
        storage::advance(&mut job, now)?;
        storage::writer(&mut transaction).await?;
        storage::commit(
            self,
            transaction,
            actor,
            storage::Mutation {
                request: receipt,
                operation,
                original,
                job: job.clone(),
                now,
            },
        )
        .await?;
        Ok(job)
    }

    /// Read a committed Reconcile receipt under current owner and operation
    /// authority, without reading model history or granting execution. The
    /// caller's ordinal is ignored; the original receipt binds its ordinal and
    /// response, while actor, key, job and expected revision must still match.
    /// Missing receipts return `None`; conflicting or corrupt receipts fail
    /// closed. Cancellation drops the bounded read transaction without writes.
    pub(crate) async fn reconcile_replay(
        &self,
        actor: &Actor,
        mut command: ModelStepCommand,
    ) -> StoreResult<Option<JobRecord>> {
        if command.lease_id.is_some() {
            return Err(StoreError::Invalid("model reconcile envelope"));
        }
        command.ordinal = 0;
        let operation = "loop.model.reconcile";
        let (mut transaction, _, job) = storage::begin(self, actor, &command, operation).await?;
        let context = command
            .context
            .as_ref()
            .ok_or(StoreError::Invalid("model context"))?;
        let actor_id = context
            .actor
            .as_ref()
            .and_then(|actor| actor.actor_id.as_ref())
            .ok_or(StoreError::Invalid("model actor ID"))?;
        let key = context
            .idempotency_key
            .as_ref()
            .ok_or(StoreError::Invalid("model key"))?;
        let row = sqlx::query(
            "SELECT request_blob,request_sha256 FROM command_receipts WHERE actor_id=$1 AND operation=$2 AND idempotency_key=$3",
        )
        .bind(&actor_id.value)
        .bind(operation)
        .bind(&key.value)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            transaction.commit().await?;
            return Ok(None);
        };
        let old = ReceiptRequest::decode(
            postgres::verified_blob(&row, "request_blob", "request_sha256")?.as_slice(),
        )
        .map_err(|_| StoreError::Corrupt("model request receipt"))?;
        command.ordinal = old
            .command
            .as_ref()
            .filter(|command| command.ordinal <= 1)
            .ok_or(StoreError::Corrupt("model reconcile receipt"))?
            .ordinal;
        let receipt = ReceiptRequest {
            command: Some(command),
            response: Some(
                old.response
                    .ok_or(StoreError::Corrupt("model reconcile response"))?,
            ),
            ..Default::default()
        };
        if !storage::replay(&mut transaction, &receipt, operation, &job).await? {
            return Err(StoreError::Corrupt("model reconcile disappeared"));
        }
        transaction.commit().await?;
        Ok(Some(job))
    }

    /// Append a verified late Provider response without execution authority.
    /// Only paused/cancelled/expired/failed jobs accept a current-ordinal CAS;
    /// the outcome, reservation and all retry counts remain unchanged. No lease
    /// or tool execution is allowed. Conflicts, corruption and failed binding
    /// roll back with their receipt/audit; replay is observational.
    pub(crate) async fn reconcile_model(
        &self,
        actor: &Actor,
        command: ModelStepCommand,
        response: ModelResponse,
    ) -> StoreResult<ModelStep> {
        if command.lease_id.is_some() {
            return Err(StoreError::Invalid("model reconcile envelope"));
        }
        let operation = "loop.model.reconcile";
        let receipt = ReceiptRequest {
            command: Some(command.clone()),
            response: Some(response.clone()),
            ..Default::default()
        };
        let (mut transaction, now, mut job) =
            storage::begin(self, actor, &command, operation).await?;
        if storage::replay(&mut transaction, &receipt, operation, &job).await? {
            return storage::finish_read(transaction, job, command.ordinal).await;
        }
        storage::revision(&job, &command)?;
        if !matches!(
            JobState::try_from(job.state),
            Ok(JobState::Paused
                | JobState::Cancelled
                | JobState::BudgetExhausted
                | JobState::InfrastructureFailed)
        ) {
            return Err(StoreError::InvalidTransition);
        }
        let mut step = storage::current(&mut transaction, &job, command.ordinal).await?;
        if !matches!(
            step.state,
            ModelStepState::Dispatched | ModelStepState::Ambiguous
        ) {
            return Err(StoreError::InvalidTransition);
        }
        validation::response(&step, &response)?;
        let original = job.clone();
        storage::advance(&mut job, now)?;
        storage::writer(&mut transaction).await?;
        storage::transition(
            &mut transaction,
            &job,
            "completed",
            Some(&response),
            now,
            command.ordinal,
        )
        .await?;
        storage::commit(
            self,
            transaction,
            actor,
            storage::Mutation {
                request: receipt,
                operation,
                original,
                job: job.clone(),
                now,
            },
        )
        .await?;
        step.job = job;
        step.response = Some(response);
        step.state = ModelStepState::Completed;
        Ok(step)
    }
}

pub(super) fn fence_failure(
    job: &JobRecord,
    actor: &Actor,
    command: &ModelStepCommand,
    now: i64,
) -> StoreResult<()> {
    let lease = job.active_lease.as_ref().ok_or(StoreError::LeaseFenced)?;
    if job.state != JobState::Running as i32
        || command.lease_id.is_none()
        || lease.lease_id != command.lease_id
        || lease.owner.as_ref() != Some(actor)
    {
        return Err(StoreError::LeaseFenced);
    }
    if now < storage::deadline(job)? {
        storage::fence(job, actor, command, now)?;
    }
    Ok(())
}

fn exhaust(job: &mut JobRecord, now: i64) -> StoreResult<()> {
    let specification = job
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("model specification"))?;
    let budget = postgres::budget(specification)?.clone();
    job.state = JobState::BudgetExhausted as i32;
    job.active_lease = None;
    job.outcome = Some(JobOutcome {
        outcome: Some(job_outcome::Outcome::BudgetExhaustion(BudgetExhaustion {
            exhausted_limit: "maximum_wall_time".into(),
            enforced_budget: Some(budget),
            exhausted_at: Some(postgres::timestamp(now)),
        })),
    });
    Ok(())
}
