//! Opt-in numerical transitions reuse the ordinary authorized submission path.

use loop_protocol::wire::discovery::v1 as discovery;
use loop_protocol::wire::research::v1 as research;
use loop_protocol::wire::runs::v1::{
    ResearchRunView, RoundDisposition, RunSpecification, RunStage, RunStatus, RunView,
    StartRunRequest, StepRunRequest, research_run_view::CurrentChild,
};
use loop_protocol::wire::v1::{
    Actor, CommandContext, IdempotencyKey, JobId, JobSpecification, JobState, RequestId,
    job_specification,
};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};

use crate::runtime::workflow::{PreparedRun, PreparedStage};

use super::super::postgres::{budget, timestamp, timestamp_millis};
use super::super::{
    PgJobStore, RoleCommand, StoreError, StoreResult, SubmissionMetadata, evaluation, submission,
};
use super::storage::{
    ReceiptInput, load, persist, receipt, record_receipt, reserve_budget, wall_millis,
};
use super::transitions::{self, RunBinding};
use super::{
    RunPermit, RunSnapshot, check_time, check_version, new_permit, normalize_start, normalize_step,
    queued_handle, refresh, step_id, validate_context,
};

struct Child {
    permit: RunPermit,
    command: ChildCommand,
    specification: JobSpecification,
    metadata: SubmissionMetadata,
}

enum ChildCommand {
    Ordinary(Box<RoleCommand>),
    Perturbation(Box<submission::PerturbationCommand>),
}

struct StepInputs<'a> {
    snapshot: &'a RunSnapshot,
    context: &'a CommandContext,
    prepared: Option<&'a PreparedStage>,
    now: i64,
}

