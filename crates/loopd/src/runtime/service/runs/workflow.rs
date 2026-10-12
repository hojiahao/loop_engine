//! Bounded numerical stages share one operation-local authority and file proof.

use std::sync::Arc;

use loop_protocol::wire::{research::v1 as research, runs::v1 as runs, v1};
use prost::Message;
use sha2::{Digest, Sha256};

use super::RuntimeService;
use crate::manifests::numerical::{FrozenNumerical, NumericalResolver};
use crate::runtime::workflow::{WorkflowAuthority, WorkflowEvidence};
use crate::store::{PgJobStore, RunSnapshot, StoreError, StoreResult};

mod execution;
mod preparation;

/// Private per-request state keeps large child futures out of the parent frame.
/// The frozen recipe and derived evidence remain live through the final commit.
struct StageContext<'a> {
    specification: &'a runs::RunSpecification,
    resolver: NumericalResolver,
    frozen: Arc<FrozenNumerical>,
    actor: v1::Actor,
    store: PgJobStore,
    evidence: WorkflowEvidence,
    stage: runs::RunStage,
}

impl RuntimeService {
    pub(in crate::runtime::service) async fn frozen_numerical(
        &self,
        specification: &runs::RunSpecification,
    ) -> StoreResult<(NumericalResolver, Arc<FrozenNumerical>)> {
        // Both installed workers must exist before a v2 run consumes any budget.
        self.evaluator.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let portfolio = self.portfolio.as_ref().ok_or(StoreError::AdmissionDenied)?;
        self.runs
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?
            .numerical(specification, &portfolio.evidence)
            .await
    }

    pub(in crate::runtime::service) async fn step_numerical(
        &self,
        owner: &v1::Actor,
        request: &runs::StepRunRequest,
        snapshot: RunSnapshot,
    ) -> StoreResult<runs::RunView> {
        let snapshot = Box::new(snapshot);
        let deadline = millis(
            snapshot
                .view
                .deadline
                .as_ref()
                .ok_or(StoreError::Corrupt("run deadline"))?,
        )?;
        let now = self.authority.now()?;
        if now >= deadline {
            return self
                .store
                .advance_numerical(owner, request, &snapshot.specification, None)
                .await;
        }
        // Include input verification and successful-child replay in the parent
        // deadline. Dropping the worker future kills its child process; receipts
        // already committed remain available to the next recovery request.
        let remaining =
            u64::try_from(deadline - now).map_err(|_| StoreError::Corrupt("run deadline range"))?;
        tokio::time::timeout(
            std::time::Duration::from_millis(remaining),
            Box::pin(self.execute_numerical(owner, request, &snapshot)),
        )
        .await
        .map_err(|_| StoreError::Unavailable("run execution deadline"))?
    }

    async fn execute_numerical(
        &self,
        owner: &v1::Actor,
        request: &runs::StepRunRequest,
        snapshot: &RunSnapshot,
    ) -> StoreResult<runs::RunView> {
        let mut context = Box::pin(self.numerical_stage(snapshot)).await?;
        let specification = context.specification;
        let deadline = millis(
            snapshot
                .view
                .deadline
                .as_ref()
                .ok_or(StoreError::Corrupt("run deadline"))?,
        )?;
        if snapshot.view.status != runs::RunStatus::Active as i32 {
            return Err(StoreError::InvalidTransition);
        }
        if self.authority.now()? >= deadline {
            return context
                .store
                .advance_numerical(owner, request, specification, None)
                .await;
        }
        let execution::CompletedChild { job, portfolio } =
            Box::pin(self.execute_child(&mut context, Box::new(snapshot.current_job.clone())))
                .await?;
        self.authority.verify_run(owner, specification)?;
        if job.state != v1::JobState::Succeeded as i32 {
            return context
                .store
                .advance_numerical(owner, request, specification, None)
                .await;
        }
        let next_id = child_id(specification, snapshot.view.revision, &job);
        let prepared = match context.stage {
            runs::RunStage::Discovery => {
                Box::pin(self.prepare_discovery(&mut context, &job, &next_id)).await?
            }
            runs::RunStage::Evaluation => {
                Box::pin(self.prepare_evaluation(&mut context, &job, &next_id)).await?
            }
            runs::RunStage::Portfolio => {
                Box::pin(self.prepare_portfolio(&mut context, &job, portfolio)).await?
            }
            _ => return Err(StoreError::Corrupt("run stage")),
        };
        let scope = WorkflowAuthority::new(
            self.authority.clone(),
            specification,
            job.specification
                .as_ref()
                .ok_or(StoreError::Corrupt("run child"))?,
            Some(&prepared),
        )?;
        context
            .store
            .with_workflow_authority(Arc::new(scope))
            .advance_numerical(owner, request, specification, Some(&prepared))
            .await
    }

