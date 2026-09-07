"""Canonical identities and fail-closed wire validation for locked holdouts."""

from __future__ import annotations

import hashlib
import hmac
import json
import re
from collections.abc import Mapping
from dataclasses import dataclass
from enum import StrEnum
from typing import Any, Final, NoReturn

from loop.v1 import data_pb2, holdout_pb2
from loop.v1.artifact_pb2 import ArtifactRef
from loop.v1.common_pb2 import Sha256Digest

from .artifact import ArtifactValidationError, validate_artifact_ref

_PERIOD_SCHEMA: Final = "loop.holdout-period/v1"
_PLAN_SCHEMA: Final = "loop.holdout-evaluation-plan/v1"
_PERIOD_DOMAIN: Final = b"loop.holdout-period/v1\x00"
_PLAN_DOMAIN: Final = b"loop.holdout-evaluation-plan/v1\x00"
_BACKTEST_SCHEMA_NAME: Final = "loop.backtest_spec"
_PLAN_ARTIFACT_SCHEMA_NAME: Final = "loop.holdout_evaluation_plan"
_JSON_MEDIA_TYPE: Final = "application/json"
_SHA256_RE: Final = re.compile(r"^sha256:[0-9a-f]{64}$", re.ASCII)
_UNSIGNED_RE: Final = re.compile(r"^(?:0|[1-9][0-9]*)$", re.ASCII)
_DATE_RE: Final = re.compile(r"^(\d{4})-(\d{2})-(\d{2})$", re.ASCII)
_CURRENCY_RE: Final = re.compile(r"^[A-Z]{3}$", re.ASCII)
_COST_RE: Final = re.compile(r"^(0|[1-9][0-9]*)(?:\.([0-9]*[1-9]))?$", re.ASCII)

MAX_HOLDOUT_PERIOD_BYTES: Final = 64 * 1_024
MAX_HOLDOUT_PLAN_BYTES: Final = 8 * 1_024 * 1_024
MAX_HOLDOUT_SNAPSHOTS: Final = 128
MAX_HOLDOUT_PLAN_ENTRIES: Final = 4_096
MAX_BACKTEST_ARTIFACT_BYTES: Final = 268_435_456
MAX_HOLDOUT_STEPS: Final = 1_000_000
MAX_HOLDOUT_TOKENS: Final = 1_000_000_000_000
MAX_HOLDOUT_WALL_TIME_NS: Final = 604_800_000_000_000


class HoldoutValidationError(ValueError):
    """A canonical holdout value or transport reference failed validation."""

    def __init__(self, code: str, field: str) -> None:
        self.code = code
        self.field = field
        super().__init__(f"{field} failed holdout validation ({code})")


class LockedSampleRole(StrEnum):
    FIRST_LOCKED_CONFIRMATION = "first_locked_confirmation"
    SECOND_LOCKED_HISTORICAL_HOLDOUT = "second_locked_historical_holdout"


@dataclass(frozen=True, slots=True)
class HoldoutSampleWindow:
    role: LockedSampleRole
    start_inclusive: str
    end_inclusive: str


@dataclass(frozen=True, slots=True)
class HoldoutPeriodValue:
    sample: HoldoutSampleWindow
    snapshot_ids: tuple[str, ...]
    snapshot_manifest_sha256: str


@dataclass(frozen=True, slots=True)
class CanonicalHoldoutPeriod:
    value: HoldoutPeriodValue
    canonical_bytes: bytes
    canonical_period_sha256: bytes
    holdout_period_id: str


@dataclass(frozen=True, slots=True)
class BacktestSpecArtifactValue:
    artifact_id: str
    uri: str
    sha256: str
    schema_name: str
    schema_version: str
    schema_sha256: str
    media_type: str
    byte_size: str


@dataclass(frozen=True, slots=True)
class HoldoutMoneyBudget:
    amount: str
    currency_code: str


@dataclass(frozen=True, slots=True)
class HoldoutJobBudget:
    maximum_steps: str
    maximum_input_tokens: str
    maximum_output_tokens: str
    maximum_cost: HoldoutMoneyBudget
    maximum_wall_time_ns: str