impl PgJobStore {
    /// Start an explicitly opted-in v2 run from a server-verified frozen recipe.
    /// Owner authorization, first reservation, real child and evidence commit
    /// atomically. Replays are observational; cancellation rolls back all writes.
    /// File guards, live authority and the deadline are rechecked before commit.
    pub(crate) async fn start_numerical(
        &self,
        principal: &Actor,
        request: &StartRunRequest,
        specification: &RunSpecification,
        prepared: &PreparedRun,
    ) -> StoreResult<RunView> {
        check_version(request.workflow_version, specification)?;
        if specification.numerical.is_none() {
            return Err(StoreError::AdmissionDenied);
        }
        prepared.check_for(specification)?;
        let context = validate_context(request.context.as_ref(), principal)?;
        let mut transaction = self.pool.begin().await?;
        let now = self.observe_clock(&mut transaction).await?;
        check_time(context, now)?;
        loop_protocol::runs::validate_specification(specification, &timestamp(now))?;
        self.admission.authorize_run(principal, specification)?;
        if request.plan != specification.plan || specification.owner.as_ref() != Some(principal) {
            return Err(StoreError::AdmissionDenied);
        }
        let run_id = &specification.run_id.as_ref().expect("validated run").value;
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
            if current.specification != *specification || view.revision != 1 {
                return Err(StoreError::IdempotencyConflict);
            }
            transaction.commit().await?;
            return Ok(view);
        }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM research_runs WHERE run_id=$1) OR EXISTS(SELECT 1 FROM jobs WHERE run_id=$1)")
            .bind(run_id).fetch_one(&mut *transaction).await?;
        if exists {
            return Err(StoreError::DuplicateJob);
        }
        let deadline = now
            .checked_add(wall_millis(
                specification
                    .budget
                    .as_ref()
                    .expect("validated budget")
                    .maximum_wall_time
                    .as_ref(),
            )?)
            .ok_or(StoreError::Invalid("run deadline"))?;
        let child = build_child(specification, context, 1, None, None, now)?;
        let mut view = RunView {
            run_id: specification.run_id.clone(),
            status: RunStatus::Active as i32,
            revision: 1,
            maximum_rounds: specification.maximum_rounds,
            budget: specification.budget.clone(),
            reserved_cost: Some(super::storage::money(0)),
            submitted_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            deadline: Some(timestamp(deadline)),
            research: Some(ResearchRunView {
                round: 1,
                ..Default::default()
            }),
            ..Default::default()
        };
        project_child(&mut view, &child, RunStage::Discovery, now)?;
        if !super::storage::reserve(&mut view, specification, now)? {
            return Err(StoreError::Invalid("initial run reservation"));
        }
        let mut binding = Box::new(RunBinding {
            current_specification: Some(child.specification.clone()),
            created_child: true,
            evidence: prepared.binding().to_vec(),
            adaptive: prepared.adaptive().map(|proof| proof.current().clone()),
            ..Default::default()
        });
        view.adaptive = binding
            .adaptive
            .as_ref()
            .map(|state| state.project())
            .transpose()?;
        let bytes = transitions::seal(specification, &mut view, &mut binding)?;
        transitions::check_adaptive(specification, None, &view, &binding)?;
        prepared.check_for(specification)?;
        let checked = final_time(self, now, Some(deadline))?;
        persist(
            &mut transaction,
            specification,
            &view,
            Some(child.permit.job_id()),
        )
        .await?;
        insert_child(self, &mut transaction, &child, now).await?;
        transitions::insert(&mut transaction, &view, &binding, &bytes).await?;
        record_receipt(
            &mut transaction,
            &self.ledger_id,
            context,
            ReceiptInput {
                operation: "loop.runs.start",
                request: &normalized,
                view: &view,
                first_job: child.permit.job_id(),
                now,
            },
        )
        .await?;
        #[cfg(test)]
        super::super::crash_tests::fault_point("run_start_before").await;
        final_time(self, checked, Some(deadline))?;
        self.admission.authorize_run(principal, specification)?;
        self.admission.validate_submission(&child.specification)?;
        prepared.check_for(specification)?;
        transaction.commit().await?;
        #[cfg(test)]
        super::super::crash_tests::fault_point("run_start_after").await;
        Ok(view)
    }

    /// Commit one v2 stage boundary after server-verified numerical preparation.
    /// Actual source records, trial coverage and duplicate evidence are checked
    /// again transactionally. No arbitrary child command or unverified rejection
    /// can bypass admission. Stale revisions and active children deny; retries
    /// return the original receipt and never execute or reserve another child.
    /// Before commit, revalidate live owner/reader/child authority and time;
    /// expiry or clock regression rolls back the entire pending transition.
    pub(crate) async fn advance_numerical(
        &self,
        principal: &Actor,
        request: &StepRunRequest,
        specification: &RunSpecification,
        prepared: Option<&PreparedStage>,
    ) -> StoreResult<RunView> {
        check_version(request.workflow_version, specification)?;
        if specification.numerical.is_none() {
            return Err(StoreError::AdmissionDenied);
        }
        if let Some(prepared) = prepared {
            prepared.check_for(specification)?;
        }
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
        if snapshot.view.revision != request.expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        if snapshot.view.status != RunStatus::Active as i32 {
            return Err(StoreError::InvalidTransition);
        }
        let state = JobState::try_from(snapshot.current_job.state)
            .map_err(|_| StoreError::Corrupt("run child state"))?;
        let deadline = timestamp_millis(
            snapshot.view.deadline.as_ref().expect("validated deadline"),
            true,
        )?;
        let expired = now >= deadline;
        if !expired
            && matches!(
                state,
                JobState::Queued | JobState::Leased | JobState::Running
            )
        {
            return Err(StoreError::InvalidTransition);
        }
        if !expired && state == JobState::Succeeded && prepared.is_none() {
            return Err(StoreError::AdmissionDenied);
        }
        if prepared.is_some_and(|proof| proof.source() != &snapshot.current_job) {
            return Err(StoreError::RevisionConflict);
        }
        if let Some(proof) = prepared.and_then(PreparedStage::adaptive)
            && proof.before()
                != snapshot
                    .binding
                    .as_ref()
                    .and_then(|binding| binding.adaptive.as_ref())
        {
            return Err(StoreError::Corrupt("adaptive predecessor"));
        }
        let mut view = snapshot.view.clone();
        refresh(&mut view, &snapshot.current_job)?;
        view.revision += 1;
        view.updated_at = Some(timestamp(now));
        let mut binding = Box::new(RunBinding {
            previous_sha256: snapshot
                .view
                .research
                .as_ref()
                .expect("v2 projection")
                .binding_sha256
                .as_ref()
                .expect("verified binding")
                .value
                .clone(),
            current_specification: snapshot.current_job.specification.clone(),
            source_record: Some(snapshot.current_job.clone()),
            source_trial: evaluation::read(&mut transaction, &snapshot.current_job).await?,
            source_backtest: super::super::backtest::verify_stored(
                &mut transaction,
                &snapshot.current_job,
            )
            .await?,
            evidence: prepared
                .map(|proof| proof.binding().to_vec())
                .unwrap_or_else(|| {
                    snapshot
                        .binding
                        .as_ref()
                        .expect("v2 binding")
                        .evidence
                        .clone()
                }),
            adaptive: prepared
                .filter(|_| !expired)
                .and_then(PreparedStage::adaptive)
                .map(|proof| proof.current().clone())
                .or_else(|| {
                    snapshot
                        .binding
                        .as_ref()
                        .and_then(|binding| binding.adaptive.clone())
                }),
            ..Default::default()
        });
        if prepared.is_some_and(|proof| proof.source_trial() != binding.source_trial.as_ref()) {
            return Err(StoreError::Corrupt("prepared source trial"));
        }
        let next = if expired {
            view.status = RunStatus::DeadlineExceeded as i32;
            None
        } else if state != JobState::Succeeded {
            view.status = if state == JobState::BudgetExhausted {
                RunStatus::BudgetExhausted
            } else {
                RunStatus::InfrastructureFailed
            } as i32;
            None
        } else {
            advance_stage(
                self,
                &mut transaction,
                &mut view,
                &mut binding,
                StepInputs {
                    snapshot: &snapshot,
                    context,
                    prepared,
                    now,
                },
            )
            .await?
        };
        binding.created_child = next.is_some();
        if let Some(child) = &next {
            binding.current_specification = Some(child.specification.clone());
        }
        view.adaptive = binding
            .adaptive
            .as_ref()
            .map(|state| state.project())
            .transpose()?;
        let bytes = transitions::seal(specification, &mut view, &mut binding)?;
        transitions::check_adaptive(specification, snapshot.binding.as_deref(), &view, &binding)?;
        if let Some(prepared) = prepared {
            prepared.check_for(specification)?;
        }
        // A command which began before the deadline cannot publish a late
        // advancement. Roll it back so a retry can record the expired state.
        // Commands already observing expiry may commit that terminal record.
        let final_deadline = (!expired).then_some(deadline);
        let checked = final_time(self, now, final_deadline)?;
        persist(&mut transaction, specification, &view, None).await?;
        if let Some(child) = &next {
            insert_child(self, &mut transaction, child, now).await?;
        }
        transitions::insert(&mut transaction, &view, &binding, &bytes).await?;
        let first: String =
            sqlx::query_scalar("SELECT first_job_id FROM research_runs WHERE run_id=$1")
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
        super::super::crash_tests::fault_point("run_step_before").await;
        final_time(self, checked, final_deadline)?;
        self.admission.authorize_run(principal, specification)?;
        if !expired
            && matches!(
                snapshot
                    .current_job
                    .specification
                    .as_ref()
                    .and_then(|job| job.input.as_ref()),
                Some(job_specification::Input::Perturbation(_))
            )
        {
            self.admission.authorize_job_command(
                "loop.jobs.read",
                specification
                    .numerical
                    .as_ref()
                    .and_then(|workflow| workflow.research_executor.as_ref())
                    .ok_or(StoreError::AdmissionDenied)?,
                &snapshot.current_job,
            )?;
        }
        // Recheck the live reader identity for numerical observations, including
        // a pre-screened candidate with no new child. Pure Discovery expiry
        // needs only its owner; it neither reads nor authorizes numerical work.
        for job in snapshot
            .current_job
            .specification
            .iter()
            .chain(binding.attempted_specification.iter())
        {
            if matches!(
                job.input,
                Some(
                    job_specification::Input::FactorEvaluation(_)
                        | job_specification::Input::Backtest(_)
                )
            ) {
                self.admission.authorize_run_trial(
                    specification
                        .numerical
                        .as_ref()
                        .expect("v2 specification")
                        .research_executor
                        .as_ref()
                        .expect("validated executor"),
                    specification,
                    job,
                )?;
            }
        }
        if let Some(child) = &next {
            self.admission.validate_submission(&child.specification)?;
        }
        if let Some(prepared) = prepared {
            prepared.check_for(specification)?;
        }
        transaction.commit().await?;
        #[cfg(test)]
        super::super::crash_tests::fault_point("run_step_after").await;
        Ok(view)
    }
}

