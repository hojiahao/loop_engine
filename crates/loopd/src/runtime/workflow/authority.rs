use std::sync::Arc;

use loop_protocol::wire::{
    runs::v1::RunSpecification,
    v1::{Actor, JobRecord, JobSpecification, job_specification},
};

use super::PreparedStage;
use crate::runtime::RuntimeAuthority;
use crate::store::{AdmissionPolicy, StoreError, StoreResult};

/// One operation's exact current child and optional verified next-child template.
/// It is never registered globally and cannot authorize a different run or role.
pub(crate) struct WorkflowAuthority {
    base: Arc<RuntimeAuthority>,
    run: RunSpecification,
    current: JobSpecification,
    next: Option<(String, job_specification::Input)>,
}

impl WorkflowAuthority {
    pub(in crate::runtime) fn new(
        base: Arc<RuntimeAuthority>,
        run: &RunSpecification,
        current: &JobSpecification,
        prepared: Option<&PreparedStage>,
    ) -> StoreResult<Self> {
        base.research_actor(run)?;
        if current.run_id != run.run_id {
            return Err(StoreError::AdmissionDenied);
        }
        let next = prepared
            .map(|stage| {
                stage.check_for(run)?;
                if stage.source().specification.as_ref() != Some(current) {
                    return Err(StoreError::AdmissionDenied);
                }
                match (stage.child_id(), stage.input()) {
                    (Some(id), Some(input)) => Ok(Some((id.to_owned(), input.clone()))),
                    (_, None) => Ok(None),
                    _ => Err(StoreError::AdmissionDenied),
                }
            })
            .transpose()?
            .flatten();
        Ok(Self {
            base,
            run: run.clone(),
            current: current.clone(),
            next,
        })
    }

    fn matches(&self, job: &JobSpecification) -> StoreResult<()> {
        let actor = self.base.research_actor(&self.run)?;
        if job.submitted_by.as_ref() != Some(&actor) || job.run_id != self.run.run_id {
            return Err(StoreError::AdmissionDenied);
        }
        if job == &self.current {
            return Ok(());
        }
        let (id, input) = self.next.as_ref().ok_or(StoreError::AdmissionDenied)?;
        if job.job_id.as_ref().map(|value| value.value.as_str()) != Some(id.as_str())
            || job.input.as_ref() != Some(input)
            || job.protocol_selection
                != self
                    .run
                    .numerical
                    .as_ref()
                    .and_then(|workflow| workflow.protocol_selection.clone())
        {
            return Err(StoreError::AdmissionDenied);
        }
        loop_protocol::job::validate_job_specification(job)?;
        Ok(())
    }
}

impl AdmissionPolicy for WorkflowAuthority {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        if self.matches(job).is_ok() {
            return Ok(());
        }
        self.base.validate_submission(job)
    }

    fn authorize_run(&self, actor: &Actor, run: &RunSpecification) -> StoreResult<()> {
        self.base.authorize_run(actor, run)
    }

    fn authorize_run_trial(
        &self,
        actor: &Actor,
        run: &RunSpecification,
        job: &JobSpecification,
    ) -> StoreResult<()> {
        self.base.authorize_run_trial(actor, run, job)
    }

    fn authorize_job_command(
        &self,
        operation: &str,
        actor: &Actor,
        record: &JobRecord,
    ) -> StoreResult<()> {
        let job = record
            .specification
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        if self.matches(job).is_err() {
            return self.base.authorize_job_command(operation, actor, record);
        }
        if actor != &self.base.research_actor(&self.run)?
            || !matches!(
                operation,
                "loop.jobs.read"
                    | "loop.jobs.acquire"
                    | "loop.jobs.heartbeat"
                    | "loop.jobs.complete"
                    | "loop.jobs.artifacts"
                    | "loop.jobs.evaluate"
                    | "loop.jobs.backtest"
                    | "loop.jobs.recover"
                    | "loop.factors.read_trials"
                    | "loop.backtests.read_current"
            )
            || !matches!(
                job.input,
                Some(
                    job_specification::Input::FactorEvaluation(_)
                        | job_specification::Input::Backtest(_)
                )
            )
        {
            return Err(StoreError::AdmissionDenied);
        }
        loop_protocol::job::validate_job_record(record)?;
        Ok(())
    }
}
