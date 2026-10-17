//! Immutable numerical evidence and bounded reconstruction; jobs remain authoritative.

use std::collections::{BTreeMap, BTreeSet};

use loop_protocol::wire::runs::v1::{
    RoundDisposition, RunSpecification, RunStage, RunStatus, RunView,
};
use loop_protocol::wire::v1::{
    BacktestResult, JobRecord, JobSpecification, JobState, Sha256Digest, job_specification,
};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use super::super::postgres::{
    budget, deadline_millis, record_from_row, timestamp_millis, verified_blob,
};
use super::super::{EvaluationTrial, PgJobStore, StoreError, StoreResult, evaluation, model_money};
use super::{RunSnapshot, storage};
use crate::runtime::workflow::adaptive::AdaptiveBinding;

/// Internal evidence, never returned by the role RPC projection. The projection
/// omits its own binding digest, avoiding a recursive content identity.
#[derive(Clone, PartialEq, Message)]
pub(crate) struct RunBinding {
    /// Versioned internal encoding.
    #[prost(string, tag = "1")]
    pub schema: String,
    /// Frozen parent specification digest.
    #[prost(bytes = "vec", tag = "2")]
    pub specification_sha256: Vec<u8>,
    /// Previous transition's digest, empty only at creation.
    #[prost(bytes = "vec", tag = "3")]
    pub previous_sha256: Vec<u8>,
    /// Committed projection with its binding digest omitted.
    #[prost(message, optional, tag = "4")]
    pub projection: Option<RunView>,
    /// Exact durable child identity including submission time and causation.
    #[prost(message, optional, tag = "5")]
    pub current_specification: Option<JobSpecification>,
    /// Whether this transaction reserved and inserted a child.
    #[prost(bool, tag = "6")]
    pub created_child: bool,
    /// Verified predecessor observation used to decide this transition.
    #[prost(message, optional, tag = "7")]
    pub source_record: Option<JobRecord>,
    /// Verified numerical trial, when the predecessor is an evaluation.
    #[prost(message, optional, tag = "8")]
    pub source_trial: Option<EvaluationTrial>,
    /// Verified matching historical evaluation for a pre-screen outcome.
    #[prost(message, optional, tag = "9")]
    pub previous_record: Option<JobRecord>,
    /// Matching historical trial; not a newly performed evaluation.
    #[prost(message, optional, tag = "10")]
    pub previous_trial: Option<EvaluationTrial>,
    /// Domain-separated frozen evaluation context of the historical match.
    #[prost(bytes = "vec", tag = "11")]
    pub previous_context: Vec<u8>,
    /// Server-prepared manifest references with complete object identities.
    #[prost(bytes = "vec", tag = "12")]
    pub evidence: Vec<u8>,
    /// Exact candidate input used in duplicate lookup, never an inserted job.
    #[prost(message, optional, tag = "13")]
    pub attempted_specification: Option<JobSpecification>,
    /// Actual registered portfolio result; completion alone is insufficient.
    #[prost(message, optional, tag = "14")]
    pub source_backtest: Option<BacktestResult>,
    /// v3-only feedback and deterministic proposal state; absent preserves v2 bytes.
    #[prost(message, optional, tag = "15")]
    pub adaptive: Option<AdaptiveBinding>,
}

fn transition_schema(specification: &RunSpecification) -> &'static str {
    if specification.feedback_policy.is_some() {
        "loop.run-transition/v2"
    } else {
        "loop.run-transition/v1"
    }
}

fn check_artifact(
    artifact: Option<&loop_protocol::wire::v1::ArtifactRef>,
    schema: &str,
    bytes: &[u8],
) -> StoreResult<()> {
    let artifact = artifact.ok_or(StoreError::Corrupt("adaptive result artifact"))?;
    loop_protocol::artifact::validate_artifact_ref(artifact)
        .map_err(|_| StoreError::Corrupt("adaptive artifact contract"))?;
    if artifact.byte_size != bytes.len() as u64
        || artifact.sha256.as_ref().map(|value| value.value.as_slice())
            != Some(Sha256::digest(bytes).as_slice())
        || artifact
            .schema
            .as_ref()
            .is_none_or(|value| value.name != schema || value.version != 1)
        || artifact.media_type != "application/x-protobuf"
    {
        return Err(StoreError::Corrupt("adaptive artifact identity"));
    }
    Ok(())
}

/// Validate one v3 delta against its exact committed predecessor and actual
/// source observation. This grants no file or execution authority; writers call
/// it after sealing, while readers replay it over the bounded transition chain.
pub(super) fn check_adaptive(
    specification: &RunSpecification,
    previous: Option<&RunBinding>,
    view: &RunView,
    binding: &RunBinding,
) -> StoreResult<()> {
    let Some(policy) = &specification.feedback_policy else {
        if binding.adaptive.is_some() || view.adaptive.is_some() {
            return Err(StoreError::Corrupt("unexpected adaptive state"));
        }
        return Ok(());
    };
    let current = binding
        .adaptive
        .as_ref()
        .ok_or(StoreError::Corrupt("missing adaptive state"))?;
    if current.policy.as_ref() != Some(policy)
        || view.adaptive.as_ref() != Some(&current.project()?)
    {
        return Err(StoreError::Corrupt("adaptive policy binding"));
    }
    let Some(previous) = previous else {
        if current.context_round != 1
            || !current.sources.is_empty()
            || current.family_ast.is_some()
            || current.state.is_some()
            || current.work.is_some()
            || current.step.is_some()
            || !current.failed_factor_ids.is_empty()
            || !current.completed_perturbations.is_empty()
        {
            return Err(StoreError::Corrupt("adaptive initial state"));
        }
        return Ok(());
    };
    let before = previous
        .adaptive
        .as_ref()
        .ok_or(StoreError::Corrupt("adaptive ancestry"))?;
    let prior_view = previous
        .projection
        .as_ref()
        .ok_or(StoreError::Corrupt("adaptive prior projection"))?;
    let prior = prior_view
        .research
        .as_ref()
        .ok_or(StoreError::Corrupt("adaptive prior stage"))?;
    let source = binding
        .source_record
        .as_ref()
        .ok_or(StoreError::Corrupt("adaptive source"))?;
    let expired = timestamp_millis(
        view.updated_at
            .as_ref()
            .ok_or(StoreError::Corrupt("adaptive update"))?,
        true,
    )? >= timestamp_millis(
        view.deadline
            .as_ref()
            .ok_or(StoreError::Corrupt("adaptive deadline"))?,
        true,
    )?;
    if source.state != JobState::Succeeded as i32 || expired {
        if current != before {
            return Err(StoreError::Corrupt("adaptive failed advancement"));
        }
        return Ok(());
    }
    check_feedback(before, current, previous, binding)?;
    check_completions(before, current, prior.stage, prior.round, source)?;
    check_family(before, current, prior.stage, binding)?;
    if prior.stage == RunStage::Perturbation as i32 {
        check_selection(before, current, source)?;
    } else if before.state.is_some() && current.state != before.state {
        return Err(StoreError::Corrupt("unexplained adaptive random state"));
    } else if current.step != before.step {
        return Err(StoreError::Corrupt("unexplained adaptive selection"));
    }
    let planned_round = view.completed_rounds > prior_view.completed_rounds
        && view.completed_rounds < view.maximum_rounds
        && view.status != RunStatus::SearchExhausted as i32;
    let context_round = if planned_round {
        prior.round + 1
    } else {
        before.context_round
    };
    if current.context_round != context_round {
        return Err(StoreError::Corrupt("adaptive context round"));
    }
    if !planned_round
        && (current.feedback_reference != before.feedback_reference
            || current.feedback_text != before.feedback_text
            || current.invocation != before.invocation
            || current.tool_invocation != before.tool_invocation
            || current.work != before.work)
    {
        return Err(StoreError::Corrupt("adaptive context changed within round"));
    }
    if planned_round && let Some(work) = &current.work {
        check_work(current, work)?;
    }
    Ok(())
}

