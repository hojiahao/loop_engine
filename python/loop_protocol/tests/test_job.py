from __future__ import annotations

import csv
import json
from pathlib import Path
from typing import Any, cast

import pytest

from loop.v1 import (
    artifact_pb2,
    common_pb2,
    data_pb2,
    development_data_pb2,
    factor_pb2,
    job_pb2,
    model_pb2,
    research_common_pb2,
)
from loop_protocol.job import (
    JobValidationCode,
    JobValidationError,
    canonical_protocol_selection_bytes,
    factor_spec_identity_sha256,
    protocol_selection_sha256,
    validate_holdout_backtest_plan_entry_binding,
    validate_job_record,
    validate_job_specification,
)
from loop_protocol.runtime_validation import (
    RuntimeValidationCode,
    RuntimeValidationError,
    validate_job_wire_dispatch_candidate,
)

VECTORS = Path(__file__).parents[3] / "tests" / "contracts" / "job_record_vectors.tsv"
PROTOCOL_GOLDEN = (
    Path(__file__).parents[3] / "tests" / "contracts" / "protocol_selection_golden.tsv"
)
PROTOCOL_NEGATIVE = (
    Path(__file__).parents[3] / "tests" / "contracts" / "protocol_selection_negative.tsv"
)
HOLDOUT_BINDING = (
    Path(__file__).parents[3] / "tests" / "contracts" / "holdout_job_binding_vectors.tsv"
)
HOLDOUT_GRANT_LIFETIME = (
    Path(__file__).parents[3] / "tests" / "contracts" / "holdout_grant_lifetime_vectors.tsv"
)
HOLDOUT_GOLDEN = cast(
    dict[str, Any],
    json.loads(
        (
            Path(__file__).parents[3] / "tests" / "contracts" / "holdout_identity_golden.json"
        ).read_text("ascii")
    ),
)
FACTOR_ID = "sha256:3a4f28e6e918ec379af280264bf314eaac843a0aded6fefcdcdd06cf925b895a"
EXPRESSION_ID = "sha256:2b93fad0265af4e02df2b2dce69d3b5a221bfd553aacaa52bb652666374d7cb3"
OTHER_FACTOR_ID = f"sha256:{'b' * 64}"
SHARED_VECTORS = list(csv.DictReader(VECTORS.open(), delimiter="\t"))


def test_protocol_selection_producer_matches_shared_golden() -> None:
    row = next(csv.DictReader(PROTOCOL_GOLDEN.open(), delimiter="\t"))
    selection = _protocol_selection()
    assert canonical_protocol_selection_bytes(selection).decode("ascii") == row["canonical_utf8"]
    assert protocol_selection_sha256(selection).hex() == row["selection_sha256"]


@pytest.mark.parametrize(
    "vector",
    list(csv.DictReader(PROTOCOL_NEGATIVE.open(), delimiter="\t")),
    ids=lambda vector: vector["name"],
)
def test_protocol_selection_shared_negatives_fail_closed(vector: dict[str, str]) -> None:
    specification = job_pb2.JobSpecification()
    _set_valid_specification(
        specification,
        {"kind": "discovery", "input": "discovery"},
    )
    selection = specification.protocol_selection
    mutation = vector["mutation"]
    if mutation == "digest_mismatch":
        value = bytearray(selection.selection_sha256.value)
        value[0] ^= 0xFF
        selection.selection_sha256.value = bytes(value)
    elif mutation == "unsorted_features":
        selection.enabled_features.reverse()
    elif mutation == "duplicate_features":
        selection.enabled_features[1] = selection.enabled_features[0]
    elif mutation == "zero_limit":
        selection.effective_limits.maximum_ast_nodes = 0
    elif mutation == "future_timestamp":
        selection.selected_at.seconds = 6
        selection.selection_sha256.value = protocol_selection_sha256(selection)
    elif mutation == "invalid_package":
        selection.selected_package = "Loop.v1"
    elif mutation == "zero_package_version":
        selection.selected_package = "loop.v0"
    elif mutation == "leading_zero_package_version":
        selection.selected_package = "loop.v01"
    else:
        raise AssertionError(f"unknown selection mutation {mutation}")
    with pytest.raises(JobValidationError) as captured:
        validate_job_specification(specification)
    assert captured.value.code == JobValidationCode(vector["expected"])


