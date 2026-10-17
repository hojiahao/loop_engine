"""Run projection shape checks; server authority and spend evidence stay separate."""

from loop.discovery.v1 import service_pb2 as discovery
from loop.research.v1 import service_pb2 as research
from loop.runs.v1 import service_pb2 as runs
from loop.v1.common_pb2 import Money

from .job import (
    _require_timestamp,
    _require_token_id,
    _Timestamp,
    _validate_money,
    _validate_policy,
)

_MAX_SIGNED = 9_223_372_036_854_775_807
_MAX_WALL = 2_592_000


def validate_workflow(version: int) -> None:
    """Reject unknown workflow semantics before observation, mutation or dispatch."""
    if type(version) is not int or version not in (0, 2, 3):
        raise ValueError("invalid_workflow_version")


def validate_view(view: runs.RunView) -> None:
    """Reject missing fields, unknown states, contradictory rounds and overspend."""
    _require_token_id(view.run_id.value if view.HasField("run_id") else None, "run.run_id")
    if (
        view.status
        not in (
            runs.RUN_STATUS_ACTIVE,
            runs.RUN_STATUS_COMPLETED,
            runs.RUN_STATUS_BUDGET_EXHAUSTED,
            runs.RUN_STATUS_INFRASTRUCTURE_FAILED,
            runs.RUN_STATUS_DEADLINE_EXCEEDED,
            runs.RUN_STATUS_SEARCH_EXHAUSTED,
        )
        or not 1 <= view.revision <= _MAX_SIGNED
    ):
        raise ValueError("invalid_run_view")
    if (
        not 1 <= view.maximum_rounds <= 64
        or view.completed_rounds > view.maximum_rounds
        or (
            view.status == runs.RUN_STATUS_COMPLETED
            and view.completed_rounds != view.maximum_rounds
        )
        or (view.status == runs.RUN_STATUS_ACTIVE and view.completed_rounds == view.maximum_rounds)
    ):
        raise ValueError("invalid_run_view")
    budget = view.budget
    if (
        not view.HasField("budget")
        or not 1 <= budget.maximum_steps <= _MAX_SIGNED
        or not 1 <= budget.maximum_input_tokens <= _MAX_SIGNED
        or not 1 <= budget.maximum_output_tokens <= _MAX_SIGNED
    ):
        raise ValueError("invalid_run_view")
    wall = budget.maximum_wall_time
    if (
        not budget.HasField("maximum_wall_time")
        or not 0 <= wall.seconds <= _MAX_WALL
        or not 0 <= wall.nanos < 1_000_000_000
        or wall.nanos % 1_000_000 != 0
        or (wall.seconds == 0 and wall.nanos == 0)
        or (wall.seconds == _MAX_WALL and wall.nanos != 0)
    ):
        raise ValueError("invalid_run_view")
    if (
        not 1 <= view.reserved_steps <= budget.maximum_steps
        or not 1 <= view.reserved_input_tokens <= budget.maximum_input_tokens
        or not 1 <= view.reserved_output_tokens <= budget.maximum_output_tokens
        or _usd_nanos(view.reserved_cost if view.HasField("reserved_cost") else None) == 0
        or _usd_nanos(budget.maximum_cost if budget.HasField("maximum_cost") else None) == 0
        or _usd_nanos(view.reserved_cost if view.HasField("reserved_cost") else None)
        > _usd_nanos(budget.maximum_cost if budget.HasField("maximum_cost") else None)
    ):
        raise ValueError("invalid_run_view")
    submitted = _time_value(view.submitted_at if view.HasField("submitted_at") else None)
    deadline = _time_value(view.deadline if view.HasField("deadline") else None)
    if (
        _time_value(view.updated_at if view.HasField("updated_at") else None) < submitted
        or deadline <= submitted
        or (deadline[0] - submitted[0]) * 1_000_000_000 + deadline[1] - submitted[1]
        != wall.seconds * 1_000_000_000 + wall.nanos
    ):
        raise ValueError("invalid_run_view")
    if view.HasField("adaptive"):
        _validate_adaptive(view)
    elif view.status == runs.RUN_STATUS_SEARCH_EXHAUSTED:
        raise ValueError("invalid_adaptive_view")
    if view.HasField("research"):
        _validate_research(view)
        return
    if not view.HasField("current_job"):
        raise ValueError("invalid_run_view")
    child = view.current_job
    _require_token_id(child.job_id.value if child.HasField("job_id") else None, "run.current_job")
    if (
        child.status not in discovery.DiscoveryJobStatus.values()
        or child.status == discovery.DISCOVERY_JOB_STATUS_UNSPECIFIED
        or not 1 <= child.revision <= _MAX_SIGNED
        or (
            view.status == runs.RUN_STATUS_COMPLETED
            and child.status != discovery.DISCOVERY_JOB_STATUS_SUCCEEDED
        )
    ):
        raise ValueError("invalid_run_view")
    child_submitted = _time_value(child.submitted_at if child.HasField("submitted_at") else None)
    if (
        child_submitted < submitted
        or _time_value(child.updated_at if child.HasField("updated_at") else None) < child_submitted
    ):
        raise ValueError("invalid_run_view")


