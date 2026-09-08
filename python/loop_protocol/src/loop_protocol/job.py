"""Fail-closed structural validation for durable job records."""

from __future__ import annotations

import hashlib
import hmac
import re
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from enum import StrEnum
from itertools import pairwise
from typing import NoReturn, Protocol

from loop.v1 import (
    backtest_pb2,
    common_pb2,
    data_pb2,
    development_data_pb2,
    factor_pb2,
    holdout_pb2,
    job_pb2,
    model_pb2,
    research_common_pb2,
)
from loop.v1.artifact_pb2 import ArtifactRef

from .artifact import ArtifactValidationError, validate_artifact_ref
from .holdout import (
    HoldoutJobBudget,
    HoldoutValidationError,
    parse_canonical_holdout_evaluation_plan,
    verify_holdout_period_identity,
)

_MAX_ID_BYTES = 128
_MAX_REASON_BYTES = 2_048
_MAX_ERROR_CODE_BYTES = 128
_MAX_ERROR_MESSAGE_BYTES = 2_048
_MAX_ERROR_FIELD_PATH_BYTES = 512
_MAX_ERROR_DETAILS = 32
_MAX_OUTCOME_ARTIFACTS = 64
_MAX_DATASET_SNAPSHOTS = 128
_MAX_PROTOCOL_FEATURES = 256
_MAX_PROTOCOL_NAME_BYTES = 128
_MAX_BUILD_VERSION_BYTES = 128
_MAX_ACTOR_DISPLAY_NAME_BYTES = 256
_MAX_AUTHENTICATED_SUBJECT_BYTES = 512
_MAX_JOB_STEPS = 1_000_000
_MAX_JOB_TOKENS = 1_000_000_000_000
_MAX_JOB_WALL_TIME_SECONDS = 604_800
_MAX_HOLDOUT_ENTRIES = 4_096
_MAX_CANDIDATES = 1_000_000
_MAX_PROTOCOL_UNARY_BYTES = 4_194_304
_MAX_PROTOCOL_STREAM_EVENT_BYTES = 1_048_576
_MAX_PROTOCOL_CANONICAL_AST_BYTES = 262_144
_MAX_PROTOCOL_AST_NODES = 4_096
_MAX_PROTOCOL_AST_DEPTH = 64
_MAX_PROTOCOL_AST_DIRECT_ARGUMENTS = 1_024
_MAX_PROTOCOL_PAGE_RECORDS = 500
_MAX_PROTOCOL_IDENTITY_BYTES = 128
_MAX_PROTOCOL_ARTIFACT_URI_BYTES = 2_048
_PROTOCOL_SELECTION_DOMAIN = b"loop.protocol-selection/v1"
_FACTOR_AST_DOMAIN = b"loop.factor-ast/v1"
_FACTOR_SPEC_DOMAIN = b"loop.factor-spec/v1"
_MIN_TIMESTAMP_SECONDS = -62_135_596_800
_MAX_TIMESTAMP_SECONDS = 253_402_300_799
_STABLE_ERROR_CODE_RE = re.compile(r"^[a-z][a-z0-9_.-]{0,127}$", re.ASCII)
_FACTOR_IDENTIFIER_RE = re.compile(r"^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$", re.ASCII)
_FACTOR_POSITIVE_INTEGER_RE = re.compile(r"^[1-9][0-9]{0,19}$", re.ASCII)
_FACTOR_DECIMAL_RE = re.compile(r"^-?(?:0|[1-9][0-9]*)(?:\.[0-9]*[1-9])?$", re.ASCII)

_JOB_INPUT_BY_KIND = {
    job_pb2.JOB_KIND_DISCOVERY: "discovery",
    job_pb2.JOB_KIND_FACTOR_EVALUATION: "factor_evaluation",
    job_pb2.JOB_KIND_BACKTEST: "backtest",
    job_pb2.JOB_KIND_INDEPENDENT_RECONCILIATION: "reconciliation",
    job_pb2.JOB_KIND_REPORT: "artifact",
    job_pb2.JOB_KIND_PROSPECTIVE_OBSERVATION: "artifact",
    job_pb2.JOB_KIND_HOLDOUT_BACKTEST: "holdout_backtest",
}
_TERMINAL_STATE_OUTCOME = {
    job_pb2.JOB_STATE_SUCCEEDED: "success",
    job_pb2.JOB_STATE_FACTOR_REJECTED: "factor_rejection",
    job_pb2.JOB_STATE_INFRASTRUCTURE_FAILED: "infrastructure_failure",
    job_pb2.JOB_STATE_CANCELLED: "cancellation",
    job_pb2.JOB_STATE_BUDGET_EXHAUSTED: "budget_exhaustion",
}
_ACTIVE_STATES = frozenset({job_pb2.JOB_STATE_LEASED, job_pb2.JOB_STATE_RUNNING})
_KNOWN_STATES = frozenset({job_pb2.JOB_STATE_QUEUED, *_ACTIVE_STATES, *_TERMINAL_STATE_OUTCOME})
_FACTOR_REJECTION_KINDS = frozenset(
    {
        job_pb2.JOB_KIND_FACTOR_EVALUATION,
        job_pb2.JOB_KIND_BACKTEST,
        job_pb2.JOB_KIND_HOLDOUT_BACKTEST,
    }
)
_FACTOR_REJECTION_CODES = frozenset(
    {
        job_pb2.FACTOR_REJECTION_CODE_DUPLICATE,
        job_pb2.FACTOR_REJECTION_CODE_PREVIOUSLY_FAILED,
        job_pb2.FACTOR_REJECTION_CODE_INSUFFICIENT_COVERAGE,
        job_pb2.FACTOR_REJECTION_CODE_DETERMINISTIC_FILTER,
        job_pb2.FACTOR_REJECTION_CODE_PERFORMANCE,
        job_pb2.FACTOR_REJECTION_CODE_CORRELATION,
        job_pb2.FACTOR_REJECTION_CODE_SEMANTIC_REVIEW,
        job_pb2.FACTOR_REJECTION_CODE_POLICY,
    }
)
_ERROR_CATEGORIES = frozenset(
    {
        common_pb2.ERROR_CATEGORY_VALIDATION,
        common_pb2.ERROR_CATEGORY_AUTHENTICATION,
        common_pb2.ERROR_CATEGORY_AUTHORIZATION,
        common_pb2.ERROR_CATEGORY_NOT_FOUND,
        common_pb2.ERROR_CATEGORY_CONFLICT,
        common_pb2.ERROR_CATEGORY_RATE_LIMIT,
        common_pb2.ERROR_CATEGORY_TIMEOUT,
        common_pb2.ERROR_CATEGORY_CANCELLED,
        common_pb2.ERROR_CATEGORY_DEPENDENCY,
        common_pb2.ERROR_CATEGORY_INTERNAL,
        common_pb2.ERROR_CATEGORY_BUDGET_EXHAUSTED,
    }
)
_ACTOR_KINDS = frozenset(
    {
        common_pb2.ACTOR_KIND_HUMAN,
        common_pb2.ACTOR_KIND_SERVICE,
        common_pb2.ACTOR_KIND_AGENT,
        common_pb2.ACTOR_KIND_SCHEDULER,
    }
)


class _Timestamp(Protocol):
    seconds: int
    nanos: int


class JobValidationCode(StrEnum):
    MISSING_FIELD = "missing_field"
    UNKNOWN_ENUM = "unknown_enum"
    KIND_INPUT_MISMATCH = "kind_input_mismatch"
    STATE_LEASE_MISMATCH = "state_lease_mismatch"
    STATE_OUTCOME_MISMATCH = "state_outcome_mismatch"
    REJECTION_NOT_ALLOWED = "rejection_not_allowed"
    LEASE_JOB_MISMATCH = "lease_job_mismatch"
    INVALID_ATTEMPT = "invalid_attempt"
    INVALID_REVISION = "invalid_revision"
    INVALID_IDENTITY = "invalid_identity"
    FACTOR_IDENTITY_MISMATCH = "factor_identity_mismatch"
    INVALID_LEASE = "invalid_lease"
    INVALID_TERMINAL_PAYLOAD = "invalid_terminal_payload"
    COLLECTION_LIMIT = "collection_limit"
    INVALID_ENVELOPE = "invalid_envelope"
    INVALID_BUDGET = "invalid_budget"
    INVALID_PROTOCOL_SELECTION = "invalid_protocol_selection"
    INVALID_INPUT = "invalid_input"
    INVALID_PROVENANCE = "invalid_provenance"
    BINDING_MISMATCH = "binding_mismatch"


class JobValidationError(ValueError):
    """A wire job record cannot be converted to a coherent domain record."""

    def __init__(self, code: JobValidationCode, field: str) -> None:
        self.code = code
        self.field = field
        super().__init__(f"{field} failed job validation ({code.value})")


@dataclass(frozen=True, slots=True)
class ValidatedJobSpecificationShape:
    kind: int


@dataclass(frozen=True, slots=True)
class ValidatedJobShape(ValidatedJobSpecificationShape):
    state: int