@dataclass(frozen=True, slots=True)
class HoldoutEvaluationPlanEntry:
    entry_index: str
    factor_spec_id: str
    backtest_spec_artifact: BacktestSpecArtifactValue
    job_budget: HoldoutJobBudget


@dataclass(frozen=True, slots=True)
class HoldoutEvaluationPlanValue:
    holdout_period_id: str
    canonical_period_sha256: str
    entries: tuple[HoldoutEvaluationPlanEntry, ...]


@dataclass(frozen=True, slots=True)
class CanonicalHoldoutEvaluationPlan:
    value: HoldoutEvaluationPlanValue
    canonical_bytes: bytes
    plan_sha256: bytes
    holdout_evaluation_plan_id: str


@dataclass(frozen=True, slots=True)
class PlanArtifactReference:
    artifact_id: str
    uri: str
    sha256: bytes
    schema_name: str
    schema_version: int
    schema_sha256: bytes
    media_type: str
    byte_size: int
    has_row_count: bool
    has_manifest_sha256: bool


@dataclass(frozen=True, slots=True)
class EvaluationPlanReference:
    holdout_evaluation_plan_id: str
    canonical_plan: PlanArtifactReference
    plan_sha256: bytes
    entry_count: int
    holdout_period_id: str
    canonical_period_sha256: bytes


def canonicalize_holdout_period(value: HoldoutPeriodValue) -> CanonicalHoldoutPeriod:
    """Validate and canonicalize one locked historical sample identity."""

    _validate_period(value)
    canonical_bytes = _write_period(value).encode("ascii")
    if len(canonical_bytes) > MAX_HOLDOUT_PERIOD_BYTES:
        _fail("size_limit", "period")
    digest = _domain_hash(_PERIOD_DOMAIN, canonical_bytes)
    return CanonicalHoldoutPeriod(value, canonical_bytes, digest, _encode_digest(digest))


def parse_canonical_holdout_period(value: bytes | str) -> CanonicalHoldoutPeriod:
    """Strict-parse exact period bytes without discarding JSON structure."""

    canonical_bytes, raw_value = _parse_json(value, MAX_HOLDOUT_PERIOD_BYTES, "period")
    raw = _require_dict(raw_value, "period")
    _require_exact_keys(
        raw,
        ("schema", "sample", "snapshot_ids", "snapshot_manifest_sha256"),
        "period",
    )
    if _require_string(raw["schema"], "schema") != _PERIOD_SCHEMA:
        _fail("invalid_schema", "schema")
    sample = _require_dict(raw["sample"], "sample")
    _require_exact_keys(sample, ("role", "start_inclusive", "end_inclusive"), "sample")
    raw_snapshots = _require_list(raw["snapshot_ids"], "snapshot_ids")
    parsed = canonicalize_holdout_period(
        HoldoutPeriodValue(
            HoldoutSampleWindow(
                _parse_role(_require_string(sample["role"], "sample.role")),
                _require_string(sample["start_inclusive"], "sample.start_inclusive"),
                _require_string(sample["end_inclusive"], "sample.end_inclusive"),
            ),
            tuple(
                _require_string(snapshot, f"snapshot_ids[{index}]")
                for index, snapshot in enumerate(raw_snapshots)
            ),
            _require_string(raw["snapshot_manifest_sha256"], "snapshot_manifest_sha256"),
        )
    )
    if not hmac.compare_digest(parsed.canonical_bytes, canonical_bytes):
        _fail("non_canonical", "period")
    return parsed


def verify_holdout_period_identity(
    value: bytes | str,
    holdout_period_id: str,
    canonical_period_sha256: bytes,
) -> CanonicalHoldoutPeriod:
    parsed = parse_canonical_holdout_period(value)
    if parsed.holdout_period_id != holdout_period_id or not _equal_digest(
        parsed.canonical_period_sha256, canonical_period_sha256
    ):
        _fail("period_mismatch", "holdout_period_id")
    return parsed