fn final_time(store: &PgJobStore, previous: i64, deadline: Option<i64>) -> StoreResult<i64> {
    let now = store.clock.now_millis()?;
    if now < previous {
        return Err(StoreError::ClockRegression);
    }
    if deadline.is_some_and(|deadline| now >= deadline) {
        return Err(StoreError::Unavailable("run commit deadline"));
    }
    Ok(now)
}

async fn advance_stage(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    view: &mut RunView,
    binding: &mut RunBinding,
    inputs: StepInputs<'_>,
) -> StoreResult<Option<Child>> {
    let StepInputs {
        snapshot,
        context,
        prepared,
        now,
    } = inputs;
    let stage = RunStage::try_from(view.research.as_ref().expect("validated research").stage)
        .map_err(|_| StoreError::Corrupt("numerical stage"))?;
    match stage {
        RunStage::Discovery | RunStage::Perturbation => {
            if stage == RunStage::Perturbation {
                let completed = prepared
                    .and_then(PreparedStage::adaptive)
                    .and_then(|proof| proof.completion())
                    .ok_or(StoreError::AdmissionDenied)?;
                completed.check_record(&snapshot.current_job)?;
                if completed.step().candidate.is_none() {
                    view.completed_rounds += 1;
                    view.status = RunStatus::SearchExhausted as i32;
                    view.research
                        .as_mut()
                        .expect("validated research")
                        .disposition = RoundDisposition::Exhausted as i32;
                    return Ok(None);
                }
            }
            let input = prepared
                .and_then(PreparedStage::input)
                .ok_or(StoreError::AdmissionDenied)?;
            if !matches!(input, job_specification::Input::FactorEvaluation(_)) {
                return Err(StoreError::AdmissionDenied);
            }
            let child = build_child(
                &snapshot.specification,
                context,
                view.completed_rounds + 1,
                Some(input),
                prepared.and_then(PreparedStage::child_id),
                now,
            )?;
            loop_protocol::job::validate_job_specification(&child.specification)?;
            store.admission.validate_submission(&child.specification)?;
            if let Some(previous) =
                evaluation::find_previous(transaction, &child.specification).await?
            {
                transitions::authorize_trial(
                    store,
                    transaction,
                    snapshot
                        .specification
                        .numerical
                        .as_ref()
                        .expect("v2 specification")
                        .research_executor
                        .as_ref()
                        .expect("validated executor"),
                    &previous.record,
                    &mut transitions::TrialCache::default(),
                )
                .await?;
                binding.attempted_specification = Some(child.specification);
                let disposition = if previous.trial.disposition == "insufficient_coverage" {
                    RoundDisposition::PreviouslyRejected
                } else {
                    RoundDisposition::Duplicate
                };
                binding.previous_record = Some(previous.record);
                binding.previous_trial = Some(previous.trial);
                binding.previous_context = previous.context_sha256.to_vec();
                project_factor(view, input)?;
                return finish_round(snapshot, view, binding, context, prepared, disposition, now);
            }
            let attempted = child.specification.clone();
            let next = reserve_child(snapshot, view, child, RunStage::Evaluation, now)?;
            if next.is_none() && snapshot.specification.feedback_policy.is_some() {
                binding.attempted_specification = Some(attempted);
            }
            Ok(next)
        }
        RunStage::Evaluation => {
            let trial = binding
                .source_trial
                .as_ref()
                .ok_or(StoreError::Corrupt("missing numerical trial"))?;
            if trial.disposition == "insufficient_coverage" {
                return finish_round(
                    snapshot,
                    view,
                    binding,
                    context,
                    prepared,
                    RoundDisposition::InsufficientCoverage,
                    now,
                );
            }
            if trial.disposition != "ready_for_backtest" {
                return Err(StoreError::Corrupt("numerical trial disposition"));
            }
            let input = prepared
                .and_then(PreparedStage::input)
                .ok_or(StoreError::AdmissionDenied)?;
            if !matches!(input, job_specification::Input::Backtest(_)) {
                return Err(StoreError::AdmissionDenied);
            }
            let child = build_child(
                &snapshot.specification,
                context,
                view.completed_rounds + 1,
                Some(input),
                prepared.and_then(PreparedStage::child_id),
                now,
            )?;
            reserve_child(snapshot, view, child, RunStage::Portfolio, now)
        }
        RunStage::Portfolio => {
            if binding.source_backtest.is_none() {
                return Err(StoreError::Corrupt("missing portfolio result"));
            }
            finish_round(
                snapshot,
                view,
                binding,
                context,
                prepared,
                RoundDisposition::Evaluated,
                now,
            )
        }
        RunStage::Unspecified => Err(StoreError::Corrupt("unspecified numerical stage")),
    }
}

