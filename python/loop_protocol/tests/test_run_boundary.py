import json
from pathlib import Path
from typing import Any

import pytest

from loop.discovery.v1 import service_pb2 as discovery
from loop.runs.v1 import service_pb2 as runs
from loop_protocol.runs import validate_view

_FIXTURE = json.loads(
    (
        Path(__file__).resolve().parents[3] / "fixtures/contracts/protocol/v1/run_view_v1.json"
    ).read_text(encoding="utf-8")
)
_WIRE = bytes.fromhex(_FIXTURE["wire_hex"])


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
    assert [field.name for field in runs.StartRunRequest.DESCRIPTOR.fields] == ["context", "plan"]
    assert [field.name for field in runs.StepRunRequest.DESCRIPTOR.fields] == [
        "context",
        "run_id",
        "expected_revision",
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
