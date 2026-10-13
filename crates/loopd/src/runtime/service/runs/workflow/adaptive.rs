//! Verified feedback preparation outside SQL transactions; the parent commits
//! its state and the selected next child together under the existing CAS.

use std::sync::Arc;

use super::{RuntimeService, StageContext, budget, child_id, job_id, millis, runs, success, v1};
use crate::manifests::feedback::{
    DerivedFeedback, FrozenFeedback, ObservationSource, Proposal, VerifiedObservation,
};
use crate::manifests::numerical::{FrozenNumerical, NumericalResolver, VerifiedFamily};
use crate::manifests::perturbation::{
    PerturbationEvidence, PerturbationRequest, PreparedPerturbation,
};
use crate::runtime::workflow::adaptive::{
    AdaptiveBinding, AdaptiveProof, AdaptiveProposal, FeedbackSource, SelectionSource,
};
use crate::runtime::workflow::{PreparedStage, WorkflowAuthority, WorkflowEvidence};
use crate::store::{BacktestRepository, RunSnapshot, StoreError, StoreResult};

mod execution;

pub(super) struct AdaptiveContext {
    pub(super) binding: AdaptiveBinding,
    pub(super) policy: Arc<FrozenFeedback>,
    pub(super) feedback: Arc<DerivedFeedback>,
    sources: Vec<Arc<VerifiedObservation>>,
    family: Option<Arc<VerifiedFamily>>,
    perturbation: Option<Arc<PreparedPerturbation>>,
    completed: Option<Arc<PerturbationEvidence>>,
    history: Vec<Arc<PerturbationEvidence>>,
}

impl RuntimeService {
    pub(in crate::runtime::service::runs) async fn initial_feedback(
        &self,
        specification: &runs::RunSpecification,
        resolver: &NumericalResolver,
    ) -> StoreResult<Option<AdaptiveProof>> {
        let policy = self
            .runs
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?
            .feedback(specification, resolver)
            .await?;
        let Some(policy) = policy else {
            return Ok(None);
        };
        let run = specification
            .run_id
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let feedback = Arc::new(
            resolver
                .feedback_context(&policy, &run.value, 1, vec![], false)
                .await?,
        );
        let mut binding = AdaptiveBinding {
            version: 1,
            policy: specification.feedback_policy.clone(),
            context_round: 1,
            ..Default::default()
        };
        self.freeze_feedback(specification, &feedback, &mut binding)?;
        Ok(Some(AdaptiveProof::new(
            None,
            binding,
            None,
            policy,
            vec![],
            feedback.clone(),
            feedback,
        )?))
    }