def validate_job_specification_shape(
    specification: job_pb2.JobSpecification,
) -> ValidatedJobSpecificationShape:
    """Classify only protocol v1 kind/input compatibility."""

    kind = specification.kind
    if kind not in _JOB_INPUT_BY_KIND:
        _fail(JobValidationCode.UNKNOWN_ENUM, "specification.kind")
    input_name = specification.WhichOneof("input")
    if input_name is None:
        _fail(JobValidationCode.MISSING_FIELD, "specification.input")
    if input_name != _JOB_INPUT_BY_KIND[kind]:
        _fail(JobValidationCode.KIND_INPUT_MISMATCH, "specification.input")
    return ValidatedJobSpecificationShape(kind=kind)


def validate_job_specification(
    specification: job_pb2.JobSpecification,
) -> ValidatedJobSpecificationShape:
    """Validate the wire envelope and every binding provable from inline fields.

    The wire boundary proves an attached AST and canonical JSON encode one exact
    tree. Registry resolution, AST typing, and normalization proof remain the
    factor domain binder's responsibility. Development dataset references are
    opaque identities here; Phase 4/5 server-owned snapshot registry and capability
    resolution must verify their roles before persistence or dispatch.
    """

    shape = validate_job_specification_shape(specification)
    submitted_at = _validate_job_envelope(specification)
    _validate_job_input(specification, submitted_at)
    return shape


def _validate_job_envelope(specification: job_pb2.JobSpecification) -> tuple[int, int]:
    _require_token_id(
        specification.job_id.value if specification.HasField("job_id") else None,
        "specification.job_id",
    )
    _require_token_id(
        specification.run_id.value if specification.HasField("run_id") else None,
        "specification.run_id",
    )
    submitted_at = _require_timestamp(
        specification.submitted_at if specification.HasField("submitted_at") else None,
        "specification.submitted_at",
    )
    _validate_actor(
        specification.submitted_by if specification.HasField("submitted_by") else None,
        "specification.submitted_by",
    )
    _require_token_id(
        specification.idempotency_key.value if specification.HasField("idempotency_key") else None,
        "specification.idempotency_key",
    )
    _require_token_id(
        specification.correlation_id.value if specification.HasField("correlation_id") else None,
        "specification.correlation_id",
    )
    _require_token_id(
        specification.causation_id.value if specification.HasField("causation_id") else None,
        "specification.causation_id",
    )
    if not specification.HasField("protocol_selection"):
        _fail(JobValidationCode.MISSING_FIELD, "specification.protocol_selection")
    _validate_protocol_selection(specification.protocol_selection, submitted_at)
    return submitted_at


def _validate_protocol_selection(
    selection: common_pb2.ProtocolSelectionSnapshot, submitted_at: tuple[int, int]
) -> None:
    expected = protocol_selection_sha256(selection)
    actual = _require_digest(
        selection.selection_sha256 if selection.HasField("selection_sha256") else None,
        "specification.protocol_selection.selection_sha256",
    )
    if not hmac.compare_digest(expected, actual):
        _fail(
            JobValidationCode.INVALID_PROTOCOL_SELECTION,
            "specification.protocol_selection.selection_sha256",
        )
    selected_at = _require_timestamp(
        selection.selected_at if selection.HasField("selected_at") else None,
        "specification.protocol_selection.selected_at",
    )
    if selected_at > submitted_at:
        _fail(
            JobValidationCode.INVALID_PROTOCOL_SELECTION,
            "specification.protocol_selection.selected_at",
        )


def canonical_protocol_selection_bytes(
    selection: common_pb2.ProtocolSelectionSnapshot,
) -> bytes:
    """Emit the normative canonical v1 protocol-selection document."""

    if not _is_protocol_package(selection.selected_package):
        _fail(
            JobValidationCode.INVALID_PROTOCOL_SELECTION,
            "specification.protocol_selection.selected_package",
        )
    features = list(selection.enabled_features)
    if (
        len(features) > _MAX_PROTOCOL_FEATURES
        or any(not _is_protocol_feature(feature) for feature in features)
        or any(left >= right for left, right in pairwise(features))
    ):
        _fail(
            JobValidationCode.INVALID_PROTOCOL_SELECTION,
            "specification.protocol_selection.enabled_features",
        )
    if not selection.HasField("effective_limits"):
        _fail(JobValidationCode.MISSING_FIELD, "specification.protocol_selection.effective_limits")
    limits = selection.effective_limits
    _validate_protocol_limits(limits)
    if not _is_build_version(selection.server_build_version) or not _is_build_version(
        selection.client_build_version
    ):
        _fail(
            JobValidationCode.INVALID_PROTOCOL_SELECTION,
            "specification.protocol_selection.build_version",
        )
    server_build = _require_digest(
        selection.server_build_sha256 if selection.HasField("server_build_sha256") else None,
        "specification.protocol_selection.server_build_sha256",
    )
    descriptor = _require_digest(
        selection.schema_descriptor_sha256
        if selection.HasField("schema_descriptor_sha256")
        else None,
        "specification.protocol_selection.schema_descriptor_sha256",
    )
    client_build = _require_digest(
        selection.client_build_sha256 if selection.HasField("client_build_sha256") else None,
        "specification.protocol_selection.client_build_sha256",
    )
    selected_at = _require_timestamp(
        selection.selected_at if selection.HasField("selected_at") else None,
        "specification.protocol_selection.selected_at",
    )
    feature_json = ",".join(f'"{feature}"' for feature in features)
    canonical = (
        '{"schema":"loop.protocol-selection/v1",'
        f'"selected_package":"{selection.selected_package}",'
        f'"enabled_features":[{feature_json}],'
        '"effective_limits":{'
        f'"maximum_unary_bytes":"{limits.maximum_unary_bytes}",'
        f'"maximum_stream_event_bytes":"{limits.maximum_stream_event_bytes}",'
        f'"maximum_canonical_ast_bytes":"{limits.maximum_canonical_ast_bytes}",'
        f'"maximum_ast_nodes":"{limits.maximum_ast_nodes}",'
        f'"maximum_ast_depth":"{limits.maximum_ast_depth}",'
        f'"maximum_page_records":"{limits.maximum_page_records}",'
        f'"maximum_identity_bytes":"{limits.maximum_identity_bytes}",'
        f'"maximum_artifact_uri_bytes":"{limits.maximum_artifact_uri_bytes}"}},'
        f'"server_build_version":"{selection.server_build_version}",'
        f'"server_build_sha256":"{_encode_digest(server_build)}",'
        f'"schema_descriptor_sha256":"{_encode_digest(descriptor)}",'
        f'"selected_at":{{"seconds":"{selected_at[0]}","nanos":"{selected_at[1]}"}},'
        f'"client_build_version":"{selection.client_build_version}",'
        f'"client_build_sha256":"{_encode_digest(client_build)}"}}'
    )
    return canonical.encode("ascii")


def protocol_selection_sha256(selection: common_pb2.ProtocolSelectionSnapshot) -> bytes:
    """Compute the domain-separated digest claimed by ``selection_sha256``."""

    return _domain_digest(_PROTOCOL_SELECTION_DOMAIN, canonical_protocol_selection_bytes(selection))


def _validate_protocol_limits(limits: common_pb2.ProtocolLimits) -> None:
    valid = (
        1 <= limits.maximum_unary_bytes <= _MAX_PROTOCOL_UNARY_BYTES
        and 1 <= limits.maximum_stream_event_bytes <= _MAX_PROTOCOL_STREAM_EVENT_BYTES
        and 1 <= limits.maximum_canonical_ast_bytes <= _MAX_PROTOCOL_CANONICAL_AST_BYTES
        and 1 <= limits.maximum_ast_nodes <= _MAX_PROTOCOL_AST_NODES
        and 1 <= limits.maximum_ast_depth <= _MAX_PROTOCOL_AST_DEPTH
        and 1 <= limits.maximum_page_records <= _MAX_PROTOCOL_PAGE_RECORDS
        and 1 <= limits.maximum_identity_bytes <= _MAX_PROTOCOL_IDENTITY_BYTES
        and 1 <= limits.maximum_artifact_uri_bytes <= _MAX_PROTOCOL_ARTIFACT_URI_BYTES
        and limits.maximum_canonical_ast_bytes <= limits.maximum_unary_bytes
        and limits.maximum_identity_bytes <= limits.maximum_unary_bytes
        and limits.maximum_artifact_uri_bytes <= limits.maximum_unary_bytes
    )
    if not valid:
        _fail(
            JobValidationCode.INVALID_PROTOCOL_SELECTION,
            "specification.protocol_selection.effective_limits",
        )


