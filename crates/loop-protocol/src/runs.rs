//! Shape validation for frozen run specifications and safe operator projections.
//!
//! These checks do not authenticate an owner, resolve a plan, prove persisted
//! budget accounting, or authorize a child. Those remain loopd store boundaries.

use crate::job::{
    JobValidationCode, JobValidationError, protocol_selection_sha256, require_timestamp,
    require_token_id, validate_actor, validate_discovery_input, validate_money,
    validate_policy_reference,
};
use crate::wire::discovery::v1::DiscoveryJobStatus;
use crate::wire::runs::v1::{RunBudget, RunSpecification, RunStatus, RunView};
use crate::wire::v1::{ActorKind, Money};

const MAX_WALL_SECONDS: i64 = 2_592_000;

/// Validate a server-resolved specification before creating its first child.
///
/// `submitted_at` is the actual admission time, not a caller-controlled future
/// timestamp. Transport identity, plan availability and ledger history are not
/// established by this function. It has no side effects or replay behavior.
pub fn validate_specification(
    specification: &RunSpecification,
    submitted_at: &prost_types::Timestamp,
) -> Result<(), JobValidationError> {
    validate_policy_reference(specification.plan.as_ref(), "run.plan")?;
    require_token_id(
        specification
            .run_id
            .as_ref()
            .map(|value| value.value.as_str()),
        "run.run_id",
    )?;
    validate_actor(specification.owner.as_ref(), "run.owner")?;
    validate_actor(specification.executor.as_ref(), "run.executor")?;
    let owner = specification.owner.as_ref().ok_or(error("run.owner"))?;
    let executor = specification
        .executor
        .as_ref()
        .ok_or(error("run.executor"))?;
    if owner.kind != ActorKind::Human as i32
        || executor.kind != ActorKind::Agent as i32
        || owner.actor_id == executor.actor_id
        || owner.authenticated_subject == executor.authenticated_subject
    {
        return Err(error("run.actors"));
    }
    if !(1..=64).contains(&specification.maximum_rounds) {
        return Err(error("run.maximum_rounds"));
    }
    validate_budget(specification.budget.as_ref())?;
    let discovery = specification
        .discovery
        .as_ref()
        .ok_or(error("run.discovery"))?;
    validate_discovery_input(discovery, submitted_at)?;
    let protocol = specification
        .protocol_selection
        .as_ref()
        .ok_or(error("run.protocol_selection"))?;
    let digest = protocol_selection_sha256(protocol)?;
    if protocol
        .selection_sha256
        .as_ref()
        .map(|value| value.value.as_slice())
        != Some(digest.as_slice())
        || require_timestamp(protocol.selected_at.as_ref(), "run.protocol_selection")?
            > require_timestamp(Some(submitted_at), "run.submitted_at")?
    {
        return Err(error("run.protocol_selection"));
    }
    Ok(())
}

