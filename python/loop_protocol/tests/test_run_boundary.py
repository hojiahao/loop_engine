import json
from pathlib import Path
from typing import Any

import pytest

from loop.discovery.v1 import service_pb2 as discovery
from loop.research.v1 import service_pb2 as research
from loop.runs.v1 import service_pb2 as runs
from loop_protocol.runs import validate_view, validate_workflow

_FIXTURE = json.loads(
    (
        Path(__file__).resolve().parents[3] / "fixtures/contracts/protocol/v1/run_view_v1.json"
    ).read_text(encoding="utf-8")
)
_WIRE = bytes.fromhex(_FIXTURE["wire_hex"])
_NUMERICAL = bytes.fromhex(
    json.loads(
        (
            Path(__file__).resolve().parents[3] / "fixtures/contracts/protocol/v1/run_view_v2.json"
        ).read_text(encoding="utf-8")
    )["wire_hex"]
)


def test_numerical_golden() -> None:
    view = runs.RunView.FromString(_NUMERICAL)
    validate_view(view)
    assert view.SerializeToString(deterministic=True) == _NUMERICAL
    validate_workflow(0)
    validate_workflow(2)
    for version in (1, 3, -1, True):
        with pytest.raises(ValueError):
            validate_workflow(version)


def test_numerical_terminals() -> None:
    view = runs.RunView.FromString(_NUMERICAL)
    view.status = runs.RUN_STATUS_COMPLETED
    view.completed_rounds = view.maximum_rounds
    view.research.disposition = runs.ROUND_DISPOSITION_INSUFFICIENT_COVERAGE
    view.research.research.status = research.RESEARCH_JOB_STATUS_SUCCEEDED
    validate_view(view)
    view.research.stage = runs.RUN_STAGE_PORTFOLIO
    view.research.disposition = runs.ROUND_DISPOSITION_EVALUATED
    view.research.research.job_id.value = "job.portfolio.2"
    validate_view(view)
    view.research.disposition = 0
    with pytest.raises(ValueError):
        validate_view(view)


@pytest.mark.parametrize(
    ("path", "value"),
    [
        ("revision", 8),
        ("research.round", 0),
        ("research.round", 1),
        ("research.stage", 127),
        ("research.stage", 0),
        ("research.stage", runs.RUN_STAGE_DISCOVERY),
        ("research.binding_sha256", None),
        ("research.binding_sha256.value", b"a" * 31),
        ("research.research", None),
        ("research.evaluation_job_id", None),
        ("research.discovery_job_id", None),
        ("research.expression_id", None),
        ("research.factor_spec_id.value", "sha256:" + "b" * 64 + "\n"),
        ("research.disposition", 127),
        ("research.disposition", runs.ROUND_DISPOSITION_EVALUATED),
        ("research.disposition", runs.ROUND_DISPOSITION_INSUFFICIENT_COVERAGE),
        ("research.discovery_job_id.value", "job.evaluation.2"),
    ],
)
def test_numerical_failures(path: str, value: int | str | bytes | None) -> None:
    view = runs.RunView.FromString(_NUMERICAL)
    target: Any = view
    *parents, field = path.split(".")
    for parent in parents:
        target = getattr(target, parent)
    if value is None:
        target.ClearField(field)
    else:
        setattr(target, field, value)
    with pytest.raises(ValueError):
        validate_view(view)


def test_ambiguous_handle() -> None:
    view = runs.RunView.FromString(_NUMERICAL)
    view.current_job.CopyFrom(runs.RunView.FromString(_WIRE).current_job)
    with pytest.raises(ValueError):
        validate_view(view)


def test_discovery_filter() -> None:
    view = runs.RunView.FromString(_NUMERICAL)
    view.status = runs.RUN_STATUS_COMPLETED
    view.completed_rounds = view.maximum_rounds
    view.research.stage = runs.RUN_STAGE_DISCOVERY
    view.research.discovery.CopyFrom(runs.RunView.FromString(_WIRE).current_job)
    view.research.discovery.status = discovery.DISCOVERY_JOB_STATUS_SUCCEEDED
    view.research.ClearField("evaluation_job_id")
    for disposition in (
        runs.ROUND_DISPOSITION_DUPLICATE,
        runs.ROUND_DISPOSITION_PREVIOUSLY_REJECTED,
    ):
        view.research.disposition = disposition
        validate_view(view)


