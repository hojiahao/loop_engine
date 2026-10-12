//! Human-owned run reservations. Execution remains in the existing Agent Harness.
mod storage;
#[cfg(test)]
mod tests;

use loop_protocol::wire::discovery::v1 as discovery;
use loop_protocol::wire::runs::v1::{
    RunSpecification, RunStatus, RunView, StartRunRequest, StepRunRequest,
};
use loop_protocol::wire::v1::{
    Actor, CommandContext, IdempotencyKey, JobId, JobRecord, JobSpecification, JobState, RequestId,
};
use prost::Message;
use sqlx::{Postgres, Transaction};

use super::lifecycle::validate_context;
use super::postgres::{timestamp, timestamp_millis};
use super::{PgJobStore, RoleCommand, StoreError, StoreResult, SubmissionMetadata};
use storage::{
    ReceiptInput, child_handle, load, persist, receipt, record_receipt, reserve, wall_millis,
};

/// Durable owner-scoped run and current child. Transport handlers must separately
/// verify the frozen execution plan before using this internal specification.
#[derive(Clone, Debug)]
pub struct RunSnapshot {
    /// Immutable server-resolved owner, executor, input and total ceilings.
    pub specification: RunSpecification,
    /// Safe current observation; live plan availability is initially false.
    pub view: RunView,
    /// Current child's verified durable record; never returned as a Run RPC DTO.
    pub current_job: JobRecord,
}

/// Constructible only while reserving one exact run child in this module.
pub(super) struct RunPermit {
    run_id: String,
    job_id: String,
}

impl RunPermit {
    pub(super) fn job_id(&self) -> &str {
        &self.job_id
    }

    pub(super) fn verify(&self, specification: &JobSpecification) -> StoreResult<()> {
        if specification.run_id.as_ref().map(|id| id.value.as_str()) != Some(self.run_id.as_str())
            || specification.job_id.as_ref().map(|id| id.value.as_str())
                != Some(self.job_id.as_str())
        {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(())
    }
}

impl PgJobStore {
    /// Atomically reserve and create the first child of an administrator-pinned
    /// run. The principal must be authenticated and match its frozen Human owner.
    /// A semantic replay returns the original receipt without executing a job.
    /// Invalid budgets, changed plans, reused historical run IDs and outages deny;
    /// cancellation before commit leaves no reservation, child or receipt.
    pub async fn start_run(
        &self,
        principal: &Actor,
        request: &StartRunRequest,
        specification: &RunSpecification,
    ) -> StoreResult<RunView> {
        let context = validate_context(request.context.as_ref(), principal)?;
        let mut transaction = self.pool.begin().await?;
        let now = self.observe_clock(&mut transaction).await?;
        check_time(context, now)?;
        loop_protocol::runs::validate_specification(specification, &timestamp(now))?;
        self.admission.authorize_run(principal, specification)?;
        if request.plan != specification.plan || specification.owner.as_ref() != Some(principal) {
            return Err(StoreError::AdmissionDenied);
        }
        let run_id = specification
            .run_id
            .as_ref()
            .expect("validated run")
            .value
            .as_str();
        let normalized = normalize_start(request);
        if let Some(view) = receipt(
            &mut transaction,
            context,
            "loop.runs.start",
            &normalized,
            run_id,
        )
        .await?
        {
            let current = load(&mut transaction, run_id).await?;
            self.admission
                .authorize_run(principal, &current.specification)?;
            if current.specification != *specification || view.revision != 1 {
                return Err(StoreError::IdempotencyConflict);
            }
            transaction.commit().await?;
            return Ok(view);
        }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM research_runs WHERE run_id = $1) OR EXISTS(SELECT 1 FROM jobs WHERE run_id = $1)")
            .bind(run_id).fetch_one(&mut *transaction).await?;
        if exists {
            return Err(StoreError::DuplicateJob);
        }
        let budget = specification.budget.as_ref().expect("validated budget");
        let deadline = now
            .checked_add(wall_millis(budget.maximum_wall_time.as_ref())?)
            .ok_or(StoreError::Invalid("run deadline"))?;
        let permit = new_permit(run_id);
        let mut view = RunView {
            run_id: specification.run_id.clone(),
            status: RunStatus::Active as i32,
            revision: 1,
            maximum_rounds: specification.maximum_rounds,
            completed_rounds: 0,
            current_job: Some(queued_handle(&permit, now)),
            budget: specification.budget.clone(),
            reserved_cost: Some(storage::money(0)),
            submitted_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            deadline: Some(timestamp(deadline)),
            ..Default::default()
        };
        if !reserve(&mut view, specification, now)? {
            return Err(StoreError::Invalid("initial run reservation"));
        }
        persist(
            &mut transaction,
            specification,
            &view,
            Some(permit.job_id()),
        )
        .await?;
        submit_child(
            self,
            &mut transaction,
            specification,
            context,
            &permit,
            1,
            now,
        )
        .await?;
        record_receipt(
            &mut transaction,
            &self.ledger_id,
            context,
            ReceiptInput {
                operation: "loop.runs.start",
                request: &normalized,
                view: &view,
                first_job: permit.job_id(),
                now,
            },
        )
        .await?;
        #[cfg(test)]
        super::crash_tests::fault_point("run_start_before").await;
        transaction.commit().await?;
        #[cfg(test)]
        super::crash_tests::fault_point("run_start_after").await;
        Ok(view)
    }

