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
use crate::wire::research::v1::{ResearchJobBudget, ResearchJobStatus};
use crate::wire::runs::v1::{
    RoundDisposition, RunBudget, RunSpecification, RunStage, RunStatus, RunView,
    research_run_view::CurrentChild,
};
use crate::wire::v1::{ActorKind, JobBudget, Money, ProtocolSelectionSnapshot};

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
    if let Some(numerical) = &specification.numerical {
        validate_actor(
            numerical.research_executor.as_ref(),
            "run.research_executor",
        )?;
        let research = numerical
            .research_executor
            .as_ref()
            .ok_or(error("run.research_executor"))?;
        if research.kind != ActorKind::Service as i32
            || [owner, executor].iter().any(|actor| {
                actor.actor_id == research.actor_id
                    || actor.authenticated_subject == research.authenticated_subject
            })
        {
            return Err(error("run.research_executor"));
        }
        validate_policy_reference(numerical.recipe.as_ref(), "run.recipe")?;
        numerical_budget(numerical.evaluation_budget.as_ref())?;
        numerical_budget(numerical.portfolio_budget.as_ref())?;
        validate_protocol(numerical.protocol_selection.as_ref(), submitted_at)?;
    }
    Ok(())
}

/// Reject unknown workflow versions before observation, mutation or dispatch.
/// Zero preserves the original discovery-only contract; two selects numerical
/// workflow semantics. This check does not resolve plans or establish authority.
pub fn validate_workflow(version: u32) -> Result<(), JobValidationError> {
    if matches!(version, 0 | 2) {
        Ok(())
    } else {
        Err(error("run.workflow_version"))
    }
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
    if view.research.is_some() {
        return validate_research(view, status, submitted);
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

fn validate_research(
    view: &RunView,
    status: RunStatus,
    submitted: (i64, i32),
) -> Result<(), JobValidationError> {
    let research = view.research.as_ref().ok_or(error("run.research"))?;
    let stage = RunStage::try_from(research.stage).map_err(|_| error("run.stage"))?;
    let disposition =
        RoundDisposition::try_from(research.disposition).map_err(|_| error("run.disposition"))?;
    if view.current_job.is_some()
        || view.revision > u64::from(view.maximum_rounds) * 3 + 1
        || !(1..=view.maximum_rounds).contains(&research.round)
        || research.round < view.completed_rounds
        || research.round > view.completed_rounds + 1
        || (status == RunStatus::Active && research.round != view.completed_rounds + 1)
        || (status == RunStatus::Completed && research.round != view.completed_rounds)
        || research
            .binding_sha256
            .as_ref()
            .is_none_or(|digest| digest.value.len() != 32)
    {
        return Err(error("run.research"));
    }
    require_token_id(
        research
            .discovery_job_id
            .as_ref()
            .map(|id| id.value.as_str()),
        "run.discovery_job_id",
    )?;
    let filtered = matches!(
        disposition,
        RoundDisposition::Duplicate | RoundDisposition::PreviouslyRejected
    );
    let has_factor = stage != RunStage::Discovery || filtered;
    if has_factor {
        factor_id(research.expression_id.as_ref().map(|id| id.value.as_str()))?;
        factor_id(research.factor_spec_id.as_ref().map(|id| id.value.as_str()))?;
    } else if research.expression_id.is_some() || research.factor_spec_id.is_some() {
        return Err(error("run.factor_ids"));
    }
    if stage == RunStage::Discovery {
        if research.evaluation_job_id.is_some()
            || (!filtered && disposition != RoundDisposition::Unspecified)
        {
            return Err(error("run.discovery"));
        }
    } else {
        require_token_id(
            research
                .evaluation_job_id
                .as_ref()
                .map(|id| id.value.as_str()),
            "run.evaluation_job_id",
        )?;
        if research.evaluation_job_id == research.discovery_job_id {
            return Err(error("run.evaluation_job_id"));
        }
    }
    if (filtered && stage != RunStage::Discovery)
        || (disposition == RoundDisposition::InsufficientCoverage && stage != RunStage::Evaluation)
        || (disposition == RoundDisposition::Evaluated && stage != RunStage::Portfolio)
        || (status == RunStatus::Active && disposition != RoundDisposition::Unspecified)
        || (status == RunStatus::Completed && disposition == RoundDisposition::Unspecified)
    {
        return Err(error("run.disposition"));
    }
    let (job_id, revision, child_submitted, child_updated, succeeded) = match (
        &research.current_child,
        stage,
    ) {
        (Some(CurrentChild::Discovery(child)), RunStage::Discovery) => {
            if child.job_id != research.discovery_job_id
                || !matches!(DiscoveryJobStatus::try_from(child.status), Ok(value) if value != DiscoveryJobStatus::Unspecified)
            {
                return Err(error("run.current_child"));
            }
            (
                &child.job_id,
                child.revision,
                &child.submitted_at,
                &child.updated_at,
                child.status == DiscoveryJobStatus::Succeeded as i32,
            )
        }
        (Some(CurrentChild::Research(child)), RunStage::Evaluation | RunStage::Portfolio) => {
            if (stage == RunStage::Evaluation && child.job_id != research.evaluation_job_id)
                || (stage == RunStage::Portfolio
                    && (child.job_id == research.discovery_job_id
                        || child.job_id == research.evaluation_job_id))
                || !matches!(ResearchJobStatus::try_from(child.status), Ok(value) if value != ResearchJobStatus::Unspecified)
            {
                return Err(error("run.current_child"));
            }
            (
                &child.job_id,
                child.revision,
                &child.submitted_at,
                &child.updated_at,
                child.status == ResearchJobStatus::Succeeded as i32,
            )
        }
        _ => return Err(error("run.current_child")),
    };
    require_token_id(
        job_id.as_ref().map(|id| id.value.as_str()),
        "run.current_child",
    )?;
    let child_submitted =
        require_timestamp(child_submitted.as_ref(), "run.current_child.submitted_at")?;
    if revision == 0
        || revision > i64::MAX as u64
        || (disposition != RoundDisposition::Unspecified && !succeeded)
        || child_submitted < submitted
        || require_timestamp(child_updated.as_ref(), "run.current_child.updated_at")?
            < child_submitted
    {
        return Err(error("run.current_child"));
    }
    Ok(())
}

fn factor_id(value: Option<&str>) -> Result<(), JobValidationError> {
    if value
        .and_then(|id| id.strip_prefix("sha256:"))
        .is_none_or(|digest| {
            digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        return Err(error("run.factor_ids"));
    }
    Ok(())
}

fn numerical_budget(budget: Option<&ResearchJobBudget>) -> Result<(), JobValidationError> {
    let budget = budget.ok_or(error("run.numerical_budget"))?;
    crate::job::validate_budget(
        Some(&JobBudget {
            maximum_steps: budget.maximum_steps,
            maximum_input_tokens: budget.maximum_input_tokens,
            maximum_output_tokens: budget.maximum_output_tokens,
            maximum_cost: budget.maximum_cost.clone(),
            maximum_wall_time: budget.maximum_wall_time,
        }),
        "run.numerical_budget",
    )?;
    usd_nanos(budget.maximum_cost.as_ref())?;
    if budget
        .maximum_wall_time
        .as_ref()
        .is_none_or(|wall| wall.nanos % 1_000_000 != 0)
    {
        return Err(error("run.numerical_budget"));
    }
    Ok(())
}

fn validate_protocol(
    protocol: Option<&ProtocolSelectionSnapshot>,
    submitted: &prost_types::Timestamp,
) -> Result<(), JobValidationError> {
    let protocol = protocol.ok_or(error("run.numerical_protocol"))?;
    if protocol.selected_package != "loop.v1"
        || !protocol.enabled_features.iter().map(String::as_str).eq([
            "jobs.envelope.v1",
            "jobs.kind-input.v1",
            "jobs.prelease-terminal.v1",
        ])
        || protocol
            .selection_sha256
            .as_ref()
            .map(|digest| digest.value.as_slice())
            != Some(protocol_selection_sha256(protocol)?.as_slice())
        || require_timestamp(protocol.selected_at.as_ref(), "run.numerical_protocol")?
            > require_timestamp(Some(submitted), "run.submitted_at")?
    {
        return Err(error("run.numerical_protocol"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::v1::{ExactDecimal, ProtocolLimits, Sha256Digest};

    fn budget() -> ResearchJobBudget {
        ResearchJobBudget {
            maximum_steps: 1,
            maximum_input_tokens: 0,
            maximum_output_tokens: 0,
            maximum_cost: Some(Money {
                amount: Some(ExactDecimal { value: "0".into() }),
                currency_code: "USD".into(),
            }),
            maximum_wall_time: Some(prost_types::Duration {
                seconds: 180,
                nanos: 0,
            }),
        }
    }

    #[test]
    fn numerical_zero_cost() {
        numerical_budget(Some(&budget())).unwrap();
    }

    #[test]
    fn numerical_budget_limits() {
        assert!(numerical_budget(None).is_err());
        let changes: &[fn(&mut ResearchJobBudget)] = &[
            |value| value.maximum_steps = 0,
            |value| value.maximum_steps = 1_000_001,
            |value| value.maximum_input_tokens = 1_000_000_000_001,
            |value| value.maximum_output_tokens = 1_000_000_000_001,
            |value| value.maximum_wall_time.as_mut().unwrap().nanos = 1,
            |value| value.maximum_wall_time.as_mut().unwrap().seconds = 604_801,
            |value| value.maximum_cost.as_mut().unwrap().currency_code = "EUR".into(),
        ];
        for mutate in changes {
            let mut value = budget();
            mutate(&mut value);
            assert!(numerical_budget(Some(&value)).is_err());
        }
    }

    #[test]
    fn numerical_protocol_scope() {
        let now = prost_types::Timestamp {
            seconds: 1,
            nanos: 0,
        };
        let digest = || Some(Sha256Digest { value: vec![1; 32] });
        let mut protocol = ProtocolSelectionSnapshot {
            selected_package: "loop.v1".into(),
            enabled_features: [
                "jobs.envelope.v1",
                "jobs.kind-input.v1",
                "jobs.prelease-terminal.v1",
            ]
            .map(str::to_owned)
            .to_vec(),
            effective_limits: Some(ProtocolLimits {
                maximum_unary_bytes: 4_194_304,
                maximum_stream_event_bytes: 1_048_576,
                maximum_canonical_ast_bytes: 262_144,
                maximum_ast_nodes: 4096,
                maximum_ast_depth: 64,
                maximum_page_records: 500,
                maximum_identity_bytes: 128,
                maximum_artifact_uri_bytes: 2048,
            }),
            server_build_version: "0.2.0".into(),
            client_build_version: "0.2.0".into(),
            server_build_sha256: digest(),
            client_build_sha256: digest(),
            schema_descriptor_sha256: digest(),
            selected_at: Some(now),
            selection_sha256: None,
        };
        protocol.selection_sha256 = Some(Sha256Digest {
            value: protocol_selection_sha256(&protocol).unwrap().to_vec(),
        });
        validate_protocol(Some(&protocol), &now).unwrap();
        protocol
            .enabled_features
            .push("provider.invocation-lookup.v1".into());
        protocol.selection_sha256 = Some(Sha256Digest {
            value: protocol_selection_sha256(&protocol).unwrap().to_vec(),
        });
        assert!(validate_protocol(Some(&protocol), &now).is_err());
        protocol.enabled_features.pop();
        protocol.selected_package = "loop.discovery.v1".into();
        protocol.selection_sha256 = Some(Sha256Digest {
            value: protocol_selection_sha256(&protocol).unwrap().to_vec(),
        });
        assert!(validate_protocol(Some(&protocol), &now).is_err());
    }
}
