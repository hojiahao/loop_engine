from __future__ import annotations

import json
from dataclasses import replace
from pathlib import Path
from typing import Any, cast

import pytest

from loop.v1 import artifact_pb2, common_pb2, data_pb2, holdout_pb2
from loop_protocol import (
    CanonicalHoldoutEvaluationPlan,
    CanonicalHoldoutPeriod,
    EvaluationPlanReference,
    HoldoutValidationError,
    PlanArtifactReference,
    parse_canonical_holdout_evaluation_plan,
    parse_canonical_holdout_period,
    validate_holdout_evaluation_plan_reference,
    validate_wire_holdout_evaluation_plan_reference,
    validate_wire_holdout_period,
    verify_holdout_period_identity,
)

ROOT = Path(__file__).resolve().parents[3]
FIXTURE = cast(
    dict[str, Any],
    json.loads((ROOT / "tests/contracts/holdout_identity_golden.json").read_text("ascii")),
)
NEGATIVE_VECTORS = (ROOT / "tests/contracts/holdout_identity_negative.tsv").read_text("ascii")
TRUSTED_PLAN = bytes.fromhex(FIXTURE["trusted_plan_schema_sha256"][7:])
TRUSTED_BACKTEST = bytes.fromhex(FIXTURE["trusted_backtest_schema_sha256"][7:])
RESOLVED = {
    artifact["sha256"]: artifact["content"].encode("ascii")
    for artifact in FIXTURE["backtest_artifacts"]
}


def test_shared_holdout_golden_matches_exact_bytes_and_identities() -> None:
    for period_fixture in FIXTURE["periods"]:
        period = parse_canonical_holdout_period(period_fixture["canonical_json"])
        assert period.canonical_bytes == period_fixture["canonical_json"].encode("ascii")
        assert _encode_digest(period.canonical_period_sha256) == period_fixture["canonical_sha256"]
        assert period.holdout_period_id == period_fixture["holdout_period_id"]

        for plan_fixture in (
            candidate
            for candidate in FIXTURE["plans"]
            if candidate["period"] == period_fixture["name"]
        ):
            plan = parse_canonical_holdout_evaluation_plan(
                plan_fixture["canonical_json"],
                period,
                TRUSTED_BACKTEST,
                RESOLVED,
            )
            assert plan.canonical_bytes == plan_fixture["canonical_json"].encode("ascii")
            assert _encode_digest(plan.plan_sha256) == plan_fixture["plan_sha256"]
            assert plan.holdout_evaluation_plan_id == plan_fixture["holdout_evaluation_plan_id"]
            assert _encode_digest(plan.plan_sha256) != plan.holdout_evaluation_plan_id
            validated = validate_holdout_evaluation_plan_reference(
                _domain_reference(plan, period, plan_fixture["entry_count"]),
                plan.canonical_bytes,
                period,
                TRUSTED_PLAN,
                TRUSTED_BACKTEST,
                RESOLVED,
            )
            assert (
                validated.holdout_evaluation_plan_id == plan_fixture["holdout_evaluation_plan_id"]
            )