def canonicalize_holdout_evaluation_plan(
    value: HoldoutEvaluationPlanValue,
    expected_period: CanonicalHoldoutPeriod,
    trusted_backtest_schema_sha256: bytes,
    resolved_backtest_artifacts: Mapping[str, bytes],
) -> CanonicalHoldoutEvaluationPlan:
    """Validate a complete plan and bind its raw and domain-separated hashes."""

    _require_raw_digest(trusted_backtest_schema_sha256, "trusted_backtest_schema_sha256")
    _validate_plan(
        value,
        expected_period,
        trusted_backtest_schema_sha256,
        resolved_backtest_artifacts,
    )
    canonical_bytes = _write_plan(value).encode("ascii")
    if len(canonical_bytes) > MAX_HOLDOUT_PLAN_BYTES:
        _fail("size_limit", "plan")
    return CanonicalHoldoutEvaluationPlan(
        value,
        canonical_bytes,
        hashlib.sha256(canonical_bytes).digest(),
        _encode_digest(_domain_hash(_PLAN_DOMAIN, canonical_bytes)),
    )


def parse_canonical_holdout_evaluation_plan(
    value: bytes | str,
    expected_period: CanonicalHoldoutPeriod,
    trusted_backtest_schema_sha256: bytes,
    resolved_backtest_artifacts: Mapping[str, bytes],
) -> CanonicalHoldoutEvaluationPlan:
    """Strict-parse exact plan bytes before resolving referenced artifacts."""

    canonical_bytes, raw_value = _parse_json(value, MAX_HOLDOUT_PLAN_BYTES, "plan")
    raw = _require_dict(raw_value, "plan")
    _require_exact_keys(
        raw,
        ("schema", "holdout_period_id", "canonical_period_sha256", "entries"),
        "plan",
    )
    if _require_string(raw["schema"], "schema") != _PLAN_SCHEMA:
        _fail("invalid_schema", "schema")
    entries = tuple(
        _decode_plan_entry(entry, index)
        for index, entry in enumerate(_require_list(raw["entries"], "entries"))
    )
    parsed = canonicalize_holdout_evaluation_plan(
        HoldoutEvaluationPlanValue(
            _require_string(raw["holdout_period_id"], "holdout_period_id"),
            _require_string(raw["canonical_period_sha256"], "canonical_period_sha256"),
            entries,
        ),
        expected_period,
        trusted_backtest_schema_sha256,
        resolved_backtest_artifacts,
    )
    if not hmac.compare_digest(parsed.canonical_bytes, canonical_bytes):
        _fail("non_canonical", "plan")
    return parsed


def validate_holdout_evaluation_plan_reference(
    reference: EvaluationPlanReference,
    canonical_plan_bytes: bytes,
    expected_period: CanonicalHoldoutPeriod,
    trusted_plan_schema_sha256: bytes,
    trusted_backtest_schema_sha256: bytes,
    resolved_backtest_artifacts: Mapping[str, bytes],
) -> CanonicalHoldoutEvaluationPlan:
    """Resolve and validate every identity repeated by a plan artifact reference."""

    _require_raw_digest(trusted_plan_schema_sha256, "trusted_plan_schema_sha256")
    parsed = parse_canonical_holdout_evaluation_plan(
        canonical_plan_bytes,
        expected_period,
        trusted_backtest_schema_sha256,
        resolved_backtest_artifacts,
    )
    raw_id = _encode_digest(parsed.plan_sha256)
    artifact = reference.canonical_plan
    if (
        artifact.artifact_id != raw_id
        or artifact.uri != f"artifact://sha256/{raw_id[7:]}"
        or not _equal_digest(artifact.sha256, parsed.plan_sha256)
        or not _equal_digest(reference.plan_sha256, parsed.plan_sha256)
        or artifact.byte_size != len(canonical_plan_bytes)
        or artifact.has_row_count
        or artifact.has_manifest_sha256
    ):
        _fail("reference_mismatch", "canonical_plan")
    if (
        artifact.schema_name != _PLAN_ARTIFACT_SCHEMA_NAME
        or artifact.schema_version != 1
        or artifact.media_type != _JSON_MEDIA_TYPE
        or not _equal_digest(artifact.schema_sha256, trusted_plan_schema_sha256)
    ):
        _fail("schema_mismatch", "canonical_plan.schema")
    if (
        reference.holdout_evaluation_plan_id != parsed.holdout_evaluation_plan_id
        or reference.entry_count != len(parsed.value.entries)
        or reference.holdout_period_id != expected_period.holdout_period_id
        or not _equal_digest(
            reference.canonical_period_sha256,
            expected_period.canonical_period_sha256,
        )
    ):
        _fail("reference_mismatch", "plan_reference")
    return parsed


