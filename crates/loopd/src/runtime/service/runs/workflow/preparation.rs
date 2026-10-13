//! Phase-specific proofs remain owned until the parent transition commits.

use std::sync::Arc;

use prost::Message;
use sha2::{Digest, Sha256};

use super::{RuntimeService, StageContext, budget, job_id, success, v1};
use crate::manifests::numerical::PortfolioDerivation;
use crate::manifests::portfolio::PreparedPortfolio;
use crate::runtime::workflow::{PreparedStage, ProposalEvidence};
use crate::runtime::{DataPin, model_codec};
use crate::store::{StoreError, StoreResult};

impl RuntimeService {
    pub(super) async fn prepare_discovery(
        &self,
        stage: &mut StageContext<'_>,
        job: &v1::JobRecord,
        next_id: &str,
    ) -> StoreResult<Box<PreparedStage>> {
        let actor = stage
            .specification
            .executor
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let workflow = stage
            .specification
            .numerical
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let history = stage.store.model_history(actor, &job_id(job)?).await?;
        let discoverer = self
            .discoverer
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let scoped = stage
            .adaptive
            .as_ref()
            .map(|adaptive| {
                discoverer.scope_existing(
                    job.specification
                        .as_ref()
                        .ok_or(StoreError::Corrupt("adaptive source job"))?,
                    &adaptive.binding,
                    adaptive.feedback.clone(),
                )
            })
            .transpose()?;
        scoped.as_ref().unwrap_or(discoverer).view(job, &history)?;
        let step = history
            .last()
            .ok_or(StoreError::Corrupt("run candidate receipt"))?;
        let response = step
            .response
            .as_ref()
            .ok_or(StoreError::Corrupt("run candidate response"))?;
        let registry = loop_core::factor::us_equities::registry()
            .map_err(|_| StoreError::Unavailable("numerical registry"))?;
        let ast = model_codec::response_ast(&step.request, response, &registry)?;
        let input = stage
            .frozen
            .evaluation(&ast, budget(workflow.evaluation_budget.as_ref())?)?;
        stage.evidence.evaluation = Some(stage.frozen.evaluation_pin(next_id));
        stage.evidence.portfolio = None;
        stage.evidence.portfolio_context = None;
        stage.evidence.proposal = Some(ProposalEvidence {
            job_id: job_id(job)?,
            ordinal: step.ordinal,
            request_sha256: format!(
                "sha256:{}",
                step.request_sha256
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
            response_sha256: format!("sha256:{:x}", Sha256::digest(response.encode_to_vec())),
            ast_sha256: format!("sha256:{:x}", Sha256::digest(ast.encode_to_vec())),
        });
        Ok(Box::new(PreparedStage::new(
            stage.frozen.clone(),
            job.clone(),
            None,
            Some(v1::job_specification::Input::FactorEvaluation(input)),
            stage.evidence.clone(),
            None,
        )?))
    }

    pub(super) async fn prepare_evaluation(
        &self,
        stage: &mut StageContext<'_>,
        job: &v1::JobRecord,
        next_id: &str,
    ) -> StoreResult<Box<PreparedStage>> {
        let workflow = stage
            .specification
            .numerical
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let trial = stage.store.workflow_trial(&stage.actor, job).await?;
        let result = trial
            .result
            .as_ref()
            .ok_or(StoreError::Corrupt("run evaluation result"))?;
        // Always resolve actual completed files, including a coverage rejection.
        let executor = self.evaluator.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let inputs = executor
            .resolver
            .prepare_pin(
                job.specification
                    .as_ref()
                    .ok_or(StoreError::Corrupt("evaluation job"))?,
                result
                    .lease_id
                    .as_ref()
                    .ok_or(StoreError::Corrupt("evaluation lease"))?,
                stage
                    .evidence
                    .evaluation
                    .as_ref()
                    .ok_or(StoreError::Corrupt("evaluation pin"))?,
            )
            .await?;
        let verified = inputs.resolve(&executor.outputs, success(job)?).await?;
        verified.check(job)?;
        if verified.result != *result {
            return Err(StoreError::Corrupt("run trial result"));
        }
        if trial.disposition == "insufficient_coverage" {
            return Ok(Box::new(
                PreparedStage::new(
                    stage.frozen.clone(),
                    job.clone(),
                    Some(trial),
                    None,
                    stage.evidence.clone(),
                    None,
                )?
                .with_evaluation(verified)?,
            ));
        }
        let backtest_id = format!("backtest.{}", &next_id[4..]);
        let derived = Box::pin(stage.resolver.portfolio(
            &stage.frozen,
            PortfolioDerivation {
                record: job,
                result,
                budget: budget(workflow.portfolio_budget.as_ref())?,
                job_id: next_id,
                backtest_id: &backtest_id,
                outputs: &executor.outputs,
                replay: false,
            },
        ))
        .await?;
        stage.evidence.portfolio = Some(derived.pin.clone());
        stage.evidence.portfolio_context = Some(derived.context.clone());
        let pinned = self
            .portfolio
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?
            .resolver
            .prepare_pin(&derived.pin)
            .await?;
        stage.store = stage.store.with_backtest_policy(Arc::new(pinned));
        Ok(Box::new(
            PreparedStage::new(
                stage.frozen.clone(),
                job.clone(),
                Some(trial),
                Some(v1::job_specification::Input::Backtest(
                    derived.input.clone(),
                )),
                stage.evidence.clone(),
                Some(derived),
            )?
            .with_evaluation(verified)?,
        ))
    }

    pub(super) async fn prepare_portfolio(
        &self,
        stage: &mut StageContext<'_>,
        job: &v1::JobRecord,
        completed: Option<Arc<PreparedPortfolio>>,
    ) -> StoreResult<Box<PreparedStage>> {
        let pin = stage
            .evidence
            .portfolio
            .as_ref()
            .ok_or(StoreError::Corrupt("run portfolio pin"))?;
        let executor = self.portfolio.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let current = job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("portfolio job"))?;
        let proof = if let Some(proof) = completed {
            proof
        } else {
            let inputs = executor
                .resolver
                .prepare_pin(pin)
                .await?
                .bind_job(current)?;
            let data = DataPin {
                job_id: job_id(job)?,
                manifest: stage.evidence.data.clone(),
                protected: false,
            };
            Arc::new(
                Box::pin(executor.replay_pin(inputs, current, success(job)?, Some(&data))).await?,
            )
        };
        stage.store = stage.store.with_backtest_policy(proof.clone());
        Ok(Box::new(
            PreparedStage::new(
                stage.frozen.clone(),
                job.clone(),
                None,
                None,
                stage.evidence.clone(),
                None,
            )?
            .with_portfolio(proof)?,
        ))
    }
}
