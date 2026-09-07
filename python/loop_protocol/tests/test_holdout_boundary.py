from pathlib import Path

import pytest

from loop.holdout.v1 import service_pb2
from loop.v1 import artifact_pb2, common_pb2, holdout_pb2, job_pb2


def _digest(byte: int) -> common_pb2.Sha256Digest:
    return common_pb2.Sha256Digest(value=bytes([byte]) * 32)


PLAN_ID = f"sha256:{'0a' * 32}"
PERIOD_ID = f"sha256:{'09' * 32}"
BOUNDARY_VECTORS = (
    Path(__file__).resolve().parents[3] / "tests" / "contracts" / "holdout_boundary_vectors.tsv"
)


def _grant_reference() -> holdout_pb2.HoldoutGrantReference:
    return holdout_pb2.HoldoutGrantReference(
        holdout_grant_id=common_pb2.HoldoutGrantId(value="grant.holdout.0001"),
        holdout_period_id=common_pb2.HoldoutPeriodId(value=PERIOD_ID),
        freeze_manifest_sha256=_digest(6),
        holdout_evaluation_plan_id=common_pb2.HoldoutEvaluationPlanId(value=PLAN_ID),
        evaluation_plan_sha256=_digest(7),
        evaluation_plan_entry_count=2,
        canonical_period_sha256=_digest(9),
    )


def test_freeze_manifest_round_trip_pins_complete_evaluation_plan() -> None:
    plan = holdout_pb2.HoldoutEvaluationPlanReference(
        holdout_evaluation_plan_id=common_pb2.HoldoutEvaluationPlanId(value=PLAN_ID),
        canonical_plan=artifact_pb2.ArtifactRef(
            artifact_id=common_pb2.ArtifactId(value=f"sha256:{'07' * 32}"),
            uri=f"artifact://sha256/{'07' * 32}",
            sha256=_digest(7),
            schema=artifact_pb2.ArtifactSchemaReference(
                name="loop.holdout_evaluation_plan",
                version=1,
                schema_sha256=_digest(8),
            ),
            media_type="application/json",
            byte_size=512,
        ),
        plan_sha256=_digest(7),
        entry_count=2,
        holdout_period_id=common_pb2.HoldoutPeriodId(value=PERIOD_ID),
        canonical_period_sha256=_digest(9),
    )
    plan.canonical_plan.created_at.seconds = 1
    freeze = holdout_pb2.FreezeManifestReference(holdout_evaluation_plan=plan)
    decoded = holdout_pb2.FreezeManifestReference.FromString(freeze.SerializeToString())

    assert decoded.holdout_evaluation_plan.entry_count == 2
    assert decoded.holdout_evaluation_plan.canonical_plan.sha256 == _digest(7)


def test_consume_surface_has_no_caller_supplied_specification_or_budget() -> None:
    request_fields = {
        field.name for field in service_pb2.ConsumeGrantAndEnqueueBacktestRequest.DESCRIPTOR.fields
    }
    assert request_fields == {
        "context",
        "grant_reference",
        "expected_grant_revision",
        "expected_period_revision",
    }
    with pytest.raises(ValueError, match='no "frozen_backtest_spec" field'):
        service_pb2.ConsumeGrantAndEnqueueBacktestRequest(frozen_backtest_spec=None)
    with pytest.raises(ValueError, match='no "budget" field'):
        service_pb2.ConsumeGrantAndEnqueueBacktestRequest(budget=None)

    request = service_pb2.ConsumeGrantAndEnqueueBacktestRequest(
        grant_reference=_grant_reference(),
        expected_grant_revision=3,
        expected_period_revision=4,
    )
    decoded = service_pb2.ConsumeGrantAndEnqueueBacktestRequest.FromString(
        request.SerializeToString()
    )
    assert decoded.expected_grant_revision == 3
    assert decoded.expected_period_revision == 4


def test_consume_response_is_a_narrow_batch_and_internal_job_retains_binding() -> None:
    batch = service_pb2.JobBatchHandle(
        job_batch_id=common_pb2.JobBatchId(value="batch.holdout.0001"),
        holdout_grant_id=common_pb2.HoldoutGrantId(value="grant.holdout.0001"),
        holdout_evaluation_plan_id=common_pb2.HoldoutEvaluationPlanId(value=PLAN_ID),
        evaluation_plan_sha256=_digest(7),
        evaluation_plan_entry_count=2,
        job_count=2,
        job_ids=[
            common_pb2.JobId(value="job.holdout.0001"),
            common_pb2.JobId(value="job.holdout.0002"),
        ],
        revision=1,
    )
    response = service_pb2.ConsumeGrantAndEnqueueBacktestResponse(
        consumed_grant=_grant_reference(), job_batch=batch
    )
    decoded_response = service_pb2.ConsumeGrantAndEnqueueBacktestResponse.FromString(
        response.SerializeToString()
    )
    assert len(decoded_response.job_batch.job_ids) == 2
    assert {field.name for field in decoded_response.job_batch.DESCRIPTOR.fields}.isdisjoint(
        {"specification", "factor_spec", "backtest_spec", "budget"}
    )

    internal = job_pb2.HoldoutBacktestJobInput(
        consumed_grant=_grant_reference(),
        consumed_grant_revision=4,
        job_batch_id=common_pb2.JobBatchId(value="batch.holdout.0001"),
        holdout_evaluation_plan_id=common_pb2.HoldoutEvaluationPlanId(value=PLAN_ID),
        evaluation_plan_sha256=_digest(7),
        evaluation_plan_entry_index=2,
    )
    decoded_internal = job_pb2.HoldoutBacktestJobInput.FromString(internal.SerializeToString())
    assert decoded_internal.evaluation_plan_entry_index == 2
    assert decoded_internal.holdout_evaluation_plan_id.value == PLAN_ID


def test_holdout_service_generated_module_has_no_job_or_backtest_dependency() -> None:
    assert {dependency.name for dependency in service_pb2.DESCRIPTOR.dependencies} == {
        "google/protobuf/timestamp.proto",
        "loop/v1/artifact.proto",
        "loop/v1/common.proto",
        "loop/v1/holdout.proto",
    }


def test_shared_holdout_surface_vectors_fail_closed() -> None:
    surfaces = {
        "consume_request": service_pb2.ConsumeGrantAndEnqueueBacktestRequest.DESCRIPTOR,
        "consume_response": service_pb2.ConsumeGrantAndEnqueueBacktestResponse.DESCRIPTOR,
        "freeze_manifest": holdout_pb2.FreezeManifestReference.DESCRIPTOR,
        "get_period_request": service_pb2.GetHoldoutPeriodRequest.DESCRIPTOR,
        "holdout_service": service_pb2.DESCRIPTOR.services_by_name["HoldoutService"],
        "internal_holdout_job": job_pb2.HoldoutBacktestJobInput.DESCRIPTOR,
    }
    for line in BOUNDARY_VECTORS.read_text(encoding="ascii").splitlines():
        if not line or line.startswith("#"):
            continue
        name, surface, member, expected = line.split("\t")
        descriptor = surfaces[surface]
        if surface == "holdout_service":
            present = member in descriptor.methods_by_name
        else:
            present = member in descriptor.fields_by_name
        assert present is (expected == "accept"), name