fn check_completions(
    before: &AdaptiveBinding,
    current: &AdaptiveBinding,
    stage: i32,
    round: u32,
    source: &JobRecord,
) -> StoreResult<()> {
    if current.completed_perturbations.len() > 63
        || !current
            .completed_perturbations
            .starts_with(&before.completed_perturbations)
    {
        return Err(StoreError::Corrupt("adaptive completion history"));
    }
    let appended = &current.completed_perturbations[before.completed_perturbations.len()..];
    if stage == RunStage::Perturbation as i32 {
        if source.state != JobState::Succeeded as i32
            || appended.len() != 1
            || appended[0].round != round
            || appended[0].record.as_ref() != Some(source)
            || before
                .completed_perturbations
                .last()
                .is_some_and(|last| last.round >= round)
        {
            return Err(StoreError::Corrupt("adaptive completion append"));
        }
    } else if !appended.is_empty() {
        return Err(StoreError::Corrupt("adaptive completion without proposal"));
    }
    Ok(())
}

fn check_feedback(
    before: &AdaptiveBinding,
    current: &AdaptiveBinding,
    previous: &RunBinding,
    binding: &RunBinding,
) -> StoreResult<()> {
    if !current.sources.starts_with(&before.sources) || current.sources.len() > 64 {
        return Err(StoreError::Corrupt("adaptive source history"));
    }
    let appended = &current.sources[before.sources.len()..];
    if let Some(result) = &binding.source_backtest {
        let source = binding
            .source_record
            .as_ref()
            .ok_or(StoreError::Corrupt("adaptive source record"))?;
        let round = previous
            .projection
            .as_ref()
            .and_then(|view| view.research.as_ref())
            .ok_or(StoreError::Corrupt("adaptive source round"))?
            .round;
        if appended.len() != 1
            || appended[0].round != round
            || appended[0].record.as_ref() != Some(source)
            || appended[0].result.as_ref() != Some(result)
            || appended[0].evidence != previous.evidence
            || before
                .sources
                .last()
                .is_some_and(|last| last.round >= round)
        {
            return Err(StoreError::Corrupt("adaptive source append"));
        }
    } else if !appended.is_empty() {
        return Err(StoreError::Corrupt("adaptive source without portfolio"));
    }
    let mut failed = before.failed_factor_ids.clone();
    if binding
        .source_trial
        .as_ref()
        .is_some_and(|trial| trial.disposition == "insufficient_coverage")
    {
        let Some(job_specification::Input::FactorEvaluation(input)) = binding
            .source_record
            .as_ref()
            .and_then(|record| record.specification.as_ref())
            .and_then(|job| job.input.as_ref())
        else {
            return Err(StoreError::Corrupt("adaptive rejection source"));
        };
        let factor = input
            .factor
            .as_ref()
            .and_then(|factor| factor.factor_spec_id.as_ref())
            .ok_or(StoreError::Corrupt("adaptive rejected factor"))?;
        if !failed.contains(factor) {
            failed.push(factor.clone());
        }
    }
    if current.failed_factor_ids != failed || failed.len() > 64 {
        return Err(StoreError::Corrupt("adaptive failure history"));
    }
    Ok(())
}

fn check_family(
    before: &AdaptiveBinding,
    current: &AdaptiveBinding,
    stage: i32,
    binding: &RunBinding,
) -> StoreResult<()> {
    if before.family_ast.is_some() {
        if current.family_ast != before.family_ast || current.family_sha256 != before.family_sha256
        {
            return Err(StoreError::Corrupt("adaptive family changed"));
        }
    } else if let Some(ast) = &current.family_ast {
        let candidate = binding
            .attempted_specification
            .as_ref()
            .or(binding.current_specification.as_ref());
        let Some(job_specification::Input::FactorEvaluation(input)) =
            candidate.and_then(|job| job.input.as_ref())
        else {
            return Err(StoreError::Corrupt("adaptive family candidate"));
        };
        let state = current
            .state
            .as_ref()
            .ok_or(StoreError::Corrupt("adaptive family state"))?;
        if stage != RunStage::Discovery as i32
            || input
                .factor
                .as_ref()
                .and_then(|factor| factor.expression.as_ref())
                != Some(ast)
            || state.version != 1
            || state
                .random_seed
                .as_ref()
                .is_none_or(|seed| seed.value.len() != 32)
            || state.random_draws != 0
            || !state.history.is_empty()
            || !state.proposed_factor_ids.is_empty()
            || state.momentum != 0.0
            || state.second_moment != 0.0
        {
            return Err(StoreError::Corrupt("adaptive family initialization"));
        }
    }
    Ok(())
}

fn check_work(
    current: &AdaptiveBinding,
    work: &loop_protocol::wire::v1::PerturbationWork,
) -> StoreResult<()> {
    if work.state != current.state || work.candidates.len() < 2 || work.candidates.len() > 64 {
        return Err(StoreError::Corrupt("adaptive work state"));
    }
    let state = work
        .state
        .as_ref()
        .ok_or(StoreError::Corrupt("adaptive work state"))?;
    if (work.observation.is_none() && state.history.is_empty())
        || work.observation.as_ref().is_some_and(|observation| {
            state
                .history
                .iter()
                .any(|prior| prior.source_job_id == observation.source_job_id)
        })
    {
        return Err(StoreError::Corrupt("adaptive work observation history"));
    }
    let space = loop_protocol::wire::v1::PerturbationSpace {
        random_seed: state.random_seed.clone(),
        candidates: work.candidates.clone(),
        ..Default::default()
    };
    super::super::validate_perturbation_state(state, &space)?;
    let mut ids = BTreeSet::new();
    let mut previous = 0;
    for candidate in &work.candidates {
        let id = candidate
            .factor_spec_id
            .as_ref()
            .ok_or(StoreError::Corrupt("adaptive candidate ID"))?;
        loop_core::factor::FactorSpecId::parse(&id.value)
            .map_err(|_| StoreError::Corrupt("adaptive candidate ID"))?;
        if candidate.window <= previous || candidate.window > 4096 || !ids.insert(&id.value) {
            return Err(StoreError::Corrupt("adaptive candidate order"));
        }
        previous = candidate.window;
    }
    if !work
        .candidates
        .iter()
        .any(|candidate| candidate.window == work.current_window)
    {
        return Err(StoreError::Corrupt("adaptive current window"));
    }
    let expected = current
        .failed_factor_ids
        .iter()
        .filter(|id| {
            work.candidates
                .iter()
                .any(|candidate| candidate.factor_spec_id.as_ref() == Some(*id))
        })
        .map(|id| id.value.as_str())
        .collect::<BTreeSet<_>>();
    let actual = work
        .failed_factor_ids
        .iter()
        .map(|id| id.value.as_str())
        .collect::<BTreeSet<_>>();
    if expected != actual || actual.len() != work.failed_factor_ids.len() {
        return Err(StoreError::Corrupt("adaptive work failure memory"));
    }
    let observed: BTreeSet<_> = current
        .sources
        .iter()
        .filter_map(|source| {
            let job = source.record.as_ref()?.specification.as_ref()?;
            let Some(job_specification::Input::Backtest(input)) = &job.input else {
                return None;
            };
            input.factor_spec_id.as_ref().map(|id| id.value.as_str())
        })
        .collect();
    let expected_observed: Vec<_> = work
        .candidates
        .iter()
        .filter_map(|candidate| candidate.factor_spec_id.as_ref())
        .filter(|id| observed.contains(id.value.as_str()))
        .cloned()
        .collect();
    if work.observed_factor_ids != expected_observed {
        return Err(StoreError::Corrupt("adaptive work observation memory"));
    }
    for observation in work
        .state
        .iter()
        .flat_map(|state| &state.history)
        .chain(work.observation.iter())
    {
        let source = current.sources.iter().find(|source| source.record.as_ref()
            .and_then(|record| record.specification.as_ref()).is_some_and(|job|
                job.job_id == observation.source_job_id && matches!(&job.input,
                    Some(job_specification::Input::Backtest(input))
                        if input.factor_spec_id == observation.candidate.as_ref().and_then(|candidate| candidate.factor_spec_id.clone()))));
        let source = source.ok_or(StoreError::Corrupt("adaptive work observation source"))?;
        let metrics = source
            .result
            .as_ref()
            .ok_or(StoreError::Corrupt("adaptive observation result"))?
            .metrics
            .iter()
            .filter(|metric| metric.name == "sharpe_annualized")
            .collect::<Vec<_>>();
        if !observation.net_sharpe.is_finite()
            || observation.net_sharpe.abs() > 1_000_000.0
            || observation
                .candidate
                .as_ref()
                .is_none_or(|candidate| !work.candidates.contains(candidate))
            || metrics.len() != 1
            || metrics[0].unit != "ratio"
            || metrics[0].estimator != "daily-statistics.1"
            || metrics[0]
                .value
                .as_ref()
                .and_then(|value| value.value.parse::<f64>().ok())
                != Some(observation.net_sharpe)
        {
            return Err(StoreError::Corrupt("adaptive work observation value"));
        }
    }
    Ok(())
}

