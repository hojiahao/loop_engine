//! Durable bounded model execution. No database transaction spans network I/O.

mod validation;
pub(crate) use validation::{duration as model_duration, money as model_money};
mod lifecycle;
mod observation;
mod storage;
pub(crate) use lifecycle::{ModelControl, ModelRetry};
#[cfg(test)]
mod tests;

use super::{PgJobStore, StoreError, StoreResult};
use loop_protocol::wire::provider::v1::InvokeModelRequest;
use loop_protocol::wire::v1::ToolResultContent;
use loop_protocol::wire::v1::{Actor, CommandContext, JobId, JobRecord, LeaseId, ModelResponse};
use loop_protocol::wire::v1::{JobOutcome, JobState, job_outcome};
use prost::Message;

/// Lease-fenced command envelope for an authenticated Harness worker.
#[derive(Clone, PartialEq, Message)]
pub struct ModelStepCommand {
    /// Fresh command identity, bound to the authenticated transport principal.
    #[prost(message, optional, tag = "1")]
    pub context: Option<CommandContext>,
    /// Existing frozen Discovery job; cannot select a research or holdout job.
    #[prost(message, optional, tag = "2")]
    pub job_id: Option<JobId>,
    /// Current worker lease. Recovery never treats a stale lease as authority.
    /// Absent only for the initial reservation or an expired-lease takeover.
    #[prost(message, optional, tag = "3")]
    pub lease_id: Option<LeaseId>,
    /// Compare-and-swap job revision observed by this command.
    #[prost(uint64, tag = "4")]
    pub expected_revision: u64,
    /// Immutable model-call ordinal. Zero preserves existing single-step receipts.
    #[prost(uint32, tag = "5")]
    pub ordinal: u32,
}

/// Persistent dispatch evidence. Only a newly committed `Dispatched` transition
/// may authorize outbound generation; reading this enum never grants that right.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelStepState {
    /// The full invocation and conservative budget are committed, not dispatched.
    Reserved,
    /// Outbound generation may have happened; recovery is lookup-only.
    Dispatched,
    /// The outcome is uncertain; the complete reservation remains held.
    Ambiguous,
    /// An immutable Provider response was committed. An intermediate tool call
    /// keeps the job running until its result and final model response complete.
    Completed,
}

/// Verified model-step evidence and the current associated job revision.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelStep {
    /// Immutable ordinal within the closed one-call or two-call job profile.
    pub ordinal: u32,
    /// Current job, which may be newer than the original step receipt.
    pub job: JobRecord,
    /// Original immutable Provider request. Timestamps and keys never change.
    pub request: InvokeModelRequest,
    /// Provider's canonical invocation identity, distinct from the byte checksum.
    pub request_sha256: [u8; 32],
    /// Conservative persistent dispatch state.
    pub state: ModelStepState,
    /// Complete response only after a committed completion.
    pub response: Option<ModelResponse>,
    /// Input-token ceiling reserved before any Provider RPC.
    pub reserved_input: u64,
    /// Output-token ceiling reserved before any Provider RPC.
    pub reserved_output: u64,
    /// Conservative USD ceiling in billionths; not an invoice or actual charge.
    pub reserved_nano_usd: u64,
    /// Verified immutable result of the sole registered intermediate tool.
    pub tool_result: Option<ToolResultContent>,
    /// Lookup attempts durably consumed before the read-only Provider request.
    pub lookup_attempts: u32,
    /// Tool attempts durably consumed before verifying the development files.
    pub tool_attempts: u32,
    /// Absolute earliest time for another safe attempt; zero before any attempt.
    pub retry_after_ms: i64,
}

impl ModelStep {
    /// Verify identity, finish semantics and usage against this immutable
    /// reservation before runtime result handling. Invalid evidence grants no
    /// retry, mutation or execution authority and never exposes raw contents.
    pub(crate) fn check_response(&self, response: &ModelResponse) -> StoreResult<()> {
        validation::response(self, response)
    }
}