def _validate_job_input(
    specification: job_pb2.JobSpecification, submitted_at: tuple[int, int]
) -> None:
    input_name = specification.WhichOneof("input")
    if input_name == "discovery":
        value = specification.discovery
        _validate_development_dataset(value.dataset if value.HasField("dataset") else None)
        _validate_policy(
            value.research_policy if value.HasField("research_policy") else None,
            "specification.input.discovery.research_policy",
        )
        _validate_model_resolution(
            value.maker_model if value.HasField("maker_model") else None,
            "specification.input.discovery.maker_model",
            submitted_at,
        )
        _validate_model_resolution(
            value.checker_model if value.HasField("checker_model") else None,
            "specification.input.discovery.checker_model",
            submitted_at,
        )
        _validate_budget(
            value.budget if value.HasField("budget") else None,
            "specification.input.discovery.budget",
        )
        if not 1 <= value.maximum_candidates <= _MAX_CANDIDATES:
            _fail(
                JobValidationCode.INVALID_INPUT, "specification.input.discovery.maximum_candidates"
            )
    elif input_name == "factor_evaluation":
        value = specification.factor_evaluation
        if not value.HasField("factor"):
            _fail(JobValidationCode.MISSING_FIELD, "specification.input.factor_evaluation.factor")
        validate_factor_spec_identity_envelope(value.factor)
        _validate_development_dataset(value.dataset if value.HasField("dataset") else None)
        _validate_budget(
            value.budget if value.HasField("budget") else None,
            "specification.input.factor_evaluation.budget",
        )
    elif input_name == "backtest":
        value = specification.backtest
        _validate_budget(
            value.budget if value.HasField("budget") else None,
            "specification.input.backtest.budget",
        )
        _require_sha256_id(
            value.factor_spec_id.value if value.HasField("factor_spec_id") else None,
            "specification.input.backtest.factor_spec_id",
        )
        _validate_development_dataset(value.dataset if value.HasField("dataset") else None)
        _validate_simple_return(
            value.return_definition, "specification.input.backtest.return_definition"
        )
        _validate_provenance(
            value.provenance if value.HasField("provenance") else None,
            "specification.input.backtest.provenance",
        )
        _require_digest(
            value.deterministic_seed if value.HasField("deterministic_seed") else None,
            "specification.input.backtest.deterministic_seed",
        )
    elif input_name == "reconciliation":
        value = specification.reconciliation
        primary = _require_token_id(
            value.primary_backtest_id.value if value.HasField("primary_backtest_id") else None,
            "specification.input.reconciliation.primary_backtest_id",
        )
        independent = _require_token_id(
            value.independent_backtest_id.value
            if value.HasField("independent_backtest_id")
            else None,
            "specification.input.reconciliation.independent_backtest_id",
        )
        if primary == independent:
            _fail(
                JobValidationCode.BINDING_MISMATCH,
                "specification.input.reconciliation.backtest_ids",
            )
        _validate_policy(
            value.reconciliation_policy if value.HasField("reconciliation_policy") else None,
            "specification.input.reconciliation.reconciliation_policy",
        )
        _validate_budget(
            value.budget if value.HasField("budget") else None,
            "specification.input.reconciliation.budget",
        )
    elif input_name == "holdout_backtest":
        _validate_holdout_backtest_input(specification.holdout_backtest, submitted_at)
    elif input_name == "artifact":
        value = specification.artifact
        if not value.HasField("input"):
            _fail(JobValidationCode.MISSING_FIELD, "specification.input.artifact.input")
        try:
            validate_artifact_ref(value.input)
        except ArtifactValidationError as error:
            raise JobValidationError(
                JobValidationCode.INVALID_INPUT, "specification.input.artifact.input"
            ) from error
        _validate_policy(
            value.policy if value.HasField("policy") else None,
            "specification.input.artifact.policy",
        )
        _validate_budget(
            value.budget if value.HasField("budget") else None,
            "specification.input.artifact.budget",
        )
    else:
        _fail(JobValidationCode.MISSING_FIELD, "specification.input")


def _validate_development_dataset(
    dataset: development_data_pb2.DevelopmentDatasetReference | None,
) -> None:
    if dataset is None:
        _fail(JobValidationCode.MISSING_FIELD, "specification.input.dataset")
    snapshot_ids = dataset.snapshot_ids
    if not 1 <= len(snapshot_ids) <= _MAX_DATASET_SNAPSHOTS:
        _fail(JobValidationCode.INVALID_INPUT, "specification.input.dataset.snapshot_ids")
    previous: str | None = None
    for snapshot in snapshot_ids:
        value = _require_token_id(snapshot.value, "specification.input.dataset.snapshot_ids")
        if previous is not None and previous >= value:
            _fail(JobValidationCode.INVALID_INPUT, "specification.input.dataset.snapshot_ids")
        previous = value
    _require_digest(
        dataset.manifest_sha256 if dataset.HasField("manifest_sha256") else None,
        "specification.input.dataset.manifest_sha256",
    )


def _validate_policy(policy: common_pb2.PolicyReference | None, field: str) -> None:
    if policy is None:
        _fail(JobValidationCode.MISSING_FIELD, field)
    if not policy.HasField("policy_id"):
        _fail(JobValidationCode.MISSING_FIELD, field)
    if re.fullmatch(r"[a-z][a-z0-9_.-]{0,127}", policy.policy_id.value, re.ASCII) is None:
        _fail(JobValidationCode.INVALID_INPUT, field)
    if (
        re.fullmatch(r"[1-9][0-9]{0,19}", policy.revision, re.ASCII) is None
        or int(policy.revision) > 18_446_744_073_709_551_615
    ):
        _fail(JobValidationCode.INVALID_INPUT, field)
    _require_digest(policy.sha256 if policy.HasField("sha256") else None, field)


def canonical_factor_spec_identity_bytes(factor: factor_pb2.FactorSpec) -> bytes:
    """Emit identity bytes after proving wire AST and canonical JSON agree."""

    factor_expression_id = _require_sha256_id(
        factor.expression_id.value if factor.HasField("expression_id") else None,
        "specification.input.factor_evaluation.factor.expression_id",
    )
    if not factor.HasField("expression"):
        _fail(
            JobValidationCode.MISSING_FIELD,
            "specification.input.factor_evaluation.factor.expression",
        )
    expression = factor.expression
    attached_expression_id = _require_sha256_id(
        expression.expression_id.value if expression.HasField("expression_id") else None,
        "specification.input.factor_evaluation.factor.expression.expression_id",
    )
    if not hmac.compare_digest(factor_expression_id, attached_expression_id):
        _fail(
            JobValidationCode.BINDING_MISMATCH,
            "specification.input.factor_evaluation.factor.expression.expression_id",
        )
    if (
        expression.canonicalization_profile != "loop.factor-ast/v1"
        or not expression.canonical_json
        or len(expression.canonical_json) > _MAX_PROTOCOL_CANONICAL_AST_BYTES
    ):
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.factor_evaluation.factor.expression",
        )
    if (
        not expression.HasField("ast")
        or expression.ast.schema_version != 1
        or not expression.ast.HasField("root")
        or expression.ast.root.WhichOneof("node") is None
    ):
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.factor_evaluation.factor.expression.ast",
        )
    attached_ast = _canonical_wire_factor_ast_bytes(expression.ast)
    if not hmac.compare_digest(attached_ast, bytes(expression.canonical_json)):
        _fail(
            JobValidationCode.BINDING_MISMATCH,
            "specification.input.factor_evaluation.factor.expression.ast",
        )
    computed_expression_id = _encode_digest(
        _domain_digest(_FACTOR_AST_DOMAIN, bytes(expression.canonical_json))
    )
    if not hmac.compare_digest(factor_expression_id, computed_expression_id):
        _fail(
            JobValidationCode.BINDING_MISMATCH,
            "specification.input.factor_evaluation.factor.expression.canonical_json",
        )

    registry = _require_digest(
        factor.operator_registry_sha256 if factor.HasField("operator_registry_sha256") else None,
        "specification.input.factor_evaluation.factor.operator_registry_sha256",
    )
    if factor.direction == factor_pb2.FACTOR_DIRECTION_HIGHER_IS_BETTER:
        direction = "higher_is_better"
    elif factor.direction == factor_pb2.FACTOR_DIRECTION_LOWER_IS_BETTER:
        direction = "lower_is_better"
    elif factor.direction == factor_pb2.FACTOR_DIRECTION_UNSPECIFIED:
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.factor_evaluation.factor.direction",
        )
    else:
        _fail(
            JobValidationCode.UNKNOWN_ENUM,
            "specification.input.factor_evaluation.factor.direction",
        )
    if not factor.HasField("frozen_policy"):
        _fail(
            JobValidationCode.MISSING_FIELD,
            "specification.input.factor_evaluation.factor.frozen_policy",
        )
    policies = factor.frozen_policy
    canonical = (
        '{"schema":"loop.factor-spec/v1",'
        f'"expression_id":"{factor_expression_id}",'
        f'"operator_registry_sha256":"{_encode_digest(registry)}",'
        f'"direction":"{direction}"'
        + _write_factor_policy("universe_policy", policies, "universe_policy")
        + _write_factor_policy("data_policy", policies, "data_policy")
        + _write_factor_policy("calendar_policy", policies, "calendar_policy")
        + _write_factor_policy("preprocess_policy", policies, "preprocess_policy")
        + _write_factor_policy("neutralization_policy", policies, "neutralization_policy")
        + _write_factor_policy("portfolio_policy", policies, "portfolio_policy")
        + _write_factor_policy("execution_policy", policies, "execution_policy")
        + _write_factor_policy("cost_policy", policies, "cost_policy")
        + _write_factor_policy("evaluation_policy", policies, "evaluation_policy")
        + "}"
    )
    return canonical.encode("ascii")