    pub(super) async fn load_feedback(
        &self,
        stage: &StageContext<'_>,
        snapshot: &RunSnapshot,
    ) -> StoreResult<Option<AdaptiveContext>> {
        let policy = self
            .runs
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?
            .feedback(stage.specification, &stage.resolver)
            .await?;
        let Some(policy) = policy else {
            return Ok(None);
        };
        let binding = snapshot
            .binding
            .as_ref()
            .and_then(|binding| binding.adaptive.as_ref())
            .ok_or(StoreError::Corrupt("missing adaptive binding"))?
            .clone();
        let mut sources = Vec::new();
        for source in &binding.sources {
            sources.push(Arc::new(
                Box::pin(self.feedback_source(stage, source)).await?,
            ));
        }
        let family = resolve_family(&stage.frozen, &policy, &binding)?;
        let mut history = Vec::new();
        for selection in &binding.completed_perturbations {
            let record = selection
                .record
                .as_ref()
                .ok_or(StoreError::Corrupt("selection history record"))?;
            let job = record
                .specification
                .as_ref()
                .ok_or(StoreError::Corrupt("selection history job"))?;
            let scope =
                WorkflowAuthority::new(self.authority.clone(), stage.specification, job, None)?;
            let actual = self
                .store
                .with_workflow_authority(Arc::new(scope))
                .runtime_job(&stage.actor, &job_id(record)?, "loop.jobs.read")
                .await?;
            if actual != *record {
                return Err(StoreError::Corrupt("selection history changed"));
            }
            let work = stage.resolver.read_work(record).await?;
            let prepared = prepare_selection(
                stage,
                &policy,
                family.as_ref(),
                &sources,
                record,
                work,
                Some(selection.round),
            )
            .await?;
            history.push(Arc::new(prepared.recover(&stage.resolver, record).await?));
        }
        let run = stage
            .specification
            .run_id
            .as_ref()
            .ok_or(StoreError::Corrupt("adaptive run ID"))?;
        let feedback = Arc::new(
            stage
                .resolver
                .feedback_context(
                    &policy,
                    &run.value,
                    binding.context_round,
                    sources
                        .iter()
                        .filter(|source| source.entry().round < binding.context_round)
                        .cloned()
                        .collect(),
                    true,
                )
                .await?,
        );
        let mut expected = binding.clone();
        self.freeze_feedback(stage.specification, &feedback, &mut expected)?;
        if expected != binding {
            return Err(StoreError::Corrupt("adaptive frozen context changed"));
        }
        let perturbation = if stage.stage == runs::RunStage::Perturbation {
            Some(
                prepare_selection(
                    stage,
                    &policy,
                    family.as_ref(),
                    &sources,
                    &snapshot.current_job,
                    binding
                        .work
                        .clone()
                        .ok_or(StoreError::Corrupt("perturbation work"))?,
                    None,
                )
                .await?,
            )
        } else {
            None
        };
        Ok(Some(AdaptiveContext {
            binding,
            policy,
            feedback,
            sources,
            family,
            perturbation,
            completed: None,
            history,
        }))
    }

