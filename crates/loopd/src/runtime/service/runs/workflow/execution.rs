//! Separate child futures bound stack use without changing lease ownership.

use std::sync::Arc;

use loop_protocol::wire::jobs::v1 as jobs;

use super::{RuntimeService, StageContext, job_id, runs, terminal, v1};
use crate::manifests::portfolio::PreparedPortfolio;
use crate::runtime::DataPin;
use crate::store::{JobMutation, JobRepository, RecoveryCommand, StoreError, StoreResult};

pub(super) struct CompletedChild {
    pub(super) job: Box<v1::JobRecord>,
    pub(super) portfolio: Option<Arc<PreparedPortfolio>>,
}

impl RuntimeService {
    pub(super) async fn execute_child(
        &self,
        stage: &mut StageContext<'_>,
        job: Box<v1::JobRecord>,
    ) -> StoreResult<CompletedChild> {
        if stage.stage == runs::RunStage::Discovery {
            return Box::pin(self.discover_child(stage, job)).await;
        }
        Box::pin(self.execute_research(stage, job)).await
    }

    async fn discover_child(
        &self,
        stage: &StageContext<'_>,
        job: Box<v1::JobRecord>,
    ) -> StoreResult<CompletedChild> {
        let discoverer = self
            .discoverer
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let actor = stage
            .specification
            .executor
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let id = job_id(&job)?;
        let revision = job.revision;
        Box::pin(discoverer.execute(&stage.store, &self.authority, actor, *job, revision)).await?;
        Ok(CompletedChild {
            job: Box::new(
                stage
                    .store
                    .runtime_job(actor, &id, "loop.model.read")
                    .await?,
            ),
            portfolio: None,
        })
    }

    async fn execute_research(
        &self,
        stage: &mut StageContext<'_>,
        mut job: Box<v1::JobRecord>,
    ) -> StoreResult<CompletedChild> {
        let data = DataPin {
            job_id: job_id(&job)?,
            manifest: stage.evidence.data.clone(),
            protected: false,
        };
        if stage.stage == runs::RunStage::Portfolio {
            let pin = stage
                .evidence
                .portfolio
                .as_ref()
                .ok_or(StoreError::Corrupt("run portfolio pin"))?;
            let prepared = self
                .portfolio
                .as_ref()
                .ok_or(StoreError::AdmissionDenied)?
                .resolver
                .prepare_pin(pin)
                .await?;
            prepared.bind_job(
                job.specification
                    .as_ref()
                    .ok_or(StoreError::Corrupt("run child"))?,
            )?;
            stage.store = stage.store.with_backtest_policy(Arc::new(prepared));
        }
        if !terminal(job.state) {
            job = Box::pin(self.claim_numerical(stage, &job)).await?;
        }
        if terminal(job.state) {
            return Ok(CompletedChild {
                job,
                portfolio: None,
            });
        }
        match stage.stage {
            runs::RunStage::Evaluation => Box::pin(self.evaluate_child(stage, job, &data)).await,
            runs::RunStage::Portfolio => Box::pin(self.portfolio_child(stage, job, &data)).await,
            _ => Err(StoreError::Corrupt("numerical execution stage")),
        }
    }

    async fn evaluate_child(
        &self,
        stage: &StageContext<'_>,
        job: Box<v1::JobRecord>,
        data: &DataPin,
    ) -> StoreResult<CompletedChild> {
        let request = jobs::EvaluateFactorRequest {
            context: Some(self.numerical_context(&stage.actor, &job_id(&job)?, "complete")?),
            job_id: job
                .specification
                .as_ref()
                .and_then(|job| job.job_id.clone()),
            lease_id: Some(lease(&job)?),
            expected_revision: job.revision,
        };
        let completed = Box::pin(
            self.run_evaluation(
                &stage.store,
                &stage.actor,
                &request,
                *job,
                Some((
                    stage
                        .evidence
                        .evaluation
                        .as_ref()
                        .ok_or(StoreError::Corrupt("run evaluation pin"))?,
                    data,
                )),
            ),
        )
        .await?;
        Ok(CompletedChild {
            job: Box::new(completed),
            portfolio: None,
        })
    }

    async fn portfolio_child(
        &self,
        stage: &StageContext<'_>,
        job: Box<v1::JobRecord>,
        data: &DataPin,
    ) -> StoreResult<CompletedChild> {
        let request = jobs::ExecuteBacktestRequest {
            context: Some(self.numerical_context(&stage.actor, &job_id(&job)?, "complete")?),
            job_id: job
                .specification
                .as_ref()
                .and_then(|job| job.job_id.clone()),
            lease_id: Some(lease(&job)?),
            expected_revision: job.revision,
        };
        let (completed, proof) = Box::pin(
            self.run_portfolio(
                &stage.store,
                &stage.actor,
                &request,
                *job,
                Some((
                    stage
                        .evidence
                        .portfolio
                        .as_ref()
                        .ok_or(StoreError::Corrupt("run portfolio pin"))?,
                    data,
                )),
            ),
        )
        .await?;
        Ok(CompletedChild {
            job: Box::new(completed),
            portfolio: Some(proof),
        })
    }

    async fn claim_numerical(
        &self,
        stage: &StageContext<'_>,
        job: &v1::JobRecord,
    ) -> StoreResult<Box<v1::JobRecord>> {
        let id = job_id(job)?;
        let mutation = if job.state != v1::JobState::Queued as i32 {
            // Never borrow an in-flight handler's lease. An abandoned numerical
            // execution becomes a distinct infrastructure failure after expiry.
            JobMutation::Recover(RecoveryCommand {
                context: Some(self.numerical_context(&stage.actor, &id, "recover")?),
                job_id: Some(v1::JobId { value: id }),
                expected_revision: job.revision,
            })
        } else {
            JobMutation::Acquire(jobs::AcquireJobLeaseRequest {
                context: Some(self.numerical_context(&stage.actor, &id, "acquire")?),
                job_id: Some(v1::JobId { value: id }),
                expected_revision: job.revision,
                requested_duration: Some(prost_types::Duration {
                    seconds: if stage.stage == runs::RunStage::Portfolio {
                        240
                    } else {
                        90
                    },
                    nanos: 0,
                }),
            })
        };
        Ok(Box::new(
            stage.store.mutate(&stage.actor, mutation).await?.job,
        ))
    }

    fn numerical_context(
        &self,
        actor: &v1::Actor,
        job: &str,
        operation: &str,
    ) -> StoreResult<v1::CommandContext> {
        let now = self.authority.now()?;
        Ok(v1::CommandContext {
            request_id: Some(v1::RequestId {
                value: uuid::Uuid::new_v4().to_string(),
            }),
            correlation_id: Some(v1::CorrelationId {
                value: job.to_owned(),
            }),
            causation_id: Some(v1::CausationId {
                value: job.to_owned(),
            }),
            idempotency_key: Some(v1::IdempotencyKey {
                value: format!("{job}.{operation}"),
            }),
            actor: Some(actor.clone()),
            requested_at: Some(prost_types::Timestamp {
                seconds: now / 1000,
                nanos: ((now % 1000) * 1_000_000) as i32,
            }),
        })
    }
}

fn lease(job: &v1::JobRecord) -> StoreResult<v1::LeaseId> {
    job.active_lease
        .as_ref()
        .and_then(|lease| lease.lease_id.clone())
        .ok_or(StoreError::LeaseFenced)
}