def test_legacy_version() -> None:
    assert runs.StartRunRequest().SerializeToString(deterministic=True) == b""
    assert (
        runs.StartRunRequest(workflow_version=2).SerializeToString(deterministic=True)
        == b"\x18\x02"
    )


def test_exact_reservations() -> None:
    view = runs.RunView.FromString(_WIRE)
    validate_view(view)
    assert str(view.revision) == _FIXTURE["revision"]
    assert str(view.reserved_input_tokens) == _FIXTURE["reserved_input_tokens"]
    assert view.reserved_cost.amount.value == _FIXTURE["reserved_cost"]
    assert view.SerializeToString(deterministic=True) == _WIRE


def test_pending_advancement() -> None:
    view = runs.RunView.FromString(_WIRE)
    view.current_job.status = discovery.DISCOVERY_JOB_STATUS_SUCCEEDED
    view.current_job.updated_at.seconds += 2
    view.plan_verified = False
    validate_view(view)


@pytest.mark.parametrize(
    ("path", "value"),
    [
        ("run_id.value", "run\n"),
        ("reserved_cost.amount.value", "1\n"),
        ("reserved_input_tokens", 0),
        ("reserved_output_tokens", 0),
        ("budget.maximum_input_tokens", 0),
        ("budget.maximum_output_tokens", 0),
        ("reserved_cost.amount.value", "0"),
        ("status", 127),
        ("status", 0),
        ("revision", 0),
        ("revision", 18_446_744_073_709_551_615),
        ("budget", None),
        ("current_job", None),
        ("maximum_rounds", 65),
        ("completed_rounds", 3),
        ("completed_rounds", 2),
        ("status", runs.RUN_STATUS_COMPLETED),
        ("reserved_steps", 9),
        ("reserved_input_tokens", 9_007_199_254_740_994),
        ("reserved_output_tokens", 2049),
        ("reserved_cost.currency_code", "EUR"),
        ("reserved_cost.amount.value", "0.6"),
        ("reserved_cost.amount.value", "0.1250"),
        ("reserved_cost.amount.value", "0.0000000001"),
        ("reserved_cost.amount.value", "-0.1"),
        ("budget.maximum_wall_time.nanos", 1),
        ("deadline.seconds", 2_000_000_301),
        ("updated_at.seconds", 1_999_999_999),
        ("submitted_at", None),
        ("current_job.status", 127),
        ("current_job.revision", 0),
        ("current_job.submitted_at.seconds", 1_999_999_999),
        ("run_id.value", "run bad"),
    ],
)
def test_invalid_views(path: str, value: int | str | None) -> None:
    view = runs.RunView.FromString(_WIRE)
    target: Any = view
    *parents, field = path.split(".")
    for parent in parents:
        target = getattr(target, parent)
    if value is None:
        target.ClearField(field)
    else:
        setattr(target, field, value)
    with pytest.raises(ValueError):
        validate_view(view)


def test_operator_surface() -> None:
    service = runs.DESCRIPTOR.services_by_name["RunService"]
    assert list(service.methods_by_name) == ["StartRun", "StepRun", "GetRun"]
    assert [field.name for field in runs.StartRunRequest.DESCRIPTOR.fields] == [
        "context",
        "plan",
        "workflow_version",
    ]
    assert [field.name for field in runs.StepRunRequest.DESCRIPTOR.fields] == [
        "context",
        "run_id",
        "expected_revision",
        "workflow_version",
    ]
    visited: set[str] = set()
    pending = [
        item for method in service.methods for item in (method.input_type, method.output_type)
    ]
    while pending:
        message = pending.pop()
        if message.full_name in visited:
            continue
        visited.add(message.full_name)
        pending.extend(field.message_type for field in message.fields if field.message_type)
    assert "loop.discovery.v1.DiscoveryJobHandle" in visited
    assert not any(
        token in name
        for name in visited
        for token in (
            "RunSpecification",
            "DiscoveryJobInput",
            "DiscoveryCandidate",
            "Holdout",
            "Grant",
        )
    )
