use loop_protocol::job::validate_job_specification;
use loop_protocol::wire::discovery::v1 as discovery;
use loop_protocol::wire::research::v1 as research;
use loop_protocol::wire::v1::{
    Actor, BacktestJobInput, CommandContext, DiscoveryJobInput, FactorEvaluationJobInput,
    JobBudget, JobId, JobKind, JobRecord, JobSpecification, JobState, ProtocolSelectionSnapshot,
    ReconciliationJobInput, RunId, job_specification,
};
use prost::Message;
use sqlx::Row;

use super::lifecycle::{save_receipt, validate_context};
use super::postgres::{
    decode_record, encode_message, insert_job, record_from_row, timestamp, timestamp_millis,
    verified_blob,
};
use super::{PgJobStore, StoreError, StoreResult, SubmitJob, audit, validate_id};

/// Narrow role-owned requests. No generic job input or holdout variant is accepted.
#[derive(Clone, Debug, PartialEq)]
pub enum RoleCommand {
    /// Begin a development-only discovery job.
    Discovery(discovery::StartDiscoveryRequest),
    /// Evaluate a frozen factor against development references.
    FactorEvaluation(research::EnqueueFactorEvaluationRequest),
    /// Queue a development backtest with frozen provenance.
    Backtest(research::EnqueueBacktestRequest),
    /// Compare two already registered, independent backtest results.
    Reconciliation(research::EnqueueReconciliationRequest),
}

/// Server-resolved run and protocol context, never taken from a role request body.
#[derive(Clone, Debug)]
pub struct SubmissionMetadata {
    /// Authorized run aggregate owning the new job.
    pub run_id: RunId,
    /// Negotiated snapshot; an idempotent retry retains the original selection.
    pub protocol_selection: ProtocolSelectionSnapshot,
}

/// Safe role projection. Specifications, leases, outcomes, and artifacts stay internal.
#[derive(Clone, Debug, PartialEq)]
pub enum RoleJobHandle {
    /// Discovery service's narrow response value.
    Discovery(discovery::DiscoveryJobHandle),
    /// Research service's narrow response value.
    Research(research::ResearchJobHandle),
}

/// Result returned to the role handler; replay never authorizes redispatch.
#[derive(Clone, Debug, PartialEq)]
pub struct RoleSubmissionResult {
    /// Narrow, role-owned job projection at original acceptance.
    pub handle: RoleJobHandle,
    /// True when this command returned its original durable receipt.
    pub replayed: bool,
}