    /// Observe the authenticated owner's immutable specification and verified
    /// current child without requiring a live execution plan. Denies other
    /// owners, unknown IDs and corrupt projections; cancellation changes no data.
    pub async fn read_run(&self, principal: &Actor, run_id: &str) -> StoreResult<RunSnapshot> {
        super::validate_id(run_id)?;
        let mut transaction = self.pool.begin().await?;
        // A single MVCC snapshot prevents a concurrent advancement from mixing
        // the old parent with a new child while retaining bounded read waits.
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await?;
        let mut snapshot = load(&mut transaction, run_id).await?;
        self.admission
            .authorize_run(principal, &snapshot.specification)?;
        if snapshot.specification.owner.as_ref() != Some(principal) {
            return Err(StoreError::AdmissionDenied);
        }
        snapshot.view.current_job = Some(child_handle(&snapshot.current_job)?);
        transaction.commit().await?;
        Ok(snapshot)
    }

    /// Return an already committed step receipt after reauthorizing its owner.
    /// A returned receipt never permits dispatch, even if a later child is active.
    /// Missing receipts return None; semantic key reuse and corrupt binding deny.
    pub async fn replay_step(
        &self,
        principal: &Actor,
        request: &StepRunRequest,
    ) -> StoreResult<Option<RunView>> {
        let context = validate_context(request.context.as_ref(), principal)?;
        let run_id = step_id(request)?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await?;
        let snapshot = load(&mut transaction, run_id).await?;
        self.admission
            .authorize_run(principal, &snapshot.specification)?;
        if snapshot.specification.owner.as_ref() != Some(principal) {
            return Err(StoreError::AdmissionDenied);
        }
        let result = receipt(
            &mut transaction,
            context,
            "loop.runs.step",
            &normalize_step(request),
            run_id,
        )
        .await?;
        if result
            .as_ref()
            .is_some_and(|view| view.revision != request.expected_revision + 1)
        {
            return Err(StoreError::Corrupt("run step receipt revision"));
        }
        transaction.commit().await?;
        Ok(result)
    }

    /// Account for a terminal current child and reserve at most one next child in
    /// the same transaction. This never performs external model calls. The exact
    /// frozen specification and expected revision are required; stale CAS, missing
    /// authorization, active children and corrupt evidence deny. Replays return
    /// the original result. Cancellation rolls back the whole advance.
    pub async fn advance_run(
        &self,
        principal: &Actor,
        request: &StepRunRequest,
        specification: &RunSpecification,
    ) -> StoreResult<RunView> {
        let context = validate_context(request.context.as_ref(), principal)?;
        let run_id = step_id(request)?;
        let mut transaction = self.pool.begin().await?;
        let now = self.observe_clock(&mut transaction).await?;
        check_time(context, now)?;
        let snapshot = load(&mut transaction, run_id).await?;
        self.admission
            .authorize_run(principal, &snapshot.specification)?;
        if snapshot.specification != *specification
            || specification.owner.as_ref() != Some(principal)
        {
            return Err(StoreError::AdmissionDenied);
        }
        let normalized = normalize_step(request);
        if let Some(view) = receipt(
            &mut transaction,
            context,
            "loop.runs.step",
            &normalized,
            run_id,
        )
        .await?
        {
            if view.revision != request.expected_revision + 1 {
                return Err(StoreError::Corrupt("run step receipt revision"));
            }
            transaction.commit().await?;
            return Ok(view);
        }
        let mut view = snapshot.view;
        if view.revision != request.expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        if view.status != RunStatus::Active as i32 {
            return Err(StoreError::InvalidTransition);
        }
        let state = JobState::try_from(snapshot.current_job.state)
            .map_err(|_| StoreError::Corrupt("run child state"))?;
        let expired =
            now >= timestamp_millis(view.deadline.as_ref().expect("validated deadline"), true)?;
        if !expired
            && matches!(
                state,
                JobState::Queued | JobState::Leased | JobState::Running | JobState::Paused
            )
        {
            return Err(StoreError::InvalidTransition);
        }
        view.current_job = Some(child_handle(&snapshot.current_job)?);
        view.revision += 1;
        view.updated_at = Some(timestamp(now));
        if state == JobState::Succeeded {
            view.completed_rounds += 1;
        }
        let mut next = None;
        view.status = if expired {
            RunStatus::DeadlineExceeded as i32
        } else if state == JobState::BudgetExhausted {
            RunStatus::BudgetExhausted as i32
        } else if state != JobState::Succeeded {
            RunStatus::InfrastructureFailed as i32
        } else if view.completed_rounds == view.maximum_rounds {
            RunStatus::Completed as i32
        } else if reserve(&mut view, specification, now)? {
            let permit = new_permit(run_id);
            view.current_job = Some(queued_handle(&permit, now));
            next = Some(permit);
            RunStatus::Active as i32
        } else {
            RunStatus::BudgetExhausted as i32
        };
        persist(&mut transaction, specification, &view, None).await?;
        if let Some(permit) = &next {
            submit_child(
                self,
                &mut transaction,
                specification,
                context,
                permit,
                view.completed_rounds + 1,
                now,
            )
            .await?;
        }
        let first: String =
            sqlx::query_scalar("SELECT first_job_id FROM research_runs WHERE run_id = $1")
                .bind(run_id)
                .fetch_one(&mut *transaction)
                .await?;
        record_receipt(
            &mut transaction,
            &self.ledger_id,
            context,
            ReceiptInput {
                operation: "loop.runs.step",
                request: &normalized,
                view: &view,
                first_job: &first,
                now,
            },
        )
        .await?;
        #[cfg(test)]
        super::crash_tests::fault_point("run_step_before").await;
        transaction.commit().await?;
        #[cfg(test)]
        super::crash_tests::fault_point("run_step_after").await;
        Ok(view)
    }
}