def _canonical_wire_factor_ast_bytes(ast: factor_pb2.FactorAst) -> bytes:
    if ast.schema_version != 1 or not ast.HasField("root"):
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.factor_evaluation.factor.expression.ast",
        )
    output: list[str] = []
    node_count = [0]
    _write_wire_factor_ast_node(ast.root, 1, node_count, output)
    canonical = "".join(output).encode("ascii")
    if len(canonical) > _MAX_PROTOCOL_CANONICAL_AST_BYTES:
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.factor_evaluation.factor.expression.ast",
        )
    return canonical


def _write_wire_factor_ast_node(
    node: factor_pb2.FactorAstNode,
    depth: int,
    node_count: list[int],
    output: list[str],
) -> None:
    if depth > _MAX_PROTOCOL_AST_DEPTH:
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.factor_evaluation.factor.expression.ast",
        )
    node_count[0] += 1
    if node_count[0] > _MAX_PROTOCOL_AST_NODES:
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.factor_evaluation.factor.expression.ast",
        )

    node_kind = node.WhichOneof("node")
    if node_kind == "field":
        field = _require_factor_identifier(node.field.field)
        output.extend(('{"node":"field","field":"', field, '"}'))
        return
    if node_kind == "literal":
        literal_kind = node.literal.WhichOneof("value")
        if literal_kind == "decimal":
            decimal = node.literal.decimal.value
            if decimal == "-0" or _FACTOR_DECIMAL_RE.fullmatch(decimal) is None:
                _fail(
                    JobValidationCode.INVALID_INPUT,
                    "specification.input.factor_evaluation.factor.expression.ast.literal.decimal",
                )
            output.extend(('{"node":"decimal","value":"', decimal, '"}'))
            return
        if literal_kind == "boolean":
            output.append(
                '{"node":"boolean","value":true}'
                if node.literal.boolean
                else '{"node":"boolean","value":false}'
            )
            return
        if literal_kind == "enumeration":
            enum_type = _require_factor_identifier(node.literal.enumeration.enum_type)
            value = _require_factor_identifier(node.literal.enumeration.value)
            output.extend(('{"node":"enum","enum_type":"', enum_type, '","value":"', value, '"}'))
            return
        _fail(
            JobValidationCode.MISSING_FIELD,
            "specification.input.factor_evaluation.factor.expression.ast.literal",
        )
    if node_kind == "call":
        if not node.call.HasField("operator"):
            _fail(
                JobValidationCode.MISSING_FIELD,
                "specification.input.factor_evaluation.factor.expression.ast.call.operator",
            )
        operator = _require_factor_identifier(node.call.operator.operator)
        operator_version = node.call.operator.operator_version
        if (
            _FACTOR_POSITIVE_INTEGER_RE.fullmatch(operator_version) is None
            or int(operator_version) > 18_446_744_073_709_551_615
        ):
            _fail(
                JobValidationCode.INVALID_INPUT,
                "specification.input.factor_evaluation.factor.expression.ast.call.operator_version",
            )
        if len(node.call.arguments) > _MAX_PROTOCOL_AST_DIRECT_ARGUMENTS:
            _fail(
                JobValidationCode.INVALID_INPUT,
                "specification.input.factor_evaluation.factor.expression.ast.call.arguments",
            )
        output.extend(
            (
                '{"node":"call","operator":"',
                operator,
                '","operator_version":"',
                operator_version,
                '","arguments":[',
            )
        )
        for index, argument in enumerate(node.call.arguments):
            if index:
                output.append(",")
            _write_wire_factor_ast_node(argument, depth + 1, node_count, output)
        output.append("]}")
        return
    _fail(
        JobValidationCode.MISSING_FIELD,
        "specification.input.factor_evaluation.factor.expression.ast.node",
    )


def _require_factor_identifier(value: str) -> str:
    if (
        len(value.encode("ascii", errors="ignore")) > 128
        or _FACTOR_IDENTIFIER_RE.fullmatch(value) is None
    ):
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.factor_evaluation.factor.expression.ast",
        )
    return value


def factor_spec_identity_sha256(factor: factor_pb2.FactorSpec) -> bytes:
    """Compute the domain-separated digest claimed by ``factor_spec_id``."""

    return _domain_digest(_FACTOR_SPEC_DOMAIN, canonical_factor_spec_identity_bytes(factor))


def validate_factor_spec_identity_envelope(factor: factor_pb2.FactorSpec) -> None:
    """Validate both content-addressed identities in an inline factor."""

    claimed = _require_sha256_id(
        factor.factor_spec_id.value if factor.HasField("factor_spec_id") else None,
        "specification.input.factor_evaluation.factor.factor_spec_id",
    )
    computed = _encode_digest(factor_spec_identity_sha256(factor))
    if not hmac.compare_digest(claimed, computed):
        _fail(
            JobValidationCode.BINDING_MISMATCH,
            "specification.input.factor_evaluation.factor.factor_spec_id",
        )


def _write_factor_policy(
    output_name: str,
    policies: factor_pb2.FrozenResearchPolicyReference,
    field_name: str,
) -> str:
    if not policies.HasField(field_name):
        _fail(
            JobValidationCode.MISSING_FIELD,
            "specification.input.factor_evaluation.factor.frozen_policy",
        )
    policy = getattr(policies, field_name)
    _validate_policy(policy, "specification.input.factor_evaluation.factor.frozen_policy")
    digest = _require_digest(
        policy.sha256 if policy.HasField("sha256") else None,
        "specification.input.factor_evaluation.factor.frozen_policy",
    )
    return (
        f',"{output_name}":{{"policy_id":"{policy.policy_id.value}",'
        f'"revision":"{policy.revision}","sha256":"{_encode_digest(digest)}"}}'
    )


def _validate_model_resolution(
    model: model_pb2.ModelResolutionSnapshot | None,
    field: str,
    submitted_at: tuple[int, int],
) -> None:
    if model is None:
        _fail(JobValidationCode.MISSING_FIELD, field)
    _require_token_id(model.resolution_id.value if model.HasField("resolution_id") else None, field)
    _require_token_id(model.provider_id.value if model.HasField("provider_id") else None, field)
    _require_token_id(model.model_id.value if model.HasField("model_id") else None, field)
    if (
        not _bounded_text(model.requested_alias, _MAX_PROTOCOL_NAME_BYTES)
        or not _bounded_text(model.provider_plugin_name, _MAX_PROTOCOL_NAME_BYTES)
        or not _is_build_version(model.provider_plugin_version)
    ):
        _fail(JobValidationCode.INVALID_INPUT, field)
    if model.protocol_family not in {
        model_pb2.MODEL_PROTOCOL_FAMILY_OPENAI_RESPONSES,
        model_pb2.MODEL_PROTOCOL_FAMILY_OPENAI_CHAT_COMPLETIONS,
        model_pb2.MODEL_PROTOCOL_FAMILY_ANTHROPIC_MESSAGES,
        model_pb2.MODEL_PROTOCOL_FAMILY_GOOGLE_GENERATE_CONTENT,
        model_pb2.MODEL_PROTOCOL_FAMILY_GOOGLE_INTERACTIONS,
        model_pb2.MODEL_PROTOCOL_FAMILY_AWS_BEDROCK_CONVERSE,
        model_pb2.MODEL_PROTOCOL_FAMILY_COHERE_V2_CHAT,
    }:
        _fail(JobValidationCode.UNKNOWN_ENUM, field)
    if not model.HasField("capabilities"):
        _fail(JobValidationCode.MISSING_FIELD, field)
    capabilities = model.capabilities
    if (
        not 1 <= capabilities.context_window_tokens <= _MAX_JOB_TOKENS
        or not 1 <= capabilities.maximum_output_tokens <= capabilities.context_window_tokens
    ):
        _fail(JobValidationCode.INVALID_INPUT, field)
    if not model.HasField("pricing"):
        _fail(JobValidationCode.MISSING_FIELD, field)
    pricing = model.pricing
    _validate_money(
        pricing.input_per_million_tokens if pricing.HasField("input_per_million_tokens") else None,
        field,
    )
    _validate_money(
        pricing.output_per_million_tokens
        if pricing.HasField("output_per_million_tokens")
        else None,
        field,
    )
    _validate_money(
        pricing.cached_input_per_million_tokens
        if pricing.HasField("cached_input_per_million_tokens")
        else None,
        field,
    )
    for digest_name in (
        "capability_sha256",
        "catalog_sha256",
        "provider_plugin_sha256",
        "snapshot_sha256",
    ):
        _require_digest(
            getattr(model, digest_name) if model.HasField(digest_name) else None,
            field,
        )
    resolved_at = _require_timestamp(
        model.resolved_at if model.HasField("resolved_at") else None, field
    )
    if resolved_at > submitted_at:
        _fail(JobValidationCode.INVALID_INPUT, field)