/// Result of a dispatch CAS. A replay is deliberately incapable of redispatch.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelDispatch {
    /// Persisted step, including its original request for execution or lookup.
    pub step: ModelStep,
    /// True only for the caller that newly committed RESERVED -> DISPATCHED.
    /// A process must not serialize this local permission or retry after a crash.
    pub send: bool,
}

#[derive(Clone, PartialEq, Message)]
struct ReceiptRequest {
    #[prost(message, optional, tag = "1")]
    command: Option<ModelStepCommand>,
    #[prost(message, optional, tag = "2")]
    invocation: Option<InvokeModelRequest>,
    #[prost(bytes = "vec", tag = "3")]
    invocation_sha256: Vec<u8>,
    #[prost(message, optional, tag = "4")]
    response: Option<ModelResponse>,
    #[prost(message, optional, tag = "5")]
    outcome: Option<loop_protocol::wire::v1::JobOutcome>,
    #[prost(message, optional, tag = "6")]
    duration: Option<prost_types::Duration>,
    #[prost(message, optional, tag = "7")]
    tool_result: Option<ToolResultContent>,
    #[prost(string, tag = "8")]
    failure_code: String,
    #[prost(uint32, tag = "9")]
    retry_kind: u32,
}

impl ModelStepState {
    fn parse(value: &str) -> super::StoreResult<Self> {
        match value {
            "reserved" => Ok(Self::Reserved),
            "dispatched" => Ok(Self::Dispatched),
            "ambiguous" => Ok(Self::Ambiguous),
            "completed" => Ok(Self::Completed),
            _ => Err(super::StoreError::Corrupt("model step state")),
        }
    }
}