def validate_wire_holdout_period(
    wire: holdout_pb2.HoldoutPeriod,
    canonical_period_bytes: bytes,
) -> CanonicalHoldoutPeriod:
    """Validate a wire period as a projection of independently canonical bytes."""

    if not wire.HasField("holdout_period_id"):
        _fail("reference_mismatch", "holdout_period_id")
    digest = _require_wire_digest(
        wire.canonical_period_sha256 if wire.HasField("canonical_period_sha256") else None,
        "canonical_period_sha256",
    )
    parsed = verify_holdout_period_identity(
        canonical_period_bytes,
        wire.holdout_period_id.value,
        digest,
    )
    if not wire.HasField("sample"):
        _fail("reference_mismatch", "sample")
    role_by_wire = {
        data_pb2.SAMPLE_ROLE_FIRST_LOCKED_CONFIRMATION: (
            LockedSampleRole.FIRST_LOCKED_CONFIRMATION
        ),
        data_pb2.SAMPLE_ROLE_SECOND_LOCKED_HISTORICAL_HOLDOUT: (
            LockedSampleRole.SECOND_LOCKED_HISTORICAL_HOLDOUT
        ),
    }
    role = role_by_wire.get(wire.sample.role)
    if role is None:
        _fail("reference_mismatch", "sample.role")
    start = _format_wire_date(
        wire.sample.start_inclusive if wire.sample.HasField("start_inclusive") else None,
        "sample.start_inclusive",
    )
    end = _format_wire_date(
        wire.sample.end_inclusive if wire.sample.HasField("end_inclusive") else None,
        "sample.end_inclusive",
    )
    manifest = _require_wire_digest(
        wire.snapshot_manifest_sha256 if wire.HasField("snapshot_manifest_sha256") else None,
        "snapshot_manifest_sha256",
    )
    if (
        parsed.value.sample.role != role
        or parsed.value.sample.start_inclusive != start
        or parsed.value.sample.end_inclusive != end
        or parsed.value.snapshot_ids != tuple(item.value for item in wire.snapshot_ids)
        or parsed.value.snapshot_manifest_sha256 != _encode_digest(manifest)
    ):
        _fail("reference_mismatch", "holdout_period")
    return parsed


def validate_wire_holdout_evaluation_plan_reference(
    wire: holdout_pb2.HoldoutEvaluationPlanReference,
    canonical_plan_bytes: bytes,
    expected_period: CanonicalHoldoutPeriod,
    trusted_plan_schema_sha256: bytes,
    trusted_backtest_schema_sha256: bytes,
    resolved_backtest_artifacts: Mapping[str, bytes],
) -> CanonicalHoldoutEvaluationPlan:
    """Validate a generated plan DTO before it can enter persistence."""

    if not wire.HasField("canonical_plan"):
        _fail("reference_mismatch", "canonical_plan")
    artifact = _validate_plan_wire_artifact(wire.canonical_plan)
    if not wire.HasField("holdout_evaluation_plan_id"):
        _fail("reference_mismatch", "holdout_evaluation_plan_id")
    if not wire.HasField("holdout_period_id"):
        _fail("reference_mismatch", "holdout_period_id")
    return validate_holdout_evaluation_plan_reference(
        EvaluationPlanReference(
            wire.holdout_evaluation_plan_id.value,
            artifact,
            _require_wire_digest(
                wire.plan_sha256 if wire.HasField("plan_sha256") else None,
                "plan_sha256",
            ),
            wire.entry_count,
            wire.holdout_period_id.value,
            _require_wire_digest(
                wire.canonical_period_sha256 if wire.HasField("canonical_period_sha256") else None,
                "canonical_period_sha256",
            ),
        ),
        canonical_plan_bytes,
        expected_period,
        trusted_plan_schema_sha256,
        trusted_backtest_schema_sha256,
        resolved_backtest_artifacts,
    )