@pytest.mark.parametrize(
    "vector",
    list(csv.DictReader(HOLDOUT_BINDING.open(), delimiter="\t")),
    ids=lambda vector: vector["name"],
)
def test_holdout_job_shared_plan_entry_bindings_fail_closed(vector: dict[str, str]) -> None:
    input_value, period_bytes, plan_bytes, trusted_backtest, resolved = _holdout_binding_fixture()
    mutation = vector["mutation"]
    if mutation == "plan_identity":
        identity = _digest_id(99)
        input_value.holdout_evaluation_plan_id.value = identity
        input_value.consumed_grant.holdout_evaluation_plan_id.value = identity
    elif mutation == "entry_factor":
        input_value.frozen_backtest_spec.factor_spec_id.value = OTHER_FACTOR_ID
    elif mutation == "entry_budget":
        input_value.budget.maximum_steps = 41
    elif mutation == "entry_artifact":
        input_value.frozen_backtest_spec.canonical_spec_sha256.CopyFrom(_digest(99))
    elif mutation == "canonical_plan_tampered":
        plan_bytes += b" "
    elif mutation == "canonical_period_tampered":
        period_bytes += b" "
    elif mutation != "none":
        raise AssertionError(f"unknown holdout mutation {mutation}")
    submitted_at = job_pb2.JobSpecification().submitted_at
    submitted_at.seconds = 5
    if vector["expected"] == "accept":
        validate_holdout_backtest_plan_entry_binding(
            input_value,
            submitted_at,
            period_bytes,
            plan_bytes,
            trusted_backtest,
            resolved,
        )
    else:
        with pytest.raises(JobValidationError) as captured:
            validate_holdout_backtest_plan_entry_binding(
                input_value,
                submitted_at,
                period_bytes,
                plan_bytes,
                trusted_backtest,
                resolved,
            )
        assert captured.value.code == JobValidationCode(vector["expected"])


@pytest.mark.parametrize(
    "vector",
    list(csv.DictReader(HOLDOUT_GRANT_LIFETIME.open(), delimiter="\t")),
    ids=lambda vector: vector["name"],
)
def test_holdout_grant_lifetime_boundaries_cover_binder_and_wire_candidate(
    vector: dict[str, str],
) -> None:
    submitted_at = job_pb2.JobSpecification().submitted_at
    submitted_at.seconds = int(vector["submitted_seconds"])
    submitted_at.nanos = int(vector["submitted_nanos"])
    input_value, period_bytes, plan_bytes, trusted_backtest, resolved = _holdout_binding_fixture()

    if vector["expected_binder"] == "accept":
        validate_holdout_backtest_plan_entry_binding(
            input_value,
            submitted_at,
            period_bytes,
            plan_bytes,
            trusted_backtest,
            resolved,
        )
    else:
        with pytest.raises(JobValidationError) as captured:
            validate_holdout_backtest_plan_entry_binding(
                input_value,
                submitted_at,
                period_bytes,
                plan_bytes,
                trusted_backtest,
                resolved,
            )
        assert captured.value.code == JobValidationCode(vector["expected_binder"])

    specification = job_pb2.JobSpecification()
    _set_valid_specification(
        specification,
        {"kind": "holdout_backtest", "input": "holdout_backtest"},
    )
    specification.submitted_at.CopyFrom(submitted_at)
    if vector["expected_wire"] == "accept":
        assert (
            validate_job_wire_dispatch_candidate(specification, {job_pb2.JOB_KIND_HOLDOUT_BACKTEST})
            == job_pb2.JOB_KIND_HOLDOUT_BACKTEST
        )
    else:
        with pytest.raises(RuntimeValidationError) as captured:
            validate_job_wire_dispatch_candidate(specification, {job_pb2.JOB_KIND_HOLDOUT_BACKTEST})
        assert captured.value.code == RuntimeValidationCode(vector["expected_wire"])


