use std::sync::Arc;
use std::time::Duration;

use loop_protocol::wire::v1::{JobOutcome, JobRecord, JobState, job_outcome};

use super::{PortfolioRun, RuntimeService, v1};
use crate::manifests::portfolio::{PortfolioInputs, PreparedPortfolio};
use crate::store::{JobMutation, JobRepository, StoreError, StoreResult};

impl RuntimeService {
    pub(super) async fn portfolio_inputs(
        &self,
        run: &PortfolioRun<'_>,
    ) -> StoreResult<Box<PortfolioInputs>> {
        let executor = self.portfolio.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let specification = run
            .job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("portfolio job"))?;
        let inputs = match run.pins {
            Some((pin, _)) => executor
                .resolver
                .prepare_pin(pin)
                .await?
                .bind_job(specification)?,
            None => executor.inputs(specification).await?,
        };
        Ok(Box::new(inputs))
    }

    pub(super) async fn portfolio_result(
        &self,
        run: &PortfolioRun<'_>,
        inputs: Box<PortfolioInputs>,
    ) -> StoreResult<Arc<PreparedPortfolio>> {
        if run.job.state != JobState::Succeeded as i32 {
            return Box::pin(self.portfolio_fresh(run, inputs)).await;
        }
        let executor = self.portfolio.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let specification = run
            .job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("portfolio job"))?;
        let Some(job_outcome::Outcome::Success(success)) = run
            .job
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.outcome.as_ref())
        else {
            return Err(StoreError::Corrupt("portfolio replay outcome"));
        };
        let prepared = executor
            .replay_pin(
                *inputs,
                specification,
                success,
                run.pins.map(|(_, data)| data),
            )
            .await?;
        Ok(Arc::new(prepared))
    }

    async fn portfolio_fresh(
        &self,
        run: &PortfolioRun<'_>,
        inputs: Box<PortfolioInputs>,
    ) -> StoreResult<Arc<PreparedPortfolio>> {
        let executor = self.portfolio.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let command = run.command;
        let lease = command
            .lease_id
            .as_ref()
            .ok_or(StoreError::Invalid("portfolio lease"))?;
        let trials = run.store.trial_ledger(run.actor).await?;
        run.store
            .portfolio_source(run.actor, &run.job, &inputs.lineage(trials.clone()))
            .await?;
        let preparation = v1::PrepareJobArtifactsRequest {
            context: command.context.clone(),
            job_id: command.job_id.clone(),
            lease_id: command.lease_id.clone(),
            expected_revision: command.expected_revision,
        };
        let prepared = match run.pins {
            Some((_, pin)) => {
                self.artifacts
                    .prepare_pin(run.store, run.actor, &preparation, &run.job, pin)
                    .await?
            }
            None => {
                self.artifacts
                    .prepare(run.store, run.actor, &preparation, &run.job)
                    .await?
            }
        };
        let view = self.artifacts.evaluation_view(&prepared).await?;
        let current = run
            .store
            .runtime_job(
                run.actor,
                &command
                    .job_id
                    .as_ref()
                    .ok_or(StoreError::Invalid("portfolio job ID"))?
                    .value,
                "loop.jobs.backtest",
            )
            .await?;
        if current.revision != command.expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        let now = self.authority.now()?;
        let expires = crate::store::live_lease(&current, run.actor, &lease.value, now)?;
        let remaining = Duration::from_millis(
            u64::try_from(expires - now).map_err(|_| StoreError::LeaseFenced)?,
        )
        .min(Duration::from_secs(180));
        let work = inputs.work(&lease.value, trials, None);
        let proof = executor.execute(*inputs, &work, &view, remaining).await?;
        Ok(Arc::new(proof))
    }

    pub(super) async fn portfolio_complete(
        &self,
        run: &PortfolioRun<'_>,
        prepared: Arc<PreparedPortfolio>,
    ) -> StoreResult<Box<JobRecord>> {
        let command = run.command;
        let lease = command
            .lease_id
            .as_ref()
            .ok_or(StoreError::Invalid("portfolio lease"))?;
        if prepared.lease_id.as_deref() != Some(lease.value.as_str()) {
            return Err(StoreError::LeaseFenced);
        }
        let success = prepared
            .success
            .clone()
            .ok_or(StoreError::Corrupt("portfolio output set"))?;
        run.store
            .runtime_job(
                run.actor,
                &command
                    .job_id
                    .as_ref()
                    .ok_or(StoreError::Invalid("portfolio job ID"))?
                    .value,
                "loop.jobs.backtest",
            )
            .await?;
        let completed = run
            .store
            .with_backtest_policy(prepared)
            .mutate(
                run.actor,
                JobMutation::Complete(v1::CompleteJobRequest {
                    context: command.context.clone(),
                    job_id: command.job_id.clone(),
                    lease_id: command.lease_id.clone(),
                    expected_revision: command.expected_revision,
                    outcome: Some(JobOutcome {
                        outcome: Some(job_outcome::Outcome::Success(success)),
                    }),
                }),
            )
            .await?;
        Ok(Box::new(completed.job))
    }
}