def _validate_plan_wire_artifact(value: ArtifactRef) -> PlanArtifactReference:
    try:
        artifact = validate_artifact_ref(value)
    except ArtifactValidationError:
        _fail("reference_mismatch", "canonical_plan")
    return PlanArtifactReference(
        artifact.artifact_id,
        artifact.uri,
        artifact.sha256,
        artifact.schema_name,
        artifact.schema_version,
        artifact.schema_sha256,
        artifact.media_type,
        artifact.byte_size,
        artifact.row_count is not None,
        artifact.manifest_sha256 is not None,
    )


def _validate_period(value: HoldoutPeriodValue) -> None:
    if not isinstance(value.sample.role, LockedSampleRole):
        _fail("invalid_role", "sample.role")
    start = _parse_date(value.sample.start_inclusive)
    end = _parse_date(value.sample.end_inclusive)
    if start > end:
        _fail("invalid_window", "sample")
    if not 1 <= len(value.snapshot_ids) <= MAX_HOLDOUT_SNAPSHOTS:
        _fail("invalid_snapshots", "snapshot_ids")
    previous: str | None = None
    for snapshot_id in value.snapshot_ids:
        _require_digest_text(snapshot_id, "snapshot_ids")
        if previous is not None and previous >= snapshot_id:
            _fail("invalid_snapshots", "snapshot_ids")
        previous = snapshot_id
    _require_digest_text(value.snapshot_manifest_sha256, "snapshot_manifest_sha256")


def _validate_plan(
    value: HoldoutEvaluationPlanValue,
    expected_period: CanonicalHoldoutPeriod,
    trusted_backtest_schema_sha256: bytes,
    resolved: Mapping[str, bytes],
) -> None:
    if (
        value.holdout_period_id != expected_period.holdout_period_id
        or value.canonical_period_sha256 != expected_period.holdout_period_id
    ):
        _fail("period_mismatch", "holdout_period_id")
    if not 1 <= len(value.entries) <= MAX_HOLDOUT_PLAN_ENTRIES:
        _fail("invalid_entries", "entries")
    factors: set[str] = set()
    artifacts: set[str] = set()
    for position, entry in enumerate(value.entries, start=1):
        if (
            _parse_unsigned(
                entry.entry_index,
                1,
                MAX_HOLDOUT_PLAN_ENTRIES,
                "entry_index",
            )
            != position
        ):
            _fail("invalid_entries", "entry_index")
        _require_digest_text(entry.factor_spec_id, "factor_spec_id")
        if entry.factor_spec_id in factors:
            _fail("duplicate_identity", "factor_spec_id")
        factors.add(entry.factor_spec_id)
        _validate_backtest_artifact(
            entry.backtest_spec_artifact,
            trusted_backtest_schema_sha256,
            resolved,
        )
        if entry.backtest_spec_artifact.artifact_id in artifacts:
            _fail("duplicate_identity", "backtest_spec_artifact.artifact_id")
        artifacts.add(entry.backtest_spec_artifact.artifact_id)
        _validate_budget(entry.job_budget)


