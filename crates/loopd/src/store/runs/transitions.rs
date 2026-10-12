//! Immutable v2 evidence and bounded reconstruction; jobs remain authoritative.

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
}

pub(super) fn seal(
    specification: &RunSpecification,
    view: &mut RunView,
    binding: &mut RunBinding,
) -> StoreResult<Vec<u8>> {
    binding.schema = "loop.run-transition/v1".into();
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
        || binding.schema != "loop.run-transition/v1"
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
    let mut last_binding = None;
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
            check_child(&specification, &projection, job)?;
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
                && current.stage == prior.stage + 1
                && view.completed_rounds == previous.completed_rounds;
            let next_round = current.round == prior.round + 1
                && current.stage == RunStage::Discovery as i32
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
            .and_then(|record| record.specification.as_ref())
            .ok_or(StoreError::Corrupt("run previous specification"))?;
        let expected = evaluation::context_key(attempted)?
            .ok_or(StoreError::Corrupt("run attempted context"))?;
        if evaluation::context_key(previous)? != Some(expected)
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
        RunStage::Discovery if binding.previous_trial.is_some() => {
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
        _ => RoundDisposition::Unspecified,
    };
    let expired = timestamp_millis(view.updated_at.as_ref().expect("validated update"), true)?
        >= timestamp_millis(view.deadline.as_ref().expect("validated deadline"), true)?;
    let completed = succeeded && !expired && disposition != RoundDisposition::Unspecified;
    if view.completed_rounds != previous.completed_rounds + u32::from(completed)
        || (!binding.created_child
            && current.disposition != if completed { disposition as i32 } else { 0 })
        || (binding.created_child && (!succeeded || expired || current.disposition != 0))
        || (binding.previous_trial.is_some() && prior.stage != RunStage::Discovery as i32)
        || (binding.source_trial.is_some() && prior.stage != RunStage::Evaluation as i32)
        || (binding.source_backtest.is_some() && prior.stage != RunStage::Portfolio as i32)
    {
        return Err(StoreError::Corrupt("run completion evidence"));
    }
    if binding.created_child {
        if completed {
            if current.stage != RunStage::Discovery as i32 {
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
                || current.evaluation_job_id != prior.evaluation_job_id
            {
                return Err(StoreError::Corrupt("run portfolio lineage"));
            }
        } else if prior.stage != RunStage::Discovery as i32
            || current.discovery_job_id != prior.discovery_job_id
        {
            return Err(StoreError::Corrupt("run evaluation lineage"));
        }
    } else {
        if current.discovery_job_id != prior.discovery_job_id
            || current.evaluation_job_id != prior.evaluation_job_id
        {
            return Err(StoreError::Corrupt("terminal run sources"));
        }
        if prior.stage == RunStage::Discovery as i32 && completed {
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
            let bytes = if completed {
                specification
                    .discovery
                    .as_ref()
                    .expect("validated discovery")
                    .budget
                    .as_ref()
                    .expect("validated budget")
                    .encode_to_vec()
            } else if prior.stage == RunStage::Discovery as i32 {
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
            if storage::reserve_budget(
                &mut attempt,
                specification,
                &budget,
                timestamp_millis(view.updated_at.as_ref().expect("validated update"), true)?,
            )? {
                return Err(StoreError::Corrupt("run false exhaustion"));
            }
        }
    }
    Ok(())
}

fn check_child(
    specification: &RunSpecification,
    view: &RunView,
    job: &JobSpecification,
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
            "SELECT EXISTS(SELECT 1 FROM research_runs WHERE run_id=$1 AND workflow_version=2)",
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