fn check_selection(
    before: &AdaptiveBinding,
    current: &AdaptiveBinding,
    source: &JobRecord,
) -> StoreResult<()> {
    use loop_protocol::wire::v1::{PerturbationSpace, job_outcome};
    let work = before
        .work
        .as_ref()
        .ok_or(StoreError::Corrupt("adaptive consumed work"))?;
    let step = current
        .step
        .as_ref()
        .ok_or(StoreError::Corrupt("adaptive selected result"))?;
    if work.state != before.state || step.state != current.state {
        return Err(StoreError::Corrupt("adaptive selected state"));
    }
    check_work(before, work)?;
    let space = PerturbationSpace {
        random_seed: before
            .state
            .as_ref()
            .and_then(|state| state.random_seed.clone()),
        candidates: work.candidates.clone(),
        ..Default::default()
    };
    super::super::validate_perturbation(work, &space, step)?;
    let Some(job_specification::Input::Perturbation(input)) = source
        .specification
        .as_ref()
        .and_then(|job| job.input.as_ref())
    else {
        return Err(StoreError::Corrupt("adaptive selection job"));
    };
    check_artifact(
        input.work.as_ref(),
        "loop.perturbation_work",
        &work.encode_to_vec(),
    )?;
    let Some(job_outcome::Outcome::Success(success)) = source
        .outcome
        .as_ref()
        .and_then(|outcome| outcome.outcome.as_ref())
    else {
        return Err(StoreError::Corrupt("adaptive selection completion"));
    };
    let outputs = success
        .outputs
        .iter()
        .filter(|artifact| {
            artifact.schema.as_ref().is_some_and(|schema| {
                schema.name == "loop.perturbation_step" && schema.version == 1
            })
        })
        .collect::<Vec<_>>();
    if outputs.len() != 1 {
        return Err(StoreError::Corrupt("adaptive selection output"));
    }
    check_artifact(
        outputs.first().copied(),
        "loop.perturbation_step",
        &step.encode_to_vec(),
    )
}

pub(super) fn seal(
    specification: &RunSpecification,
    view: &mut RunView,
    binding: &mut RunBinding,
) -> StoreResult<Vec<u8>> {
    binding.schema = transition_schema(specification).into();
    if specification.feedback_policy.is_some() != binding.adaptive.is_some() {
        return Err(StoreError::Corrupt("adaptive binding presence"));
    }
    view.adaptive = binding
        .adaptive
        .as_ref()
        .map(AdaptiveBinding::project)
        .transpose()?;
    binding.specification_sha256 = Sha256::digest(specification.encode_to_vec()).to_vec();
    let mut projection = view.clone();
    projection.plan_verified = false;
    projection
        .research
        .as_mut()
        .ok_or(StoreError::Corrupt("numerical projection"))?
        .binding_sha256 = None;
    binding.projection = Some(projection);
    let bytes = binding.encode_to_vec();
    if bytes.len() > 4 * 1024 * 1024 || binding.evidence.is_empty() {
        return Err(StoreError::Invalid("run evidence size"));
    }
    view.research
        .as_mut()
        .expect("checked projection")
        .binding_sha256 = Some(Sha256Digest {
        value: Sha256::digest(&bytes).to_vec(),
    });
    loop_protocol::runs::validate_view(view)?;
    Ok(bytes)
}