def test_generated_holdout_wire_references_are_exact_canonical_projections() -> None:
    period_fixture = FIXTURE["periods"][0]
    plan_fixture = FIXTURE["plans"][0]
    period = parse_canonical_holdout_period(period_fixture["canonical_json"])
    plan = parse_canonical_holdout_evaluation_plan(
        plan_fixture["canonical_json"], period, TRUSTED_BACKTEST, RESOLVED
    )
    period_json = json.loads(period_fixture["canonical_json"])
    wire_period = holdout_pb2.HoldoutPeriod(
        holdout_period_id=common_pb2.HoldoutPeriodId(value=period.holdout_period_id),
        sample=data_pb2.SampleWindow(
            role=data_pb2.SAMPLE_ROLE_FIRST_LOCKED_CONFIRMATION,
            start_inclusive=_civil_date(period_json["sample"]["start_inclusive"]),
            end_inclusive=_civil_date(period_json["sample"]["end_inclusive"]),
        ),
        snapshot_ids=[common_pb2.SnapshotId(value=value) for value in period_json["snapshot_ids"]],
        snapshot_manifest_sha256=_wire_digest(
            bytes.fromhex(period_json["snapshot_manifest_sha256"][7:])
        ),
        canonical_period_sha256=_wire_digest(period.canonical_period_sha256),
    )
    assert (
        validate_wire_holdout_period(wire_period, period.canonical_bytes).holdout_period_id
        == period.holdout_period_id
    )

    reference = _domain_reference(plan, period, plan_fixture["entry_count"])
    wire_plan = holdout_pb2.HoldoutEvaluationPlanReference(
        holdout_evaluation_plan_id=common_pb2.HoldoutEvaluationPlanId(
            value=reference.holdout_evaluation_plan_id
        ),
        canonical_plan=artifact_pb2.ArtifactRef(
            artifact_id=common_pb2.ArtifactId(value=reference.canonical_plan.artifact_id),
            uri=reference.canonical_plan.uri,
            sha256=_wire_digest(reference.canonical_plan.sha256),
            schema=artifact_pb2.ArtifactSchemaReference(
                name=reference.canonical_plan.schema_name,
                version=reference.canonical_plan.schema_version,
                schema_sha256=_wire_digest(reference.canonical_plan.schema_sha256),
            ),
            media_type=reference.canonical_plan.media_type,
            byte_size=reference.canonical_plan.byte_size,
        ),
        plan_sha256=_wire_digest(reference.plan_sha256),
        entry_count=reference.entry_count,
        holdout_period_id=common_pb2.HoldoutPeriodId(value=reference.holdout_period_id),
        canonical_period_sha256=_wire_digest(reference.canonical_period_sha256),
    )
    wire_plan.canonical_plan.created_at.seconds = 1
    assert (
        validate_wire_holdout_evaluation_plan_reference(
            wire_plan,
            plan.canonical_bytes,
            period,
            TRUSTED_PLAN,
            TRUSTED_BACKTEST,
            RESOLVED,
        ).holdout_evaluation_plan_id
        == plan.holdout_evaluation_plan_id
    )

    wire_plan.canonical_plan.ClearField("created_at")
    with pytest.raises(HoldoutValidationError):
        validate_wire_holdout_evaluation_plan_reference(
            wire_plan,
            plan.canonical_bytes,
            period,
            TRUSTED_PLAN,
            TRUSTED_BACKTEST,
            RESOLVED,
        )
    wire_plan.canonical_plan.created_at.seconds = 1
    wire_plan.canonical_plan.row_count = 1
    with pytest.raises(HoldoutValidationError):
        validate_wire_holdout_evaluation_plan_reference(
            wire_plan,
            plan.canonical_bytes,
            period,
            TRUSTED_PLAN,
            TRUSTED_BACKTEST,
            RESOLVED,
        )


def test_every_shared_holdout_negative_vector_fails_closed() -> None:
    period_fixture = FIXTURE["periods"][0]
    plan_fixture = FIXTURE["plans"][0]
    period = parse_canonical_holdout_period(period_fixture["canonical_json"])
    plan = parse_canonical_holdout_evaluation_plan(
        plan_fixture["canonical_json"], period, TRUSTED_BACKTEST, RESOLVED
    )
    for line in NEGATIVE_VECTORS.splitlines():
        if not line or line.startswith("#"):
            continue
        columns = line.split("\t")
        assert len(columns) == 3, f"invalid shared holdout vector: {line}"
        _name, target, mutation = columns
        with pytest.raises(HoldoutValidationError, match="holdout validation"):
            _execute_negative(
                target,
                mutation,
                period_fixture["canonical_json"],
                plan_fixture["canonical_json"],
                period,
                plan,
            )


def _execute_negative(
    target: str,
    mutation: str,
    period_source: str,
    plan_source: str,
    period: CanonicalHoldoutPeriod,
    plan: CanonicalHoldoutEvaluationPlan,
) -> None:
    if target == "period":
        parse_canonical_holdout_period(_mutate_period(period_source, mutation))
        return
    if target == "period_reference":
        verify_holdout_period_identity(
            period_source,
            _digest_text(238) if mutation == "period_id_mismatch" else period.holdout_period_id,
            bytes([238]) * 32
            if mutation == "period_digest_mismatch"
            else period.canonical_period_sha256,
        )
        return
    if target == "plan":
        mutated, trusted, resolved = _mutate_plan(plan_source, mutation)
        parse_canonical_holdout_evaluation_plan(
            mutated,
            period,
            trusted or TRUSTED_BACKTEST,
            resolved or RESOLVED,
        )
        return
    if target == "plan_reference":
        reference = _mutate_reference(
            _domain_reference(plan, period, len(plan.value.entries)), mutation
        )
        validate_holdout_evaluation_plan_reference(
            reference,
            plan.canonical_bytes,
            period,
            bytes([239]) * 32 if mutation == "plan_schema_digest_mismatch" else TRUSTED_PLAN,
            TRUSTED_BACKTEST,
            RESOLVED,
        )
        return
    raise AssertionError(f"unimplemented negative target {target}")