fn check_time(context: &CommandContext, now: i64) -> StoreResult<()> {
    if timestamp_millis(
        context
            .requested_at
            .as_ref()
            .expect("validated command time"),
        true,
    )? > now
    {
        return Err(StoreError::Invalid("future run command"));
    }
    Ok(())
}

fn step_id(request: &StepRunRequest) -> StoreResult<&str> {
    let id = request
        .run_id
        .as_ref()
        .ok_or(StoreError::Invalid("run ID"))?
        .value
        .as_str();
    super::validate_id(id)?;
    if request.expected_revision == 0 || request.expected_revision >= i64::MAX as u64 {
        return Err(StoreError::Invalid("run revision"));
    }
    Ok(id)
}

fn new_permit(run_id: &str) -> RunPermit {
    RunPermit {
        run_id: run_id.into(),
        job_id: format!("job.{}", uuid::Uuid::new_v4().simple()),
    }
}

fn queued_handle(permit: &RunPermit, now: i64) -> discovery::DiscoveryJobHandle {
    discovery::DiscoveryJobHandle {
        job_id: Some(JobId {
            value: permit.job_id.clone(),
        }),
        status: discovery::DiscoveryJobStatus::Queued as i32,
        revision: 1,
        submitted_at: Some(timestamp(now)),
        updated_at: Some(timestamp(now)),
    }
}

fn normalize_start(request: &StartRunRequest) -> Vec<u8> {
    let mut request = request.clone();
    if let Some(context) = &mut request.context {
        context.request_id = None;
        context.requested_at = None;
    }
    request.encode_to_vec()
}

fn normalize_step(request: &StepRunRequest) -> Vec<u8> {
    let mut request = request.clone();
    if let Some(context) = &mut request.context {
        context.request_id = None;
        context.requested_at = None;
    }
    request.encode_to_vec()
}

async fn submit_child(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    specification: &RunSpecification,
    parent: &CommandContext,
    permit: &RunPermit,
    round: u32,
    now: i64,
) -> StoreResult<()> {
    use sha2::{Digest, Sha256};
    let identity = format!("run.{:x}.{round}", Sha256::digest(permit.run_id.as_bytes()));
    let context = CommandContext {
        request_id: Some(RequestId {
            value: format!("request.{}", uuid::Uuid::new_v4().simple()),
        }),
        idempotency_key: Some(IdempotencyKey { value: identity }),
        actor: specification.executor.clone(),
        requested_at: Some(timestamp(now)),
        correlation_id: parent.correlation_id.clone(),
        causation_id: Some(loop_protocol::wire::v1::CausationId {
            value: parent
                .request_id
                .as_ref()
                .expect("validated parent request")
                .value
                .clone(),
        }),
    };
    let command = RoleCommand::Discovery(discovery::StartDiscoveryRequest {
        context: Some(context),
        discovery: specification.discovery.clone(),
    });
    let result = super::submission::submit_in(
        store,
        transaction,
        specification.executor.as_ref().expect("validated executor"),
        command,
        SubmissionMetadata {
            run_id: specification.run_id.clone().expect("validated run"),
            protocol_selection: specification
                .protocol_selection
                .clone()
                .expect("validated protocol"),
        },
        now,
        Some(permit),
    )
    .await?;
    if result.replayed {
        return Err(StoreError::Corrupt("run child was already submitted"));
    }
    Ok(())
}