pub(super) async fn insert(
    transaction: &mut Transaction<'_, Postgres>,
    view: &RunView,
    binding: &RunBinding,
    bytes: &[u8],
) -> StoreResult<()> {
    let research = view
        .research
        .as_ref()
        .ok_or(StoreError::Corrupt("numerical projection"))?;
    sqlx::query("INSERT INTO run_transitions (run_id,revision,current_job_id,round,stage,created_child,binding_blob,binding_sha256) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(&view.run_id.as_ref().expect("validated run").value)
        .bind(view.revision as i64).bind(storage::current_id(view)?)
        .bind(research.round as i32).bind(research.stage).bind(binding.created_child)
        .bind(bytes).bind(Sha256::digest(bytes).as_slice())
        .execute(&mut **transaction).await?;
    Ok(())
}

fn decode(
    row: &PgRow,
    specification: &RunSpecification,
) -> StoreResult<(Box<RunBinding>, RunView)> {
    let bytes = verified_blob(row, "binding_blob", "binding_sha256")?;
    // Decode directly into the heap allocation so complete historical records
    // are not copied through this return value and every awaiting run loader.
    let mut binding = Box::<RunBinding>::default();
    binding
        .merge(bytes.as_slice())
        .map_err(|_| StoreError::Corrupt("run transition"))?;
    let mut view = binding
        .projection
        .clone()
        .ok_or(StoreError::Corrupt("run transition projection"))?;
    let research = view
        .research
        .as_mut()
        .ok_or(StoreError::Corrupt("run transition workflow"))?;
    if binding.encode_to_vec() != bytes
        || binding.schema != transition_schema(specification)
        || binding.specification_sha256 != Sha256::digest(specification.encode_to_vec()).as_slice()
        || research.binding_sha256.is_some()
        || view.plan_verified
        || binding.evidence.is_empty()
        || row.try_get::<String, _>("run_id")?
            != specification.run_id.as_ref().expect("validated run").value
        || row.try_get::<i64, _>("revision")? != view.revision as i64
        || row.try_get::<i32, _>("round")? != research.round as i32
        || row.try_get::<i32, _>("stage")? != research.stage
        || row.try_get::<bool, _>("created_child")? != binding.created_child
    {
        return Err(StoreError::Corrupt("run transition binding"));
    }
    research.binding_sha256 = Some(Sha256Digest {
        value: Sha256::digest(&bytes).to_vec(),
    });
    loop_protocol::runs::validate_view(&view)
        .map_err(|_| StoreError::Corrupt("transition contract"))?;
    if specification.feedback_policy.is_some() != binding.adaptive.is_some()
        || view.adaptive
            != binding
                .adaptive
                .as_ref()
                .map(AdaptiveBinding::project)
                .transpose()?
    {
        return Err(StoreError::Corrupt("adaptive transition projection"));
    }
    if view.run_id != specification.run_id
        || view.budget != specification.budget
        || view.maximum_rounds != specification.maximum_rounds
        || row.try_get::<String, _>("current_job_id")? != storage::current_id(&view)?
    {
        return Err(StoreError::Corrupt("transition projection binding"));
    }
    Ok((binding, view))
}

pub(super) async fn load(
    transaction: &mut Transaction<'_, Postgres>,
    specification: RunSpecification,
    view: RunView,
    row: &PgRow,
) -> StoreResult<RunSnapshot> {
    let run_id = &specification.run_id.as_ref().expect("validated run").value;
    let rows =
        sqlx::query("SELECT * FROM run_transitions WHERE run_id=$1 ORDER BY revision LIMIT 194")
            .bind(run_id)
            .fetch_all(&mut **transaction)
            .await?;
    if rows.is_empty()
        || rows.len() != view.revision as usize
        || rows.len() > 3 * specification.maximum_rounds as usize + 1
    {
        return Err(StoreError::Corrupt("run transition count"));
    }
    let records = sqlx::query("SELECT * FROM jobs WHERE run_id=$1 ORDER BY job_id LIMIT 193")
        .bind(run_id)
        .fetch_all(&mut **transaction)
        .await?;
    if records.is_empty() || records.len() > 3 * specification.maximum_rounds as usize {
        return Err(StoreError::Corrupt("run child count"));
    }
    let jobs = records
        .iter()
        .map(|row| {
            let record = record_from_row(row)?;
            let id = record
                .specification
                .as_ref()
                .and_then(|job| job.job_id.as_ref())
                .ok_or(StoreError::Corrupt("run child ID"))?
                .value
                .clone();
            Ok((id, record))
        })
        .collect::<StoreResult<BTreeMap<_, _>>>()?;
    let mut created = BTreeSet::new();
    let mut previous: Option<RunView> = None;
    let mut last_binding: Option<Box<RunBinding>> = None;
    let mut totals = [0_u64; 4];
    for (index, row) in rows.iter().enumerate() {
        let (binding, projection) = decode(row, &specification)?;
        if projection.revision != index as u64 + 1 {
            return Err(StoreError::Corrupt("run transition sequence"));
        }
        let research = projection.research.as_ref().expect("validated research");
        let id = storage::current_id(&projection)?;
        let record = jobs
            .get(id)
            .ok_or(StoreError::Corrupt("transition child missing"))?;
        if record.specification != binding.current_specification {
            return Err(StoreError::Corrupt("transition child specification"));
        }
        check_chain(&specification, previous.as_ref(), &projection, &binding)?;
        check_adaptive(
            &specification,
            last_binding.as_deref(),
            &projection,
            &binding,
        )?;
        check_source(transaction, &binding).await?;
        check_observation(&projection, record)?;
        if binding.created_child {
            if !created.insert(id.to_owned()) {
                return Err(StoreError::Corrupt("duplicate run child"));
            }
            let job = record
                .specification
                .as_ref()
                .expect("verified specification");
            check_child(&specification, &projection, job, &binding)?;
            let allowance = budget(job)?;
            let amounts = [
                u64::from(allowance.maximum_steps),
                allowance.maximum_input_tokens,
                allowance.maximum_output_tokens,
                model_money(allowance.maximum_cost.as_ref())?,
            ];
            for (total, amount) in totals.iter_mut().zip(amounts) {
                *total = total
                    .checked_add(amount)
                    .ok_or(StoreError::Corrupt("run reservation overflow"))?;
            }
        }
        if totals
            != [
                projection.reserved_steps,
                projection.reserved_input_tokens,
                projection.reserved_output_tokens,
                model_money(projection.reserved_cost.as_ref())?,
            ]
        {
            return Err(StoreError::Corrupt("run reservation history"));
        }
        if index == 0 && (research.round != 1 || research.stage != RunStage::Discovery as i32) {
            return Err(StoreError::Corrupt("run initial transition"));
        }
        previous = Some(projection);
        last_binding = Some(binding);
    }
    if previous.as_ref() != Some(&view)
        || created.len() != jobs.len()
        || row.try_get::<String, _>("first_job_id")?
            != rows[0].try_get::<String, _>("current_job_id")?
    {
        return Err(StoreError::Corrupt("run final projection"));
    }
    let current = storage::current_id(&view)?;
    for (id, job) in &jobs {
        if id != current && job.state != JobState::Succeeded as i32 {
            return Err(StoreError::Corrupt("unfinished prior run child"));
        }
    }
    let current_job = jobs.get(current).expect("verified current").clone();
    Ok(RunSnapshot {
        specification,
        view,
        current_job,
        binding: last_binding,
    })
}

fn check_chain(
    specification: &RunSpecification,
    previous: Option<&RunView>,
    view: &RunView,
    binding: &RunBinding,
) -> StoreResult<()> {
    let current = view.research.as_ref().expect("validated research");
    if let Some(previous) = previous {
        let prior = previous.research.as_ref().expect("verified research");
        if binding.previous_sha256
            != prior
                .binding_sha256
                .as_ref()
                .expect("verified digest")
                .value
            || previous.status != RunStatus::Active as i32
            || view.submitted_at != previous.submitted_at
            || view.deadline != previous.deadline
            || timestamp_millis(view.updated_at.as_ref().expect("validated time"), true)?
                < timestamp_millis(previous.updated_at.as_ref().expect("validated time"), true)?
            || view.completed_rounds < previous.completed_rounds
            || view.completed_rounds > previous.completed_rounds + 1
            || binding
                .source_record
                .as_ref()
                .and_then(|record| record.specification.as_ref())
                .and_then(|job| job.job_id.as_ref())
                .map(|id| id.value.as_str())
                != Some(storage::current_id(previous)?)
        {
            return Err(StoreError::Corrupt("run transition ancestry"));
        }
        if binding.created_child {
            let next_stage = current.round == prior.round
                && (current.stage == prior.stage + 1
                    || (specification.feedback_policy.is_some()
                        && prior.stage == RunStage::Perturbation as i32
                        && current.stage == RunStage::Evaluation as i32))
                && view.completed_rounds == previous.completed_rounds;
            let next_round = current.round == prior.round + 1
                && (current.stage == RunStage::Discovery as i32
                    || (specification.feedback_policy.is_some()
                        && current.stage == RunStage::Perturbation as i32))
                && view.completed_rounds == previous.completed_rounds + 1;
            if view.status != RunStatus::Active as i32 || (!next_stage && !next_round) {
                return Err(StoreError::Corrupt("run stage progression"));
            }
        } else if view.status == RunStatus::Active as i32
            || storage::current_id(view)? != storage::current_id(previous)?
            || current.stage != prior.stage
            || current.round != prior.round
        {
            return Err(StoreError::Corrupt("run terminal transition"));
        }
        check_progress(specification, previous, view, binding)?;
    } else if !binding.previous_sha256.is_empty()
        || !binding.created_child
        || binding.source_record.is_some()
        || binding.source_trial.is_some()
        || binding.source_backtest.is_some()
        || binding.previous_record.is_some()
        || binding.previous_trial.is_some()
        || binding.attempted_specification.is_some()
        || !binding.previous_context.is_empty()
        || view.revision != 1
        || view.completed_rounds != 0
        || current.stage != RunStage::Discovery as i32
        || current.round != 1
        || view.status != RunStatus::Active as i32
        || view.maximum_rounds != specification.maximum_rounds
    {
        return Err(StoreError::Corrupt("run initial evidence"));
    }
    Ok(())
}

async fn check_source(
    transaction: &mut Transaction<'_, Postgres>,
    binding: &RunBinding,
) -> StoreResult<()> {
    for (record, trial) in [
        (
            binding.source_record.as_ref(),
            binding.source_trial.as_ref(),
        ),
        (
            binding.previous_record.as_ref(),
            binding.previous_trial.as_ref(),
        ),
    ] {
        if let Some(record) = record {
            let id = record
                .specification
                .as_ref()
                .and_then(|job| job.job_id.as_ref())
                .ok_or(StoreError::Corrupt("run source ID"))?;
            let row = sqlx::query("SELECT * FROM jobs WHERE job_id=$1")
                .bind(&id.value)
                .fetch_optional(&mut **transaction)
                .await?
                .ok_or(StoreError::Corrupt("missing run source"))?;
            let current = record_from_row(&row)?;
            loop_protocol::job::validate_job_record(record)
                .map_err(|_| StoreError::Corrupt("historical run record"))?;
            let ended = !matches!(
                JobState::try_from(record.state),
                Ok(JobState::Queued | JobState::Leased | JobState::Running)
            );
            if current.specification != record.specification
                || current.revision < record.revision
                || (ended && current != *record)
                || evaluation::read(transaction, record).await?.as_ref() != trial
            {
                return Err(StoreError::Corrupt("run source evidence"));
            }
        } else if trial.is_some() {
            return Err(StoreError::Corrupt("run orphan trial"));
        }
    }
    let portfolio = if let Some(record) = &binding.source_record {
        super::super::backtest::verify_stored(transaction, record).await?
    } else {
        None
    };
    if portfolio != binding.source_backtest {
        return Err(StoreError::Corrupt("run portfolio evidence"));
    }
    if let Some(attempted) = &binding.attempted_specification {
        loop_protocol::job::validate_job_specification(attempted)
            .map_err(|_| StoreError::Corrupt("attempted run specification"))?;
        // Verify the specific immutable match recorded at admission. A later
        // successful concurrent evaluation may sort before it; that must not
        // retroactively change an earlier pre-screen's historical evidence.
        let previous = binding
            .previous_record
            .as_ref()
            .and_then(|record| record.specification.as_ref());
        if previous.is_none() {
            let view = binding
                .projection
                .as_ref()
                .ok_or(StoreError::Corrupt("attempted run projection"))?;
            let stage = view
                .research
                .as_ref()
                .ok_or(StoreError::Corrupt("attempted run stage"))?
                .stage;
            if binding.adaptive.is_none()
                || view.status != RunStatus::BudgetExhausted as i32
                || binding.created_child
                || binding
                    .source_record
                    .as_ref()
                    .is_none_or(|source| source.state != JobState::Succeeded as i32)
                || !matches!(
                    RunStage::try_from(stage),
                    Ok(RunStage::Discovery | RunStage::Perturbation)
                )
                || !matches!(
                    attempted.input,
                    Some(job_specification::Input::FactorEvaluation(_))
                )
                || binding.previous_trial.is_some()
                || !binding.previous_context.is_empty()
            {
                return Err(StoreError::Corrupt("unexplained attempted candidate"));
            }
            return Ok(());
        }
        let expected = evaluation::context_key(attempted)?
            .ok_or(StoreError::Corrupt("run attempted context"))?;
        if evaluation::context_key(previous.expect("checked historical match"))? != Some(expected)
            || binding.previous_trial.is_none()
            || binding.previous_context != expected
        {
            return Err(StoreError::Corrupt("run previous evaluation"));
        }
    } else if binding.previous_record.is_some() || !binding.previous_context.is_empty() {
        return Err(StoreError::Corrupt("run unexplained filter"));
    }
    Ok(())
}

fn check_observation(view: &RunView, record: &JobRecord) -> StoreResult<()> {
    use loop_protocol::wire::runs::v1::research_run_view::CurrentChild;
    let research = view.research.as_ref().expect("validated research");
    let (revision, status, submitted, updated) = match research.current_child.as_ref() {
        Some(CurrentChild::Discovery(handle)) => (
            handle.revision,
            handle.status,
            handle.submitted_at,
            handle.updated_at,
        ),
        Some(CurrentChild::Research(handle)) => (
            handle.revision,
            handle.status,
            handle.submitted_at,
            handle.updated_at,
        ),
        None => return Err(StoreError::Corrupt("run child handle")),
    };
    let job = record
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("run child specification"))?;
    if submitted != job.submitted_at
        || revision > record.revision
        || (view.status == RunStatus::Active as i32
            && (revision != 1 || status != JobState::Queued as i32 || submitted != updated))
        || (revision == record.revision && (status != record.state || updated != record.updated_at))
    {
        return Err(StoreError::Corrupt("run child observation"));
    }
    Ok(())
}

