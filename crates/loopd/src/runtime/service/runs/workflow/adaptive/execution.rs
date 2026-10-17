//! One leased numerical proposal. Cancellation drops the supervised Python
//! future; immutable completion is verified before the parent consumes it.

use std::sync::Arc;
use std::time::Duration;

use loop_protocol::wire::jobs::v1 as jobs;

use super::super::execution::CompletedChild;
use super::*;
use crate::manifests::perturbation::PerturbationCompletion;
use crate::research_worker::{PerturbationWorker, PythonPerturber};
use crate::store::{JobMutation, JobRepository};

impl RuntimeService {
    pub(in crate::runtime::service::runs::workflow) async fn perturb_child(
        &self,
        stage: &mut StageContext<'_>,
        mut job: Box<v1::JobRecord>,
    ) -> StoreResult<CompletedChild> {
        let prepared = stage
            .adaptive
            .as_ref()
            .and_then(|state| state.perturbation.clone())
            .ok_or(StoreError::AdmissionDenied)?;
        if !super::super::terminal(job.state) {
            job = Box::pin(self.claim_numerical(stage, &job)).await?;
        }
        if job.state == v1::JobState::Succeeded as i32 {
            let proof = Arc::new(prepared.recover(&stage.resolver, &job).await?);
            stage
                .store
                .with_perturbation_proof(proof.clone())
                .current_perturbation(&stage.actor, &proof)
                .await?;
            return Ok(CompletedChild {
                job,
                portfolio: None,
                perturbation: Some(proof),
            });
        }
        if super::super::terminal(job.state) {
            return Ok(CompletedChild {
                job,
                portfolio: None,
                perturbation: None,
            });
        }
        let executor = self.portfolio.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let worker = PythonPerturber::verified_evaluation(&executor.python, prepared.build())?;
        let result = Box::pin(self.guarded_perturbation(
            stage,
            &job,
            worker.advance(prepared.work().clone()),
        ))
        .await;
        let proof = match result {
            Ok(step) => Arc::new(
                prepared
                    .seal(
                        &stage.resolver,
                        PerturbationCompletion {
                            record: (*job).clone(),
                            step,
                            build: worker.build_identity().ok_or(StoreError::AdmissionDenied)?,
                            started_at_ms: millis(
                                job.active_lease
                                    .as_ref()
                                    .and_then(|lease| lease.issued_at.as_ref())
                                    .ok_or(StoreError::LeaseFenced)?,
                            )?,
                            completed_at_ms: self.authority.now()?,
                        },
                    )
                    .await?,
            ),
            Err(error) => {
                // An invalid, unavailable or timed-out worker is infrastructure
                // failure. Never teach the search that the factor was rejected.
                Box::pin(self.check_perturbation(stage, &job)).await?;
                let now = self.authority.now()?;
                let failure = v1::InfrastructureFailure {
                    error: Some(v1::ServiceError {
                        category: v1::ErrorCategory::Dependency as i32,
                        code: "perturbation_worker_failed".into(),
                        message: "Installed numerical proposal could not be verified".into(),
                        retryable: false,
                        details: vec![],
                    }),
                    attempt: job.attempt,
                    failed_at: Some(prost_types::Timestamp {
                        seconds: now / 1000,
                        nanos: ((now % 1000) * 1_000_000) as i32,
                    }),
                };
                let completed = Box::pin(self.complete_perturbation(
                    stage,
                    &job,
                    v1::job_outcome::Outcome::InfrastructureFailure(failure),
                ))
                .await?;
                tracing::warn!(error = %error, "numerical proposal failed closed");
                return Ok(CompletedChild {
                    job: Box::new(completed),
                    portfolio: None,
                    perturbation: None,
                });
            }
        };
        Box::pin(self.check_perturbation(stage, &job)).await?;
        stage.store = stage.store.with_perturbation_proof(proof.clone());
        let completed = Box::pin(self.complete_perturbation(
            stage,
            &job,
            v1::job_outcome::Outcome::Success(proof.success()?),
        ))
        .await?;
        Ok(CompletedChild {
            job: Box::new(completed),
            portfolio: None,
            perturbation: Some(proof),
        })
    }