@pytest.mark.parametrize("vector", SHARED_VECTORS, ids=[v["name"] for v in SHARED_VECTORS])
def test_shared_job_record_matrix_fails_closed(vector: dict[str, str]) -> None:
    assert len(SHARED_VECTORS) == 109
    record = _record(vector)
    expected = vector["expected"]
    if expected == "accept":
        validated = validate_job_record(record)
        assert validated.kind == record.specification.kind
        assert validated.state == record.state
    else:
        with pytest.raises(JobValidationError) as captured:
            validate_job_record(record)
        assert captured.value.code == JobValidationCode(expected)


def _record(vector: dict[str, str]) -> job_pb2.JobRecord:
    record = job_pb2.JobRecord(
        state=cast(Any, _state(vector["state"])), revision=1, attempt=int(vector["attempt"])
    )
    if vector["kind"] != "missing":
        _set_valid_specification(record.specification, vector)
    if vector["lease"] == "present":
        _set_valid_lease(record.active_lease)
    _set_outcome(record, vector["outcome"])
    record.updated_at.seconds = 20
    _mutate(record, vector["mutation"])
    return record


def _set_valid_specification(
    specification: job_pb2.JobSpecification, vector: dict[str, str]
) -> None:
    specification.job_id.value = "job.01"
    specification.run_id.value = "run.01"
    specification.kind = cast(Any, _kind(vector["kind"]))
    _set_input(specification, vector["input"])
    specification.submitted_at.seconds = 5
    _set_actor(specification.submitted_by)
    specification.idempotency_key.value = "idem.01"
    specification.correlation_id.value = "corr.01"
    specification.causation_id.value = "cause.01"
    specification.protocol_selection.CopyFrom(_protocol_selection())
    specification.protocol_selection.selection_sha256.value = protocol_selection_sha256(
        specification.protocol_selection
    )


def _set_input(specification: job_pb2.JobSpecification, value: str) -> None:
    if value == "missing":
        return
    target = getattr(specification, value)
    target.SetInParent()
    if value == "discovery":
        target.dataset.CopyFrom(_development_dataset())
        target.research_policy.CopyFrom(_policy("policy.research"))
        target.maker_model.CopyFrom(_model("resolution.maker"))
        target.checker_model.CopyFrom(_model("resolution.checker"))
        target.budget.CopyFrom(_valid_budget())
        target.maximum_candidates = 40
    elif value == "factor_evaluation":
        target.factor.CopyFrom(_valid_factor_spec())
        target.dataset.CopyFrom(_development_dataset())
        target.budget.CopyFrom(_valid_budget())
    elif value == "backtest":
        target.budget.CopyFrom(_valid_budget())
        target.factor_spec_id.value = FACTOR_ID
        target.dataset.CopyFrom(_development_dataset())
        target.return_definition = research_common_pb2.RETURN_DEFINITION_SIMPLE_NAV_RETURN
        target.provenance.CopyFrom(_provenance())
        target.deterministic_seed.CopyFrom(_digest(9))
    elif value == "reconciliation":
        target.primary_backtest_id.value = "backtest.primary"
        target.independent_backtest_id.value = "backtest.independent"
        target.reconciliation_policy.CopyFrom(_policy("policy.reconciliation"))
        target.budget.CopyFrom(_valid_budget())
    elif value == "holdout_backtest":
        target.CopyFrom(_holdout_input())
    elif value == "artifact":
        target.input.CopyFrom(_artifact())
        target.policy.CopyFrom(_policy("policy.artifact"))
        target.budget.CopyFrom(_valid_budget())


def _set_outcome(record: job_pb2.JobRecord, value: str) -> None:
    if value == "absent":
        return
    if value == "empty":
        record.outcome.SetInParent()
        return
    target = getattr(record.outcome, value)
    target.SetInParent()
    if value == "factor_rejection":
        target.factor_spec_id.value = FACTOR_ID
        target.code = job_pb2.FACTOR_REJECTION_CODE_PERFORMANCE
        target.reason = "fails frozen performance threshold"
        target.rejected_at.seconds = 20
    elif value == "infrastructure_failure":
        target.error.category = common_pb2.ERROR_CATEGORY_DEPENDENCY
        target.error.code = "dataset_unavailable"
        target.error.message = "dataset unavailable"
        target.attempt = record.attempt
        target.failed_at.seconds = 20
    elif value == "cancellation":
        target.reason = "cancelled by operator"
        _set_actor(target.cancelled_by)
        target.cancelled_at.seconds = 20
    elif value == "budget_exhaustion":
        target.exhausted_limit = "maximum_steps"
        target.enforced_budget.CopyFrom(_job_budget(record.specification))
        target.exhausted_at.seconds = 20


