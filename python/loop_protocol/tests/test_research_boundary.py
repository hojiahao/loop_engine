from pathlib import Path

from loop.research.v1 import service_pb2
from loop.v1 import common_pb2, development_data_pb2

BOUNDARY_VECTORS = (
    Path(__file__).resolve().parents[3] / "tests" / "contracts" / "research_boundary_vectors.tsv"
)
EXPECTED_CLOSURE = {
    "google/protobuf/duration.proto",
    "google/protobuf/timestamp.proto",
    "loop/research/v1/service.proto",
    "loop/v1/common.proto",
    "loop/v1/development_data.proto",
    "loop/v1/factor.proto",
    "loop/v1/research_common.proto",
}
FORBIDDEN_TYPE_TOKENS = (
    "approval",
    "backtestspec",
    "grant",
    "holdout",
    "joblease",
    "joboutcome",
    "jobrecord",
    "jobspecification",
    "samplerole",
    "samplewindow",
)


def test_research_dependency_closure_excludes_internal_and_locked_contracts() -> None:
    pending = [service_pb2.DESCRIPTOR]
    visited: set[str] = set()
    while pending:
        descriptor = pending.pop()
        if descriptor.name in visited:
            continue
        visited.add(descriptor.name)
        pending.extend(descriptor.dependencies)

    assert visited == EXPECTED_CLOSURE
    assert "loop/v1/data.proto" not in visited
    assert not hasattr(service_pb2, "loop_dot_v1_dot_data__pb2")
    assert service_pb2.loop_dot_v1_dot_development__data__pb2 is development_data_pb2


def test_research_request_and_response_graph_is_development_only() -> None:
    service = service_pb2.DESCRIPTOR.services_by_name["ResearchService"]
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

    assert "loop.research.v1.ResearchJobHandle" in visited
    assert "loop.v1.DevelopmentDatasetReference" in visited
    assert "loop.v1.FactorSpec" in visited
    assert "loop.v1.ResearchProvenanceFingerprint" in visited


def test_research_shared_surface_vectors_fail_closed() -> None:
    surfaces = {
        "backtest_input": service_pb2.BacktestInput.DESCRIPTOR,
        "factor_input": service_pb2.FactorEvaluationInput.DESCRIPTOR,
        "job_handle": service_pb2.ResearchJobHandle.DESCRIPTOR,
        "reconciliation_input": service_pb2.ReconciliationInput.DESCRIPTOR,
    }
    for line in BOUNDARY_VECTORS.read_text(encoding="ascii").splitlines():
        if not line or line.startswith("#"):
            continue
        name, surface, member, expected = line.split("\t")
        assert (member in surfaces[surface].fields_by_name) is (expected == "accept"), name


def test_research_response_round_trip_has_only_the_safe_projection() -> None:
    response = service_pb2.EnqueueBacktestResponse(
        job=service_pb2.ResearchJobHandle(
            job_id=common_pb2.JobId(value="job.research.0001"),
            status=service_pb2.RESEARCH_JOB_STATUS_RUNNING,
            revision=7,
        )
    )
    decoded = service_pb2.EnqueueBacktestResponse.FromString(response.SerializeToString())

    assert decoded.job.job_id.value == "job.research.0001"
    assert decoded.job.status == service_pb2.RESEARCH_JOB_STATUS_RUNNING
    assert {field.name for field in decoded.job.DESCRIPTOR.fields} == {
        "job_id",
        "status",
        "revision",
        "submitted_at",
        "updated_at",
    }


def _assert_safe_name(value: str) -> None:
    lowered = value.lower()
    assert not any(token in lowered for token in FORBIDDEN_TYPE_TOKENS), value