    async fn complete_perturbation(
        &self,
        stage: &StageContext<'_>,
        job: &v1::JobRecord,
        outcome: v1::job_outcome::Outcome,
    ) -> StoreResult<v1::JobRecord> {
        let id = job_id(job)?;
        let result = stage
            .store
            .mutate(
                &stage.actor,
                JobMutation::Complete(jobs::CompleteJobRequest {
                    context: Some(self.numerical_context(&stage.actor, &id, "complete")?),
                    job_id: Some(v1::JobId { value: id }),
                    lease_id: job
                        .active_lease
                        .as_ref()
                        .and_then(|lease| lease.lease_id.clone()),
                    expected_revision: job.revision,
                    outcome: Some(v1::JobOutcome {
                        outcome: Some(outcome),
                    }),
                }),
            )
            .await?;
        Ok(result.job)
    }

    async fn guarded_perturbation<T>(
        &self,
        stage: &StageContext<'_>,
        job: &v1::JobRecord,
        operation: impl std::future::Future<Output = StoreResult<T>>,
    ) -> StoreResult<T> {
        Box::pin(self.check_perturbation(stage, job)).await?;
        tokio::pin!(operation);
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            tokio::select! {
                result = &mut operation => return result,
                _ = interval.tick() => Box::pin(self.check_perturbation(stage, job)).await?,
            }
        }
    }

    async fn check_perturbation(
        &self,
        stage: &StageContext<'_>,
        job: &v1::JobRecord,
    ) -> StoreResult<()> {
        let current = tokio::time::timeout(
            Duration::from_secs(1),
            stage
                .store
                .runtime_job(&stage.actor, &job_id(job)?, "loop.jobs.read"),
        )
        .await
        .map_err(|_| StoreError::Unavailable("perturbation control check"))??;
        if current != *job {
            return Err(StoreError::LeaseFenced);
        }
        let lease = job
            .active_lease
            .as_ref()
            .and_then(|lease| lease.lease_id.as_ref())
            .ok_or(StoreError::LeaseFenced)?;
        crate::store::live_lease(job, &stage.actor, &lease.value, self.authority.now()?)?;
        stage
            .adaptive
            .as_ref()
            .and_then(|adaptive| adaptive.perturbation.as_ref())
            .ok_or(StoreError::AdmissionDenied)?
            .check()
    }

    pub(in crate::runtime::service::runs::workflow) async fn prepare_perturbation(
        &self,
        stage: &mut StageContext<'_>,
        job: &v1::JobRecord,
        next_id: &str,
        completed: Option<Arc<PerturbationEvidence>>,
    ) -> StoreResult<Box<PreparedStage>> {
        let proof = completed.ok_or(StoreError::AdmissionDenied)?;
        proof.check_record(job)?;
        let adaptive = stage.adaptive.as_mut().ok_or(StoreError::AdmissionDenied)?;
        let input = if let Some(candidate) = &proof.step().candidate {
            let family = adaptive
                .family
                .as_ref()
                .ok_or(StoreError::Corrupt("perturbation family"))?;
            let ast = family.ast(candidate)?;
            let input = stage.frozen.evaluation(
                ast,
                budget(
                    stage
                        .specification
                        .numerical
                        .as_ref()
                        .and_then(|workflow| workflow.evaluation_budget.as_ref()),
                )?,
            )?;
            stage.evidence.evaluation = Some(stage.frozen.evaluation_pin(next_id));
            Some(v1::job_specification::Input::FactorEvaluation(input))
        } else {
            stage.evidence.evaluation = None;
            None
        };
        stage.evidence.proposal = None;
        stage.evidence.portfolio = None;
        stage.evidence.portfolio_context = None;
        adaptive.completed = Some(proof);
        Ok(Box::new(PreparedStage::new(
            stage.frozen.clone(),
            job.clone(),
            None,
            input,
            stage.evidence.clone(),
            None,
        )?))
    }
}