def _validate_budget(budget: job_pb2.JobBudget | None, field: str) -> None:
    if budget is None:
        _fail(JobValidationCode.MISSING_FIELD, field)
    if (
        not 1 <= budget.maximum_steps <= _MAX_JOB_STEPS
        or budget.maximum_input_tokens > _MAX_JOB_TOKENS
        or budget.maximum_output_tokens > _MAX_JOB_TOKENS
    ):
        _fail(JobValidationCode.INVALID_BUDGET, field)
    _validate_money(budget.maximum_cost if budget.HasField("maximum_cost") else None, field)
    if not budget.HasField("maximum_wall_time"):
        _fail(JobValidationCode.MISSING_FIELD, field)
    duration = budget.maximum_wall_time
    if (
        duration.seconds < 0
        or duration.seconds > _MAX_JOB_WALL_TIME_SECONDS
        or duration.nanos < 0
        or duration.nanos >= 1_000_000_000
        or (duration.seconds == 0 and duration.nanos == 0)
        or (duration.seconds == _MAX_JOB_WALL_TIME_SECONDS and duration.nanos != 0)
    ):
        _fail(JobValidationCode.INVALID_BUDGET, field)


def _validate_money(money: common_pb2.Money | None, field: str) -> None:
    if money is None or not money.HasField("amount"):
        _fail(JobValidationCode.MISSING_FIELD, field)
    if re.fullmatch(r"[A-Z]{3}", money.currency_code, re.ASCII) is None or not _is_normalized_cost(
        money.amount.value
    ):
        _fail(JobValidationCode.INVALID_BUDGET, field)


def _is_normalized_cost(amount: str) -> bool:
    match = re.fullmatch(r"(0|[1-9][0-9]*)(?:\.([0-9]*[1-9]))?", amount, re.ASCII)
    if match is None:
        return False
    integer, fraction = match.group(1), match.group(2)
    if fraction is not None and len(fraction) > 9:
        return False
    significant = (
        max(len((fraction or "").lstrip("0")), 1)
        if integer == "0"
        else len(integer) + len(fraction or "")
    )
    return (
        significant <= 18
        and int(integer) <= 1_000_000
        and not (integer == "1000000" and fraction is not None)
    )


def _validate_provenance(
    provenance: research_common_pb2.ResearchProvenanceFingerprint | None, field: str
) -> None:
    if provenance is None:
        _fail(JobValidationCode.MISSING_FIELD, field)
    for digest_name in (
        "source_code_sha256",
        "operator_registry_sha256",
        "configuration_sha256",
        "data_manifest_sha256",
        "trading_calendar_sha256",
        "environment_sha256",
    ):
        if (
            not provenance.HasField(digest_name)
            or len(getattr(provenance, digest_name).value) != 32
        ):
            _fail(JobValidationCode.INVALID_PROVENANCE, field)


def _validate_holdout_backtest_input(
    input_value: job_pb2.HoldoutBacktestJobInput,
    submitted_at: tuple[int, int],
) -> None:
    if not input_value.HasField("consumed_grant"):
        _fail(
            JobValidationCode.MISSING_FIELD,
            "specification.input.holdout_backtest.consumed_grant",
        )
    grant = input_value.consumed_grant
    grant_issued_at, grant_expires_at = _validate_holdout_grant(grant)
    if submitted_at < grant_issued_at or submitted_at >= grant_expires_at:
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.holdout_backtest.consumed_grant.validity_window",
        )
    if input_value.consumed_grant_revision == 0:
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.holdout_backtest.consumed_grant_revision",
        )
    batch_id = _require_token_id(
        input_value.job_batch_id.value if input_value.HasField("job_batch_id") else None,
        "specification.input.holdout_backtest.job_batch_id",
    )
    if batch_id == grant.holdout_grant_id.value:
        _fail(
            JobValidationCode.BINDING_MISMATCH, "specification.input.holdout_backtest.job_batch_id"
        )
    plan_id = _require_sha256_id(
        input_value.holdout_evaluation_plan_id.value
        if input_value.HasField("holdout_evaluation_plan_id")
        else None,
        "specification.input.holdout_backtest.holdout_evaluation_plan_id",
    )
    grant_plan_id = _require_sha256_id(
        grant.holdout_evaluation_plan_id.value
        if grant.HasField("holdout_evaluation_plan_id")
        else None,
        "specification.input.holdout_backtest.consumed_grant.holdout_evaluation_plan_id",
    )
    if plan_id != grant_plan_id:
        _fail(
            JobValidationCode.BINDING_MISMATCH,
            "specification.input.holdout_backtest.holdout_evaluation_plan_id",
        )
    plan_digest = _require_digest(
        input_value.evaluation_plan_sha256
        if input_value.HasField("evaluation_plan_sha256")
        else None,
        "specification.input.holdout_backtest.evaluation_plan_sha256",
    )
    grant_plan_digest = _require_digest(
        grant.evaluation_plan_sha256 if grant.HasField("evaluation_plan_sha256") else None,
        "specification.input.holdout_backtest.consumed_grant.evaluation_plan_sha256",
    )
    if not hmac.compare_digest(plan_digest, grant_plan_digest):
        _fail(
            JobValidationCode.BINDING_MISMATCH,
            "specification.input.holdout_backtest.evaluation_plan_sha256",
        )
    if not 1 <= input_value.evaluation_plan_entry_index <= grant.evaluation_plan_entry_count:
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.holdout_backtest.evaluation_plan_entry_index",
        )
    if not input_value.HasField("frozen_backtest_spec"):
        _fail(
            JobValidationCode.MISSING_FIELD,
            "specification.input.holdout_backtest.frozen_backtest_spec",
        )
    _validate_frozen_backtest_spec(input_value.frozen_backtest_spec, grant)
    _validate_budget(
        input_value.budget if input_value.HasField("budget") else None,
        "specification.input.holdout_backtest.budget",
    )


def validate_holdout_backtest_plan_entry_binding(
    input_value: job_pb2.HoldoutBacktestJobInput,
    submitted_at: _Timestamp,
    canonical_period_bytes: bytes | str,
    canonical_plan_bytes: bytes | str,
    trusted_backtest_schema_sha256: bytes,
    resolved_backtest_artifacts: Mapping[str, bytes],
) -> None:
    """Bind a holdout job to freshly verified canonical period and plan bytes.

    Both documents are reparsed on every call, so mutable or caller-constructed
    canonical DTOs cannot cross this boundary. The bytes, trusted schema digest,
    and artifact map must come from a server-owned resolver, never the request or
    worker. This binds only factor ID, budget, and the canonical-spec digest. The
    Phase 7 owning parser must use the exact referenced artifact bytes to derive
    or validate and bind every frozen BacktestSpec field, including sample,
    snapshots, return definition, provenance, and seed. Persisted grant
    resolution and runtime authorization remain external Phase 4 gates.
    """

    validated_submitted_at = _require_timestamp(submitted_at, "specification.submitted_at")
    _validate_holdout_backtest_input(input_value, validated_submitted_at)
    grant = input_value.consumed_grant
    frozen = input_value.frozen_backtest_spec
    budget = input_value.budget
    plan_id = _require_sha256_id(
        input_value.holdout_evaluation_plan_id.value,
        "specification.input.holdout_backtest.holdout_evaluation_plan_id",
    )
    plan_digest = _require_digest(
        input_value.evaluation_plan_sha256,
        "specification.input.holdout_backtest.evaluation_plan_sha256",
    )
    period_id = _require_sha256_id(
        grant.holdout_period_id.value,
        "specification.input.holdout_backtest.consumed_grant.holdout_period_id",
    )
    period_digest = _require_digest(
        grant.canonical_period_sha256,
        "specification.input.holdout_backtest.consumed_grant.canonical_period_sha256",
    )
    try:
        period = verify_holdout_period_identity(canonical_period_bytes, period_id, period_digest)
        plan = parse_canonical_holdout_evaluation_plan(
            canonical_plan_bytes,
            period,
            trusted_backtest_schema_sha256,
            resolved_backtest_artifacts,
        )
    except HoldoutValidationError:
        _fail(
            JobValidationCode.BINDING_MISMATCH,
            "specification.input.holdout_backtest.resolved_plan",
        )
    if (
        not hmac.compare_digest(plan_id, plan.holdout_evaluation_plan_id)
        or not hmac.compare_digest(plan_digest, plan.plan_sha256)
        or not hmac.compare_digest(period_id, plan.value.holdout_period_id)
        or not hmac.compare_digest(
            _encode_digest(period_digest), plan.value.canonical_period_sha256
        )
        or grant.evaluation_plan_entry_count != len(plan.value.entries)
    ):
        _fail(
            JobValidationCode.BINDING_MISMATCH,
            "specification.input.holdout_backtest.resolved_plan",
        )
    entry = next(
        (
            candidate
            for candidate in plan.value.entries
            if candidate.entry_index == str(input_value.evaluation_plan_entry_index)
        ),
        None,
    )
    if entry is None:
        _fail(
            JobValidationCode.BINDING_MISMATCH,
            "specification.input.holdout_backtest.evaluation_plan_entry_index",
        )
    factor_id = _require_sha256_id(
        frozen.factor_spec_id.value,
        "specification.input.holdout_backtest.frozen_backtest_spec.factor_spec_id",
    )
    canonical_spec = _require_digest(
        frozen.canonical_spec_sha256,
        "specification.input.holdout_backtest.frozen_backtest_spec.canonical_spec_sha256",
    )
    if (
        not hmac.compare_digest(factor_id, entry.factor_spec_id)
        or not hmac.compare_digest(
            _encode_digest(canonical_spec), entry.backtest_spec_artifact.sha256
        )
        or not _holdout_budget_matches_plan(budget, entry.job_budget)
    ):
        _fail(
            JobValidationCode.BINDING_MISMATCH,
            "specification.input.holdout_backtest.resolved_plan_entry",
        )


