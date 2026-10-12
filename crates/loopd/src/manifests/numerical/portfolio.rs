use super::*;
use loop_protocol::wire::v1::{job_outcome, job_specification};
use prost::Message;

#[derive(Serialize)]
struct PortfolioRecipe<'a> {
    schema: &'static str,
    evaluation_work: ObjectRef,
    evaluation_result: ObjectRef,
    factor_values: ObjectRef,
    execution_tape: &'a ObjectRef,
    policies: BTreeMap<String, &'a model::PolicyDocument>,
}

impl NumericalResolver {
    pub(crate) async fn portfolio(
        &self,
        frozen: &FrozenNumerical,
        request: PortfolioDerivation<'_>,
    ) -> StoreResult<DerivedPortfolio> {
        frozen.check()?;
        if request.record.state != wire::JobState::Succeeded as i32 {
            return Err(StoreError::AdmissionDenied);
        }
        let job = request
            .record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("evaluation job"))?;
        let Some(job_specification::Input::FactorEvaluation(input)) = &job.input else {
            return Err(StoreError::AdmissionDenied);
        };
        let factor = input.factor.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let ast = factor
            .expression
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        if &frozen.evaluation(
            ast,
            input.budget.clone().ok_or(StoreError::AdmissionDenied)?,
        )? != input
        {
            return Err(StoreError::Corrupt("numerical source recipe"));
        }
        let job_id = &job
            .job_id
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?
            .value;
        crate::store::validate_id(request.job_id)?;
        crate::store::validate_id(request.backtest_id)?;
        let pin = frozen.evaluation_pin(job_id);
        let prepared = self
            .evaluation
            .prepare_pin(
                job,
                request
                    .result
                    .lease_id
                    .as_ref()
                    .ok_or(StoreError::Corrupt("evaluation lease"))?,
                &pin,
            )
            .await?;
        let work = prepared.work.clone();
        let Some(job_outcome::Outcome::Success(success)) = request
            .record
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.outcome.as_ref())
        else {
            return Err(StoreError::AdmissionDenied);
        };
        let evidence = prepared.resolve(request.outputs, success).await?;
        evidence.check(request.record)?;
        if &evidence.result != request.result {
            return Err(StoreError::Corrupt("registered evaluation result"));
        }
        if request.result.eligible_observations == 0
            || u128::from(request.result.valid_observations) * 10_000
                < u128::from(request.result.eligible_observations)
                    * u128::from(evidence.minimum_coverage_bps)
        {
            return Err(StoreError::AdmissionDenied);
        }
        let mut files = frozen.files.clone();
        files.extend(evidence.guards());
        let work_ref = self
            .publish(&work.encode_to_vec(), request.replay, &mut files)
            .await?;
        let result_ref = self
            .copy_result(
                request.outputs,
                request
                    .result
                    .manifest
                    .as_ref()
                    .ok_or(StoreError::Corrupt("evaluation manifest"))?,
                request.replay,
                &mut files,
            )
            .await?;
        let values_ref = self
            .copy_result(
                request.outputs,
                request
                    .result
                    .values
                    .as_ref()
                    .ok_or(StoreError::Corrupt("evaluation values"))?,
                request.replay,
                &mut files,
            )
            .await?;
        let documents = frozen
            .recipe
            .policies
            .iter()
            .map(|(role, id)| {
                let document = frozen
                    .context
                    .policy_documents
                    .iter()
                    .find(|document| &document.policy_id == id)
                    .ok_or(StoreError::Corrupt("derived portfolio policy"))?;
                Ok((role.clone(), document))
            })
            .collect::<StoreResult<_>>()?;
        let recipe = self
            .json(
                &PortfolioRecipe {
                    schema: "loop.portfolio-request/v1",
                    evaluation_work: work_ref,
                    evaluation_result: result_ref,
                    factor_values: values_ref,
                    execution_tape: &frozen.recipe.execution_tape,
                    policies: documents,
                },
                request.replay,
                &mut files,
            )
            .await?;
        let mut loader = Materializer::for_evaluation(&self.artifacts, &[]);
        let mut configuration: model::Configuration =
            loader.json(&frozen.context.manifest.configuration).await?;
        configuration.backtest_engine_version = "authorized-portfolio.1".to_owned();
        configuration.portfolio_request = Some(recipe.clone());
        files.extend(loader.files);
        let mut context = frozen.context.manifest.clone();
        context.configuration = self
            .json(&configuration, request.replay, &mut files)
            .await?;
        let context_ref = self.json(&context, request.replay, &mut files).await?;
        let specification = self
            .publish(
                &loop_protocol::job::factor_identity_bytes(factor)?,
                request.replay,
                &mut files,
            )
            .await?;
        let expression = self
            .publish(&ast.canonical_json, request.replay, &mut files)
            .await?;
        let factor_ref = self
            .json(
                &model::Factor {
                    schema: "loop.factor-manifest/v1".to_owned(),
                    factor_spec_id: factor
                        .factor_spec_id
                        .as_ref()
                        .ok_or(StoreError::AdmissionDenied)?
                        .value
                        .clone(),
                    specification,
                    expression,
                },
                request.replay,
                &mut files,
            )
            .await?;
        let backtest_ref = self
            .json(
                &model::Backtest {
                    schema: "loop.backtest-spec/v1".to_owned(),
                    backtest_id: request.backtest_id.to_owned(),
                    context: context_ref.clone(),
                    factor: factor_ref,
                    engine: model::Engine::PrimaryCrossSectional,
                    engine_version: "authorized-portfolio.1".to_owned(),
                    sample: frozen.context.dataset.sample.clone(),
                    return_definition: "simple_nav_return".to_owned(),
                    deterministic_seed: frozen.recipe.deterministic_seed.clone(),
                },
                request.replay,
                &mut files,
            )
            .await?;
        frozen.check()?;
        evidence.check(request.record)?;
        check_files(&files)?;
        Ok(DerivedPortfolio {
            input: wire::BacktestJobInput {
                budget: Some(request.budget),
                factor_spec_id: factor.factor_spec_id.clone(),
                dataset: input.dataset.clone(),
                return_definition: wire::ReturnDefinition::SimpleNavReturn as i32,
                provenance: Some(context.provenance(*self.registry.identity().as_bytes())?),
                deterministic_seed: input.deterministic_seed.clone(),
            },
            pin: PortfolioPin {
                job_id: request.job_id.to_owned(),
                specification: backtest_ref,
                request: recipe,
                evaluation_job_id: job_id.clone(),
            },
            context: context_ref,
            files,
        })
    }
}