fn check_progress(
    specification: &RunSpecification,
    previous: &RunView,
    view: &RunView,
    binding: &RunBinding,
) -> StoreResult<()> {
    let prior = previous.research.as_ref().expect("verified research");
    let current = view.research.as_ref().expect("verified research");
    let source = binding
        .source_record
        .as_ref()
        .ok_or(StoreError::Corrupt("run source"))?;
    if timestamp_millis(
        source
            .updated_at
            .as_ref()
            .ok_or(StoreError::Corrupt("run source time"))?,
        true,
    )? > timestamp_millis(view.updated_at.as_ref().expect("validated update"), true)?
    {
        return Err(StoreError::Corrupt("run source chronology"));
    }
    let succeeded = source.state == JobState::Succeeded as i32;
    let disposition = match RunStage::try_from(prior.stage)
        .map_err(|_| StoreError::Corrupt("run source stage"))?
    {
        RunStage::Discovery | RunStage::Perturbation if binding.previous_trial.is_some() => {
            if binding
                .previous_trial
                .as_ref()
                .expect("present trial")
                .disposition
                == "insufficient_coverage"
            {
                RoundDisposition::PreviouslyRejected
            } else {
                RoundDisposition::Duplicate
            }
        }
        RunStage::Evaluation
            if binding
                .source_trial
                .as_ref()
                .is_some_and(|trial| trial.disposition == "insufficient_coverage") =>
        {
            RoundDisposition::InsufficientCoverage
        }
        RunStage::Portfolio if binding.source_backtest.is_some() => RoundDisposition::Evaluated,
        RunStage::Perturbation
            if binding
                .adaptive
                .as_ref()
                .and_then(|adaptive| adaptive.step.as_ref())
                .is_some_and(|step| {
                    step.candidate.is_none()
                        && step.reason
                            == loop_protocol::wire::v1::PerturbationReason::Exhausted as i32
                }) =>
        {
            RoundDisposition::Exhausted
        }
        _ => RoundDisposition::Unspecified,
    };
    let expired = timestamp_millis(view.updated_at.as_ref().expect("validated update"), true)?
        >= timestamp_millis(view.deadline.as_ref().expect("validated deadline"), true)?;
    let completed = succeeded && !expired && disposition != RoundDisposition::Unspecified;
    if specification.feedback_policy.is_some() && succeeded && !expired {
        if let Some(attempted) = &binding.attempted_specification {
            check_attempt(specification, view, attempted)?;
        }
        if prior.stage == RunStage::Perturbation as i32 {
            let selected = binding
                .adaptive
                .as_ref()
                .and_then(|adaptive| adaptive.step.as_ref())
                .ok_or(StoreError::Corrupt("run selected proposal"))?;
            if let Some(candidate) = &selected.candidate {
                let attempted = binding
                    .attempted_specification
                    .as_ref()
                    .or(binding.current_specification.as_ref());
                let Some(job_specification::Input::FactorEvaluation(input)) =
                    attempted.and_then(|job| job.input.as_ref())
                else {
                    return Err(StoreError::Corrupt("run selected evaluation"));
                };
                if input
                    .factor
                    .as_ref()
                    .and_then(|factor| factor.factor_spec_id.as_ref())
                    != candidate.factor_spec_id.as_ref()
                {
                    return Err(StoreError::Corrupt("run selected identity"));
                }
            } else if binding.created_child
                || binding.previous_trial.is_some()
                || binding.attempted_specification.is_some()
                || view.status != RunStatus::SearchExhausted as i32
            {
                return Err(StoreError::Corrupt("run exhausted proposal"));
            }
        }
    }
    if view.completed_rounds != previous.completed_rounds + u32::from(completed)
        || (!binding.created_child
            && current.disposition != if completed { disposition as i32 } else { 0 })
        || (binding.created_child && (!succeeded || expired || current.disposition != 0))
        || (binding.previous_trial.is_some()
            && prior.stage != RunStage::Discovery as i32
            && prior.stage != RunStage::Perturbation as i32)
        || (binding.source_trial.is_some() && prior.stage != RunStage::Evaluation as i32)
        || (binding.source_backtest.is_some() && prior.stage != RunStage::Portfolio as i32)
    {
        return Err(StoreError::Corrupt("run completion evidence"));
    }
    if binding.created_child {
        if completed {
            if current.stage != RunStage::Discovery as i32
                && !(specification.feedback_policy.is_some()
                    && current.stage == RunStage::Perturbation as i32)
            {
                return Err(StoreError::Corrupt("run round boundary"));
            }
        } else if prior.stage == RunStage::Evaluation as i32 {
            if binding
                .source_trial
                .as_ref()
                .is_none_or(|trial| trial.disposition != "ready_for_backtest")
                || current.expression_id != prior.expression_id
                || current.factor_spec_id != prior.factor_spec_id
                || current.discovery_job_id != prior.discovery_job_id
                || current.perturbation_job_id != prior.perturbation_job_id
                || current.evaluation_job_id != prior.evaluation_job_id
            {
                return Err(StoreError::Corrupt("run portfolio lineage"));
            }
        } else if (prior.stage != RunStage::Discovery as i32
            && prior.stage != RunStage::Perturbation as i32)
            || current.discovery_job_id != prior.discovery_job_id
            || current.perturbation_job_id != prior.perturbation_job_id
        {
            return Err(StoreError::Corrupt("run evaluation lineage"));
        }
    } else {
        if current.discovery_job_id != prior.discovery_job_id
            || current.perturbation_job_id != prior.perturbation_job_id
            || current.evaluation_job_id != prior.evaluation_job_id
        {
            return Err(StoreError::Corrupt("terminal run sources"));
        }
        if matches!(
            RunStage::try_from(prior.stage),
            Ok(RunStage::Discovery | RunStage::Perturbation)
        ) && completed
            && disposition != RoundDisposition::Exhausted
        {
            let Some(job_specification::Input::FactorEvaluation(attempt)) = binding
                .attempted_specification
                .as_ref()
                .and_then(|job| job.input.as_ref())
            else {
                return Err(StoreError::Corrupt("terminal filtered candidate"));
            };
            let factor = attempt
                .factor
                .as_ref()
                .ok_or(StoreError::Corrupt("terminal filtered factor"))?;
            if current.expression_id != factor.expression_id
                || current.factor_spec_id != factor.factor_spec_id
            {
                return Err(StoreError::Corrupt("terminal filtered identity"));
            }
        } else if current.expression_id != prior.expression_id
            || current.factor_spec_id != prior.factor_spec_id
        {
            return Err(StoreError::Corrupt("terminal factor identity"));
        }
        let expected = if expired {
            RunStatus::DeadlineExceeded
        } else if source.state == JobState::BudgetExhausted as i32 {
            RunStatus::BudgetExhausted
        } else if !succeeded {
            RunStatus::InfrastructureFailed
        } else if disposition == RoundDisposition::Exhausted {
            RunStatus::SearchExhausted
        } else if view.completed_rounds == specification.maximum_rounds {
            RunStatus::Completed
        } else {
            RunStatus::BudgetExhausted
        };
        if view.status != expected as i32 {
            return Err(StoreError::Corrupt("run terminal status"));
        }
        if expected == RunStatus::BudgetExhausted && succeeded {
            let numerical = specification.numerical.as_ref().expect("v2 specification");
            let planned_perturbation = completed
                && binding
                    .adaptive
                    .as_ref()
                    .is_some_and(|adaptive| adaptive.work.is_some());
            let bytes = if planned_perturbation {
                numerical
                    .perturbation_budget
                    .as_ref()
                    .ok_or(StoreError::Corrupt("perturbation allowance"))?
                    .encode_to_vec()
            } else if completed {
                specification
                    .discovery
                    .as_ref()
                    .expect("validated discovery")
                    .budget
                    .as_ref()
                    .expect("validated budget")
                    .encode_to_vec()
            } else if matches!(
                RunStage::try_from(prior.stage),
                Ok(RunStage::Discovery | RunStage::Perturbation)
            ) {
                numerical
                    .evaluation_budget
                    .as_ref()
                    .expect("validated budget")
                    .encode_to_vec()
            } else {
                numerical
                    .portfolio_budget
                    .as_ref()
                    .expect("validated budget")
                    .encode_to_vec()
            };
            let budget = loop_protocol::wire::v1::JobBudget::decode(bytes.as_slice())
                .map_err(|_| StoreError::Corrupt("run child budget"))?;
            let mut attempt = previous.clone();
            let now = timestamp_millis(view.updated_at.as_ref().expect("validated update"), true)?;
            let reserved = if completed && !planned_perturbation {
                storage::reserve(&mut attempt, specification, now)?
            } else {
                storage::reserve_budget(&mut attempt, specification, &budget, now)?
            };
            if reserved {
                return Err(StoreError::Corrupt("run false exhaustion"));
            }
        }
    }
    Ok(())
}