impl PgJobStore {
    /// Atomically reserve a single frozen invocation and acquire its initial
    /// lease. A replay returns evidence only; it never authorizes dispatch.
    /// Cancellation rolls back the transaction. Missing references, changed
    /// budgets, another key for this job, or a competing revision fail closed.
    pub(crate) async fn reserve_model(
        &self,
        actor: &Actor,
        command: ModelStepCommand,
        request: InvokeModelRequest,
    ) -> StoreResult<ModelStep> {
        let operation = "loop.model.reserve";
        let digest = crate::runtime::model_codec::request_digest(&request)
            .map_err(|_| StoreError::Invalid("model invocation digest"))?;
        let receipt = ReceiptRequest {
            command: Some(command.clone()),
            invocation: Some(request.clone()),
            invocation_sha256: digest.to_vec(),
            ..Default::default()
        };
        let (mut transaction, now, mut job) =
            storage::begin(self, actor, &command, operation).await?;
        if storage::replay(&mut transaction, &receipt, operation, &job).await? {
            return storage::finish_read(transaction, job, command.ordinal).await;
        }
        storage::revision(&job, &command)?;
        let mut history = storage::history(&mut transaction, &job).await?;
        if command.ordinal == 0 {
            if job.state != JobState::Queued as i32
                || command.lease_id.is_some()
                || !history.is_empty()
            {
                return Err(StoreError::InvalidTransition);
            }
        } else {
            storage::fence(&job, actor, &command, now)?;
            if job.state != JobState::Running as i32
                || history.len() != 1
                || history[0].state != ModelStepState::Completed
            {
                return Err(StoreError::InvalidTransition);
            }
            validation::continuation(&history[0], &request)?;
        }
        self.admission.validate_submission(
            job.specification
                .as_ref()
                .ok_or(StoreError::Corrupt("model specification"))?,
        )?;
        let (input, output, cost, wall) =
            validation::invocation(&job, actor, &request, command.ordinal)?;
        let requested = super::postgres::timestamp_millis(
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
        let deadline = storage::deadline(&job)?;
        let expiry = now
            .checked_add(wall + 5_000)
            .ok_or(StoreError::Invalid("model lease expiry"))?;
        if expiry > deadline {
            return Err(StoreError::Invalid("model invocation exceeds deadline"));
        }
        let original = job.clone();
        storage::lease(&mut job, actor, now, expiry)?;
        storage::advance(&mut job, now)?;
        let blob = super::postgres::encode_message(&request)?;
        if blob.len() > 1_048_576 {
            return Err(StoreError::Invalid("model invocation size"));
        }
        storage::writer(&mut transaction).await?;
        let step = ModelStep {
            ordinal: command.ordinal,
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
        history.push(step.clone());
        validation::totals(&job, &history)?;
        storage::insert(&mut transaction, &step, &blob, now).await?;
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

    /// Commit dispatch intent before network I/O. Only this newly committed CAS
    /// returns `send = true`; all replays and recovery observations are lookup-only.
    /// The full reservation is retained on cancellation, timeout, or process death.
    pub(crate) async fn dispatch_model(
        &self,
        actor: &Actor,
        command: ModelStepCommand,
    ) -> StoreResult<ModelDispatch> {
        let operation = "loop.model.dispatch";
        let receipt = ReceiptRequest {
            command: Some(command.clone()),
            ..Default::default()
        };
        let (mut transaction, now, mut job) =
            storage::begin(self, actor, &command, operation).await?;
        if storage::replay(&mut transaction, &receipt, operation, &job).await? {
            return Ok(ModelDispatch {
                step: storage::finish_read(transaction, job, command.ordinal).await?,
                send: false,
            });
        }
        storage::revision(&job, &command)?;
        storage::fence(&job, actor, &command, now)?;
        let mut step = storage::current(&mut transaction, &job, command.ordinal).await?;
        if step.state != ModelStepState::Reserved {
            transaction.commit().await?;
            return Ok(ModelDispatch { step, send: false });
        }
        let wall = validation::duration(
            step.request
                .invocation
                .as_ref()
                .and_then(|invocation| invocation.budget.as_ref())
                .and_then(|budget| budget.maximum_wall_time.as_ref()),
        )?;
        let expiry = super::postgres::timestamp_millis(
            job.active_lease
                .as_ref()
                .and_then(|lease| lease.expires_at.as_ref())
                .ok_or(StoreError::LeaseFenced)?,
            false,
        )?;
        if wall > expiry - now {
            return Err(StoreError::LeaseFenced);
        }
        let original = job.clone();
        storage::advance(&mut job, now)?;
        storage::writer(&mut transaction).await?;
        storage::transition(
            &mut transaction,
            &job,
            "dispatched",
            None,
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
        step.state = ModelStepState::Dispatched;
        Ok(ModelDispatch { step, send: true })
    }

    /// Retain an uncertain paid attempt without releasing budget or regenerating.
    /// Only DISPATCHED becomes AMBIGUOUS. Replays are observational and preserve
    /// the original request; a stale lease cannot mutate the evidence.
    pub(crate) async fn uncertain_model(
        &self,
        actor: &Actor,
        command: ModelStepCommand,
    ) -> StoreResult<ModelStep> {
        let operation = "loop.model.uncertain";
        let receipt = ReceiptRequest {
            command: Some(command.clone()),
            ..Default::default()
        };
        let (mut transaction, now, mut job) =
            storage::begin(self, actor, &command, operation).await?;
        if storage::replay(&mut transaction, &receipt, operation, &job).await? {
            return storage::finish_read(transaction, job, command.ordinal).await;
        }
        storage::revision(&job, &command)?;
        storage::fence(&job, actor, &command, now)?;
        let mut step = storage::current(&mut transaction, &job, command.ordinal).await?;
        if step.state != ModelStepState::Dispatched {
            return Err(StoreError::InvalidTransition);
        }
        let original = job.clone();
        storage::advance(&mut job, now)?;
        storage::writer(&mut transaction).await?;
        storage::transition(
            &mut transaction,
            &job,
            "ambiguous",
            None,
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
        step.state = ModelStepState::Ambiguous;
        Ok(step)
    }

    /// Commit a verified response with its terminal outcome. Only the trusted
    /// runtime calls this after candidate validation; this is not a public RPC
    /// and does not admit a factor. The reserve remains a conservative ceiling.
    pub(crate) async fn finish_model(
        &self,
        actor: &Actor,
        command: ModelStepCommand,
        response: ModelResponse,
        outcome: JobOutcome,
    ) -> StoreResult<ModelStep> {
        self.finish_step(actor, command, response, Some(outcome))
            .await
    }

    /// Commit the sole registered intermediate tool call without terminating the
    /// job or releasing its lease. Replays preserve the original response; a
    /// changed call, stale lease, or missing authority fails transactionally.
    pub(crate) async fn finish_call(
        &self,
        actor: &Actor,
        command: ModelStepCommand,
        response: ModelResponse,
    ) -> StoreResult<ModelStep> {
        self.finish_step(actor, command, response, None).await
    }

    async fn finish_step(
        &self,
        actor: &Actor,
        command: ModelStepCommand,
        response: ModelResponse,
        outcome: Option<JobOutcome>,
    ) -> StoreResult<ModelStep> {
        let operation = if outcome.is_some() {
            "loop.model.finish"
        } else {
            "loop.model.call"
        };
        let receipt = ReceiptRequest {
            command: Some(command.clone()),
            response: Some(response.clone()),
            outcome: outcome.clone(),
            ..Default::default()
        };
        let (mut transaction, now, mut job) =
            storage::begin(self, actor, &command, operation).await?;
        if storage::replay(&mut transaction, &receipt, operation, &job).await? {
            return storage::finish_read(transaction, job, command.ordinal).await;
        }
        storage::revision(&job, &command)?;
        storage::fence(&job, actor, &command, now)?;
        let mut step = storage::current(&mut transaction, &job, command.ordinal).await?;
        let recovered = step.state == ModelStepState::Completed
            && outcome.is_some()
            && step.response.as_ref() == Some(&response);
        if !recovered
            && !matches!(
                step.state,
                ModelStepState::Dispatched | ModelStepState::Ambiguous
            )
        {
            return Err(StoreError::InvalidTransition);
        }
        validation::response(&step, &response)?;
        let original = job.clone();
        if let Some(outcome) = outcome {
            if matches!(outcome.outcome, Some(job_outcome::Outcome::Success(_)))
                && step
                    .request
                    .invocation
                    .as_ref()
                    .is_some_and(|input| input.structured_output.is_none())
            {
                return Err(StoreError::Invalid("intermediate model success"));
            }
            job.state = match &outcome.outcome {
                Some(job_outcome::Outcome::Success(_)) => JobState::Succeeded as i32,
                Some(job_outcome::Outcome::InfrastructureFailure(_)) => {
                    JobState::InfrastructureFailed as i32
                }
                _ => return Err(StoreError::Invalid("model completion outcome")),
            };
            job.outcome = Some(outcome);
            job.active_lease = None;
        } else {
            validation::call(&step, &response)?;
            // The registered read-only tool has a 30-second ceiling, followed by
            // bounded result registration. This never extends the job deadline.
            let expiry = now
                .checked_add(35_000)
                .ok_or(StoreError::Invalid("tool lease expiry"))?
                .min(storage::deadline(&job)?);
            storage::lease(&mut job, actor, now, expiry)?;
        }
        storage::advance(&mut job, now)?;
        storage::writer(&mut transaction).await?;
        if !recovered {
            storage::transition(
                &mut transaction,
                &job,
                "completed",
                Some(&response),
                now,
                command.ordinal,
            )
            .await?;
        }
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
        step.state = ModelStepState::Completed;
        step.response = Some(response);
        Ok(step)
    }

    /// Append the exact result of the committed registered tool call. The caller
    /// has already checked the actual research source and result schema. Storage
    /// binds the call identity, document checksums and current lease, and appends
    /// the immutable result with its command receipt and audit in one transaction.
    /// A duplicate identical result is observational; conflicting replay fails.
    pub(crate) async fn record_tool(
        &self,
        actor: &Actor,
        command: ModelStepCommand,
        result: ToolResultContent,
    ) -> StoreResult<ModelStep> {
        let operation = "loop.tool.record";
        let receipt = ReceiptRequest {
            command: Some(command.clone()),
            tool_result: Some(result.clone()),
            ..Default::default()
        };
        let (mut transaction, now, mut job) =
            storage::begin(self, actor, &command, operation).await?;
        if storage::replay(&mut transaction, &receipt, operation, &job).await? {
            return storage::finish_read(transaction, job, command.ordinal).await;
        }
        storage::revision(&job, &command)?;
        storage::fence(&job, actor, &command, now)?;
        let mut step = storage::current(&mut transaction, &job, command.ordinal).await?;
        if step.state != ModelStepState::Completed || job.state != JobState::Running as i32 {
            return Err(StoreError::InvalidTransition);
        }
        validation::tool(&step, &result)?;
        if let Some(prior) = &step.tool_result {
            if prior != &result {
                return Err(StoreError::IdempotencyConflict);
            }
            transaction.commit().await?;
            return Ok(step);
        }
        let original = job.clone();
        storage::advance(&mut job, now)?;
        storage::writer(&mut transaction).await?;
        step.job = job.clone();
        storage::insert_tool(&mut transaction, &step, &result, now).await?;
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
        step.tool_result = Some(result);
        Ok(step)
    }

    /// Read authenticated verified historical evidence, including terminal or
    /// expired jobs. Reading grants no dispatch permission and performs no repair.
    #[cfg(test)]
    pub(crate) async fn model_step(
        &self,
        actor: &Actor,
        job_id: &str,
    ) -> StoreResult<Option<ModelStep>> {
        Ok(self.model_history(actor, job_id).await?.pop())
    }

    /// Read the entire bounded call history in ordinal order under one consistent
    /// observation. Every request, response, tool result and cumulative budget is
    /// verified; a missing or corrupt earlier turn denies the whole read.
    pub(crate) async fn model_history(
        &self,
        actor: &Actor,
        job_id: &str,
    ) -> StoreResult<Vec<ModelStep>> {
        super::validate_id(job_id)?;
        let mut transaction = self.pool.begin().await?;
        // Keep the job and step projections in one serialized observation.
        self.observe_clock(&mut transaction).await?;
        let row = sqlx::query("SELECT * FROM jobs WHERE job_id=$1")
            .bind(job_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(StoreError::NotFound)?;
        let job = super::postgres::record_from_row(&row)?;
        self.admission
            .authorize_job_command("loop.model.read", actor, &job)?;
        let history = storage::history(&mut transaction, &job).await?;
        transaction.commit().await?;
        Ok(history)
    }

    /// Take over an expired tracked lease before the absolute deadline. It never
    /// changes the original model request, dispatch state, or reserved budget.
    /// A DISPATCHED/AMBIGUOUS takeover may only query the Provider receipt.
    pub(crate) async fn takeover_model(
        &self,
        actor: &Actor,
        command: ModelStepCommand,
        duration: prost_types::Duration,
    ) -> StoreResult<ModelStep> {
        let operation = "loop.model.takeover";
        let wall = validation::duration(Some(&duration))?;
        let receipt = ReceiptRequest {
            command: Some(command.clone()),
            duration: Some(duration),
            ..Default::default()
        };
        let (mut transaction, now, mut job) =
            storage::begin(self, actor, &command, operation).await?;
        if storage::replay(&mut transaction, &receipt, operation, &job).await? {
            return storage::finish_read(transaction, job, command.ordinal).await;
        }
        storage::revision(&job, &command)?;
        let mut step = storage::current(&mut transaction, &job, command.ordinal).await?;
        let old = job.active_lease.as_ref().ok_or(StoreError::LeaseFenced)?;
        let expiry = super::postgres::timestamp_millis(
            old.expires_at.as_ref().ok_or(StoreError::LeaseFenced)?,
            false,
        )?;
        if command.lease_id.is_some()
            || old.owner.as_ref() != Some(actor)
            || now < expiry
            || now >= storage::deadline(&job)?
            || job.state != JobState::Running as i32
        {
            return Err(StoreError::LeaseFenced);
        }
        let original = job.clone();
        let expiry = now
            .checked_add(wall)
            .ok_or(StoreError::Invalid("model takeover expiry"))?
            .min(storage::deadline(&job)?);
        storage::lease(&mut job, actor, now, expiry)?;
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
        step.job = job;
        Ok(step)
    }
}