    async fn feedback_source(
        &self,
        stage: &StageContext<'_>,
        source: &FeedbackSource,
    ) -> StoreResult<VerifiedObservation> {
        let evidence = WorkflowEvidence::decode(&source.evidence)?;
        if evidence.recipe != *stage.frozen.recipe()
            || evidence.data != *stage.frozen.data_manifest()
        {
            return Err(StoreError::Corrupt("feedback recipe drift"));
        }
        let record = source
            .record
            .as_ref()
            .ok_or(StoreError::Corrupt("feedback record"))?;
        let job = record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("feedback specification"))?;
        let executor = self.portfolio.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let inputs = executor
            .resolver
            .prepare_pin(
                evidence
                    .portfolio
                    .as_ref()
                    .ok_or(StoreError::Corrupt("feedback portfolio pin"))?,
            )
            .await?
            .bind_job(job)?;
        let context_id = inputs.context_id().to_owned();
        // This is an earlier registered result, not a requested numerical replay.
        // Re-resolve its complete immutable output set and current provenance;
        // launching the backtest again here would multiply work every round.
        let artifact = crate::runtime::portfolio::manifest_artifact(success(record)?)?;
        let portfolio = Arc::new(inputs.seal(&executor.outputs, &artifact).await?);
        let scope = WorkflowAuthority::new(self.authority.clone(), stage.specification, job, None)?;
        let store = self
            .store
            .with_workflow_authority(Arc::new(scope))
            .with_backtest_policy(portfolio.clone());
        let actual = store
            .current_backtest(&stage.actor, &job_id(record)?, &context_id)
            .await?;
        if source.result.as_ref() != Some(&actual) {
            return Err(StoreError::Corrupt("feedback registration changed"));
        }
        stage
            .resolver
            .observation(
                &stage.frozen,
                ObservationSource {
                    run_id: &stage
                        .specification
                        .run_id
                        .as_ref()
                        .ok_or(StoreError::AdmissionDenied)?
                        .value,
                    round: source.round,
                    before_round: source.round + 1,
                    record,
                    registered: &actual,
                    portfolio,
                    outputs: &executor.outputs,
                },
            )
            .await
    }

    fn freeze_feedback(
        &self,
        specification: &runs::RunSpecification,
        feedback: &DerivedFeedback,
        binding: &mut AdaptiveBinding,
    ) -> StoreResult<()> {
        let context = self
            .discoverer
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?
            .freeze_context(specification, feedback)?;
        binding.feedback_reference = serde_json::to_vec(feedback.reference())
            .map_err(|_| StoreError::Corrupt("feedback reference encoding"))?;
        binding.feedback_text = feedback.text().to_owned();
        binding.invocation = Some(context.invocation().clone());
        binding.tool_invocation = context.tool_invocation().cloned();
        binding.project()?;
        Ok(())
    }

    pub(super) async fn prepare_feedback(
        &self,
        stage: &mut StageContext<'_>,
        snapshot: &RunSnapshot,
        prepared: Box<PreparedStage>,
    ) -> StoreResult<Box<PreparedStage>> {
        let adaptive = stage.adaptive.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let before = adaptive.binding.clone();
        let mut current = before.clone();
        let mut sources = adaptive.sources.clone();
        let mut family = adaptive.family.clone();
        let mut history = adaptive.history.clone();
        let round = snapshot
            .view
            .research
            .as_ref()
            .ok_or(StoreError::Corrupt("adaptive round"))?
            .round;
        match stage.stage {
            runs::RunStage::Discovery if family.is_none() => {
                let Some(v1::job_specification::Input::FactorEvaluation(input)) = prepared.input()
                else {
                    return Err(StoreError::Corrupt("adaptive candidate input"));
                };
                let ast = input
                    .factor
                    .as_ref()
                    .and_then(|factor| factor.expression.as_ref())
                    .ok_or(StoreError::Corrupt("adaptive candidate AST"))?;
                if let Some(found) = stage.frozen.family(ast, &adaptive.policy)? {
                    current.family_ast = Some(ast.clone());
                    current.family_sha256 = Some(v1::Sha256Digest {
                        value: found.digest().to_vec(),
                    });
                    current.state = Some(v1::PerturbationState {
                        version: 1,
                        random_seed: found.space().random_seed.clone(),
                        ..Default::default()
                    });
                    family = Some(Arc::new(found));
                }
            }
            runs::RunStage::Evaluation => {
                if prepared
                    .source_trial()
                    .is_some_and(|trial| trial.disposition == "insufficient_coverage")
                {
                    let Some(v1::job_specification::Input::FactorEvaluation(input)) = prepared
                        .source()
                        .specification
                        .as_ref()
                        .and_then(|job| job.input.as_ref())
                    else {
                        return Err(StoreError::Corrupt("failed evaluation input"));
                    };
                    let id = input
                        .factor
                        .as_ref()
                        .and_then(|factor| factor.factor_spec_id.as_ref())
                        .ok_or(StoreError::Corrupt("failed factor ID"))?;
                    if !current.failed_factor_ids.contains(id) {
                        current.failed_factor_ids.push(id.clone());
                    }
                }
            }
            runs::RunStage::Portfolio => {
                let portfolio = prepared
                    .portfolio_proof()
                    .ok_or(StoreError::AdmissionDenied)?
                    .clone();
                let record = prepared.source();
                let job = record
                    .specification
                    .as_ref()
                    .ok_or(StoreError::Corrupt("feedback portfolio job"))?;
                let pin = stage
                    .evidence
                    .portfolio
                    .as_ref()
                    .ok_or(StoreError::Corrupt("feedback portfolio pin"))?;
                let executor = self.portfolio.as_ref().ok_or(StoreError::AdmissionDenied)?;
                let context_id = executor
                    .resolver
                    .prepare_pin(pin)
                    .await?
                    .bind_job(job)?
                    .context_id()
                    .to_owned();
                let result = stage
                    .store
                    .current_backtest(&stage.actor, &job_id(record)?, &context_id)
                    .await?;
                let observation = Arc::new(
                    stage
                        .resolver
                        .observation(
                            &stage.frozen,
                            ObservationSource {
                                run_id: &job
                                    .run_id
                                    .as_ref()
                                    .ok_or(StoreError::Corrupt("feedback run"))?
                                    .value,
                                round,
                                before_round: round + 1,
                                record,
                                registered: &result,
                                portfolio,
                                outputs: &executor.outputs,
                            },
                        )
                        .await?,
                );
                current.sources.push(FeedbackSource {
                    round,
                    record: Some(record.clone()),
                    result: Some(result),
                    evidence: snapshot
                        .binding
                        .as_ref()
                        .ok_or(StoreError::Corrupt("feedback previous binding"))?
                        .evidence
                        .clone(),
                });
                sources.push(observation);
            }
            runs::RunStage::Perturbation => {
                let completed = adaptive
                    .completed
                    .as_ref()
                    .ok_or(StoreError::AdmissionDenied)?;
                completed.check_record(prepared.source())?;
                current.step = Some(completed.step().clone());
                current.state = completed.step().state.clone();
                current.completed_perturbations.push(SelectionSource {
                    round,
                    record: Some(prepared.source().clone()),
                });
                history.push(completed.clone());
            }
            _ => {}
        }
        let can_finish = matches!(
            stage.stage,
            runs::RunStage::Discovery | runs::RunStage::Portfolio
        ) || (stage.stage == runs::RunStage::Evaluation
            && prepared.input().is_none())
            || (stage.stage == runs::RunStage::Perturbation && prepared.input().is_some());
        let (next, feedback) = if can_finish && round < snapshot.view.maximum_rounds {
            Box::pin(self.feedback_proposal(
                stage,
                snapshot,
                &current,
                family,
                sources.clone(),
                prepared.source(),
            ))
            .await?
        } else {
            (None, adaptive.feedback.clone())
        };
        let mut proof = AdaptiveProof::new(
            Some(before),
            current,
            next,
            adaptive.policy.clone(),
            sources,
            adaptive.feedback.clone(),
            feedback,
        )?;
        if let Some(completed) = &adaptive.completed {
            proof = proof.with_completion(completed.clone())?;
        }
        proof = proof.with_history(history)?;
        Ok(Box::new((*prepared).with_adaptive(proof)?))
    }

    async fn feedback_proposal(
        &self,
        stage: &StageContext<'_>,
        snapshot: &RunSnapshot,
        current: &AdaptiveBinding,
        family: Option<Arc<VerifiedFamily>>,
        sources: Vec<Arc<VerifiedObservation>>,
        source: &v1::JobRecord,
    ) -> StoreResult<(Option<AdaptiveProposal>, Arc<DerivedFeedback>)> {
        let adaptive = stage.adaptive.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let round = snapshot
            .view
            .research
            .as_ref()
            .ok_or(StoreError::Corrupt("adaptive round"))?
            .round
            + 1;
        let run = stage
            .specification
            .run_id
            .as_ref()
            .ok_or(StoreError::Corrupt("adaptive run"))?;
        let feedback = Arc::new(
            stage
                .resolver
                .feedback_context(&adaptive.policy, &run.value, round, sources.clone(), false)
                .await?,
        );
        let mut next = current.clone();
        next.context_round = round;
        next.work = None;
        self.freeze_feedback(stage.specification, &feedback, &mut next)?;
        let mut perturbation = None;
        if adaptive.policy.proposal(round)? == Proposal::Perturbation
            && let Some(family) = family
        {
            let observations = sources
                .iter()
                .map(|source| source.eligible(&family))
                .collect::<StoreResult<Vec<_>>>()?;
            let eligible: Vec<_> = observations.into_iter().flatten().collect();
            if let Some(latest) = eligible.last() {
                let state = current
                    .state
                    .as_ref()
                    .ok_or(StoreError::Corrupt("adaptive random state"))?;
                let observation = eligible
                    .iter()
                    .rev()
                    .find(|observation| {
                        !state
                            .history
                            .iter()
                            .any(|old| old.source_job_id == observation.source_job_id)
                    })
                    .cloned();
                let work = v1::PerturbationWork {
                    state: Some(state.clone()),
                    candidates: family.space().candidates.clone(),
                    failed_factor_ids: current
                        .failed_factor_ids
                        .iter()
                        .filter(|id| family.candidate(id).is_some())
                        .cloned()
                        .collect(),
                    current_window: latest
                        .candidate
                        .as_ref()
                        .ok_or(StoreError::Corrupt("adaptive observation window"))?
                        .window,
                    observation,
                };
                let reference = snapshot
                    .view
                    .research
                    .as_ref()
                    .and_then(|view| view.binding_sha256.as_ref())
                    .ok_or(StoreError::Corrupt("adaptive parent digest"))?;
                let prepared = stage
                    .resolver
                    .perturbation(
                        &stage.frozen,
                        PerturbationRequest {
                            policy: adaptive.policy.clone(),
                            family,
                            work: work.clone(),
                            sources,
                            parent_binding: reference
                                .value
                                .as_slice()
                                .try_into()
                                .map_err(|_| StoreError::Corrupt("adaptive parent digest"))?,
                            budget: budget(
                                stage
                                    .specification
                                    .numerical
                                    .as_ref()
                                    .and_then(|workflow| workflow.perturbation_budget.as_ref()),
                            )?,
                            created_at_ms: self.authority.now()?,
                            replay: false,
                        },
                    )
                    .await?;
                next.work = Some(work);
                perturbation = Some(Arc::new(prepared));
            }
        }
        let id = child_id(stage.specification, snapshot.view.revision, source);
        Ok((
            Some(AdaptiveProposal {
                binding: next,
                job_id: id,
                perturbation,
            }),
            feedback,
        ))
    }
}

