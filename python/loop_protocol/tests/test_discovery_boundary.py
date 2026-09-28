from loop.discovery.v1 import service_pb2
from loop.v1 import development_data_pb2
from loop.v1.common_pb2 import JobId

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