def _set_valid_lease(lease: job_pb2.JobLease) -> None:
    lease.lease_id.value = "lease.01"
    lease.job_id.value = "job.01"
    _set_actor(lease.owner)
    lease.acquired_revision = 1
    lease.issued_at.seconds = 10
    lease.heartbeat_at.seconds = 11
    lease.expires_at.seconds = 30


def _set_actor(actor: common_pb2.Actor) -> None:
    actor.actor_id.value = "worker.01"
    actor.kind = common_pb2.ACTOR_KIND_SERVICE
    actor.display_name = "Loop worker"
    actor.authenticated_subject = "service:loop-worker"


def _valid_budget() -> job_pb2.JobBudget:
    value = job_pb2.JobBudget(
        maximum_steps=40,
        maximum_input_tokens=100_000,
        maximum_output_tokens=20_000,
    )
    value.maximum_cost.CopyFrom(_money("12.5"))
    value.maximum_wall_time.seconds = 3_600
    return value


def _money(amount: str) -> common_pb2.Money:
    value = common_pb2.Money(currency_code="USD")
    value.amount.value = amount
    return value


def _digest(byte: int) -> common_pb2.Sha256Digest:
    return common_pb2.Sha256Digest(value=bytes([byte]) * 32)


def _digest_id(byte: int) -> str:
    return f"sha256:{byte:02x}" + f"{byte:02x}" * 31


def _policy(value: str) -> common_pb2.PolicyReference:
    policy = common_pb2.PolicyReference(revision="1")
    policy.policy_id.value = value
    policy.sha256.CopyFrom(_digest(6))
    return policy


def _valid_factor_spec() -> factor_pb2.FactorSpec:
    factor = factor_pb2.FactorSpec(direction=factor_pb2.FACTOR_DIRECTION_HIGHER_IS_BETTER)
    factor.expression_id.value = EXPRESSION_ID
    factor.expression.expression_id.value = EXPRESSION_ID
    factor.expression.ast.schema_version = 1
    factor.expression.ast.root.field.field = "market.close"
    factor.expression.canonicalization_profile = "loop.factor-ast/v1"
    factor.expression.canonical_json = b'{"node":"field","field":"market.close"}'
    factor.operator_registry_sha256.CopyFrom(_digest(2))
    for field_name in (
        "universe_policy",
        "data_policy",
        "calendar_policy",
        "preprocess_policy",
        "neutralization_policy",
        "portfolio_policy",
        "execution_policy",
        "cost_policy",
        "evaluation_policy",
    ):
        getattr(factor.frozen_policy, field_name).CopyFrom(
            _policy(f"policy.{field_name.removesuffix('_policy')}")
        )
    assert f"sha256:{factor_spec_identity_sha256(factor).hex()}" == FACTOR_ID
    factor.factor_spec_id.value = FACTOR_ID
    return factor


def _development_dataset() -> development_data_pb2.DevelopmentDatasetReference:
    value = development_data_pb2.DevelopmentDatasetReference()
    value.snapshot_ids.add().value = "snapshot.dev.01"
    value.manifest_sha256.CopyFrom(_digest(7))
    return value


def _provenance() -> research_common_pb2.ResearchProvenanceFingerprint:
    value = research_common_pb2.ResearchProvenanceFingerprint()
    for index, field_name in enumerate(
        (
            "source_code_sha256",
            "operator_registry_sha256",
            "configuration_sha256",
            "data_manifest_sha256",
            "trading_calendar_sha256",
            "environment_sha256",
        ),
        start=1,
    ):
        getattr(value, field_name).CopyFrom(_digest(index))
    return value