def _holdout_budget_matches_plan(budget: job_pb2.JobBudget, expected: HoldoutJobBudget) -> bool:
    if not budget.HasField("maximum_cost") or not budget.maximum_cost.HasField("amount"):
        return False
    if not budget.HasField("maximum_wall_time"):
        return False
    wall = budget.maximum_wall_time
    wall_nanoseconds = wall.seconds * 1_000_000_000 + wall.nanos
    return (
        str(budget.maximum_steps) == expected.maximum_steps
        and str(budget.maximum_input_tokens) == expected.maximum_input_tokens
        and str(budget.maximum_output_tokens) == expected.maximum_output_tokens
        and budget.maximum_cost.amount.value == expected.maximum_cost.amount
        and budget.maximum_cost.currency_code == expected.maximum_cost.currency_code
        and str(wall_nanoseconds) == expected.maximum_wall_time_ns
    )


def _validate_holdout_grant(
    grant: holdout_pb2.HoldoutGrantReference,
) -> tuple[tuple[int, int], tuple[int, int]]:
    _require_token_id(
        grant.holdout_grant_id.value if grant.HasField("holdout_grant_id") else None,
        "specification.input.holdout_backtest.consumed_grant.holdout_grant_id",
    )
    period_id = _require_sha256_id(
        grant.holdout_period_id.value if grant.HasField("holdout_period_id") else None,
        "specification.input.holdout_backtest.consumed_grant.holdout_period_id",
    )
    _require_digest(
        grant.freeze_manifest_sha256 if grant.HasField("freeze_manifest_sha256") else None,
        "specification.input.holdout_backtest.consumed_grant.freeze_manifest_sha256",
    )
    issued_at = _require_timestamp(
        grant.issued_at if grant.HasField("issued_at") else None,
        "specification.input.holdout_backtest.consumed_grant.issued_at",
    )
    expires_at = _require_timestamp(
        grant.expires_at if grant.HasField("expires_at") else None,
        "specification.input.holdout_backtest.consumed_grant.expires_at",
    )
    if issued_at >= expires_at:
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.holdout_backtest.consumed_grant.expires_at",
        )
    _require_sha256_id(
        grant.holdout_evaluation_plan_id.value
        if grant.HasField("holdout_evaluation_plan_id")
        else None,
        "specification.input.holdout_backtest.consumed_grant.holdout_evaluation_plan_id",
    )
    _require_digest(
        grant.evaluation_plan_sha256 if grant.HasField("evaluation_plan_sha256") else None,
        "specification.input.holdout_backtest.consumed_grant.evaluation_plan_sha256",
    )
    if not 1 <= grant.evaluation_plan_entry_count <= _MAX_HOLDOUT_ENTRIES:
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.holdout_backtest.consumed_grant.evaluation_plan_entry_count",
        )
    period_digest = _require_digest(
        grant.canonical_period_sha256 if grant.HasField("canonical_period_sha256") else None,
        "specification.input.holdout_backtest.consumed_grant.canonical_period_sha256",
    )
    if period_id != _encode_digest(period_digest):
        _fail(
            JobValidationCode.BINDING_MISMATCH,
            "specification.input.holdout_backtest.consumed_grant.canonical_period_sha256",
        )
    return issued_at, expires_at


def _validate_frozen_backtest_spec(
    backtest: backtest_pb2.BacktestSpec, grant: holdout_pb2.HoldoutGrantReference
) -> None:
    _require_token_id(
        backtest.backtest_id.value if backtest.HasField("backtest_id") else None,
        "specification.input.holdout_backtest.frozen_backtest_spec.backtest_id",
    )
    if backtest.schema_version != 1:
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.holdout_backtest.frozen_backtest_spec.schema_version",
        )
    _require_sha256_id(
        backtest.factor_spec_id.value if backtest.HasField("factor_spec_id") else None,
        "specification.input.holdout_backtest.frozen_backtest_spec.factor_spec_id",
    )
    if not 1 <= len(backtest.snapshot_ids) <= _MAX_DATASET_SNAPSHOTS:
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.holdout_backtest.frozen_backtest_spec.snapshot_ids",
        )
    previous: str | None = None
    for snapshot in backtest.snapshot_ids:
        value = _require_sha256_id(
            snapshot.value,
            "specification.input.holdout_backtest.frozen_backtest_spec.snapshot_ids",
        )
        if previous is not None and previous >= value:
            _fail(
                JobValidationCode.INVALID_INPUT,
                "specification.input.holdout_backtest.frozen_backtest_spec.snapshot_ids",
            )
        previous = value
    _validate_locked_sample(backtest.sample if backtest.HasField("sample") else None)
    _validate_simple_return(
        backtest.return_definition,
        "specification.input.holdout_backtest.frozen_backtest_spec.return_definition",
    )
    _validate_provenance(
        backtest.provenance if backtest.HasField("provenance") else None,
        "specification.input.holdout_backtest.frozen_backtest_spec.provenance",
    )
    _require_digest(
        backtest.canonical_spec_sha256 if backtest.HasField("canonical_spec_sha256") else None,
        "specification.input.holdout_backtest.frozen_backtest_spec.canonical_spec_sha256",
    )
    _require_digest(
        backtest.deterministic_seed if backtest.HasField("deterministic_seed") else None,
        "specification.input.holdout_backtest.frozen_backtest_spec.deterministic_seed",
    )
    created_at = _require_timestamp(
        backtest.created_at if backtest.HasField("created_at") else None,
        "specification.input.holdout_backtest.frozen_backtest_spec.created_at",
    )
    grant_issued_at = _require_timestamp(
        grant.issued_at if grant.HasField("issued_at") else None,
        "specification.input.holdout_backtest.consumed_grant.issued_at",
    )
    if created_at > grant_issued_at:
        _fail(
            JobValidationCode.BINDING_MISMATCH,
            "specification.input.holdout_backtest.frozen_backtest_spec.created_at",
        )


def _validate_locked_sample(sample: data_pb2.SampleWindow | None) -> None:
    if sample is None:
        _fail(
            JobValidationCode.MISSING_FIELD,
            "specification.input.holdout_backtest.frozen_backtest_spec.sample",
        )
    if sample.role not in {
        data_pb2.SAMPLE_ROLE_UNSPECIFIED,
        data_pb2.SAMPLE_ROLE_OPERATOR_WARMUP,
        data_pb2.SAMPLE_ROLE_IN_SAMPLE,
        data_pb2.SAMPLE_ROLE_DEVELOPMENT_VALIDATION,
        data_pb2.SAMPLE_ROLE_FIRST_LOCKED_CONFIRMATION,
        data_pb2.SAMPLE_ROLE_SECOND_LOCKED_HISTORICAL_HOLDOUT,
        data_pb2.SAMPLE_ROLE_TEMPORAL_ISOLATION,
        data_pb2.SAMPLE_ROLE_PROSPECTIVE_OBSERVATION,
    }:
        _fail(
            JobValidationCode.UNKNOWN_ENUM,
            "specification.input.holdout_backtest.frozen_backtest_spec.sample.role",
        )
    if sample.role not in {
        data_pb2.SAMPLE_ROLE_FIRST_LOCKED_CONFIRMATION,
        data_pb2.SAMPLE_ROLE_SECOND_LOCKED_HISTORICAL_HOLDOUT,
    }:
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.holdout_backtest.frozen_backtest_spec.sample.role",
        )
    start = _validate_civil_date(
        sample.start_inclusive if sample.HasField("start_inclusive") else None,
        "specification.input.holdout_backtest.frozen_backtest_spec.sample.start_inclusive",
    )
    end = _validate_civil_date(
        sample.end_inclusive if sample.HasField("end_inclusive") else None,
        "specification.input.holdout_backtest.frozen_backtest_spec.sample.end_inclusive",
    )
    if start > end:
        _fail(
            JobValidationCode.INVALID_INPUT,
            "specification.input.holdout_backtest.frozen_backtest_spec.sample",
        )


def _validate_civil_date(date: common_pb2.CivilDate | None, field: str) -> tuple[int, int, int]:
    if date is None:
        _fail(JobValidationCode.MISSING_FIELD, field)
    leap = date.year % 4 == 0 and (date.year % 100 != 0 or date.year % 400 == 0)
    maximum_day = {
        1: 31,
        2: 29 if leap else 28,
        3: 31,
        4: 30,
        5: 31,
        6: 30,
        7: 31,
        8: 31,
        9: 30,
        10: 31,
        11: 30,
        12: 31,
    }.get(date.month)
    if not 1 <= date.year <= 9_999 or maximum_day is None or not 1 <= date.day <= maximum_day:
        _fail(JobValidationCode.INVALID_INPUT, field)
    return date.year, date.month, date.day