def _mutate_period(source: str, mutation: str) -> str:
    parsed = json.loads(source)
    if mutation == "unknown_field":
        return source.replace("{", '{"unknown":"x",', 1)
    if mutation == "duplicate_schema":
        return source.replace(
            '"schema":"loop.holdout-period/v1"',
            '"schema":"loop.holdout-period/v1","schema":"loop.holdout-period/v1"',
            1,
        )
    if mutation == "reorder_top_level":
        return json.dumps(
            {
                "sample": parsed["sample"],
                "schema": parsed["schema"],
                "snapshot_ids": parsed["snapshot_ids"],
                "snapshot_manifest_sha256": parsed["snapshot_manifest_sha256"],
            },
            separators=(",", ":"),
        )
    if mutation == "leading_whitespace":
        return f" {source}"
    if mutation == "wrong_schema":
        return source.replace("loop.holdout-period/v1", "loop.holdout-period/v2", 1)
    if mutation == "forbidden_role":
        return source.replace("first_locked_confirmation", "development_validation", 1)
    if mutation == "invalid_date":
        return source.replace("2024-12-31", "2024-02-30", 1)
    if mutation == "reversed_window":
        return source.replace("2021-01-01", "2025-01-01", 1)
    if mutation == "empty_snapshots":
        parsed["snapshot_ids"] = []
    elif mutation == "unsorted_snapshots":
        parsed["snapshot_ids"].reverse()
    elif mutation == "duplicate_snapshots":
        parsed["snapshot_ids"] = [parsed["snapshot_ids"][0]] * 2
    elif mutation == "bad_snapshot_digest":
        parsed["snapshot_ids"][0] = f"sha256:{'A' * 64}"
    elif mutation == "bad_manifest_digest":
        parsed["snapshot_manifest_sha256"] = f"sha256:{'g' * 64}"
    elif mutation == "number_date":
        parsed["sample"]["start_inclusive"] = 20210101
    elif mutation == "deep_nesting":
        return "[" * 40 + "0" + "]" * 40
    else:
        raise AssertionError(f"unimplemented period mutation {mutation}")
    return json.dumps(parsed, separators=(",", ":"))


def _mutate_plan(source: str, mutation: str) -> tuple[str, bytes | None, dict[str, bytes] | None]:
    parsed = json.loads(source)
    first = parsed["entries"][0]
    second = parsed["entries"][1]
    artifact = first["backtest_spec_artifact"]
    budget = first["job_budget"]
    if mutation == "unknown_field":
        parsed["unknown"] = "x"
    elif mutation == "duplicate_schema":
        return (
            source.replace(
                f'"schema":"{parsed["schema"]}"',
                f'"schema":"{parsed["schema"]}","schema":"{parsed["schema"]}"',
                1,
            ),
            None,
            None,
        )
    elif mutation == "reorder_top_level":
        return (
            json.dumps(
                {
                    "holdout_period_id": parsed["holdout_period_id"],
                    "schema": parsed["schema"],
                    "canonical_period_sha256": parsed["canonical_period_sha256"],
                    "entries": parsed["entries"],
                },
                separators=(",", ":"),
            ),
            None,
            None,
        )
    elif mutation == "leading_whitespace":
        return f" {source}", None, None
    elif mutation == "wrong_schema":
        parsed["schema"] = "loop.holdout-evaluation-plan/v2"
    elif mutation == "period_id_mismatch":
        parsed["holdout_period_id"] = _digest_text(225)
    elif mutation == "period_digest_mismatch":
        parsed["canonical_period_sha256"] = _digest_text(226)
    elif mutation == "empty_entries":
        parsed["entries"] = []
    elif mutation == "noncontiguous_entries":
        second["entry_index"] = "3"
    elif mutation == "duplicate_factor":
        second["factor_spec_id"] = first["factor_spec_id"]
    elif mutation == "duplicate_backtest":
        second["backtest_spec_artifact"] = dict(first["backtest_spec_artifact"])
    elif mutation == "inline_backtest":
        artifact["inline"] = {"schema": "forbidden"}
    elif mutation == "bad_locator":
        artifact["uri"] = f"https://user:secret@example.invalid/{artifact['sha256']}"
    elif mutation == "wrong_artifact_schema":
        artifact["schema_name"] = "loop.other_spec"
    elif mutation == "wrong_artifact_version":
        artifact["schema_version"] = "2"
    elif mutation == "wrong_backtest_schema_digest":
        artifact["schema_sha256"] = _digest_text(227)
    elif mutation == "wrong_media_type":
        artifact["media_type"] = "application/octet-stream"
    elif mutation == "zero_artifact_size":
        artifact["byte_size"] = "0"
    elif mutation == "oversized_artifact":
        artifact["byte_size"] = "268435457"
    elif mutation == "non_normalized_artifact_size":
        artifact["byte_size"] = "050"
    elif mutation == "zero_steps":
        budget["maximum_steps"] = "0"
    elif mutation == "non_normalized_steps":
        budget["maximum_steps"] = "040"
    elif mutation == "token_overflow":
        budget["maximum_input_tokens"] = "1000000000001"
    elif mutation == "negative_token":
        budget["maximum_output_tokens"] = "-1"
    elif mutation == "cost_overflow":
        budget["maximum_cost"]["amount"] = "1000000.1"
    elif mutation == "cost_precision":
        budget["maximum_cost"]["amount"] = "1234567890123456789"
    elif mutation == "cost_scale":
        budget["maximum_cost"]["amount"] = "0.1234567891"
    elif mutation == "lowercase_currency":
        budget["maximum_cost"]["currency_code"] = "usd"
    elif mutation == "zero_wall_time":
        budget["maximum_wall_time_ns"] = "0"
    elif mutation == "wall_time_overflow":
        budget["maximum_wall_time_ns"] = "604800000000001"
    elif mutation == "unresolved_artifact":
        reduced = dict(RESOLVED)
        del reduced[artifact["sha256"]]
        return source, None, reduced
    elif mutation == "artifact_content_mismatch":
        changed = dict(RESOLVED)
        changed[artifact["sha256"]] = b"different bytes of exactly no relevance"
        return source, None, changed
    elif mutation == "deep_nesting":
        return "[" * 40 + "0" + "]" * 40, None, None
    else:
        raise AssertionError(f"unimplemented plan mutation {mutation}")
    return json.dumps(parsed, separators=(",", ":")), None, None