fn finish_round(
    snapshot: &RunSnapshot,
    view: &mut RunView,
    binding: &mut RunBinding,
    context: &CommandContext,
    prepared: Option<&PreparedStage>,
    disposition: RoundDisposition,
    now: i64,
) -> StoreResult<Option<Child>> {
    view.completed_rounds += 1;
    view.research
        .as_mut()
        .expect("validated research")
        .disposition = disposition as i32;
    if view.completed_rounds == view.maximum_rounds {
        view.status = RunStatus::Completed as i32;
        return Ok(None);
    }
    if snapshot.specification.feedback_policy.is_some() {
        let proposal = prepared
            .and_then(PreparedStage::adaptive)
            .and_then(|proof| proof.next())
            .ok_or(StoreError::AdmissionDenied)?;
        let input = proposal
            .perturbation
            .as_ref()
            .map(|prepared| job_specification::Input::Perturbation(prepared.input().clone()));
        let stage = if input.is_some() {
            RunStage::Perturbation
        } else {
            RunStage::Discovery
        };
        let child = build_child(
            &snapshot.specification,
            context,
            view.completed_rounds + 1,
            input.as_ref(),
            Some(&proposal.job_id),
            now,
        )?;
        binding.adaptive = Some(proposal.binding.clone());
        return reserve_child(snapshot, view, child, stage, now);
    }
    let child = build_child(
        &snapshot.specification,
        context,
        view.completed_rounds + 1,
        None,
        None,
        now,
    )?;
    reserve_child(snapshot, view, child, RunStage::Discovery, now)
}