def _validate_simple_return(value: int, field: str) -> None:
    if value not in {
        research_common_pb2.RETURN_DEFINITION_UNSPECIFIED,
        research_common_pb2.RETURN_DEFINITION_SIMPLE_NAV_RETURN,
    }:
        _fail(JobValidationCode.UNKNOWN_ENUM, field)
    if value != research_common_pb2.RETURN_DEFINITION_SIMPLE_NAV_RETURN:
        _fail(JobValidationCode.INVALID_INPUT, field)


def validate_job_record(record: job_pb2.JobRecord) -> ValidatedJobShape:
    """Validate kind/input, state/lease/outcome, and terminal bindings."""

    if not record.HasField("specification"):
        _fail(JobValidationCode.MISSING_FIELD, "specification")
    if record.revision == 0:
        _fail(JobValidationCode.INVALID_REVISION, "revision")
    specification = record.specification
    kind = validate_job_specification(specification).kind
    specification_job_id = _require_token_id(
        specification.job_id.value if specification.HasField("job_id") else None,
        "specification.job_id",
    )
    submitted_at = _require_timestamp(
        specification.submitted_at if specification.HasField("submitted_at") else None,
        "specification.submitted_at",
    )
    updated_at = _require_timestamp(
        record.updated_at if record.HasField("updated_at") else None, "updated_at"
    )
    if updated_at < submitted_at:
        _fail(JobValidationCode.INVALID_ENVELOPE, "updated_at")

    state = record.state
    if state not in _KNOWN_STATES:
        _fail(JobValidationCode.UNKNOWN_ENUM, "state")
    has_lease = record.HasField("active_lease")
    if has_lease != (state in _ACTIVE_STATES):
        _fail(JobValidationCode.STATE_LEASE_MISMATCH, "active_lease")
    if state == job_pb2.JOB_STATE_QUEUED:
        if record.attempt != 0:
            _fail(JobValidationCode.INVALID_ATTEMPT, "attempt")
    elif record.attempt == 0 and state not in (
        job_pb2.JOB_STATE_CANCELLED,
        job_pb2.JOB_STATE_BUDGET_EXHAUSTED,
    ):
        _fail(JobValidationCode.INVALID_ATTEMPT, "attempt")

    outcome_name: str | None = None
    if record.HasField("outcome"):
        outcome_name = record.outcome.WhichOneof("outcome")
        if outcome_name is None:
            _fail(JobValidationCode.MISSING_FIELD, "outcome.outcome")
    if state == job_pb2.JOB_STATE_QUEUED or state in _ACTIVE_STATES:
        if outcome_name is not None:
            _fail(JobValidationCode.STATE_OUTCOME_MISMATCH, "outcome")
    elif _TERMINAL_STATE_OUTCOME[state] != outcome_name:
        _fail(JobValidationCode.STATE_OUTCOME_MISMATCH, "outcome")

    if outcome_name is not None:
        _validate_terminal_payload(record, specification, kind, outcome_name)
    if has_lease:
        _validate_lease(
            record.active_lease,
            specification_job_id,
            record.revision,
            submitted_at,
            updated_at,
        )
    return ValidatedJobShape(kind=kind, state=state)


def _validate_terminal_payload(
    record: job_pb2.JobRecord,
    specification: job_pb2.JobSpecification,
    kind: int,
    outcome_name: str,
) -> None:
    submitted_at = _require_timestamp(
        specification.submitted_at if specification.HasField("submitted_at") else None,
        "specification.submitted_at",
    )
    updated_at = _require_timestamp(
        record.updated_at if record.HasField("updated_at") else None, "updated_at"
    )
    if outcome_name == "success":
        _validate_artifacts(record.outcome.success.outputs, "outcome.success.outputs")
    elif outcome_name == "factor_rejection":
        _validate_factor_rejection(specification, kind, record.outcome.factor_rejection)
        _validate_event_timestamp(
            _require_timestamp(
                record.outcome.factor_rejection.rejected_at,
                "outcome.factor_rejection.rejected_at",
            ),
            submitted_at,
            updated_at,
            "outcome.factor_rejection.rejected_at",
        )
    elif outcome_name == "infrastructure_failure":
        failure = record.outcome.infrastructure_failure
        if not failure.HasField("error"):
            _fail(JobValidationCode.MISSING_FIELD, "outcome.infrastructure_failure.error")
        error = failure.error
        validate_service_error(error)
        if failure.attempt == 0 or failure.attempt != record.attempt:
            _fail(
                JobValidationCode.INVALID_TERMINAL_PAYLOAD,
                "outcome.infrastructure_failure.attempt",
            )
        _require_timestamp(
            failure.failed_at if failure.HasField("failed_at") else None,
            "outcome.infrastructure_failure.failed_at",
        )
        _validate_event_timestamp(
            _require_timestamp(failure.failed_at, "outcome.infrastructure_failure.failed_at"),
            submitted_at,
            updated_at,
            "outcome.infrastructure_failure.failed_at",
        )
    elif outcome_name == "cancellation":
        cancellation = record.outcome.cancellation
        if not _bounded_text(cancellation.reason, _MAX_REASON_BYTES):
            _fail(JobValidationCode.INVALID_TERMINAL_PAYLOAD, "outcome.cancellation.reason")
        _validate_actor(
            cancellation.cancelled_by if cancellation.HasField("cancelled_by") else None,
            "outcome.cancellation.cancelled_by",
        )
        _require_timestamp(
            cancellation.cancelled_at if cancellation.HasField("cancelled_at") else None,
            "outcome.cancellation.cancelled_at",
        )
        _validate_event_timestamp(
            _require_timestamp(cancellation.cancelled_at, "outcome.cancellation.cancelled_at"),
            submitted_at,
            updated_at,
            "outcome.cancellation.cancelled_at",
        )
    elif outcome_name == "budget_exhaustion":
        exhaustion = record.outcome.budget_exhaustion
        if not _bounded_text(exhaustion.exhausted_limit, _MAX_ERROR_CODE_BYTES):
            _fail(
                JobValidationCode.INVALID_TERMINAL_PAYLOAD,
                "outcome.budget_exhaustion.exhausted_limit",
            )
        _validate_budget(
            exhaustion.enforced_budget if exhaustion.HasField("enforced_budget") else None,
            "outcome.budget_exhaustion.enforced_budget",
        )
        if exhaustion.enforced_budget != _job_budget(specification):
            _fail(JobValidationCode.BINDING_MISMATCH, "outcome.budget_exhaustion.enforced_budget")
        exhausted_at = _require_timestamp(
            exhaustion.exhausted_at if exhaustion.HasField("exhausted_at") else None,
            "outcome.budget_exhaustion.exhausted_at",
        )
        _validate_event_timestamp(
            exhausted_at,
            submitted_at,
            updated_at,
            "outcome.budget_exhaustion.exhausted_at",
        )


def _job_budget(specification: job_pb2.JobSpecification) -> job_pb2.JobBudget:
    input_name = specification.WhichOneof("input")
    if input_name == "discovery":
        budget = specification.discovery.budget
        present = specification.discovery.HasField("budget")
    elif input_name == "factor_evaluation":
        budget = specification.factor_evaluation.budget
        present = specification.factor_evaluation.HasField("budget")
    elif input_name == "backtest":
        budget = specification.backtest.budget
        present = specification.backtest.HasField("budget")
    elif input_name == "reconciliation":
        budget = specification.reconciliation.budget
        present = specification.reconciliation.HasField("budget")
    elif input_name == "holdout_backtest":
        budget = specification.holdout_backtest.budget
        present = specification.holdout_backtest.HasField("budget")
    elif input_name == "artifact":
        budget = specification.artifact.budget
        present = specification.artifact.HasField("budget")
    else:
        _fail(JobValidationCode.MISSING_FIELD, "specification.input")
    if not present:
        _fail(JobValidationCode.MISSING_FIELD, "specification.input.budget")
    return budget


def _validate_event_timestamp(
    event_at: tuple[int, int],
    submitted_at: tuple[int, int],
    updated_at: tuple[int, int],
    field: str,
) -> None:
    if event_at < submitted_at or event_at > updated_at:
        _fail(JobValidationCode.INVALID_ENVELOPE, field)