def _model(resolution: str) -> model_pb2.ModelResolutionSnapshot:
    value = model_pb2.ModelResolutionSnapshot(
        requested_alias="fixture-model",
        protocol_family=model_pb2.MODEL_PROTOCOL_FAMILY_OPENAI_RESPONSES,
        provider_plugin_name="fixture-provider",
        provider_plugin_version="1.0.0",
    )
    value.resolution_id.value = resolution
    value.provider_id.value = "provider.fixture"
    value.model_id.value = "model.fixture"
    value.capabilities.context_window_tokens = 100_000
    value.capabilities.maximum_output_tokens = 20_000
    value.pricing.input_per_million_tokens.CopyFrom(_money("1"))
    value.pricing.output_per_million_tokens.CopyFrom(_money("2"))
    value.pricing.cached_input_per_million_tokens.CopyFrom(_money("0.5"))
    value.capability_sha256.CopyFrom(_digest(11))
    value.catalog_sha256.CopyFrom(_digest(12))
    value.resolved_at.seconds = 1
    value.provider_plugin_sha256.CopyFrom(_digest(13))
    value.snapshot_sha256.CopyFrom(_digest(14))
    return value


def _protocol_selection() -> common_pb2.ProtocolSelectionSnapshot:
    value = common_pb2.ProtocolSelectionSnapshot(
        selected_package="loop.v1",
        enabled_features=["jobs.envelope.v1", "jobs.kind-input.v1"],
        server_build_version="0.2.0-alpha.1",
        client_build_version="0.2.0-alpha.1",
    )
    value.effective_limits.CopyFrom(
        common_pb2.ProtocolLimits(
            maximum_unary_bytes=4_194_304,
            maximum_stream_event_bytes=1_048_576,
            maximum_canonical_ast_bytes=262_144,
            maximum_ast_nodes=4_096,
            maximum_ast_depth=64,
            maximum_page_records=500,
            maximum_identity_bytes=128,
            maximum_artifact_uri_bytes=2_048,
        )
    )
    value.server_build_sha256.CopyFrom(_digest(21))
    value.schema_descriptor_sha256.CopyFrom(_digest(22))
    value.selected_at.seconds = 2
    value.client_build_sha256.CopyFrom(_digest(23))
    return value


def _artifact() -> artifact_pb2.ArtifactRef:
    identity = _digest_id(24)
    value = artifact_pb2.ArtifactRef(
        uri=f"artifact://sha256/{identity[7:]}", media_type="application/json", byte_size=1
    )
    value.artifact_id.value = identity
    value.sha256.CopyFrom(_digest(24))
    value.schema.name = "loop.report"
    value.schema.version = 1
    value.schema.schema_sha256.CopyFrom(_digest(25))
    value.created_at.seconds = 1
    return value


def _holdout_input() -> job_pb2.HoldoutBacktestJobInput:
    value = job_pb2.HoldoutBacktestJobInput(
        consumed_grant_revision=1,
        evaluation_plan_entry_index=1,
    )
    plan_id = _digest_id(32)
    grant = value.consumed_grant
    grant.holdout_grant_id.value = "grant.01"
    grant.holdout_period_id.value = _digest_id(31)
    grant.freeze_manifest_sha256.CopyFrom(_digest(33))
    grant.issued_at.seconds = 4
    grant.expires_at.seconds = 100
    grant.holdout_evaluation_plan_id.value = plan_id
    grant.evaluation_plan_sha256.CopyFrom(_digest(34))
    grant.evaluation_plan_entry_count = 2
    grant.canonical_period_sha256.CopyFrom(_digest(31))
    frozen = value.frozen_backtest_spec
    frozen.backtest_id.value = "backtest.holdout.01"
    frozen.schema_version = 1
    frozen.factor_spec_id.value = FACTOR_ID
    frozen.snapshot_ids.add().value = _digest_id(35)
    frozen.sample.role = data_pb2.SAMPLE_ROLE_SECOND_LOCKED_HISTORICAL_HOLDOUT
    frozen.sample.start_inclusive.CopyFrom(common_pb2.CivilDate(year=2025, month=1, day=1))
    frozen.sample.end_inclusive.CopyFrom(common_pb2.CivilDate(year=2026, month=8, day=31))
    frozen.return_definition = research_common_pb2.RETURN_DEFINITION_SIMPLE_NAV_RETURN
    frozen.provenance.CopyFrom(_provenance())
    frozen.canonical_spec_sha256.CopyFrom(_digest(36))
    frozen.created_at.seconds = 3
    frozen.deterministic_seed.CopyFrom(_digest(37))
    value.budget.CopyFrom(_valid_budget())
    value.job_batch_id.value = "batch.01"
    value.holdout_evaluation_plan_id.value = plan_id
    value.evaluation_plan_sha256.CopyFrom(_digest(34))
    return value