fn reserve_child(
    snapshot: &RunSnapshot,
    view: &mut RunView,
    child: Child,
    stage: RunStage,
    now: i64,
) -> StoreResult<Option<Child>> {
    let reserved = if stage == RunStage::Discovery {
        super::storage::reserve(view, &snapshot.specification, now)?
    } else {
        reserve_budget(
            view,
            &snapshot.specification,
            budget(&child.specification)?,
            now,
        )?
    };
    if !reserved {
        view.status = RunStatus::BudgetExhausted as i32;
        return Ok(None);
    }
    project_child(view, &child, stage, now)?;
    view.status = RunStatus::Active as i32;
    Ok(Some(child))
}

fn project_factor(view: &mut RunView, input: &job_specification::Input) -> StoreResult<()> {
    let job_specification::Input::FactorEvaluation(input) = input else {
        return Err(StoreError::Corrupt("factor projection input"));
    };
    let factor = input
        .factor
        .as_ref()
        .ok_or(StoreError::Invalid("prepared factor"))?;
    let research = view.research.as_mut().expect("validated research");
    research.factor_spec_id = factor.factor_spec_id.clone();
    research.expression_id = factor.expression_id.clone();
    Ok(())
}

fn project_child(view: &mut RunView, child: &Child, stage: RunStage, now: i64) -> StoreResult<()> {
    if stage == RunStage::Evaluation {
        project_factor(
            view,
            child.specification.input.as_ref().expect("prepared input"),
        )?;
    }
    let research = view.research.as_mut().expect("validated research");
    research.stage = stage as i32;
    research.disposition = RoundDisposition::Unspecified as i32;
    research.round = view.completed_rounds + 1;
    let id = Some(JobId {
        value: child.permit.job_id.clone(),
    });
    if stage == RunStage::Discovery {
        research.discovery_job_id = id;
        research.perturbation_job_id = None;
        research.evaluation_job_id = None;
        research.expression_id = None;
        research.factor_spec_id = None;
        research.current_child = Some(CurrentChild::Discovery(queued_handle(&child.permit, now)));
    } else {
        if stage == RunStage::Perturbation {
            research.perturbation_job_id = id.clone();
            research.discovery_job_id = None;
            research.evaluation_job_id = None;
            research.expression_id = None;
            research.factor_spec_id = None;
        }
        if stage == RunStage::Evaluation {
            research.evaluation_job_id = id.clone();
        }
        research.current_child = Some(CurrentChild::Research(research::ResearchJobHandle {
            job_id: id,
            status: research::ResearchJobStatus::Queued as i32,
            revision: 1,
            submitted_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
        }));
    }
    Ok(())
}