def _validate_factor_rejection(
    specification: job_pb2.JobSpecification,
    kind: int,
    rejection: job_pb2.FactorRejection,
) -> None:
    if kind not in _FACTOR_REJECTION_KINDS:
        _fail(JobValidationCode.REJECTION_NOT_ALLOWED, "outcome.factor_rejection")
    expected = _factor_id_from_input(specification)
    actual = _require_sha256_id(
        rejection.factor_spec_id.value if rejection.HasField("factor_spec_id") else None,
        "outcome.factor_rejection.factor_spec_id",
    )
    if expected != actual:
        _fail(JobValidationCode.FACTOR_IDENTITY_MISMATCH, "outcome.factor_rejection.factor_spec_id")
    if rejection.code not in _FACTOR_REJECTION_CODES:
        _fail(JobValidationCode.UNKNOWN_ENUM, "outcome.factor_rejection.code")
    if not _bounded_text(rejection.reason, _MAX_REASON_BYTES):
        _fail(JobValidationCode.INVALID_TERMINAL_PAYLOAD, "outcome.factor_rejection.reason")
    _require_timestamp(
        rejection.rejected_at if rejection.HasField("rejected_at") else None,
        "outcome.factor_rejection.rejected_at",
    )
    _validate_artifacts(rejection.evidence, "outcome.factor_rejection.evidence")


def _factor_id_from_input(specification: job_pb2.JobSpecification) -> str:
    input_name = specification.WhichOneof("input")
    value: str | None = None
    if input_name == "factor_evaluation" and specification.factor_evaluation.HasField("factor"):
        factor = specification.factor_evaluation.factor
        value = factor.factor_spec_id.value if factor.HasField("factor_spec_id") else None
    elif input_name == "backtest":
        backtest = specification.backtest
        value = backtest.factor_spec_id.value if backtest.HasField("factor_spec_id") else None
    elif input_name == "holdout_backtest" and specification.holdout_backtest.HasField(
        "frozen_backtest_spec"
    ):
        backtest = specification.holdout_backtest.frozen_backtest_spec
        value = backtest.factor_spec_id.value if backtest.HasField("factor_spec_id") else None
    return _require_sha256_id(value, "specification.input.factor_spec_id")


def _validate_lease(
    lease: job_pb2.JobLease,
    job_id: str,
    revision: int,
    submitted_at: tuple[int, int],
    record_updated_at: tuple[int, int],
) -> None:
    _require_token_id(
        lease.lease_id.value if lease.HasField("lease_id") else None,
        "active_lease.lease_id",
    )
    lease_job_id = _require_token_id(
        lease.job_id.value if lease.HasField("job_id") else None,
        "active_lease.job_id",
    )
    if lease_job_id != job_id:
        _fail(JobValidationCode.LEASE_JOB_MISMATCH, "active_lease.job_id")
    _validate_actor(lease.owner if lease.HasField("owner") else None, "active_lease.owner")
    if not 1 <= lease.acquired_revision <= revision:
        _fail(JobValidationCode.INVALID_REVISION, "active_lease.acquired_revision")
    issued = _require_timestamp(
        lease.issued_at if lease.HasField("issued_at") else None, "active_lease.issued_at"
    )
    heartbeat = _require_timestamp(
        lease.heartbeat_at if lease.HasField("heartbeat_at") else None,
        "active_lease.heartbeat_at",
    )
    expires = _require_timestamp(
        lease.expires_at if lease.HasField("expires_at") else None, "active_lease.expires_at"
    )
    if (
        issued < submitted_at
        or issued > heartbeat
        or heartbeat > record_updated_at
        or record_updated_at >= expires
    ):
        _fail(JobValidationCode.INVALID_LEASE, "active_lease.timestamps")


def _validate_actor(actor: common_pb2.Actor | None, field: str) -> None:
    if actor is None:
        _fail(JobValidationCode.MISSING_FIELD, field)
    _require_token_id(actor.actor_id.value if actor.HasField("actor_id") else None, field)
    if actor.kind not in _ACTOR_KINDS:
        _fail(JobValidationCode.UNKNOWN_ENUM, field)
    if (
        actor.display_name and not _bounded_text(actor.display_name, _MAX_ACTOR_DISPLAY_NAME_BYTES)
    ) or not _bounded_text(actor.authenticated_subject, _MAX_AUTHENTICATED_SUBJECT_BYTES):
        _fail(JobValidationCode.INVALID_ENVELOPE, field)


def validate_service_error(error: common_pb2.ServiceError) -> None:
    """Validate the bounded, stable ServiceError domain projection."""

    if error.category not in _ERROR_CATEGORIES:
        _fail(JobValidationCode.UNKNOWN_ENUM, "outcome.infrastructure_failure.error.category")
    if not _stable_error_code(error.code) or not _bounded_text(
        error.message, _MAX_ERROR_MESSAGE_BYTES
    ):
        _fail(
            JobValidationCode.INVALID_TERMINAL_PAYLOAD,
            "outcome.infrastructure_failure.error",
        )
    if len(error.details) > _MAX_ERROR_DETAILS:
        _fail(JobValidationCode.COLLECTION_LIMIT, "outcome.infrastructure_failure.error.details")
    for detail in error.details:
        if (
            not _bounded_field_path(detail.field_path)
            or not _stable_error_code(detail.code)
            or not _bounded_text(detail.message, _MAX_ERROR_MESSAGE_BYTES)
        ):
            _fail(
                JobValidationCode.INVALID_TERMINAL_PAYLOAD,
                "outcome.infrastructure_failure.error.details",
            )


def _validate_artifacts(values: Iterable[ArtifactRef], field: str) -> None:
    artifacts = list(values)
    if len(artifacts) > _MAX_OUTCOME_ARTIFACTS:
        _fail(JobValidationCode.COLLECTION_LIMIT, field)
    try:
        for artifact in artifacts:
            validate_artifact_ref(artifact)
    except ArtifactValidationError as error:
        raise JobValidationError(JobValidationCode.INVALID_TERMINAL_PAYLOAD, field) from error


def _require_sha256_id(value: str | None, field: str) -> str:
    if value is None:
        _fail(JobValidationCode.MISSING_FIELD, field)
    prefix = "sha256:"
    digest = value.removeprefix(prefix)
    if (
        not value.startswith(prefix)
        or len(digest) != 64
        or any(c not in "0123456789abcdef" for c in digest)
    ):
        _fail(JobValidationCode.INVALID_IDENTITY, field)
    return value


def _require_digest(value: common_pb2.Sha256Digest | None, field: str) -> bytes:
    if value is None:
        _fail(JobValidationCode.MISSING_FIELD, field)
    if len(value.value) != 32:
        _fail(JobValidationCode.INVALID_IDENTITY, field)
    return bytes(value.value)


def _encode_digest(value: bytes) -> str:
    return f"sha256:{value.hex()}"


def _domain_digest(domain: bytes, canonical: bytes) -> bytes:
    return hashlib.sha256(domain + b"\x00" + canonical).digest()


def _is_protocol_package(value: str) -> bool:
    return (
        len(value.encode("ascii", errors="ignore")) <= _MAX_PROTOCOL_NAME_BYTES
        and value.isascii()
        and re.fullmatch(r"[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*\.v[1-9][0-9]*", value) is not None
    )


def _is_protocol_feature(value: str) -> bool:
    segment = r"[a-z0-9](?:[a-z0-9-]*[a-z0-9])?"
    return (
        len(value.encode("ascii", errors="ignore")) <= _MAX_PROTOCOL_NAME_BYTES
        and value.isascii()
        and re.fullmatch(rf"{segment}(?:\.{segment})+", value) is not None
    )


def _is_build_version(value: str) -> bool:
    return (
        len(value.encode("ascii", errors="ignore")) <= _MAX_BUILD_VERSION_BYTES
        and value.isascii()
        and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9.+_-]*", value) is not None
    )


def _require_token_id(value: str | None, field: str) -> str:
    if value is None:
        _fail(JobValidationCode.MISSING_FIELD, field)
    if not 0 < len(value.encode("ascii", errors="ignore")) <= _MAX_ID_BYTES or not value.isascii():
        _fail(JobValidationCode.INVALID_IDENTITY, field)
    if not value[0].isalnum() or any(not (c.isalnum() or c in "._:-") for c in value):
        _fail(JobValidationCode.INVALID_IDENTITY, field)
    return value


def _require_timestamp(value: _Timestamp | None, field: str) -> tuple[int, int]:
    if value is None:
        _fail(JobValidationCode.MISSING_FIELD, field)
    if (
        not _MIN_TIMESTAMP_SECONDS <= value.seconds <= _MAX_TIMESTAMP_SECONDS
        or not 0 <= value.nanos < 1_000_000_000
    ):
        _fail(JobValidationCode.INVALID_TERMINAL_PAYLOAD, field)
    return value.seconds, value.nanos


def _bounded_text(value: str, maximum_bytes: int) -> bool:
    return (
        bool(value.strip())
        and len(value.encode("utf-8")) <= maximum_bytes
        and not _has_ascii_control(value)
    )


def _stable_error_code(value: str) -> bool:
    return _STABLE_ERROR_CODE_RE.fullmatch(value) is not None


def _bounded_field_path(value: str) -> bool:
    return (
        bool(value.strip())
        and len(value.encode("utf-8")) <= _MAX_ERROR_FIELD_PATH_BYTES
        and not _has_ascii_control(value)
    )


def _has_ascii_control(value: str) -> bool:
    return any(ord(character) < 32 or ord(character) == 127 for character in value)


def _fail(code: JobValidationCode, field: str) -> NoReturn:
    raise JobValidationError(code, field)