def _holdout_binding_fixture() -> tuple[
    job_pb2.HoldoutBacktestJobInput,
    bytes,
    bytes,
    bytes,
    dict[str, bytes],
]:
    period = HOLDOUT_GOLDEN["periods"][0]
    plan = HOLDOUT_GOLDEN["plans"][0]
    plan_value = cast(dict[str, Any], json.loads(plan["canonical_json"]))
    first_entry = plan_value["entries"][0]
    input_value = _holdout_input()
    grant = input_value.consumed_grant
    grant.holdout_period_id.value = period["holdout_period_id"]
    grant.canonical_period_sha256.value = bytes.fromhex(period["canonical_sha256"][7:])
    grant.holdout_evaluation_plan_id.value = plan["holdout_evaluation_plan_id"]
    grant.evaluation_plan_sha256.value = bytes.fromhex(plan["plan_sha256"][7:])
    grant.evaluation_plan_entry_count = 2
    input_value.holdout_evaluation_plan_id.value = plan["holdout_evaluation_plan_id"]
    input_value.evaluation_plan_sha256.value = bytes.fromhex(plan["plan_sha256"][7:])
    frozen = input_value.frozen_backtest_spec
    frozen.factor_spec_id.value = first_entry["factor_spec_id"]
    frozen.canonical_spec_sha256.value = bytes.fromhex(
        first_entry["backtest_spec_artifact"]["sha256"][7:]
    )
    frozen.sample.role = data_pb2.SAMPLE_ROLE_FIRST_LOCKED_CONFIRMATION
    frozen.sample.start_inclusive.CopyFrom(common_pb2.CivilDate(year=2021, month=1, day=1))
    frozen.sample.end_inclusive.CopyFrom(common_pb2.CivilDate(year=2024, month=12, day=31))
    resolved = {
        artifact["sha256"]: artifact["content"].encode("ascii")
        for artifact in HOLDOUT_GOLDEN["backtest_artifacts"]
    }
    return (
        input_value,
        period["canonical_json"].encode("ascii"),
        plan["canonical_json"].encode("ascii"),
        bytes.fromhex(HOLDOUT_GOLDEN["trusted_backtest_schema_sha256"][7:]),
        resolved,
    )


def _job_budget(specification: job_pb2.JobSpecification) -> job_pb2.JobBudget:
    return cast(job_pb2.JobBudget, getattr(specification, specification.WhichOneof("input")).budget)