fn check_attempt(
    specification: &RunSpecification,
    view: &RunView,
    job: &JobSpecification,
) -> StoreResult<()> {
    let numerical = specification
        .numerical
        .as_ref()
        .ok_or(StoreError::Corrupt("attempted workflow"))?;
    let discovery = specification
        .discovery
        .as_ref()
        .ok_or(StoreError::Corrupt("attempted dataset"))?;
    let Some(job_specification::Input::FactorEvaluation(input)) = &job.input else {
        return Err(StoreError::Corrupt("attempted candidate input"));
    };
    if job.run_id != specification.run_id
        || job.submitted_by != numerical.research_executor
        || job.protocol_selection != numerical.protocol_selection
        || job.submitted_at != view.updated_at
        || input.dataset != discovery.dataset
        || budget(job)?.encode_to_vec()
            != numerical
                .evaluation_budget
                .as_ref()
                .ok_or(StoreError::Corrupt("attempted budget"))?
                .encode_to_vec()
    {
        return Err(StoreError::Corrupt("attempted candidate binding"));
    }
    Ok(())
}

fn check_child(
    specification: &RunSpecification,
    view: &RunView,
    job: &JobSpecification,
    binding: &RunBinding,
) -> StoreResult<()> {
    let research = view.research.as_ref().expect("validated research");
    let numerical = specification
        .numerical
        .as_ref()
        .expect("numerical workflow");
    let discovery = specification
        .discovery
        .as_ref()
        .expect("validated discovery");
    let expected_key = format!(
        "run.{:x}.{}.{}",
        Sha256::digest(
            specification
                .run_id
                .as_ref()
                .expect("validated ID")
                .value
                .as_bytes()
        ),
        research.round,
        research.stage
    );
    if job.run_id != specification.run_id
        || job.idempotency_key.as_ref().map(|key| key.value.as_str()) != Some(expected_key.as_str())
        || job.submitted_at != view.updated_at
        || deadline_millis(job)?
            > timestamp_millis(view.deadline.as_ref().expect("validated deadline"), true)?
    {
        return Err(StoreError::Corrupt("run child identity"));
    }
    let expected_budget = match &job.input {
        Some(job_specification::Input::Discovery(input))
            if research.stage == RunStage::Discovery as i32 =>
        {
            if job.submitted_by != specification.executor
                || job.protocol_selection != specification.protocol_selection
                || input.dataset != discovery.dataset
                || input.research_policy != discovery.research_policy
                || input.maker_model != discovery.maker_model
                || input.checker_model != discovery.checker_model
                || input.maximum_candidates != discovery.maximum_candidates
            {
                return Err(StoreError::Corrupt("run discovery binding"));
            }
            discovery
                .budget
                .as_ref()
                .expect("validated budget")
                .encode_to_vec()
        }
        Some(job_specification::Input::FactorEvaluation(input))
            if research.stage == RunStage::Evaluation as i32 =>
        {
            if job.submitted_by != numerical.research_executor
                || job.protocol_selection != numerical.protocol_selection
                || input.dataset != discovery.dataset
                || input
                    .factor
                    .as_ref()
                    .and_then(|factor| factor.factor_spec_id.as_ref())
                    != research.factor_spec_id.as_ref()
                || input
                    .factor
                    .as_ref()
                    .and_then(|factor| factor.expression_id.as_ref())
                    != research.expression_id.as_ref()
            {
                return Err(StoreError::Corrupt("run evaluation binding"));
            }
            numerical
                .evaluation_budget
                .as_ref()
                .expect("validated budget")
                .encode_to_vec()
        }
        Some(job_specification::Input::Backtest(input))
            if research.stage == RunStage::Portfolio as i32 =>
        {
            if job.submitted_by != numerical.research_executor
                || job.protocol_selection != numerical.protocol_selection
                || input.dataset != discovery.dataset
                || input.factor_spec_id != research.factor_spec_id
            {
                return Err(StoreError::Corrupt("run portfolio binding"));
            }
            numerical
                .portfolio_budget
                .as_ref()
                .expect("validated budget")
                .encode_to_vec()
        }
        Some(job_specification::Input::Perturbation(input))
            if research.stage == RunStage::Perturbation as i32 =>
        {
            let adaptive = binding
                .adaptive
                .as_ref()
                .ok_or(StoreError::Corrupt("perturbation context"))?;
            let work = adaptive
                .work
                .as_ref()
                .ok_or(StoreError::Corrupt("perturbation work"))?;
            if job.submitted_by != numerical.research_executor
                || job.protocol_selection != numerical.protocol_selection
                || input.dataset != discovery.dataset
                || input.feedback_policy != specification.feedback_policy
                || input.family_sha256 != adaptive.family_sha256
                || input
                    .parent_binding_sha256
                    .as_ref()
                    .map(|digest| &digest.value)
                    != Some(&binding.previous_sha256)
            {
                return Err(StoreError::Corrupt("run perturbation binding"));
            }
            check_artifact(
                input.work.as_ref(),
                "loop.perturbation_work",
                &work.encode_to_vec(),
            )?;
            numerical
                .perturbation_budget
                .as_ref()
                .ok_or(StoreError::Corrupt("perturbation allowance"))?
                .encode_to_vec()
        }
        _ => return Err(StoreError::Corrupt("run stage input")),
    };
    if budget(job)?.encode_to_vec() != expected_budget {
        return Err(StoreError::Corrupt("run frozen child budget"));
    }
    Ok(())
}