/// Validate an untrusted run projection before persistence or display.
///
/// Unknown/unspecified status, missing identity, contradictory completion,
/// noncanonical USD, and reservations above frozen ceilings fail closed. This
/// validates a snapshot, not authority or the authenticity of historical spend.
pub fn validate_view(view: &RunView) -> Result<(), JobValidationError> {
    require_token_id(
        view.run_id.as_ref().map(|value| value.value.as_str()),
        "run.run_id",
    )?;
    let status = RunStatus::try_from(view.status).map_err(|_| JobValidationError {
        code: JobValidationCode::UnknownEnum,
        field: "run.status",
    })?;
    if status == RunStatus::Unspecified {
        return Err(JobValidationError {
            code: JobValidationCode::UnknownEnum,
            field: "run.status",
        });
    }
    if view.revision == 0 || view.revision > i64::MAX as u64 {
        return Err(error("run.revision"));
    }
    if !(1..=64).contains(&view.maximum_rounds)
        || view.completed_rounds > view.maximum_rounds
        || (status == RunStatus::Completed && view.completed_rounds != view.maximum_rounds)
        || (status == RunStatus::Active && view.completed_rounds == view.maximum_rounds)
    {
        return Err(error("run.rounds"));
    }
    validate_budget(view.budget.as_ref())?;
    let budget = view.budget.as_ref().ok_or(error("run.budget"))?;
    if view.reserved_steps == 0
        || view.reserved_steps > budget.maximum_steps
        || view.reserved_input_tokens == 0
        || view.reserved_input_tokens > budget.maximum_input_tokens
        || view.reserved_output_tokens == 0
        || view.reserved_output_tokens > budget.maximum_output_tokens
        || usd_nanos(view.reserved_cost.as_ref())? == 0
        || usd_nanos(view.reserved_cost.as_ref())? > usd_nanos(budget.maximum_cost.as_ref())?
    {
        return Err(error("run.reservation"));
    }
    let submitted = require_timestamp(view.submitted_at.as_ref(), "run.submitted_at")?;
    let updated = require_timestamp(view.updated_at.as_ref(), "run.updated_at")?;
    let deadline = require_timestamp(view.deadline.as_ref(), "run.deadline")?;
    let wall = budget
        .maximum_wall_time
        .as_ref()
        .ok_or(error("run.budget.wall_time"))?;
    let elapsed =
        i128::from(deadline.0 - submitted.0) * 1_000_000_000 + i128::from(deadline.1 - submitted.1);
    if updated < submitted
        || deadline <= submitted
        || elapsed != i128::from(wall.seconds) * 1_000_000_000 + i128::from(wall.nanos)
    {
        return Err(error("run.timestamps"));
    }
    let child = view.current_job.as_ref().ok_or(error("run.current_job"))?;
    require_token_id(
        child.job_id.as_ref().map(|value| value.value.as_str()),
        "run.current_job",
    )?;
    let child_status =
        DiscoveryJobStatus::try_from(child.status).map_err(|_| JobValidationError {
            code: JobValidationCode::UnknownEnum,
            field: "run.current_job.status",
        })?;
    if child_status == DiscoveryJobStatus::Unspecified
        || child.revision == 0
        || child.revision > i64::MAX as u64
        || (status == RunStatus::Completed && child_status != DiscoveryJobStatus::Succeeded)
    {
        return Err(error("run.current_job"));
    }
    let child_submitted =
        require_timestamp(child.submitted_at.as_ref(), "run.current_job.submitted_at")?;
    let child_updated = require_timestamp(child.updated_at.as_ref(), "run.current_job.updated_at")?;
    if child_submitted < submitted || child_updated < child_submitted {
        return Err(error("run.current_job.timestamps"));
    }
    Ok(())
}

fn validate_budget(budget: Option<&RunBudget>) -> Result<(), JobValidationError> {
    let budget = budget.ok_or(error("run.budget"))?;
    if budget.maximum_steps == 0
        || budget.maximum_steps > i64::MAX as u64
        || budget.maximum_input_tokens == 0
        || budget.maximum_input_tokens > i64::MAX as u64
        || budget.maximum_output_tokens == 0
        || budget.maximum_output_tokens > i64::MAX as u64
    {
        return Err(error("run.budget"));
    }
    if usd_nanos(budget.maximum_cost.as_ref())? == 0 {
        return Err(error("run.budget.cost"));
    }
    let duration = budget
        .maximum_wall_time
        .as_ref()
        .ok_or(error("run.budget.wall_time"))?;
    if duration.seconds < 0
        || duration.seconds > MAX_WALL_SECONDS
        || !(0..1_000_000_000).contains(&duration.nanos)
        || duration.nanos % 1_000_000 != 0
        || (duration.seconds == 0 && duration.nanos == 0)
        || (duration.seconds == MAX_WALL_SECONDS && duration.nanos != 0)
    {
        return Err(error("run.budget.wall_time"));
    }
    Ok(())
}

fn usd_nanos(value: Option<&Money>) -> Result<u64, JobValidationError> {
    validate_money(value, "run.money")?;
    let value = value.ok_or(error("run.money"))?;
    let amount = &value.amount.as_ref().ok_or(error("run.money"))?.value;
    let (whole, fraction) = amount.split_once('.').unwrap_or((amount, ""));
    if value.currency_code != "USD" || whole.len() > 6 || fraction.len() > 9 {
        return Err(error("run.money"));
    }
    let whole = whole.parse::<u64>().map_err(|_| error("run.money"))?;
    let fractional = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().map_err(|_| error("run.money"))?
            * 10_u64.pow(9 - fraction.len() as u32)
    };
    Ok(whole * 1_000_000_000 + fractional)
}

fn error(field: &'static str) -> JobValidationError {
    JobValidationError {
        code: JobValidationCode::InvalidEnvelope,
        field,
    }
}