def _mutate(record: job_pb2.JobRecord, mutation: str) -> None:
    if mutation == "none":
        return
    if mutation == "zero_revision":
        record.revision = 0
    elif mutation == "missing_spec_job_id":
        record.specification.ClearField("job_id")
    elif mutation == "malformed_spec_job_id":
        record.specification.job_id.value = "bad id"
    elif mutation == "lease_job_mismatch":
        record.active_lease.job_id.value = "job.02"
    elif mutation == "lease_future_revision":
        record.active_lease.acquired_revision = 2
    elif mutation == "lease_zero_revision":
        record.active_lease.acquired_revision = 0
    elif mutation == "lease_missing_id":
        record.active_lease.ClearField("lease_id")
    elif mutation == "lease_missing_job_id":
        record.active_lease.ClearField("job_id")
    elif mutation == "lease_missing_owner":
        record.active_lease.ClearField("owner")
    elif mutation == "lease_unknown_owner_kind":
        record.active_lease.owner.kind = cast(Any, 999)
    elif mutation == "lease_missing_timestamp":
        record.active_lease.ClearField("heartbeat_at")
    elif mutation == "lease_invalid_order":
        record.active_lease.heartbeat_at.seconds = 30
    elif mutation == "factor_input_id_missing":
        _set_input_factor_id(record, None)
    elif mutation == "factor_input_id_malformed":
        _set_input_factor_id(record, "SHA256:bad")
    elif mutation == "rejection_id_missing":
        record.outcome.factor_rejection.ClearField("factor_spec_id")
    elif mutation == "rejection_id_malformed":
        record.outcome.factor_rejection.factor_spec_id.value = "SHA256:bad"
    elif mutation == "rejection_id_mismatch":
        record.outcome.factor_rejection.factor_spec_id.value = OTHER_FACTOR_ID
    elif mutation == "rejection_unspecified_code":
        record.outcome.factor_rejection.code = job_pb2.FACTOR_REJECTION_CODE_UNSPECIFIED
    elif mutation == "rejection_unknown_code":
        record.outcome.factor_rejection.code = cast(Any, 999)
    elif mutation == "rejection_missing_reason":
        record.outcome.factor_rejection.reason = " "
    elif mutation == "rejection_control_reason":
        record.outcome.factor_rejection.reason = "forged\nrecord"
    elif mutation == "rejection_missing_timestamp":
        record.outcome.factor_rejection.ClearField("rejected_at")
    elif mutation == "rejection_invalid_timestamp":
        record.outcome.factor_rejection.rejected_at.nanos = 1_000_000_000
    elif mutation == "rejection_too_many_evidence":
        for _ in range(65):
            record.outcome.factor_rejection.evidence.add()
    elif mutation == "success_too_many_outputs":
        for _ in range(65):
            record.outcome.success.outputs.add()
    elif mutation == "infra_missing_error":
        record.outcome.infrastructure_failure.ClearField("error")
    elif mutation == "infra_unspecified_category":
        record.outcome.infrastructure_failure.error.category = common_pb2.ERROR_CATEGORY_UNSPECIFIED
    elif mutation == "infra_unknown_category":
        record.outcome.infrastructure_failure.error.category = cast(Any, 999)
    elif mutation == "infra_missing_code":
        record.outcome.infrastructure_failure.error.code = ""
    elif mutation == "infra_missing_message":
        record.outcome.infrastructure_failure.error.message = ""
    elif mutation == "infra_attempt_mismatch":
        record.outcome.infrastructure_failure.attempt = 1
    elif mutation == "infra_missing_timestamp":
        record.outcome.infrastructure_failure.ClearField("failed_at")
    elif mutation == "infra_too_many_details":
        for _ in range(33):
            record.outcome.infrastructure_failure.error.details.add()
    elif mutation == "infra_invalid_detail":
        record.outcome.infrastructure_failure.error.details.add()
    elif mutation == "cancellation_missing_reason":
        record.outcome.cancellation.reason = ""
    elif mutation == "cancellation_missing_actor":
        record.outcome.cancellation.ClearField("cancelled_by")
    elif mutation == "cancellation_missing_timestamp":
        record.outcome.cancellation.ClearField("cancelled_at")
    elif mutation == "budget_missing_limit":
        record.outcome.budget_exhaustion.exhausted_limit = ""
    elif mutation == "budget_missing_object":
        record.outcome.budget_exhaustion.ClearField("enforced_budget")
    elif mutation == "budget_missing_timestamp":
        record.outcome.budget_exhaustion.ClearField("exhausted_at")
    elif mutation == "lease_predates_submission":
        record.active_lease.issued_at.seconds = 4
    elif mutation == "lease_expired_at_update":
        record.active_lease.expires_at.seconds = 20
    elif mutation == "submitted_actor_empty_subject":
        record.specification.submitted_by.authenticated_subject = ""
    elif mutation == "submitted_actor_control_subject":
        record.specification.submitted_by.authenticated_subject = "subject\nforged"
    elif mutation == "policy_revision_zero":
        record.specification.discovery.research_policy.revision = "0"
    elif mutation == "policy_revision_leading_zero":
        record.specification.discovery.research_policy.revision = "01"
    elif mutation == "policy_revision_sign":
        record.specification.discovery.research_policy.revision = "+1"
    elif mutation == "factor_expression_hash_mismatch":
        record.specification.factor_evaluation.factor.expression.canonical_json += b" "
    elif mutation == "factor_ast_canonical_mismatch":
        record.specification.factor_evaluation.factor.expression.ast.root.field.field = (
            "market.open"
        )
    elif mutation == "factor_spec_hash_mismatch":
        record.specification.factor_evaluation.factor.factor_spec_id.value = OTHER_FACTOR_ID
    elif mutation == "factor_unspecified_direction":
        record.specification.factor_evaluation.factor.direction = (
            factor_pb2.FACTOR_DIRECTION_UNSPECIFIED
        )
    elif mutation == "protocol_selection_digest_mismatch":
        value = bytearray(record.specification.protocol_selection.selection_sha256.value)
        value[0] ^= 0xFF
        record.specification.protocol_selection.selection_sha256.value = bytes(value)
    elif mutation == "holdout_plan_digest_mismatch":
        value = bytearray(record.specification.holdout_backtest.evaluation_plan_sha256.value)
        value[0] ^= 0xFF
        record.specification.holdout_backtest.evaluation_plan_sha256.value = bytes(value)
    elif mutation == "holdout_unknown_sample_role":
        record.specification.holdout_backtest.frozen_backtest_spec.sample.role = cast(Any, 999)
    elif mutation == "holdout_unknown_return_definition":
        record.specification.holdout_backtest.frozen_backtest_spec.return_definition = cast(
            Any, 999
        )
    elif mutation == "model_invalid_plugin_version":
        record.specification.discovery.maker_model.provider_plugin_version = "1 bad"
    elif mutation == "dataset_snapshot_id_malformed":
        record.specification.discovery.dataset.snapshot_ids[0].value = "bad id"
    elif mutation == "factor_ast_empty_root":
        record.specification.factor_evaluation.factor.expression.ast.root.ClearField("field")
    elif mutation == "holdout_submission_before_grant_issue":
        record.specification.holdout_backtest.consumed_grant.issued_at.seconds = 6
    elif mutation == "holdout_submission_at_grant_expiry":
        record.specification.holdout_backtest.consumed_grant.expires_at.seconds = 5
    elif mutation == "holdout_submission_after_grant_expiry":
        record.specification.submitted_at.seconds = 6
        record.specification.holdout_backtest.consumed_grant.expires_at.seconds = 5
    else:
        raise AssertionError(f"unknown mutation {mutation}")