def _validate_research(view: runs.RunView) -> None:
    projection = view.research
    if (
        view.HasField("current_job")
        or view.revision > view.maximum_rounds * 3 + 1
        or not 1 <= projection.round <= view.maximum_rounds
        or not view.completed_rounds <= projection.round <= view.completed_rounds + 1
        or (view.status == runs.RUN_STATUS_ACTIVE and projection.round != view.completed_rounds + 1)
        or (
            view.status in (runs.RUN_STATUS_COMPLETED, runs.RUN_STATUS_SEARCH_EXHAUSTED)
            and projection.round != view.completed_rounds
        )
        or len(projection.binding_sha256.value) != 32
        or projection.stage
        not in (
            runs.RUN_STAGE_DISCOVERY,
            runs.RUN_STAGE_EVALUATION,
            runs.RUN_STAGE_PORTFOLIO,
            runs.RUN_STAGE_PERTURBATION,
        )
        or projection.disposition not in runs.RoundDisposition.values()
    ):
        raise ValueError("invalid_research_view")
    perturbation = projection.HasField("perturbation_job_id")
    if (
        perturbation
        and (
            not view.HasField("adaptive")
            or projection.HasField("discovery_job_id")
            or projection.round == 1
        )
    ) or (not perturbation and not projection.HasField("discovery_job_id")):
        raise ValueError("invalid_research_view")
    proposal_id = projection.perturbation_job_id if perturbation else projection.discovery_job_id
    _require_token_id(proposal_id.value, "run.proposal_job_id")
    proposal_stage = projection.stage in (runs.RUN_STAGE_DISCOVERY, runs.RUN_STAGE_PERTURBATION)
    if (projection.stage == runs.RUN_STAGE_PERTURBATION and not perturbation) or (
        projection.stage == runs.RUN_STAGE_DISCOVERY and perturbation
    ):
        raise ValueError("invalid_research_view")
    filtered = projection.disposition in (
        runs.ROUND_DISPOSITION_DUPLICATE,
        runs.ROUND_DISPOSITION_PREVIOUSLY_REJECTED,
    )
    if not proposal_stage or filtered:
        _factor_id(projection.expression_id.value if projection.HasField("expression_id") else None)
        _factor_id(
            projection.factor_spec_id.value if projection.HasField("factor_spec_id") else None
        )
    elif projection.HasField("expression_id") or projection.HasField("factor_spec_id"):
        raise ValueError("invalid_research_view")
    if proposal_stage:
        if projection.HasField("evaluation_job_id") or (
            not filtered
            and projection.disposition
            not in (runs.ROUND_DISPOSITION_UNSPECIFIED, runs.ROUND_DISPOSITION_EXHAUSTED)
        ):
            raise ValueError("invalid_research_view")
    else:
        _require_token_id(
            projection.evaluation_job_id.value
            if projection.HasField("evaluation_job_id")
            else None,
            "run.evaluation_job_id",
        )
        if projection.evaluation_job_id == proposal_id:
            raise ValueError("invalid_research_view")
    if (
        (filtered and not proposal_stage)
        or (
            projection.disposition == runs.ROUND_DISPOSITION_INSUFFICIENT_COVERAGE
            and projection.stage != runs.RUN_STAGE_EVALUATION
        )
        or (
            projection.disposition == runs.ROUND_DISPOSITION_EVALUATED
            and projection.stage != runs.RUN_STAGE_PORTFOLIO
        )
        or (
            view.status == runs.RUN_STATUS_ACTIVE
            and projection.disposition != runs.ROUND_DISPOSITION_UNSPECIFIED
        )
        or (
            view.status == runs.RUN_STATUS_COMPLETED
            and projection.disposition == runs.ROUND_DISPOSITION_UNSPECIFIED
        )
        or (
            projection.disposition == runs.ROUND_DISPOSITION_EXHAUSTED
            and (
                view.status != runs.RUN_STATUS_SEARCH_EXHAUSTED
                or projection.stage != runs.RUN_STAGE_PERTURBATION
            )
        )
        or (
            view.status == runs.RUN_STATUS_SEARCH_EXHAUSTED
            and projection.disposition != runs.ROUND_DISPOSITION_EXHAUSTED
        )
    ):
        raise ValueError("invalid_research_view")
    selected = projection.WhichOneof("current_child")
    if selected is None or (selected == "discovery") != (
        projection.stage == runs.RUN_STAGE_DISCOVERY
    ):
        raise ValueError("invalid_research_view")
    child = projection.discovery if selected == "discovery" else projection.research
    maximum = (
        discovery.DISCOVERY_JOB_STATUS_PAUSED
        if selected == "discovery"
        else research.RESEARCH_JOB_STATUS_BUDGET_EXHAUSTED
    )
    succeeded = (
        discovery.DISCOVERY_JOB_STATUS_SUCCEEDED
        if selected == "discovery"
        else research.RESEARCH_JOB_STATUS_SUCCEEDED
    )
    if (
        not 1 <= child.status <= maximum
        or not 1 <= child.revision <= _MAX_SIGNED
        or (selected == "discovery" and child.job_id != projection.discovery_job_id)
        or (
            projection.stage == runs.RUN_STAGE_EVALUATION
            and child.job_id != projection.evaluation_job_id
        )
        or (
            projection.stage == runs.RUN_STAGE_PERTURBATION
            and child.job_id != projection.perturbation_job_id
        )
        or (
            projection.stage == runs.RUN_STAGE_PORTFOLIO
            and child.job_id in (proposal_id, projection.evaluation_job_id)
        )
        or (
            projection.disposition != runs.ROUND_DISPOSITION_UNSPECIFIED
            and child.status != succeeded
        )
    ):
        raise ValueError("invalid_research_view")
    _require_token_id(child.job_id.value if child.HasField("job_id") else None, "run.current_child")
    submitted = _time_value(child.submitted_at if child.HasField("submitted_at") else None)
    if (
        submitted < _time_value(view.submitted_at)
        or _time_value(child.updated_at if child.HasField("updated_at") else None) < submitted
    ):
        raise ValueError("invalid_research_view")


