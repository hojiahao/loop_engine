use std::time::Duration;

use loop_protocol::wire::v1::{
    FactorEvaluationResult, JobOutcome, JobRecord, JobState, JobSuccess, job_outcome,
};

use super::{EvaluationRun, RuntimeService, v1};
use crate::manifests::evaluation::{EvaluationEvidence, EvaluationInputs};
use crate::store::{JobMutation, JobRepository, StoreError, StoreResult};

pub(super) struct EvaluationResult {
    evidence: EvaluationEvidence,
    success: JobSuccess,
}

impl RuntimeService {
    pub(super) async fn evaluation_inputs(
        &self,
        run: &EvaluationRun<'_>,
    ) -> StoreResult<Box<EvaluationInputs>> {
        let executor = self.evaluator.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let specification = run
            .job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("evaluation job"))?;
        let lease = run
            .command
            .lease_id
            .as_ref()
            .ok_or(StoreError::Invalid("evaluation lease"))?;
        let inputs = match run.pins {
            Some((pin, _)) => {
                executor
                    .resolver
                    .prepare_pin(specification, lease, pin)
                    .await?
            }
            None => executor.resolver.prepare(specification, lease).await?,
        };
        Ok(Box::new(inputs))
    }

    pub(super) async fn evaluation_result(
        &self,
        run: &EvaluationRun<'_>,
        inputs: Box<EvaluationInputs>,
    ) -> StoreResult<Box<EvaluationResult>> {
        let executor = self.evaluator.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let result;
        let success = if run.job.state == JobState::Succeeded as i32 {
            executor
                .run(&inputs.work, None, Duration::from_secs(30))
                .await?;
            result = None;
            match run
                .job
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.outcome.as_ref())
            {
                Some(job_outcome::Outcome::Success(success)) => success.clone(),
                _ => return Err(StoreError::Corrupt("evaluation replay outcome")),
            }
        } else {
            let response = Box::pin(self.evaluation_fresh(run, &inputs)).await?;
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
        Ok(Box::new(EvaluationResult { evidence, success }))
    }

    async fn evaluation_fresh(
        &self,
        run: &EvaluationRun<'_>,
        inputs: &EvaluationInputs,
    ) -> StoreResult<FactorEvaluationResult> {
        let executor = self.evaluator.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let command = run.command;
        let lease = command
            .lease_id
            .as_ref()
            .ok_or(StoreError::Invalid("evaluation lease"))?;
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
        // Input preparation consumes the original lease and job budget.
        let current = run
            .store
            .runtime_job(
                run.actor,
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
        let expires = crate::store::live_lease(&current, run.actor, &lease.value, now)?;
        let remaining = Duration::from_millis(
            u64::try_from(expires - now).map_err(|_| StoreError::LeaseFenced)?,
        )
        .min(Duration::from_secs(60));
        executor
            .run(&inputs.work, Some(&view), remaining)
            .await?
            .ok_or(StoreError::Corrupt("missing evaluation result"))
    }

    pub(super) async fn evaluation_complete(
        &self,
        run: &EvaluationRun<'_>,
        result: Box<EvaluationResult>,
    ) -> StoreResult<Box<JobRecord>> {
        let executor = self.evaluator.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let command = run.command;
        run.store
            .runtime_job(
                run.actor,
                &command
                    .job_id
                    .as_ref()
                    .ok_or(StoreError::Invalid("evaluation job ID"))?
                    .value,
                "loop.jobs.evaluate",
            )
            .await?;
        executor.check_output()?;
        let EvaluationResult { evidence, success } = *result;
        let completed = run
            .store
            .with_evaluation_evidence(evidence)
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