def _set_input_factor_id(record: job_pb2.JobRecord, value: str | None) -> None:
    input_name = record.specification.WhichOneof("input")
    if input_name == "factor_evaluation":
        target = record.specification.factor_evaluation.factor
    elif input_name == "backtest":
        target = record.specification.backtest
    elif input_name == "holdout_backtest":
        target = record.specification.holdout_backtest.frozen_backtest_spec
    else:
        raise AssertionError("mutation requires factor input")
    if value is None:
        target.ClearField("factor_spec_id")
    else:
        target.factor_spec_id.value = value


def _kind(value: str) -> int:
    return {
        "unspecified": job_pb2.JOB_KIND_UNSPECIFIED,
        "unknown": 999,
        "discovery": job_pb2.JOB_KIND_DISCOVERY,
        "factor_evaluation": job_pb2.JOB_KIND_FACTOR_EVALUATION,
        "backtest": job_pb2.JOB_KIND_BACKTEST,
        "independent_reconciliation": job_pb2.JOB_KIND_INDEPENDENT_RECONCILIATION,
        "report": job_pb2.JOB_KIND_REPORT,
        "prospective_observation": job_pb2.JOB_KIND_PROSPECTIVE_OBSERVATION,
        "holdout_backtest": job_pb2.JOB_KIND_HOLDOUT_BACKTEST,
    }[value]


def _state(value: str) -> int:
    return {
        "unspecified": job_pb2.JOB_STATE_UNSPECIFIED,
        "unknown": 999,
        "queued": job_pb2.JOB_STATE_QUEUED,
        "leased": job_pb2.JOB_STATE_LEASED,
        "running": job_pb2.JOB_STATE_RUNNING,
        "succeeded": job_pb2.JOB_STATE_SUCCEEDED,
        "factor_rejected": job_pb2.JOB_STATE_FACTOR_REJECTED,
        "infrastructure_failed": job_pb2.JOB_STATE_INFRASTRUCTURE_FAILED,
        "cancelled": job_pb2.JOB_STATE_CANCELLED,
        "budget_exhausted": job_pb2.JOB_STATE_BUDGET_EXHAUSTED,
    }[value]