async fn prepare_selection(
    stage: &StageContext<'_>,
    policy: &Arc<FrozenFeedback>,
    family: Option<&Arc<VerifiedFamily>>,
    sources: &[Arc<VerifiedObservation>],
    record: &v1::JobRecord,
    work: v1::PerturbationWork,
    round: Option<u32>,
) -> StoreResult<Arc<PreparedPerturbation>> {
    let job = record
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("selection specification"))?;
    let Some(v1::job_specification::Input::Perturbation(input)) = &job.input else {
        return Err(StoreError::Corrupt("selection input"));
    };
    let sources = sources
        .iter()
        .filter(|source| round.is_none_or(|round| source.entry().round < round))
        .cloned()
        .collect();
    let prepared = stage
        .resolver
        .perturbation(
            &stage.frozen,
            PerturbationRequest {
                policy: policy.clone(),
                family: family
                    .cloned()
                    .ok_or(StoreError::Corrupt("selection family"))?,
                work,
                sources,
                parent_binding: input
                    .parent_binding_sha256
                    .as_ref()
                    .ok_or(StoreError::Corrupt("selection parent"))?
                    .value
                    .as_slice()
                    .try_into()
                    .map_err(|_| StoreError::Corrupt("selection parent digest"))?,
                budget: input
                    .budget
                    .clone()
                    .ok_or(StoreError::Corrupt("selection budget"))?,
                created_at_ms: millis(
                    input
                        .work
                        .as_ref()
                        .and_then(|work| work.created_at.as_ref())
                        .ok_or(StoreError::Corrupt("selection creation time"))?,
                )?,
                replay: true,
            },
        )
        .await?;
    prepared.bind_job(job)?;
    Ok(Arc::new(prepared))
}

fn resolve_family(
    frozen: &FrozenNumerical,
    policy: &FrozenFeedback,
    binding: &AdaptiveBinding,
) -> StoreResult<Option<Arc<VerifiedFamily>>> {
    let Some(ast) = &binding.family_ast else {
        return Ok(None);
    };
    let family = frozen
        .family(ast, policy)?
        .ok_or(StoreError::Corrupt("adaptive family AST"))?;
    if binding
        .family_sha256
        .as_ref()
        .map(|digest| digest.value.as_slice())
        != Some(family.digest().as_slice())
    {
        return Err(StoreError::Corrupt("adaptive family changed"));
    }
    crate::store::validate_perturbation_state(
        binding
            .state
            .as_ref()
            .ok_or(StoreError::Corrupt("adaptive state"))?,
        family.space(),
    )?;
    Ok(Some(Arc::new(family)))
}
