import json
from pathlib import Path
from typing import Any

import pytest

from loop.research.v1 import service_pb2 as research
from loop.runs.v1 import service_pb2 as runs
from loop.v1 import common_pb2, job_pb2, perturbation_pb2
from loop_protocol.job import (
    JobValidationCode,
    JobValidationError,
    validate_job_record,
    validate_job_specification,
)
from loop_protocol.runs import validate_view

_FIXTURE = json.loads(
    (
        Path(__file__).parents[3] / "fixtures/contracts/protocol/v1/adaptive_workflow_v3.json"
    ).read_text()
)


def _view() -> runs.RunView:
    return runs.RunView.FromString(bytes.fromhex(_FIXTURE["view_wire_hex"]))


def _job() -> job_pb2.JobSpecification:
    return job_pb2.JobSpecification.FromString(bytes.fromhex(_FIXTURE["job_wire_hex"]))


def test_adaptive_golden() -> None:
    view, job = _view(), _job()
    validate_view(view)
    validate_job_specification(job)
    assert view.SerializeToString(deterministic=True).hex() == _FIXTURE["view_wire_hex"]
    assert job.SerializeToString(deterministic=True).hex() == _FIXTURE["job_wire_hex"]
    assert runs.StartRunRequest(workflow_version=3).SerializeToString() == b"\x18\x03"


def test_observed_golden() -> None:
    work = perturbation_pb2.PerturbationWork.FromString(
        bytes.fromhex(_FIXTURE["observed_work_wire_hex"])
    )
    assert len(work.observed_factor_ids) == 2
    assert list(work.observed_factor_ids) == [item.factor_spec_id for item in work.candidates]
    assert not work.failed_factor_ids
    assert not work.state.proposed_factor_ids
    assert perturbation_pb2.PerturbationWork.FromString(work.SerializeToString()) == work


@pytest.mark.parametrize(
    "path,value",
    [
        ("adaptive", None),
        ("adaptive.feedback_policy", None),
        ("adaptive.source_count", 17),
        ("adaptive.source_count", 2),
        ("adaptive.feedback_sha256", None),
        ("adaptive.model_context_sha256", None),
        ("adaptive.family_sha256", None),
        ("adaptive.state_sha256", None),
        ("adaptive.random_draws", 97),
        ("research.discovery_job_id.value", "fake.discovery"),
        ("research.perturbation_job_id", None),
        ("research.round", 1),
        ("research.stage", runs.RUN_STAGE_DISCOVERY),
        ("research.research.job_id.value", "wrong.job"),
        ("status", runs.RUN_STATUS_SEARCH_EXHAUSTED),
    ],
)
def test_adaptive_invalid(path: str, value: int | str | None) -> None:
    view = _view()
    _mutate(view, path, value)
    with pytest.raises(ValueError):
        validate_view(view)


@pytest.mark.parametrize(
    "path,value",
    [
        ("perturbation.dataset", None),
        ("perturbation.feedback_policy", None),
        ("perturbation.parent_binding_sha256", None),
        ("perturbation.family_sha256", None),
        ("perturbation.source_code_sha256", None),
        ("perturbation.environment_sha256", None),
        ("perturbation.work", None),
        ("perturbation.work.schema.name", "loop.other"),
        ("perturbation.work.byte_size", 1048577),
        ("perturbation.work.created_at.seconds", 3),
        ("perturbation.algorithm", "unknown"),
        ("perturbation.budget.maximum_steps", 2),
        ("perturbation.budget.maximum_input_tokens", 1),
        ("perturbation.budget.maximum_output_tokens", 1),
        ("perturbation.budget.maximum_cost.amount.value", "1"),
        ("perturbation.budget.maximum_cost.currency_code", "EUR"),
        ("perturbation.budget.maximum_wall_time.seconds", 181),
        ("submitted_by.kind", common_pb2.ACTOR_KIND_AGENT),
    ],
)
def test_perturbation_invalid(path: str, value: int | str | None) -> None:
    job = _job()
    _mutate(job, path, value)
    with pytest.raises(JobValidationError):
        validate_job_specification(job)


