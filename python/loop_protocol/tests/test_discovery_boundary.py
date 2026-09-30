import hashlib
from pathlib import Path

from loop.discovery.v1 import service_pb2
from loop.v1 import development_data_pb2
from loop.v1.common_pb2 import JobId

_FIXTURES = Path(__file__).resolve().parents[3] / "fixtures/contracts/protocol/v1"

_FORBIDDEN = (
    "approval",
    "backtestspec",
    "grant",
    "holdout",
    "joblease",
    "joboutcome",
    "jobrecord",
    "jobspecification",
)


def test_execute_request() -> None:
    request = service_pb2.ExecuteDiscoveryRequest.FromString(
        (_FIXTURES / "discovery_execute_v1.binpb").read_bytes()
    )
    assert request.job_id.value == "discovery.1"
    assert request.expected_revision == 3
    assert request.context.request_id.value == "execute.1"
    assert service_pb2.ExecuteDiscoveryRequest.FromString(request.SerializeToString()) == request
    service = service_pb2.DESCRIPTOR.services_by_name["DiscoveryService"]
    assert set(service.methods_by_name) == {
        "StartDiscovery",
        "ExecuteDiscovery",
        "GetDiscovery",
        "PauseDiscovery",
        "CancelDiscovery",
        "ExpireDiscovery",
        "ResumeDiscovery",
        "ReconcileDiscovery",
    }
    assert service.methods_by_name["ExecuteDiscovery"].input_type is request.DESCRIPTOR


def test_completed_candidate() -> None:
    response = service_pb2.ExecuteDiscoveryResponse.FromString(
        (_FIXTURES / "discovery_completed_v1.binpb").read_bytes()
    )
    step = response.step
    assert step.state == service_pb2.DISCOVERY_STEP_STATE_COMPLETED
    assert step.job.status == service_pb2.DISCOVERY_JOB_STATUS_SUCCEEDED
    assert step.job.revision == 5
    assert step.reserved_input_tokens == 4096
    assert step.reserved_output_tokens == 1024
    assert step.reserved_cost.amount.value == "0.125"
    assert step.reserved_cost.currency_code == "USD"
    candidate = step.candidate
    assert candidate.canonicalization_profile == "loop.factor-ast/v1"
    assert candidate.canonical_json == b'{"node":"field","field":"market.close"}'
    expected = hashlib.sha256(b"loop.factor-ast/v1\0" + candidate.canonical_json).hexdigest()
    assert candidate.expression_id.value == f"sha256:{expected}"
    assert service_pb2.ExecuteDiscoveryResponse.FromString(response.SerializeToString()) == response


def test_lifecycle_envelope() -> None:
    service = service_pb2.DESCRIPTOR.services_by_name["DiscoveryService"]
    for name in ("Pause", "Cancel", "Expire", "Resume", "Reconcile"):
        method = service.methods_by_name[f"{name}Discovery"]
        assert {field.name: field.number for field in method.input_type.fields} == {
            "context": 1,
            "job_id": 2,
            "expected_revision": 3,
        }
        expected = "step" if name in ("Resume", "Reconcile") else "job"
        assert [field.name for field in method.output_type.fields] == [expected]


def test_paused_status() -> None:
    response = service_pb2.PauseDiscoveryResponse.FromString(bytes((10, 2, 16, 9)))
    assert response.job.status == service_pb2.DISCOVERY_JOB_STATUS_PAUSED
    assert response.SerializeToString() == bytes((10, 2, 16, 9))


def test_unknown_step() -> None:
    step = service_pb2.DiscoveryStepView.FromString(bytes((16, 127)))
    assert step.state == 127
    assert step.state not in service_pb2.DiscoveryStepState.values()
    assert not step.HasField("candidate")
    assert not step.HasField("reserved_cost")


# Scenario: generated discovery module has only safe dependencies.
def test_generated_discovery() -> None:
    assert {dependency.name for dependency in service_pb2.DESCRIPTOR.dependencies} == {
        "google/protobuf/duration.proto",
        "google/protobuf/timestamp.proto",
        "loop/v1/common.proto",
        "loop/v1/development_data.proto",
        "loop/v1/model.proto",
    }
    assert not hasattr(service_pb2, "loop_dot_v1_dot_data__pb2")
    assert service_pb2.loop_dot_v1_dot_development__data__pb2 is development_data_pb2


# Scenario: development dataset module is a safe dependency leaf.
def test_development_dataset() -> None:
    descriptor = development_data_pb2.DESCRIPTOR
    assert {dependency.name for dependency in descriptor.dependencies} == {"loop/v1/common.proto"}
    assert set(descriptor.message_types_by_name) == {"DevelopmentDatasetReference"}
    assert not descriptor.enum_types_by_name
    assert not hasattr(development_data_pb2, "SampleRole")
    assert not hasattr(development_data_pb2, "SampleWindow")
    assert not hasattr(development_data_pb2, "DataSnapshot")


# Scenario: discovery request and response graph cannot reach holdout or generic job.
def test_discovery_request() -> None:
    service = service_pb2.DESCRIPTOR.services_by_name["DiscoveryService"]
    roots = [
        message for method in service.methods for message in (method.input_type, method.output_type)
    ]
    visited: set[str] = set()
    pending = roots[:]
    while pending:
        message = pending.pop()
        if message.full_name in visited:
            continue
        visited.add(message.full_name)
        _assert_safe_name(message.full_name)
        for field in message.fields:
            if field.message_type is not None:
                _assert_safe_name(field.message_type.full_name)
                pending.append(field.message_type)
            if field.enum_type is not None:
                _assert_safe_name(field.enum_type.full_name)

    assert "loop.discovery.v1.DiscoveryJobInput" in visited
    assert "loop.discovery.v1.DiscoveryJobBudget" in visited
    assert "loop.discovery.v1.DiscoveryJobHandle" in visited
    assert "loop.v1.JobRecord" not in visited
    assert "loop.v1.JobSpecification" not in visited


# Scenario: discovery response round trip has no specification or outcome.
def test_discovery_response() -> None:
    response = service_pb2.StartDiscoveryResponse(
        job=service_pb2.DiscoveryJobHandle(
            job_id=JobId(value="job.discovery.0001"),
            status=service_pb2.DISCOVERY_JOB_STATUS_RUNNING,
            revision=7,
        )
    )
    decoded = service_pb2.StartDiscoveryResponse.FromString(response.SerializeToString())

    assert decoded.job.job_id.value == "job.discovery.0001"
    assert decoded.job.status == service_pb2.DISCOVERY_JOB_STATUS_RUNNING
    assert decoded.job.revision == 7
    assert {field.name for field in decoded.job.DESCRIPTOR.fields} == {
        "job_id",
        "status",
        "revision",
        "submitted_at",
        "updated_at",
    }


def _assert_safe_name(value: str) -> None:
    lowered = value.lower()
    assert not any(token in lowered for token in _FORBIDDEN), value