impl RoleCommand {
    fn operation(&self) -> &'static str {
        match self {
            Self::Discovery(_) => "loop.discovery.start",
            Self::FactorEvaluation(_) => "loop.research.evaluate_factor",
            Self::Backtest(_) => "loop.research.backtest",
            Self::Reconciliation(_) => "loop.research.reconcile",
        }
    }

    fn context(&self) -> Option<&CommandContext> {
        match self {
            Self::Discovery(request) => request.context.as_ref(),
            Self::FactorEvaluation(request) => request.context.as_ref(),
            Self::Backtest(request) => request.context.as_ref(),
            Self::Reconciliation(request) => request.context.as_ref(),
        }
    }

    fn normalized(&self) -> Self {
        let mut command = self.clone();
        let context = match &mut command {
            Self::Discovery(request) => &mut request.context,
            Self::FactorEvaluation(request) => &mut request.context,
            Self::Backtest(request) => &mut request.context,
            Self::Reconciliation(request) => &mut request.context,
        };
        if let Some(context) = context {
            context.request_id = None;
            context.requested_at = None;
        }
        command
    }

    fn encode(&self) -> StoreResult<Vec<u8>> {
        match self {
            Self::Discovery(request) => encode_message(request),
            Self::FactorEvaluation(request) => encode_message(request),
            Self::Backtest(request) => encode_message(request),
            Self::Reconciliation(request) => encode_message(request),
        }
    }

    fn decode_like(&self, bytes: &[u8]) -> StoreResult<Self> {
        let decoded = match self {
            Self::Discovery(_) => {
                discovery::StartDiscoveryRequest::decode(bytes).map(Self::Discovery)
            }
            Self::FactorEvaluation(_) => {
                research::EnqueueFactorEvaluationRequest::decode(bytes).map(Self::FactorEvaluation)
            }
            Self::Backtest(_) => {
                research::EnqueueBacktestRequest::decode(bytes).map(Self::Backtest)
            }
            Self::Reconciliation(_) => {
                research::EnqueueReconciliationRequest::decode(bytes).map(Self::Reconciliation)
            }
        };
        decoded.map_err(|_| StoreError::Corrupt("role command receipt"))
    }

    fn input(&self) -> StoreResult<(JobKind, job_specification::Input)> {
        let missing = StoreError::Invalid("role input");
        Ok(match self {
            Self::Discovery(request) => {
                let input = request.discovery.as_ref().ok_or(missing)?;
                let budget = input.budget.as_ref().map(|budget| JobBudget {
                    maximum_steps: budget.maximum_steps,
                    maximum_input_tokens: budget.maximum_input_tokens,
                    maximum_output_tokens: budget.maximum_output_tokens,
                    maximum_cost: budget.maximum_cost.clone(),
                    maximum_wall_time: budget.maximum_wall_time,
                });
                (
                    JobKind::Discovery,
                    job_specification::Input::Discovery(DiscoveryJobInput {
                        dataset: input.dataset.clone(),
                        research_policy: input.research_policy.clone(),
                        maker_model: input.maker_model.clone(),
                        checker_model: input.checker_model.clone(),
                        budget,
                        maximum_candidates: input.maximum_candidates,
                    }),
                )
            }
            Self::FactorEvaluation(request) => {
                let input = request.input.as_ref().ok_or(missing)?;
                (
                    JobKind::FactorEvaluation,
                    job_specification::Input::FactorEvaluation(FactorEvaluationJobInput {
                        factor: input.factor.clone(),
                        dataset: input.dataset.clone(),
                        budget: research_budget(input.budget.as_ref()),
                    }),
                )
            }
            Self::Backtest(request) => {
                let input = request.input.as_ref().ok_or(missing)?;
                (
                    JobKind::Backtest,
                    job_specification::Input::Backtest(BacktestJobInput {
                        factor_spec_id: input.factor_spec_id.clone(),
                        dataset: input.dataset.clone(),
                        return_definition: input.return_definition,
                        provenance: input.provenance.clone(),
                        deterministic_seed: input.deterministic_seed.clone(),
                        budget: research_budget(input.budget.as_ref()),
                    }),
                )
            }
            Self::Reconciliation(request) => {
                let input = request.input.as_ref().ok_or(missing)?;
                (
                    JobKind::IndependentReconciliation,
                    job_specification::Input::Reconciliation(ReconciliationJobInput {
                        primary_backtest_id: input.primary_backtest_id.clone(),
                        independent_backtest_id: input.independent_backtest_id.clone(),
                        reconciliation_policy: input.reconciliation_policy.clone(),
                        budget: research_budget(input.budget.as_ref()),
                    }),
                )
            }
        })
    }

    fn project(&self, record: JobRecord, replayed: bool) -> StoreResult<RoleSubmissionResult> {
        let specification = record
            .specification
            .ok_or(StoreError::Corrupt("role receipt specification"))?;
        let handle = if matches!(self, Self::Discovery(_)) {
            let status = discovery::DiscoveryJobStatus::try_from(record.state)
                .map_err(|_| StoreError::Corrupt("discovery projection"))?;
            RoleJobHandle::Discovery(discovery::DiscoveryJobHandle {
                job_id: specification.job_id,
                status: status as i32,
                revision: record.revision,
                submitted_at: specification.submitted_at,
                updated_at: record.updated_at,
            })
        } else {
            let status = research::ResearchJobStatus::try_from(record.state)
                .map_err(|_| StoreError::Corrupt("research projection"))?;
            RoleJobHandle::Research(research::ResearchJobHandle {
                job_id: specification.job_id,
                status: status as i32,
                revision: record.revision,
                submitted_at: specification.submitted_at,
                updated_at: record.updated_at,
            })
        };
        Ok(RoleSubmissionResult { handle, replayed })
    }
}

fn research_budget(budget: Option<&research::ResearchJobBudget>) -> Option<JobBudget> {
    budget.map(|budget| JobBudget {
        maximum_steps: budget.maximum_steps,
        maximum_input_tokens: budget.maximum_input_tokens,
        maximum_output_tokens: budget.maximum_output_tokens,
        maximum_cost: budget.maximum_cost.clone(),
        maximum_wall_time: budget.maximum_wall_time,
    })
}

