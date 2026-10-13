//! Numerical phases keep verification, execution and completion frames separate.

use std::sync::Arc;

use loop_protocol::wire::{
    jobs::v1,
    v1::{Actor, JobRecord, JobState},
};

use super::RuntimeService;
use crate::manifests::portfolio::PreparedPortfolio;
use crate::manifests::{EvaluationPin, PortfolioPin};
use crate::runtime::DataPin;
use crate::store::{PgJobStore, StoreError, StoreResult};

mod evaluation;
mod portfolio;

// Operation-local references preserve the original scope and lease. Boxing the
// record and phase futures avoids cumulative debug-build stack temporaries.
struct PortfolioRun<'a> {
    store: &'a PgJobStore,
    actor: &'a Actor,
    command: &'a v1::ExecuteBacktestRequest,
    job: Box<JobRecord>,
    pins: Option<(&'a PortfolioPin, &'a DataPin)>,
}

struct EvaluationRun<'a> {
    store: &'a PgJobStore,
    actor: &'a Actor,
    command: &'a v1::EvaluateFactorRequest,
    job: Box<JobRecord>,
    pins: Option<(&'a EvaluationPin, &'a DataPin)>,
}

impl RuntimeService {
    pub(super) async fn run_portfolio(
        &self,
        store: &PgJobStore,
        actor: &Actor,
        command: &v1::ExecuteBacktestRequest,
        job: JobRecord,
        pins: Option<(&PortfolioPin, &DataPin)>,
    ) -> StoreResult<(JobRecord, Arc<PreparedPortfolio>)> {
        crate::store::validate_runtime_context(command.context.as_ref(), actor)?;
        self.portfolio.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let lease = command
            .lease_id
            .as_ref()
            .ok_or(StoreError::Invalid("portfolio lease"))?;
        // Fence cancelled/expired work before any expensive input reads.
        if job.state != JobState::Succeeded as i32 {
            if job.revision != command.expected_revision {
                return Err(StoreError::RevisionConflict);
            }
            crate::store::live_lease(&job, actor, &lease.value, self.authority.now()?)?;
        }
        let run = PortfolioRun {
            store,
            actor,
            command,
            job: Box::new(job),
            pins,
        };
        let inputs = Box::pin(self.portfolio_inputs(&run)).await?;
        let prepared = Box::pin(self.portfolio_result(&run, inputs)).await?;
        let completed = Box::pin(self.portfolio_complete(&run, prepared.clone())).await?;
        Ok((*completed, prepared))
    }

    pub(super) async fn run_evaluation(
        &self,
        store: &PgJobStore,
        actor: &Actor,
        command: &v1::EvaluateFactorRequest,
        job: JobRecord,
        pins: Option<(&EvaluationPin, &DataPin)>,
    ) -> StoreResult<JobRecord> {
        let executor = self.evaluator.as_ref().ok_or(StoreError::AdmissionDenied)?;
        // The permit covers every phase, including final evidence registration.
        let _permit = executor
            .permits
            .try_acquire()
            .map_err(|_| StoreError::Unavailable("factor execution capacity"))?;
        crate::store::validate_runtime_context(command.context.as_ref(), actor)?;
        let lease = command
            .lease_id
            .as_ref()
            .ok_or(StoreError::Invalid("evaluation lease"))?;
        if job.state != JobState::Succeeded as i32 {
            if job.revision != command.expected_revision {
                return Err(StoreError::RevisionConflict);
            }
            crate::store::live_lease(&job, actor, &lease.value, self.authority.now()?)?;
        }
        let run = EvaluationRun {
            store,
            actor,
            command,
            job: Box::new(job),
            pins,
        };
        let inputs = Box::pin(self.evaluation_inputs(&run)).await?;
        let evidence = Box::pin(self.evaluation_result(&run, inputs)).await?;
        let completed = Box::pin(self.evaluation_complete(&run, evidence)).await?;
        Ok(*completed)
    }
}