@pytest.mark.parametrize(
    "seconds,nanos,valid",
    [
        (20, 0, True),
        (179, 999_000_000, True),
        (180, 0, True),
        (180, 1_000_000, False),
        (181, 0, False),
        (0, 0, False),
        (179, 1, False),
    ],
)
def test_perturbation_deadlines(seconds: int, nanos: int, valid: bool) -> None:
    # Preserve old 20-second jobs and enforce the new total-task boundary.
    job = _job()
    wall = job.perturbation.budget.maximum_wall_time
    wall.seconds, wall.nanos = seconds, nanos
    if valid:
        validate_job_specification(job)
    else:
        with pytest.raises(JobValidationError) as caught:
            validate_job_specification(job)
        assert caught.value.code == JobValidationCode.INVALID_BUDGET


def _exhausted_job(attempt: int = 0) -> job_pb2.JobRecord:
    job = _job()
    record = job_pb2.JobRecord(
        specification=job,
        state=job_pb2.JOB_STATE_BUDGET_EXHAUSTED,
        revision=attempt + 2,
        attempt=attempt,
        updated_at=job.submitted_at,
    )
    record.updated_at.seconds += 20
    exhausted = record.outcome.budget_exhaustion
    exhausted.exhausted_limit = "maximum_wall_time"
    exhausted.enforced_budget.CopyFrom(job.perturbation.budget)
    exhausted.exhausted_at.CopyFrom(record.updated_at)
    return record


@pytest.mark.parametrize("attempt", [0, 1])
def test_perturbation_exhausted(attempt: int) -> None:
    # Deadline expiry remains a valid terminal receipt before or after leasing.
    record = _exhausted_job(attempt)
    shape = validate_job_record(record)
    assert shape.kind == job_pb2.JOB_KIND_PERTURBATION
    assert shape.state == job_pb2.JOB_STATE_BUDGET_EXHAUSTED


@pytest.mark.parametrize("path,value", [("maximum_steps", 2), ("maximum_wall_time.seconds", 19)])
def test_perturbation_budget(path: str, value: int) -> None:
    # A well-formed receipt cannot substitute a different enforced allowance.
    record = _exhausted_job()
    _mutate(record.outcome.budget_exhaustion.enforced_budget, path, value)
    with pytest.raises(JobValidationError) as caught:
        validate_job_record(record)
    assert caught.value.code == JobValidationCode.BINDING_MISMATCH
    assert caught.value.field == "outcome.budget_exhaustion.enforced_budget"


def test_search_exhaustion() -> None:
    view = _view()
    view.status = runs.RUN_STATUS_SEARCH_EXHAUSTED
    view.completed_rounds = 2
    view.research.disposition = runs.ROUND_DISPOSITION_EXHAUSTED
    view.research.research.status = research.RESEARCH_JOB_STATUS_SUCCEEDED
    validate_view(view)
    assert view.completed_rounds < view.maximum_rounds
    view.research.research.status = research.RESEARCH_JOB_STATUS_FACTOR_REJECTED
    with pytest.raises(ValueError):
        validate_view(view)


def test_legacy_reinterpretation() -> None:
    # Published unknown tag 16 is now a known but contradictory empty input.
    job = job_pb2.JobSpecification.FromString(bytes.fromhex("1805820100"))
    assert job.WhichOneof("input") == "perturbation"
    with pytest.raises(JobValidationError):
        validate_job_specification(job)


def _mutate(target: Any, path: str, value: int | str | None) -> None:
    *parents, field = path.split(".")
    for parent in parents:
        target = getattr(target, parent)
    if value is None:
        target.ClearField(field)
    else:
        setattr(target, field, value)