pub(super) async fn check_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    snapshot: &RunSnapshot,
    view: &RunView,
    row: &PgRow,
    parent: &PgRow,
    bytes: &[u8],
) -> StoreResult<()> {
    let run_id = &snapshot
        .specification
        .run_id
        .as_ref()
        .expect("validated run")
        .value;
    let transition = sqlx::query("SELECT * FROM run_transitions WHERE run_id=$1 AND revision=$2")
        .bind(run_id)
        .bind(view.revision as i64)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::Corrupt("run receipt transition"))?;
    let (_, expected) = decode(&transition, &snapshot.specification)?;
    if *view != expected
        || view.encode_to_vec() != bytes
        || parent.try_get::<String, _>("first_job_id")? != row.try_get::<String, _>("job_id")?
        || view.revision > snapshot.view.revision
        || timestamp_millis(view.updated_at.as_ref().expect("validated time"), true)?
            != row.try_get::<i64, _>("committed_at_ms")?
    {
        return Err(StoreError::Corrupt("run receipt binding"));
    }
    Ok(())
}

pub(in crate::store) type TrialCache =
    BTreeMap<String, (RunSpecification, BTreeMap<String, JobSpecification>)>;

pub(in crate::store) async fn authorize_trial(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    principal: &loop_protocol::wire::v1::Actor,
    record: &JobRecord,
    cache: &mut TrialCache,
) -> StoreResult<()> {
    let direct =
        match store
            .admission
            .authorize_job_command("loop.factors.read_trials", principal, record)
        {
            Ok(()) => Ok(()),
            Err(StoreError::AdmissionDenied) => Err(StoreError::AdmissionDenied),
            Err(error) => return Err(error),
        };
    let job = record
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("run trial job"))?;
    let run_id = &job
        .run_id
        .as_ref()
        .ok_or(StoreError::Corrupt("run trial parent"))?
        .value;
    let job_id = &job
        .job_id
        .as_ref()
        .ok_or(StoreError::Corrupt("run trial identity"))?
        .value;
    if !cache.contains_key(run_id) {
        let managed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM research_runs WHERE run_id=$1 AND workflow_version IN (2,3))",
        )
        .bind(run_id)
        .fetch_one(&mut **transaction)
        .await?;
        if !managed {
            return direct;
        }
        let snapshot = storage::load(transaction, run_id).await?;
        let rows = sqlx::query("SELECT * FROM run_transitions WHERE run_id=$1 AND created_child ORDER BY revision LIMIT 193")
            .bind(run_id).fetch_all(&mut **transaction).await?;
        if rows.len() > 192 {
            return Err(StoreError::Corrupt("run trial child bounds"));
        }
        let mut jobs = BTreeMap::new();
        for row in rows {
            let (binding, _) = decode(&row, &snapshot.specification)?;
            let specification = binding
                .current_specification
                .ok_or(StoreError::Corrupt("run trial specification"))?;
            jobs.insert(
                specification
                    .job_id
                    .as_ref()
                    .ok_or(StoreError::Corrupt("run trial ID"))?
                    .value
                    .clone(),
                specification,
            );
        }
        cache.insert(run_id.clone(), (snapshot.specification, jobs));
    }
    let (run, jobs) = cache.get(run_id).expect("loaded run history");
    if jobs.get(job_id) != Some(job) {
        return Err(StoreError::Corrupt("run trial identity binding"));
    }
    store.admission.authorize_run_trial(principal, run, job)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::workflow::adaptive::{FeedbackSource, SelectionSource};
    use crate::test_support as fixture;
    use loop_protocol::wire::v1::{
        BacktestJobInput, BacktestMetric, ExactDecimal, JobId, PerturbationState, PerturbationWork,
        WindowObservation,
    };

    // These fixtures exercise immutable reconstruction only; installed source
    // verification and real worker execution have separate acceptance tests.
    fn work_fixture() -> (AdaptiveBinding, PerturbationWork) {
        let candidates = [5, 10].map(fixture::perturbation::candidate).to_vec();
        let source_id = Some(JobId {
            value: "job.feedback.source".into(),
        });
        let source = FeedbackSource {
            round: 1,
            record: Some(JobRecord {
                specification: Some(JobSpecification {
                    job_id: source_id.clone(),
                    input: Some(job_specification::Input::Backtest(BacktestJobInput {
                        factor_spec_id: candidates[0].factor_spec_id.clone(),
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            result: Some(BacktestResult {
                metrics: vec![BacktestMetric {
                    name: "sharpe_annualized".into(),
                    value: Some(ExactDecimal {
                        value: "1.5".into(),
                    }),
                    unit: "ratio".into(),
                    estimator: "daily-statistics.1".into(),
                }],
                ..Default::default()
            }),
            evidence: b"source manifest".to_vec(),
        };
        let state = PerturbationState {
            version: 1,
            random_seed: Some(fixture::digest(0)),
            ..Default::default()
        };
        let work = PerturbationWork {
            state: Some(state.clone()),
            observed_factor_ids: vec![candidates[0].factor_spec_id.clone().unwrap()],
            observation: Some(WindowObservation {
                source_job_id: source_id,
                candidate: Some(candidates[0].clone()),
                net_sharpe: 1.5,
            }),
            candidates,
            current_window: 5,
            ..Default::default()
        };
        (
            AdaptiveBinding {
                state: Some(state),
                sources: vec![source],
                ..Default::default()
            },
            work,
        )
    }

    #[test]
    fn sharpe_binding() {
        let (current, mut work) = work_fixture();
        check_work(&current, &work).unwrap();
        work.observation.as_mut().unwrap().net_sharpe = 2.5;
        assert!(matches!(
            check_work(&current, &work),
            Err(StoreError::Corrupt("adaptive work observation value"))
        ));
    }

    #[test]
    fn consumed_observation() {
        let (mut current, mut work) = work_fixture();
        let observation = work.observation.clone().unwrap();
        work.state.as_mut().unwrap().history.push(observation);
        current.state = work.state.clone();
        assert!(matches!(
            check_work(&current, &work),
            Err(StoreError::Corrupt("adaptive work observation history"))
        ));
        work.observation = None;
        check_work(&current, &work).unwrap();
    }

    #[test]
    fn empty_observation() {
        let (current, mut work) = work_fixture();
        work.observation = None;
        assert!(matches!(
            check_work(&current, &work),
            Err(StoreError::Corrupt("adaptive work observation history"))
        ));
    }

    #[test]
    fn failure_binding() {
        let (mut current, mut work) = work_fixture();
        let factor = work.candidates[1].factor_spec_id.clone().unwrap();
        current.failed_factor_ids.push(factor.clone());
        assert!(matches!(
            check_work(&current, &work),
            Err(StoreError::Corrupt("adaptive work failure memory"))
        ));
        work.failed_factor_ids.push(factor);
        check_work(&current, &work).unwrap();
    }

    #[test]
    fn observed_binding() {
        let (current, mut work) = work_fixture();
        check_work(&current, &work).unwrap();
        work.observed_factor_ids.clear();
        assert!(matches!(
            check_work(&current, &work),
            Err(StoreError::Corrupt("adaptive work observation memory"))
        ));
    }

    #[test]
    fn observed_forgery() {
        let (current, mut work) = work_fixture();
        work.observed_factor_ids
            .push(work.candidates[1].factor_spec_id.clone().unwrap());
        assert!(matches!(
            check_work(&current, &work),
            Err(StoreError::Corrupt("adaptive work observation memory"))
        ));
    }

    #[test]
    fn observed_duplicate() {
        let (current, mut work) = work_fixture();
        work.observed_factor_ids
            .push(work.observed_factor_ids[0].clone());
        assert!(matches!(
            check_work(&current, &work),
            Err(StoreError::Corrupt("adaptive work observation memory"))
        ));
    }

    #[test]
    fn source_evidence() {
        let (current, _) = work_fixture();
        let source = current.sources[0].clone();
        let previous = RunBinding {
            projection: Some(RunView {
                research: Some(loop_protocol::wire::runs::v1::ResearchRunView {
                    round: 1,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            evidence: source.evidence.clone(),
            ..Default::default()
        };
        let mut binding = RunBinding {
            source_record: source.record,
            source_backtest: source.result,
            ..Default::default()
        };
        check_feedback(&AdaptiveBinding::default(), &current, &previous, &binding).unwrap();
        binding.source_backtest.as_mut().unwrap().metrics[0]
            .value
            .as_mut()
            .unwrap()
            .value = "2.5".into();
        assert!(matches!(
            check_feedback(&AdaptiveBinding::default(), &current, &previous, &binding),
            Err(StoreError::Corrupt("adaptive source append"))
        ));
    }

    #[test]
    fn receipt_history() {
        let before = AdaptiveBinding::default();
        let record = JobRecord {
            state: JobState::Succeeded as i32,
            revision: 3,
            ..Default::default()
        };
        let mut current = before.clone();
        current.completed_perturbations.push(SelectionSource {
            round: 2,
            record: Some(record.clone()),
        });
        check_completions(&before, &current, RunStage::Perturbation as i32, 2, &record).unwrap();
        assert!(matches!(
            check_completions(&before, &current, RunStage::Evaluation as i32, 2, &record),
            Err(StoreError::Corrupt("adaptive completion without proposal"))
        ));
        current.completed_perturbations[0]
            .record
            .as_mut()
            .unwrap()
            .revision = 4;
        assert!(matches!(
            check_completions(&before, &current, RunStage::Perturbation as i32, 2, &record),
            Err(StoreError::Corrupt("adaptive completion append"))
        ));
    }
}