def _mutate_reference(reference: EvaluationPlanReference, mutation: str) -> EvaluationPlanReference:
    if mutation == "plan_sha256_mismatch":
        return replace(reference, plan_sha256=bytes([230]) * 32)
    if mutation == "plan_id_mismatch":
        return replace(reference, holdout_evaluation_plan_id=_digest_text(231))
    if mutation == "entry_count_mismatch":
        return replace(reference, entry_count=reference.entry_count + 1)
    if mutation == "period_id_mismatch":
        return replace(reference, holdout_period_id=_digest_text(232))
    if mutation == "period_digest_mismatch":
        return replace(reference, canonical_period_sha256=bytes([233]) * 32)
    artifact = reference.canonical_plan
    if mutation == "plan_artifact_schema_mismatch":
        artifact = replace(artifact, schema_name="loop.other_plan")
    elif mutation == "plan_schema_digest_mismatch":
        artifact = replace(artifact, schema_sha256=bytes([234]) * 32)
    elif mutation == "plan_artifact_size_mismatch":
        artifact = replace(artifact, byte_size=artifact.byte_size + 1)
    elif mutation == "plan_artifact_locator_mismatch":
        artifact = replace(artifact, uri=f"artifact://sha256/{'00' * 32}")
    else:
        raise AssertionError(f"unimplemented plan reference mutation {mutation}")
    return replace(reference, canonical_plan=artifact)


def _domain_reference(
    plan: CanonicalHoldoutEvaluationPlan,
    period: CanonicalHoldoutPeriod,
    entry_count: int,
) -> EvaluationPlanReference:
    raw_id = _encode_digest(plan.plan_sha256)
    return EvaluationPlanReference(
        plan.holdout_evaluation_plan_id,
        PlanArtifactReference(
            raw_id,
            f"artifact://sha256/{raw_id[7:]}",
            plan.plan_sha256,
            "loop.holdout_evaluation_plan",
            1,
            TRUSTED_PLAN,
            "application/json",
            len(plan.canonical_bytes),
            False,
            False,
        ),
        plan.plan_sha256,
        entry_count,
        period.holdout_period_id,
        period.canonical_period_sha256,
    )


def _civil_date(value: str) -> common_pb2.CivilDate:
    year, month, day = (int(part) for part in value.split("-"))
    return common_pb2.CivilDate(year=year, month=month, day=day)


def _wire_digest(value: bytes) -> common_pb2.Sha256Digest:
    return common_pb2.Sha256Digest(value=value)


def _encode_digest(value: bytes) -> str:
    return f"sha256:{value.hex()}"


def _digest_text(byte: int) -> str:
    return f"sha256:{bytes([byte]).hex() * 32}"