def _validate_backtest_artifact(
    artifact: BacktestSpecArtifactValue,
    trusted_schema_sha256: bytes,
    resolved: Mapping[str, bytes],
) -> None:
    digest = _require_digest_text(artifact.sha256, "backtest_spec_artifact.sha256")
    if (
        artifact.artifact_id != artifact.sha256
        or artifact.uri != f"artifact://sha256/{artifact.sha256[7:]}"
    ):
        _fail("invalid_artifact", "backtest_spec_artifact")
    if (
        artifact.schema_name != _BACKTEST_SCHEMA_NAME
        or artifact.schema_version != "1"
        or artifact.media_type != _JSON_MEDIA_TYPE
        or not _equal_digest(
            _require_digest_text(
                artifact.schema_sha256,
                "backtest_spec_artifact.schema_sha256",
            ),
            trusted_schema_sha256,
        )
    ):
        _fail("schema_mismatch", "backtest_spec_artifact.schema")
    declared_size = _parse_unsigned(
        artifact.byte_size,
        1,
        MAX_BACKTEST_ARTIFACT_BYTES,
        "backtest_spec_artifact.byte_size",
    )
    content = resolved.get(artifact.sha256)
    if content is None:
        _fail("unresolved_artifact", "backtest_spec_artifact")
    if len(content) != declared_size or not _equal_digest(hashlib.sha256(content).digest(), digest):
        _fail("invalid_artifact", "backtest_spec_artifact")


def _validate_budget(value: HoldoutJobBudget) -> None:
    _parse_unsigned(value.maximum_steps, 1, MAX_HOLDOUT_STEPS, "maximum_steps")
    _parse_unsigned(value.maximum_input_tokens, 0, MAX_HOLDOUT_TOKENS, "maximum_input_tokens")
    _parse_unsigned(
        value.maximum_output_tokens,
        0,
        MAX_HOLDOUT_TOKENS,
        "maximum_output_tokens",
    )
    _parse_unsigned(
        value.maximum_wall_time_ns,
        1,
        MAX_HOLDOUT_WALL_TIME_NS,
        "maximum_wall_time_ns",
    )
    _validate_cost(value.maximum_cost)


def _validate_cost(value: HoldoutMoneyBudget) -> None:
    if _CURRENCY_RE.fullmatch(value.currency_code) is None:
        _fail("invalid_budget", "maximum_cost.currency_code")
    match = _COST_RE.fullmatch(value.amount)
    if match is None:
        _fail("invalid_budget", "maximum_cost.amount")
    integer = match.group(1)
    fraction = match.group(2)
    if fraction is not None and len(fraction) > 9:
        _fail("invalid_budget", "maximum_cost.amount")
    significant = (
        max(len((fraction or "").lstrip("0")), 1)
        if integer == "0"
        else len(integer) + len(fraction or "")
    )
    if (
        significant > 18
        or int(integer) > 1_000_000
        or (integer == "1000000" and fraction is not None)
    ):
        _fail("invalid_budget", "maximum_cost.amount")


def _decode_plan_entry(value: object, index: int) -> HoldoutEvaluationPlanEntry:
    path = f"entries[{index}]"
    raw = _require_dict(value, path)
    _require_exact_keys(
        raw,
        ("entry_index", "factor_spec_id", "backtest_spec_artifact", "job_budget"),
        path,
    )
    artifact_path = f"{path}.backtest_spec_artifact"
    artifact = _require_dict(raw["backtest_spec_artifact"], artifact_path)
    _require_exact_keys(
        artifact,
        (
            "artifact_id",
            "uri",
            "sha256",
            "schema_name",
            "schema_version",
            "schema_sha256",
            "media_type",
            "byte_size",
        ),
        artifact_path,
    )
    budget_path = f"{path}.job_budget"
    budget = _require_dict(raw["job_budget"], budget_path)
    _require_exact_keys(
        budget,
        (
            "maximum_steps",
            "maximum_input_tokens",
            "maximum_output_tokens",
            "maximum_cost",
            "maximum_wall_time_ns",
        ),
        budget_path,
    )
    cost_path = f"{budget_path}.maximum_cost"
    cost = _require_dict(budget["maximum_cost"], cost_path)
    _require_exact_keys(cost, ("amount", "currency_code"), cost_path)
    return HoldoutEvaluationPlanEntry(
        _require_string(raw["entry_index"], f"{path}.entry_index"),
        _require_string(raw["factor_spec_id"], f"{path}.factor_spec_id"),
        BacktestSpecArtifactValue(
            *(
                _require_string(artifact[field], f"{artifact_path}.{field}")
                for field in (
                    "artifact_id",
                    "uri",
                    "sha256",
                    "schema_name",
                    "schema_version",
                    "schema_sha256",
                    "media_type",
                    "byte_size",
                )
            )
        ),
        HoldoutJobBudget(
            _require_string(budget["maximum_steps"], f"{budget_path}.maximum_steps"),
            _require_string(
                budget["maximum_input_tokens"],
                f"{budget_path}.maximum_input_tokens",
            ),
            _require_string(
                budget["maximum_output_tokens"],
                f"{budget_path}.maximum_output_tokens",
            ),
            HoldoutMoneyBudget(
                _require_string(cost["amount"], f"{cost_path}.amount"),
                _require_string(cost["currency_code"], f"{cost_path}.currency_code"),
            ),
            _require_string(
                budget["maximum_wall_time_ns"],
                f"{budget_path}.maximum_wall_time_ns",
            ),
        ),
    )