pub(super) async fn submit(
    store: &PgJobStore,
    principal: &Actor,
    command: RoleCommand,
    metadata: SubmissionMetadata,
) -> StoreResult<RoleSubmissionResult> {
    let context = validate_context(command.context(), principal)?;
    validate_id(&metadata.run_id.value)?;
    let (kind, input) = command.input()?;
    let normalized = command.normalized();
    let request_blob = normalized.encode()?;
    let mut transaction = store.pool.begin().await?;
    let now = store.observe_clock(&mut transaction).await?;
    if timestamp_millis(context.requested_at.as_ref().expect("validated time"), true)? > now {
        return Err(StoreError::Invalid("future command time"));
    }
    let operation = command.operation();
    let receipt = sqlx::query(
        "SELECT * FROM command_receipts WHERE actor_id = $1 AND operation = $2 AND idempotency_key = $3",
    ).bind(&principal.actor_id.as_ref().expect("validated actor id").value)
        .bind(operation).bind(&context.idempotency_key.as_ref().expect("validated key").value)
        .fetch_optional(&mut *transaction).await?;
    if let Some(receipt) = receipt {
        let original = verified_blob(&receipt, "request_blob", "request_sha256")?;
        if normalized.decode_like(&original)? != normalized {
            return Err(StoreError::IdempotencyConflict);
        }
        let record = decode_record(&verified_blob(
            &receipt,
            "response_blob",
            "response_sha256",
        )?)?;
        let specification = record
            .specification
            .as_ref()
            .expect("validated specification");
        if specification.run_id.as_ref() != Some(&metadata.run_id) {
            return Err(StoreError::IdempotencyConflict);
        }
        if specification.kind != kind as i32
            || specification.input.as_ref() != Some(&input)
            || specification.submitted_by.as_ref() != Some(principal)
            || record.revision != 1
            || record.state != JobState::Queued as i32
            || record.updated_at != specification.submitted_at
            || receipt.try_get::<String, _>("job_id")?
                != specification
                    .job_id
                    .as_ref()
                    .expect("validated job id")
                    .value
        {
            return Err(StoreError::Corrupt("role receipt binding"));
        }
        let row = sqlx::query("SELECT * FROM jobs WHERE job_id = $1")
            .bind(
                &specification
                    .job_id
                    .as_ref()
                    .expect("validated job id")
                    .value,
            )
            .fetch_one(&mut *transaction)
            .await?;
        if record_from_row(&row)?.specification != record.specification {
            return Err(StoreError::Corrupt("role job binding"));
        }
        store.admission.validate_submission(specification)?;
        transaction.commit().await?;
        return command.project(record, true);
    }

    let specification = JobSpecification {
        job_id: Some(JobId {
            value: format!("job.{}", uuid::Uuid::new_v4().simple()),
        }),
        run_id: Some(metadata.run_id),
        kind: kind as i32,
        input: Some(input),
        submitted_at: Some(timestamp(now)),
        submitted_by: Some(principal.clone()),
        idempotency_key: context.idempotency_key.clone(),
        correlation_id: context.correlation_id.clone(),
        causation_id: context.causation_id.clone(),
        protocol_selection: Some(metadata.protocol_selection),
    };
    validate_job_specification(&specification)?;
    store.admission.validate_submission(&specification)?;
    if !specification
        .protocol_selection
        .as_ref()
        .expect("validated protocol")
        .enabled_features
        .iter()
        .any(|feature| feature == "jobs.prelease-terminal.v1")
    {
        return Err(StoreError::AdmissionDenied);
    }
    let submitted = SubmitJob {
        request_id: context
            .request_id
            .as_ref()
            .expect("validated request")
            .value
            .clone(),
        specification,
    };
    let record = insert_job(&mut transaction, &submitted.specification, now).await?;
    let job_id = &submitted
        .specification
        .job_id
        .as_ref()
        .expect("validated job id")
        .value;
    audit::append_command(
        &mut transaction,
        &store.ledger_id,
        &submitted,
        operation,
        now,
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
    super::crash_tests::fault_point("role_before_commit").await;
    transaction.commit().await?;
    #[cfg(test)]
    super::crash_tests::fault_point("role_after_commit").await;
    command.project(record, false)
}