fn build_child(
    specification: &RunSpecification,
    parent: &CommandContext,
    round: u32,
    input: Option<&job_specification::Input>,
    child_id: Option<&str>,
    now: i64,
) -> StoreResult<Child> {
    let run_id = &specification.run_id.as_ref().expect("validated run").value;
    let numerical = specification
        .numerical
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?;
    let stage = match input {
        None => RunStage::Discovery,
        Some(job_specification::Input::FactorEvaluation(_)) => RunStage::Evaluation,
        Some(job_specification::Input::Backtest(_)) => RunStage::Portfolio,
        Some(job_specification::Input::Perturbation(_)) => RunStage::Perturbation,
        _ => return Err(StoreError::AdmissionDenied),
    };
    let actor = if stage == RunStage::Discovery {
        specification.executor.as_ref()
    } else {
        numerical.research_executor.as_ref()
    }
    .expect("validated executor");
    let context = CommandContext {
        request_id: Some(RequestId {
            value: format!("request.{}", uuid::Uuid::new_v4().simple()),
        }),
        idempotency_key: Some(IdempotencyKey {
            value: format!(
                "run.{:x}.{round}.{}",
                Sha256::digest(run_id.as_bytes()),
                stage as i32
            ),
        }),
        actor: Some(actor.clone()),
        requested_at: Some(timestamp(now)),
        correlation_id: parent.correlation_id.clone(),
        causation_id: Some(loop_protocol::wire::v1::CausationId {
            value: parent
                .request_id
                .as_ref()
                .expect("validated parent")
                .value
                .clone(),
        }),
    };
    if let Some(job_specification::Input::Perturbation(input)) = input {
        if specification.feedback_policy != input.feedback_policy
            || input.dataset
                != specification
                    .discovery
                    .as_ref()
                    .expect("validated discovery")
                    .dataset
            || input.budget.as_ref().map(Message::encode_to_vec)
                != numerical
                    .perturbation_budget
                    .as_ref()
                    .map(Message::encode_to_vec)
        {
            return Err(StoreError::AdmissionDenied);
        }
        let id = child_id.ok_or(StoreError::AdmissionDenied)?;
        super::super::validate_id(id)?;
        let permit = RunPermit {
            run_id: run_id.clone(),
            job_id: id.to_owned(),
        };
        let command = submission::PerturbationCommand {
            context: Some(context),
            input: Some(input.clone()),
        };
        let metadata = SubmissionMetadata {
            run_id: specification.run_id.clone().expect("validated run"),
            protocol_selection: numerical
                .protocol_selection
                .clone()
                .expect("validated protocol"),
        };
        let job = submission::build_perturbation(&command, actor, metadata.clone(), now, &permit)?;
        return Ok(Child {
            permit,
            command: ChildCommand::Perturbation(Box::new(command)),
            specification: job,
            metadata,
        });
    }
    let command = match input {
        None => RoleCommand::Discovery(discovery::StartDiscoveryRequest {
            context: Some(context),
            discovery: specification.discovery.clone(),
        }),
        Some(job_specification::Input::FactorEvaluation(input)) => {
            if input.dataset
                != specification
                    .discovery
                    .as_ref()
                    .expect("validated discovery")
                    .dataset
                || input.budget.as_ref().map(Message::encode_to_vec)
                    != numerical
                        .evaluation_budget
                        .as_ref()
                        .map(Message::encode_to_vec)
            {
                return Err(StoreError::AdmissionDenied);
            }
            RoleCommand::FactorEvaluation(research::EnqueueFactorEvaluationRequest {
                context: Some(context),
                input: Some(research::FactorEvaluationInput {
                    factor: input.factor.clone(),
                    dataset: input.dataset.clone(),
                    budget: numerical.evaluation_budget.clone(),
                    provenance: input.provenance.clone(),
                    deterministic_seed: input.deterministic_seed.clone(),
                }),
            })
        }
        Some(job_specification::Input::Backtest(input)) => {
            if input.dataset
                != specification
                    .discovery
                    .as_ref()
                    .expect("validated discovery")
                    .dataset
                || input.budget.as_ref().map(Message::encode_to_vec)
                    != numerical
                        .portfolio_budget
                        .as_ref()
                        .map(Message::encode_to_vec)
            {
                return Err(StoreError::AdmissionDenied);
            }
            RoleCommand::Backtest(research::EnqueueBacktestRequest {
                context: Some(context),
                input: Some(research::BacktestInput {
                    budget: numerical.portfolio_budget.clone(),
                    factor_spec_id: input.factor_spec_id.clone(),
                    dataset: input.dataset.clone(),
                    return_definition: input.return_definition,
                    provenance: input.provenance.clone(),
                    deterministic_seed: input.deterministic_seed.clone(),
                }),
            })
        }
        _ => return Err(StoreError::AdmissionDenied),
    };
    let permit = if let Some(id) = child_id {
        super::super::validate_id(id)?;
        if input.is_none() && specification.feedback_policy.is_none() {
            return Err(StoreError::AdmissionDenied);
        }
        RunPermit {
            run_id: run_id.clone(),
            job_id: id.to_owned(),
        }
    } else {
        if input.is_some() {
            return Err(StoreError::AdmissionDenied);
        }
        new_permit(run_id)
    };
    let metadata = SubmissionMetadata {
        run_id: specification.run_id.clone().expect("validated run"),
        protocol_selection: if stage == RunStage::Discovery {
            specification.protocol_selection.clone()
        } else {
            numerical.protocol_selection.clone()
        }
        .expect("validated protocol"),
    };
    let job = submission::build_job(&command, actor, metadata.clone(), now, Some(&permit))?;
    Ok(Child {
        permit,
        command: ChildCommand::Ordinary(Box::new(command)),
        specification: job,
        metadata,
    })
}

async fn insert_child(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    child: &Child,
    now: i64,
) -> StoreResult<()> {
    let actor = child
        .specification
        .submitted_by
        .as_ref()
        .expect("validated executor");
    let submitted = match &child.command {
        ChildCommand::Ordinary(command) => {
            submission::submit_in(
                store,
                transaction,
                actor,
                command.as_ref().clone(),
                child.metadata.clone(),
                now,
                Some(&child.permit),
            )
            .await?
        }
        ChildCommand::Perturbation(command) => {
            submission::submit_perturbation(
                store,
                transaction,
                actor,
                command.as_ref().clone(),
                child.metadata.clone(),
                now,
                &child.permit,
            )
            .await?
        }
    };
    if submitted.replayed {
        return Err(StoreError::Corrupt("run child already submitted"));
    }
    Ok(())
}