def _validate_adaptive(view: runs.RunView) -> None:
    value = view.adaptive
    if not view.HasField("research"):
        raise ValueError("invalid_adaptive_view")
    _validate_policy(
        value.feedback_policy if value.HasField("feedback_policy") else None, "run.feedback_policy"
    )
    if (
        len(value.feedback_sha256.value) != 32
        or len(value.model_context_sha256.value) != 32
        or value.source_count > 16
        or value.source_count > view.completed_rounds
        or value.HasField("family_sha256") != value.HasField("state_sha256")
        or value.random_draws > view.maximum_rounds * 32
        or (
            not value.HasField("family_sha256")
            and (value.random_draws != 0 or view.research.HasField("perturbation_job_id"))
        )
        or any(
            value.HasField(field) and len(getattr(value, field).value) != 32
            for field in ("family_sha256", "state_sha256")
        )
    ):
        raise ValueError("invalid_adaptive_view")


def _factor_id(value: str | None) -> None:
    if (
        value is None
        or not value.startswith("sha256:")
        or len(value) != 71
        or any(character not in "0123456789abcdef" for character in value[7:])
    ):
        raise ValueError("invalid_factor_id")


def _time_value(value: _Timestamp | None) -> tuple[int, int]:
    return _require_timestamp(value, "run.timestamp")


def _usd_nanos(value: Money | None) -> int:
    _validate_money(value, "run.money")
    if value is None or value.currency_code != "USD":
        raise ValueError("invalid_run_view")
    whole, _, fraction = value.amount.value.partition(".")
    if len(whole) > 6:
        raise ValueError("invalid_run_view")
    return int(whole) * 1_000_000_000 + int(fraction.ljust(9, "0"))