class _DuplicateKeyError(ValueError):
    pass


def _object_pairs(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise _DuplicateKeyError(key)
        result[key] = value
    return result


def _parse_json(value: bytes | str, maximum: int, field: str) -> tuple[bytes, object]:
    canonical_bytes = value.encode("utf-8") if isinstance(value, str) else bytes(value)
    _validate_json_envelope(canonical_bytes, maximum)
    try:
        text = canonical_bytes.decode("utf-8-sig")
        if text.encode("utf-8") != canonical_bytes:
            _fail("non_canonical", field)
        parsed: Any = json.loads(text, object_pairs_hook=_object_pairs)
    except HoldoutValidationError:
        raise
    except UnicodeDecodeError, json.JSONDecodeError, _DuplicateKeyError, RecursionError:
        _fail("invalid_json", field)
    return canonical_bytes, parsed


def _validate_json_envelope(value: bytes, maximum: int) -> None:
    if not 1 <= len(value) <= maximum:
        _fail("size_limit", "json")
    depth = 0
    in_string = False
    escaped = False
    for byte in value:
        if in_string:
            if escaped:
                escaped = False
            elif byte == 0x5C:
                escaped = True
            elif byte == 0x22:
                in_string = False
            continue
        if byte == 0x22:
            in_string = True
        elif byte in (0x7B, 0x5B):
            depth += 1
            if depth > 32:
                _fail("size_limit", "json.depth")
        elif byte in (0x7D, 0x5D):
            depth = max(0, depth - 1)


def _require_dict(value: object, field: str) -> dict[str, object]:
    if not isinstance(value, dict) or not all(isinstance(key, str) for key in value):
        _fail("invalid_json", field)
    return value


def _require_list(value: object, field: str) -> list[object]:
    if not isinstance(value, list):
        _fail("invalid_json", field)
    return value


def _require_exact_keys(
    value: dict[str, object],
    expected: tuple[str, ...],
    field: str,
) -> None:
    if tuple(value) != expected:
        _fail("non_canonical", field)


def _require_string(value: object, field: str) -> str:
    if not isinstance(value, str):
        _fail("invalid_json", field)
    return value


def _parse_role(value: str) -> LockedSampleRole:
    try:
        return LockedSampleRole(value)
    except ValueError:
        _fail("invalid_role", "sample.role")


def _parse_date(value: str) -> tuple[int, int, int]:
    match = _DATE_RE.fullmatch(value)
    if match is None:
        _fail("invalid_date", "sample.date")
    year, month, day = (int(part) for part in match.groups())
    if not 1900 <= year <= 9999 or not 1 <= month <= 12:
        _fail("invalid_date", "sample.date")
    leap = year % 4 == 0 and (year % 100 != 0 or year % 400 == 0)
    days = (31, 29 if leap else 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31)
    if not 1 <= day <= days[month - 1]:
        _fail("invalid_date", "sample.date")
    return year, month, day


def _parse_unsigned(value: str, minimum: int, maximum: int, field: str) -> int:
    if _UNSIGNED_RE.fullmatch(value) is None:
        _fail("invalid_budget", field)
    parsed = int(value)
    if not minimum <= parsed <= maximum:
        _fail("invalid_budget", field)
    return parsed


def _require_digest_text(value: str, field: str) -> bytes:
    if _SHA256_RE.fullmatch(value) is None:
        _fail("invalid_digest", field)
    return bytes.fromhex(value[7:])


def _require_raw_digest(value: bytes, field: str) -> None:
    if not isinstance(value, bytes) or len(value) != 32:
        _fail("invalid_digest", field)


def _require_wire_digest(value: Sha256Digest | None, field: str) -> bytes:
    if value is None:
        _fail("reference_mismatch", field)
    digest = bytes(value.value)
    if len(digest) != 32:
        _fail("reference_mismatch", field)
    return digest


def _format_wire_date(value: object | None, field: str) -> str:
    if value is None:
        _fail("reference_mismatch", field)
    year = getattr(value, "year", None)
    month = getattr(value, "month", None)
    day = getattr(value, "day", None)
    if not all(isinstance(part, int) and not isinstance(part, bool) for part in (year, month, day)):
        _fail("reference_mismatch", field)
    return f"{year:04d}-{month:02d}-{day:02d}"


def _write_period(value: HoldoutPeriodValue) -> str:
    snapshots = ",".join(f'"{snapshot}"' for snapshot in value.snapshot_ids)
    return (
        f'{{"schema":"{_PERIOD_SCHEMA}","sample":{{"role":"{value.sample.role}",'
        f'"start_inclusive":"{value.sample.start_inclusive}",'
        f'"end_inclusive":"{value.sample.end_inclusive}"}},'
        f'"snapshot_ids":[{snapshots}],'
        f'"snapshot_manifest_sha256":"{value.snapshot_manifest_sha256}"}}'
    )


def _write_plan(value: HoldoutEvaluationPlanValue) -> str:
    entries: list[str] = []
    for entry in value.entries:
        artifact = entry.backtest_spec_artifact
        budget = entry.job_budget
        entries.append(
            f'{{"entry_index":"{entry.entry_index}",'
            f'"factor_spec_id":"{entry.factor_spec_id}",'
            f'"backtest_spec_artifact":{{"artifact_id":"{artifact.artifact_id}",'
            f'"uri":"{artifact.uri}","sha256":"{artifact.sha256}",'
            f'"schema_name":"{artifact.schema_name}",'
            f'"schema_version":"{artifact.schema_version}",'
            f'"schema_sha256":"{artifact.schema_sha256}",'
            f'"media_type":"{artifact.media_type}","byte_size":"{artifact.byte_size}"}},'
            f'"job_budget":{{"maximum_steps":"{budget.maximum_steps}",'
            f'"maximum_input_tokens":"{budget.maximum_input_tokens}",'
            f'"maximum_output_tokens":"{budget.maximum_output_tokens}",'
            f'"maximum_cost":{{"amount":"{budget.maximum_cost.amount}",'
            f'"currency_code":"{budget.maximum_cost.currency_code}"}},'
            f'"maximum_wall_time_ns":"{budget.maximum_wall_time_ns}"' + "}}"
        )
    return (
        f'{{"schema":"{_PLAN_SCHEMA}",'
        f'"holdout_period_id":"{value.holdout_period_id}",'
        f'"canonical_period_sha256":"{value.canonical_period_sha256}",'
        f'"entries":[{",".join(entries)}]}}'
    )


def _domain_hash(domain: bytes, value: bytes) -> bytes:
    return hashlib.sha256(domain + value).digest()


def _encode_digest(value: bytes) -> str:
    _require_raw_digest(value, "digest")
    return f"sha256:{value.hex()}"


def _equal_digest(left: bytes, right: bytes) -> bool:
    return len(left) == 32 and len(right) == 32 and hmac.compare_digest(left, right)


def _fail(code: str, field: str) -> NoReturn:
    raise HoldoutValidationError(code, field)