    async fn numerical_stage<'a>(
        &self,
        snapshot: &'a RunSnapshot,
    ) -> StoreResult<Box<StageContext<'a>>> {
        let specification = &snapshot.specification;
        let (resolver, frozen) = self.frozen_numerical(specification).await?;
        let actor = self.authority.research_actor(specification)?;
        let current = snapshot
            .current_job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("run child"))?;
        let scope = WorkflowAuthority::new(self.authority.clone(), specification, current, None)?;
        let binding = snapshot
            .binding
            .as_ref()
            .ok_or(StoreError::Corrupt("run binding"))?;
        let evidence = WorkflowEvidence::decode(&binding.evidence)?;
        if evidence.recipe != *frozen.recipe() || evidence.data != *frozen.data_manifest() {
            return Err(StoreError::Corrupt("run recipe evidence"));
        }
        let stage = runs::RunStage::try_from(
            snapshot
                .view
                .research
                .as_ref()
                .ok_or(StoreError::Corrupt("run research view"))?
                .stage,
        )
        .map_err(|_| StoreError::Corrupt("run stage"))?;
        Ok(Box::new(StageContext {
            specification,
            resolver,
            frozen,
            actor,
            store: self.store.with_workflow_authority(Arc::new(scope)),
            evidence,
            stage,
        }))
    }
}

fn success(job: &v1::JobRecord) -> StoreResult<&v1::JobSuccess> {
    match job
        .outcome
        .as_ref()
        .and_then(|outcome| outcome.outcome.as_ref())
    {
        Some(v1::job_outcome::Outcome::Success(success)) => Ok(success),
        _ => Err(StoreError::Corrupt("numerical success")),
    }
}

fn budget(value: Option<&research::ResearchJobBudget>) -> StoreResult<v1::JobBudget> {
    let value = value.ok_or(StoreError::Corrupt("numerical budget"))?;
    Ok(v1::JobBudget {
        maximum_steps: value.maximum_steps,
        maximum_wall_time: value.maximum_wall_time,
        maximum_input_tokens: value.maximum_input_tokens,
        maximum_output_tokens: value.maximum_output_tokens,
        maximum_cost: value.maximum_cost.clone(),
    })
}

fn child_id(run: &runs::RunSpecification, revision: u64, source: &v1::JobRecord) -> String {
    let mut digest = Sha256::new();
    digest.update(b"loop.run-child/v2\0");
    digest.update(run.encode_to_vec());
    digest.update(revision.to_be_bytes());
    digest.update(source.encode_to_vec());
    format!("job.{:x}", digest.finalize())
}

fn job_id(job: &v1::JobRecord) -> StoreResult<String> {
    job.specification
        .as_ref()
        .and_then(|job| job.job_id.as_ref())
        .map(|id| id.value.clone())
        .ok_or(StoreError::Corrupt("numerical job ID"))
}

fn millis(time: &prost_types::Timestamp) -> StoreResult<i64> {
    time.seconds
        .checked_mul(1000)
        .and_then(|seconds| seconds.checked_add(i64::from(time.nanos) / 1_000_000))
        .ok_or(StoreError::Corrupt("run timestamp range"))
}

fn terminal(state: i32) -> bool {
    matches!(
        v1::JobState::try_from(state),
        Ok(v1::JobState::Succeeded
            | v1::JobState::FactorRejected
            | v1::JobState::InfrastructureFailed
            | v1::JobState::BudgetExhausted
            | v1::JobState::Cancelled)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_budget_ceiling() {
        let frozen = research::ResearchJobBudget {
            maximum_steps: 4,
            maximum_input_tokens: 123,
            maximum_output_tokens: 45,
            maximum_cost: Some(v1::Money {
                currency_code: "USD".to_owned(),
                amount: Some(v1::ExactDecimal {
                    value: "1.25".to_owned(),
                }),
            }),
            maximum_wall_time: Some(prost_types::Duration {
                seconds: 180,
                nanos: 123_000_000,
            }),
        };
        let converted = budget(Some(&frozen)).unwrap();
        assert_eq!(converted.maximum_steps, frozen.maximum_steps);
        assert_eq!(converted.maximum_input_tokens, frozen.maximum_input_tokens);
        assert_eq!(
            converted.maximum_output_tokens,
            frozen.maximum_output_tokens
        );
        assert_eq!(converted.maximum_cost, frozen.maximum_cost);
        assert_eq!(converted.maximum_wall_time, frozen.maximum_wall_time);
    }
}
