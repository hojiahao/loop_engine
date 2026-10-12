use std::sync::Arc;
use std::time::Duration;

use super::RuntimeService;
use crate::manifests::{EvaluationPin, PortfolioPin};
use crate::runtime::DataPin;
use crate::store::{JobMutation, JobRepository, PgJobStore, StoreError, StoreResult};
use loop_protocol::wire::{
    jobs::v1,
    v1::{Actor, JobOutcome, JobRecord, JobState, JobSuccess, job_outcome},
};

impl RuntimeService {
    pub(super) async fn run_portfolio(
        &self,
        store: &PgJobStore,
        actor: &Actor,
        command: &v1::ExecuteBacktestRequest,
        job: JobRecord,
        pins: Option<(&PortfolioPin, &DataPin)>,
    ) -> StoreResult<(
        JobRecord,
        Arc<crate::manifests::portfolio::PreparedPortfolio>,
    )> {
        crate::store::validate_runtime_context(command.context.as_ref(), actor)?;
        let executor = self.portfolio.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let specification = job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("portfolio job"))?;
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
        let inputs = match pins {
            Some((pin, _)) => executor
                .resolver
                .prepare_pin(pin)
                .await?
                .bind_job(specification)?,
            None => executor.inputs(specification).await?,
        };
        let prepared = if job.state == JobState::Succeeded as i32 {
            let Some(job_outcome::Outcome::Success(success)) = job
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.outcome.as_ref())
            else {
                return Err(StoreError::Corrupt("portfolio replay outcome"));
            };
            executor
                .replay_pin(inputs, specification, success, pins.map(|(_, data)| data))
                .await?
        } else {
            let trials = store.trial_ledger(actor).await?;
            store
                .portfolio_source(actor, &job, &inputs.lineage(trials.clone()))
                .await?;
            let preparation = v1::PrepareJobArtifactsRequest {
                context: command.context.clone(),
                job_id: command.job_id.clone(),
                lease_id: command.lease_id.clone(),
                expected_revision: command.expected_revision,
            };
            let prepared = match pins {
                Some((_, pin)) => {
                    self.artifacts
                        .prepare_pin(store, actor, &preparation, &job, pin)
                        .await?
                }
                None => {
                    self.artifacts
                        .prepare(store, actor, &preparation, &job)
                        .await?
                }
            };
            let view = self.artifacts.evaluation_view(&prepared).await?;
            let current = store
                .runtime_job(
                    actor,
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
            let expires = crate::store::live_lease(&current, actor, &lease.value, now)?;
            let remaining = Duration::from_millis(
                u64::try_from(expires - now).map_err(|_| StoreError::LeaseFenced)?,
            )
            .min(Duration::from_secs(180));
            let work = inputs.work(&lease.value, trials, None);
            executor.execute(inputs, &work, &view, remaining).await?
        };
        if prepared.lease_id.as_deref() != Some(lease.value.as_str()) {
            return Err(StoreError::LeaseFenced);
        }
        let success = prepared
            .success
            .clone()
            .ok_or(StoreError::Corrupt("portfolio output set"))?;
        store
            .runtime_job(
                actor,
                &command
                    .job_id
                    .as_ref()
                    .ok_or(StoreError::Invalid("portfolio job ID"))?
                    .value,
                "loop.jobs.backtest",
            )
            .await?;
        let prepared = Arc::new(prepared);
        let completed = store
            .with_backtest_policy(prepared.clone())
            .mutate(
                actor,
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
        Ok((completed.job, prepared))
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
        let _permit = executor
            .permits
            .try_acquire()
            .map_err(|_| StoreError::Unavailable("factor execution capacity"))?;
        crate::store::validate_runtime_context(command.context.as_ref(), actor)?;
        let specification = job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("evaluation job"))?;
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
        let inputs = match pins {
            Some((pin, _)) => {
                executor
                    .resolver
                    .prepare_pin(specification, lease, pin)
                    .await?
            }
            None => executor.resolver.prepare(specification, lease).await?,
        };
        let result;
        let success = if job.state == JobState::Succeeded as i32 {
            executor
                .run(&inputs.work, None, Duration::from_secs(30))
                .await?;
            result = None;
            match job
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.outcome.as_ref())
            {
                Some(job_outcome::Outcome::Success(success)) => success.clone(),
                _ => return Err(StoreError::Corrupt("evaluation replay outcome")),
            }
        } else {
            let preparation = v1::PrepareJobArtifactsRequest {
                context: command.context.clone(),
                job_id: command.job_id.clone(),
                lease_id: command.lease_id.clone(),
                expected_revision: command.expected_revision,
            };
            let prepared = match pins {
                Some((_, pin)) => {
                    self.artifacts
                        .prepare_pin(store, actor, &preparation, &job, pin)
                        .await?
                }
                None => {
                    self.artifacts
                        .prepare(store, actor, &preparation, &job)
                        .await?
                }
            };
            let view = self.artifacts.evaluation_view(&prepared).await?;
            // Preparation consumes the original lease and job budget. Recheck
            // current state immediately before spawn; never grant a fresh
            // wall-clock budget merely because artifact verification finished.
            let current = store
                .runtime_job(
                    actor,
                    &command
                        .job_id
                        .as_ref()
                        .ok_or(StoreError::Invalid("evaluation job ID"))?
                        .value,
                    "loop.jobs.evaluate",
                )
                .await?;
            if current.revision != command.expected_revision {
                return Err(StoreError::RevisionConflict);
            }
            let now = self.authority.now()?;
            let expires = crate::store::live_lease(&current, actor, &lease.value, now)?;
            let remaining = Duration::from_millis(
                u64::try_from(expires - now).map_err(|_| StoreError::LeaseFenced)?,
            )
            .min(Duration::from_secs(60));
            let response = executor
                .run(&inputs.work, Some(&view), remaining)
                .await?
                .ok_or(StoreError::Corrupt("missing evaluation result"))?;
            let success = JobSuccess {
                outputs: vec![
                    response
                        .values
                        .clone()
                        .ok_or(StoreError::Corrupt("missing factor values"))?,
                    response
                        .manifest
                        .clone()
                        .ok_or(StoreError::Corrupt("missing factor manifest"))?,
                ],
            };
            result = Some(response);
            success
        };
        let evidence = inputs.resolve(&executor.outputs, &success).await?;
        if result
            .as_ref()
            .is_some_and(|result| result != &evidence.result)
        {
            return Err(StoreError::Corrupt("worker/manifest result mismatch"));
        }
        store
            .runtime_job(
                actor,
                &command
                    .job_id
                    .as_ref()
                    .ok_or(StoreError::Invalid("evaluation job ID"))?
                    .value,
                "loop.jobs.evaluate",
            )
            .await?;
        executor.check_output()?;
        let completed = store
            .with_evaluation_evidence(evidence)
            .mutate(
                actor,
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
        Ok(completed.job)
    }
}
