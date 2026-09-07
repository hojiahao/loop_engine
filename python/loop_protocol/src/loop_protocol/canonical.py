from __future__ import annotations

import hashlib
import hmac
import json
import re
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from enum import StrEnum
from types import MappingProxyType
from typing import Any

EXPRESSION_DOMAIN = b"loop.factor-ast/v1\x00"
FACTOR_SPEC_DOMAIN = b"loop.factor-spec/v1\x00"
OPERATOR_REGISTRY_DOMAIN = b"loop.operator-registry/v1\x00"
OPERATOR_SEMANTIC_CONTRACT_SCHEMA = "loop.operator-semantic-contract/v1"

MAX_IDENTIFIER_BYTES = 128
MAX_AST_NODES = 4_096
MAX_AST_DEPTH = 64
MAX_DIRECT_ARGUMENTS = 1_024
MAX_CANONICAL_AST_BYTES = 256 * 1_024

_IDENTIFIER_RE = re.compile(r"^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$", re.ASCII)
_POLICY_ID_RE = re.compile(r"^[a-z][a-z0-9_.-]{0,127}$", re.ASCII)
_POSITIVE_INTEGER_RE = re.compile(r"^[1-9][0-9]*$", re.ASCII)
_DECIMAL_RE = re.compile(r"^-?(?:0|[1-9][0-9]*)(?:\.[0-9]*[1-9])?$", re.ASCII)
_SHA256_ID_RE = re.compile(r"^sha256:[0-9a-f]{64}$", re.ASCII)


class CanonicalizationError(ValueError):
    """The value cannot be represented by the v1 canonical identity profile."""


@dataclass(frozen=True, slots=True)
class CanonicalizationLimits:
    """Deployment limits bounded by the canonical v1 hard maxima."""

    max_nodes: int = MAX_AST_NODES
    max_depth: int = MAX_AST_DEPTH
    max_canonical_bytes: int = MAX_CANONICAL_AST_BYTES
    max_direct_arguments: int = MAX_DIRECT_ARGUMENTS

    def __post_init__(self) -> None:
        for name, value, maximum in (
            ("max_nodes", self.max_nodes, MAX_AST_NODES),
            ("max_depth", self.max_depth, MAX_AST_DEPTH),
            ("max_canonical_bytes", self.max_canonical_bytes, MAX_CANONICAL_AST_BYTES),
            ("max_direct_arguments", self.max_direct_arguments, MAX_DIRECT_ARGUMENTS),
        ):
            if isinstance(value, bool) or not isinstance(value, int) or not 1 <= value <= maximum:
                raise CanonicalizationError(f"{name} must be an integer in 1..={maximum}")


type CanonicalizationLimitOverrides = Mapping[str, int]


def resolve_canonicalization_limits(
    overrides: CanonicalizationLimits | CanonicalizationLimitOverrides | None = None,
) -> CanonicalizationLimits:
    if overrides is None:
        return CanonicalizationLimits()
    if isinstance(overrides, CanonicalizationLimits):
        return overrides
    if not isinstance(overrides, Mapping):
        raise CanonicalizationError(
            "canonicalization limits must be CanonicalizationLimits or a mapping"
        )

    values = {
        "max_nodes": MAX_AST_NODES,
        "max_depth": MAX_AST_DEPTH,
        "max_canonical_bytes": MAX_CANONICAL_AST_BYTES,
        "max_direct_arguments": MAX_DIRECT_ARGUMENTS,
    }
    for name, value in overrides.items():
        if name not in values:
            raise CanonicalizationError(f"unknown canonicalization limit: {name}")
        values[name] = value
    return CanonicalizationLimits(
        max_nodes=values["max_nodes"],
        max_depth=values["max_depth"],
        max_canonical_bytes=values["max_canonical_bytes"],
        max_direct_arguments=values["max_direct_arguments"],
    )


@dataclass(frozen=True, slots=True)
class FieldNode:
    field: str


@dataclass(frozen=True, slots=True)
class DecimalNode:
    value: str


@dataclass(frozen=True, slots=True)
class BooleanNode:
    value: bool


@dataclass(frozen=True, slots=True)
class EnumNode:
    enum_type: str
    value: str


@dataclass(frozen=True, slots=True)
class CallNode:
    operator: str
    operator_version: str
    arguments: tuple[AstNode, ...]

    def __post_init__(self) -> None:
        if not isinstance(self.arguments, tuple):
            raise CanonicalizationError("call arguments must be an immutable tuple")
        node_types = (FieldNode, DecimalNode, BooleanNode, EnumNode, CallNode)
        if any(not isinstance(argument, node_types) for argument in self.arguments):
            raise CanonicalizationError("call arguments contain an invalid AST node")


type AstNode = FieldNode | DecimalNode | BooleanNode | EnumNode | CallNode


class ScalarValueType(StrEnum):
    SERIES = "series"
    DECIMAL = "decimal"
    BOOLEAN = "boolean"


@dataclass(frozen=True, slots=True)
class EnumValueType:
    enum_type: str

    def __post_init__(self) -> None:
        _validate_identifier(self.enum_type, "enum type")


type ValueType = ScalarValueType | EnumValueType


@dataclass(frozen=True, slots=True)
class DecimalConstraints:
    max_precision: int
    max_scale: int
    minimum: str
    maximum: str

    def __post_init__(self) -> None:
        if isinstance(self.max_precision, bool) or not isinstance(self.max_precision, int):
            raise CanonicalizationError("max_precision must be an integer")
        if isinstance(self.max_scale, bool) or not isinstance(self.max_scale, int):
            raise CanonicalizationError("max_scale must be an integer")
        if not 1 <= self.max_precision <= 4_096:
            raise CanonicalizationError("max_precision must be in 1..=4096")
        if not 0 <= self.max_scale <= self.max_precision:
            raise CanonicalizationError("max_scale must be in 0..=max_precision")
        _validate_decimal(self.minimum)
        _validate_decimal(self.maximum)
        _validate_decimal_shape(self.minimum, self.max_precision, self.max_scale)
        _validate_decimal_shape(self.maximum, self.max_precision, self.max_scale)
        if _compare_decimals(self.minimum, self.maximum) > 0:
            raise CanonicalizationError("decimal minimum cannot exceed maximum")

    def validate(self, value: str) -> None:
        _validate_decimal_shape(value, self.max_precision, self.max_scale)
        if _compare_decimals(value, self.minimum) < 0:
            raise CanonicalizationError(f"decimal is below minimum {self.minimum}")
        if _compare_decimals(value, self.maximum) > 0:
            raise CanonicalizationError(f"decimal is above maximum {self.maximum}")


@dataclass(frozen=True, slots=True)
class ArgumentDefinition:
    value_type: ValueType
    literal_only: bool = False
    decimal: DecimalConstraints | None = None

    def __post_init__(self) -> None:
        if not isinstance(self.value_type, (ScalarValueType, EnumValueType)):
            raise CanonicalizationError("argument value_type is invalid")
        if not isinstance(self.literal_only, bool):
            raise CanonicalizationError("literal_only must be a boolean")
        if self.decimal is not None and not isinstance(self.decimal, DecimalConstraints):
            raise CanonicalizationError("argument decimal constraints are invalid")
        if (self.value_type == ScalarValueType.DECIMAL) != (self.decimal is not None):
            raise CanonicalizationError(
                "decimal arguments require constraints and non-decimal arguments forbid them"
            )


@dataclass(frozen=True, slots=True)
class FieldDefinition:
    field: str
    output_type: ValueType

    def __post_init__(self) -> None:
        _validate_identifier(self.field, "field")
        if not isinstance(self.output_type, (ScalarValueType, EnumValueType)):
            raise CanonicalizationError("field output_type is invalid")


@dataclass(frozen=True, slots=True)
class EnumDefinition:
    enum_type: str
    values: tuple[str, ...]

    def __post_init__(self) -> None:
        _validate_identifier(self.enum_type, "enum_type")
        if not isinstance(self.values, tuple):
            raise CanonicalizationError("enum values must be an immutable tuple")
        if not self.values:
            raise CanonicalizationError("enum domain cannot be empty")
        for value in self.values:
            _validate_identifier(value, "enum value")
        if len(set(self.values)) != len(self.values):
            raise CanonicalizationError("enum domain cannot contain duplicate values")


class NullPolicy(StrEnum):
    NOT_APPLICABLE = "not_applicable"
    PROPAGATE = "propagate"
    IGNORE_MISSING = "ignore_missing"
    PRESERVE_TARGET_IGNORE_PEERS = "preserve_target_ignore_peers"
    REJECT_MISSING = "reject_missing"


class WindowPolicy(StrEnum):
    NOT_APPLICABLE = "not_applicable"
    TRAILING_ARGUMENT_2_FULL_WINDOW_RIGHT_INCLUSIVE_CONSTANT_PRESERVE = (
        "trailing_argument_2_full_window_right_inclusive_constant_preserve"
    )
    TRAILING_DYNAMIC_MINIMUM_RIGHT_INCLUSIVE_CONSTANT_PRESERVE = (
        "trailing_argument_2_minimum_valid_min_n_max_3_"
        "floor_2n_div_3_right_inclusive_constant_preserve"
    )
    LAG_ARGUMENT_2 = "lag_argument_2"


class TiePolicy(StrEnum):
    NOT_APPLICABLE = "not_applicable"
    AVERAGE_VALID_COUNT = "average_valid_count"
    DENSE_VALID_COUNT = "dense_valid_count"
    STABLE_FIRST_VALID_COUNT_MINUS_ONE = "stable_first_valid_count_minus_one"
    ARGUMENT_2_AVERAGE_OR_DENSE_VALID_COUNT_CONSTANT_MIDPOINT = (
        "argument_2_average_or_dense_valid_count_constant_midpoint"
    )
    TARGET_LAST_STABLE_ORDER_VALID_COUNT_MINUS_ONE_CONSTANT_MIDPOINT = (
        "target_last_stable_order_valid_count_minus_one_constant_midpoint"
    )


class AlignmentPolicy(StrEnum):
    NOT_APPLICABLE = "not_applicable"
    UNARY_PRESERVE_TIMESTAMP_AND_SECURITY = "unary_preserve_timestamp_and_security"
    STRICT_TIMESTAMP_AND_SECURITY = "strict_timestamp_and_security"
    INTERSECTION_TIMESTAMP_AND_SECURITY = "intersection_timestamp_and_security"


class NumericPolicy(StrEnum):
    NOT_APPLICABLE = "not_applicable"
    EXACT_DECIMAL = "exact_decimal"
    BINARY64_NON_FINITE_TO_MISSING = "binary64_non_finite_to_missing"
    BINARY64_REJECT_NON_FINITE = "binary64_reject_non_finite"
    ORDINAL_UNIT_INTERVAL = "ordinal_unit_interval"
    BINARY64_ADJUSTED_FISHER_PEARSON_EFFECTIVE_N_MINIMUM_3_CONSTANT_ZERO_NON_FINITE_TO_MISSING = (
        "binary64_adjusted_fisher_pearson_effective_n_minimum_3_constant_zero_non_finite_to_missing"
    )


@dataclass(frozen=True, slots=True)
class OperatorSemanticContract:
    operator: str
    operator_version: str
    null_policy: NullPolicy
    window_policy: WindowPolicy
    tie_policy: TiePolicy
    alignment_policy: AlignmentPolicy
    numeric_policy: NumericPolicy

    def __post_init__(self) -> None:
        _validate_identifier(self.operator, "semantic contract operator")
        _validate_positive_integer(self.operator_version, "semantic contract operator_version")
        expected_types = (
            (self.null_policy, NullPolicy),
            (self.window_policy, WindowPolicy),
            (self.tie_policy, TiePolicy),
            (self.alignment_policy, AlignmentPolicy),
            (self.numeric_policy, NumericPolicy),
        )
        if any(not isinstance(value, expected) for value, expected in expected_types):
            raise CanonicalizationError("semantic policies must be explicit closed variants")

    @property
    def canonical_bytes(self) -> bytes:
        return _write_operator_semantic_contract(self).encode("ascii")

    @property
    def sha256(self) -> str:
        return semantic_contract_sha256(self.canonical_bytes)


@dataclass(frozen=True, slots=True)
class OperatorDefinition:
    operator: str
    operator_version: str
    parameters: tuple[ArgumentDefinition, ...]
    output_type: ValueType
    semantic_contract_sha256: str
    variadic: ArgumentDefinition | None = None
    minimum_arguments: int | None = None
    maximum_arguments: int | None = None
    associative: bool = False
    commutative: bool = False

    def __post_init__(self) -> None:
        _validate_identifier(self.operator, "operator")
        _validate_positive_integer(self.operator_version, "operator_version")
        _validate_sha256_id(self.semantic_contract_sha256, "semantic_contract_sha256")
        if not isinstance(self.output_type, (ScalarValueType, EnumValueType)):
            raise CanonicalizationError("operator output_type is invalid")
        if not isinstance(self.parameters, tuple):
            raise CanonicalizationError("operator parameters must be an immutable tuple")
        if any(not isinstance(parameter, ArgumentDefinition) for parameter in self.parameters):
            raise CanonicalizationError("operator parameters contain an invalid definition")
        if self.variadic is not None and not isinstance(self.variadic, ArgumentDefinition):
            raise CanonicalizationError("operator variadic definition is invalid")
        if len(self.parameters) > MAX_DIRECT_ARGUMENTS:
            raise CanonicalizationError("operator arity exceeds the v1 direct-argument limit")
        if not isinstance(self.associative, bool) or not isinstance(self.commutative, bool):
            raise CanonicalizationError("associative and commutative must be explicit booleans")
        for name, value in (
            ("minimum_arguments", self.minimum_arguments),
            ("maximum_arguments", self.maximum_arguments),
        ):
            if value is not None and (isinstance(value, bool) or not isinstance(value, int)):
                raise CanonicalizationError(f"{name} must be an integer")
        minimum = self.resolved_minimum_arguments
        maximum = self.resolved_maximum_arguments
        if (
            minimum < len(self.parameters)
            or (self.variadic is not None and minimum < 1)
            or maximum < minimum
            or maximum > MAX_DIRECT_ARGUMENTS
        ):
            raise CanonicalizationError("operator arity is inconsistent")
        if self.variadic is None and (
            minimum != len(self.parameters) or maximum != len(self.parameters)
        ):
            raise CanonicalizationError("fixed signatures must use their exact parameter count")
        if self.associative and self.variadic is None:
            raise CanonicalizationError("associative operators require a variadic signature")
        definitions = (*self.parameters, *((self.variadic,) if self.variadic is not None else ()))
        if self.associative or self.commutative:
            if any(definition.value_type != self.output_type for definition in definitions):
                raise CanonicalizationError(
                    "rewritable operator arguments must match its output type"
                )
            if definitions and any(definition != definitions[0] for definition in definitions[1:]):
                raise CanonicalizationError(
                    "rewritable operator argument rules must be homogeneous"
                )

    @property
    def resolved_minimum_arguments(self) -> int:
        if self.minimum_arguments is not None:
            return self.minimum_arguments
        return max(len(self.parameters), 1) if self.variadic is not None else len(self.parameters)

    @property
    def resolved_maximum_arguments(self) -> int:
        if self.maximum_arguments is not None:
            return self.maximum_arguments
        return MAX_DIRECT_ARGUMENTS if self.variadic is not None else len(self.parameters)

    def argument_definition(self, index: int) -> ArgumentDefinition | None:
        if index < len(self.parameters):
            return self.parameters[index]
        return self.variadic


# Compatibility name for the former algebra-only type. Its semantics are now a
# complete, typed operator definition and no untyped registration path remains.
OperatorPolicy = OperatorDefinition


@dataclass(frozen=True, slots=True, init=False)
class OperatorRegistry:
    """Immutable semantic names admitted by one operator-registry snapshot."""

    _fields: Mapping[str, ValueType]
    _enums: Mapping[str, frozenset[str]]
    _operators: Mapping[tuple[str, str], OperatorDefinition]
    _semantic_contracts: Mapping[str, OperatorSemanticContract]
    _canonical_bytes: bytes
    _sha256: str

    def __init__(
        self,
        *,
        fields: tuple[FieldDefinition, ...],
        enums: tuple[EnumDefinition, ...],
        operators: tuple[OperatorDefinition, ...],
        semantic_contract_resolver: Callable[[str], bytes | None],
    ) -> None:
        validated_fields: dict[str, ValueType] = {}
        for field_definition in fields:
            if field_definition.field in validated_fields:
                raise CanonicalizationError(f"duplicate field: {field_definition.field}")
            validated_fields[field_definition.field] = field_definition.output_type

        validated_enums: dict[str, frozenset[str]] = {}
        for enum_definition in enums:
            if enum_definition.enum_type in validated_enums:
                raise CanonicalizationError(f"duplicate enum: {enum_definition.enum_type}")
            validated_enums[enum_definition.enum_type] = frozenset(enum_definition.values)

        registered_types = [*validated_fields.values()]
        validated_operators: dict[tuple[str, str], OperatorDefinition] = {}
        validated_semantic_contracts: dict[str, OperatorSemanticContract] = {}
        for operator_definition in operators:
            key = (operator_definition.operator, operator_definition.operator_version)
            if key in validated_operators:
                raise CanonicalizationError(
                    "duplicate operator definition: "
                    f"{operator_definition.operator}@{operator_definition.operator_version}"
                )
            validated_operators[key] = operator_definition
            contract_bytes = semantic_contract_resolver(
                operator_definition.semantic_contract_sha256
            )
            if contract_bytes is None:
                raise CanonicalizationError(
                    "semantic contract was not resolved: "
                    f"{operator_definition.semantic_contract_sha256}"
                )
            computed = semantic_contract_sha256(contract_bytes)
            if not hmac.compare_digest(operator_definition.semantic_contract_sha256, computed):
                raise CanonicalizationError(
                    "semantic contract digest does not match resolved content"
                )
            contract = parse_canonical_operator_semantic_contract(contract_bytes)
            if (contract.operator, contract.operator_version) != key:
                raise CanonicalizationError(
                    "semantic contract operator identity does not match registry definition"
                )
            validated_semantic_contracts[computed] = contract
            registered_types.extend(
                argument.value_type for argument in operator_definition.parameters
            )
            if operator_definition.variadic is not None:
                registered_types.append(operator_definition.variadic.value_type)
            registered_types.append(operator_definition.output_type)
        for value_type in registered_types:
            if (
                isinstance(value_type, EnumValueType)
                and value_type.enum_type not in validated_enums
            ):
                raise CanonicalizationError(f"unregistered enum type: {value_type.enum_type}")

        canonical_bytes = _write_operator_registry(
            validated_fields,
            validated_enums,
            validated_operators,
        ).encode("ascii")
        object.__setattr__(self, "_fields", MappingProxyType(validated_fields))
        object.__setattr__(self, "_enums", MappingProxyType(validated_enums))
        object.__setattr__(self, "_operators", MappingProxyType(validated_operators))
        object.__setattr__(
            self,
            "_semantic_contracts",
            MappingProxyType(validated_semantic_contracts),
        )
        object.__setattr__(self, "_canonical_bytes", canonical_bytes)
        object.__setattr__(
            self,
            "_sha256",
            _content_id(OPERATOR_REGISTRY_DOMAIN, canonical_bytes),
        )

    @property
    def canonical_bytes(self) -> bytes:
        return self._canonical_bytes

    @property
    def sha256(self) -> str:
        return self._sha256

    def require_field(self, field: str) -> ValueType:
        _validate_identifier(field, "field")
        if field not in self._fields:
            raise CanonicalizationError(f"unknown field: {field}")
        return self._fields[field]

    def require_enum(self, enum_type: str, value: str) -> EnumValueType:
        _validate_identifier(enum_type, "enum_type")
        _validate_identifier(value, "enum value")
        if value not in self._enums.get(enum_type, frozenset()):
            raise CanonicalizationError(f"unknown enum literal: {enum_type}.{value}")
        return EnumValueType(enum_type)

    def require_operator(self, operator: str, operator_version: str) -> OperatorDefinition:
        _validate_identifier(operator, "operator")
        _validate_positive_integer(operator_version, "operator_version")
        try:
            return self._operators[(operator, operator_version)]
        except KeyError as error:
            raise CanonicalizationError(
                f"unknown operator: {operator}@{operator_version}"
            ) from error

    def require_semantic_contract(
        self, operator: str, operator_version: str
    ) -> OperatorSemanticContract:
        definition = self.require_operator(operator, operator_version)
        return self._semantic_contracts[definition.semantic_contract_sha256]


@dataclass(frozen=True, slots=True)
class CanonicalExpression:
    ast: AstNode
    canonical_bytes: bytes
    expression_id: str


class FactorDirection(StrEnum):
    HIGHER_IS_BETTER = "higher_is_better"
    LOWER_IS_BETTER = "lower_is_better"


@dataclass(frozen=True, slots=True)
class PolicyReference:
    policy_id: str
    revision: str
    sha256: str

    def __post_init__(self) -> None:
        _validate_policy_id(self.policy_id)
        _validate_positive_integer(self.revision, "policy revision")
        _validate_sha256_id(self.sha256, "policy sha256")


_FACTOR_SPEC_POLICY_FIELDS = (
    "universe_policy",
    "data_policy",
    "calendar_policy",
    "preprocess_policy",
    "neutralization_policy",
    "portfolio_policy",
    "execution_policy",
    "cost_policy",
    "evaluation_policy",
)


@dataclass(frozen=True, slots=True)
class FactorSpec:
    expression_id: str
    operator_registry_sha256: str
    direction: FactorDirection
    universe_policy: PolicyReference
    data_policy: PolicyReference
    calendar_policy: PolicyReference
    preprocess_policy: PolicyReference
    neutralization_policy: PolicyReference
    portfolio_policy: PolicyReference
    execution_policy: PolicyReference
    cost_policy: PolicyReference
    evaluation_policy: PolicyReference

    def __post_init__(self) -> None:
        _validate_factor_spec(self)


def _validate_policy_reference(reference: object, label: str) -> None:
    if not isinstance(reference, PolicyReference):
        raise CanonicalizationError(f"{label} must be a PolicyReference")
    try:
        _validate_policy_id(reference.policy_id)
        _validate_positive_integer(reference.revision, "policy revision")
        _validate_sha256_id(reference.sha256, "policy sha256")
    except CanonicalizationError as error:
        raise CanonicalizationError(f"{label} is invalid: {error}") from error


def _validate_factor_spec(spec: object) -> None:
    if not isinstance(spec, FactorSpec):
        raise CanonicalizationError("spec must be a FactorSpec")
    _validate_sha256_id(spec.expression_id, "expression_id")
    _validate_sha256_id(spec.operator_registry_sha256, "operator_registry_sha256")
    if not isinstance(spec.direction, FactorDirection):
        raise CanonicalizationError("direction must be a FactorDirection")
    for name in _FACTOR_SPEC_POLICY_FIELDS:
        _validate_policy_reference(getattr(spec, name), name)


@dataclass(frozen=True, slots=True, init=False)
class CanonicalFactorSpec:
    spec: FactorSpec
    expression: CanonicalExpression
    canonical_bytes: bytes
    factor_spec_id: str

    def __new__(cls) -> CanonicalFactorSpec:
        raise TypeError("CanonicalFactorSpec can only be created by bind_factor_spec")


def canonicalize_expression(
    ast: AstNode,
    registry: OperatorRegistry,
    limit_overrides: CanonicalizationLimits | CanonicalizationLimitOverrides | None = None,
) -> CanonicalExpression:
    limits = resolve_canonicalization_limits(limit_overrides)
    normalized, _ = _normalize_node(
        ast,
        registry,
        limits=limits,
        depth=1,
        node_counter=[0],
    )
    canonical_bytes = _write_node(normalized).encode("ascii")
    if len(canonical_bytes) > limits.max_canonical_bytes:
        raise CanonicalizationError(f"canonical AST exceeds {limits.max_canonical_bytes} bytes")

    reparsed = parse_canonical_ast(canonical_bytes, registry, limits)
    if reparsed != normalized:
        raise CanonicalizationError("canonical AST did not survive strict reparse")

    return CanonicalExpression(
        ast=reparsed,
        canonical_bytes=canonical_bytes,
        expression_id=_content_id(EXPRESSION_DOMAIN, canonical_bytes),
    )


def parse_canonical_ast(
    canonical_bytes: bytes,
    registry: OperatorRegistry,
    limit_overrides: CanonicalizationLimits | CanonicalizationLimitOverrides | None = None,
) -> AstNode:
    limits = resolve_canonicalization_limits(limit_overrides)
    if len(canonical_bytes) > limits.max_canonical_bytes:
        raise CanonicalizationError(f"canonical AST exceeds {limits.max_canonical_bytes} bytes")
    try:
        text = canonical_bytes.decode("ascii")
    except UnicodeDecodeError as error:
        raise CanonicalizationError("canonical AST must contain ASCII only") from error

    try:
        raw = json.loads(
            text,
            object_pairs_hook=_ObjectPairs,
            parse_float=_reject_json_number,
            parse_int=_reject_json_number,
            parse_constant=_reject_json_constant,
        )
    except (json.JSONDecodeError, CanonicalizationError, RecursionError) as error:
        raise CanonicalizationError("invalid canonical AST JSON") from error

    try:
        parsed = _parse_node(raw)
        normalized, _ = _normalize_node(
            parsed,
            registry,
            limits=limits,
            depth=1,
            node_counter=[0],
        )
    except RecursionError as error:
        raise CanonicalizationError("canonical AST nesting exceeds safe parser depth") from error
    if _write_node(normalized).encode("ascii") != canonical_bytes:
        raise CanonicalizationError("AST bytes are valid JSON but not canonical v1 bytes")
    return normalized


def bind_factor_spec(
    spec: FactorSpec,
    canonical_expression_bytes: bytes,
    registry: OperatorRegistry,
    limit_overrides: CanonicalizationLimits | CanonicalizationLimitOverrides | None = None,
) -> CanonicalFactorSpec:
    limits = resolve_canonicalization_limits(limit_overrides)
    _validate_factor_spec(spec)
    if not hmac.compare_digest(spec.operator_registry_sha256, registry.sha256):
        raise CanonicalizationError(
            "operator_registry_sha256 does not match the resolved registry snapshot"
        )
    expression = verify_expression_id(
        spec.expression_id,
        canonical_expression_bytes,
        registry,
        limits,
    )
    _, root_type = _normalize_node(
        expression.ast,
        registry,
        limits=limits,
        depth=1,
        node_counter=[0],
    )
    if root_type != ScalarValueType.SERIES:
        label = (
            root_type.value
            if isinstance(root_type, ScalarValueType)
            else f"enum:{root_type.enum_type}"
        )
        raise CanonicalizationError(f"factor root must resolve to series, received {label}")
    canonical_bytes = _write_factor_spec(spec).encode("ascii")
    if len(canonical_bytes) > limits.max_canonical_bytes:
        raise CanonicalizationError(
            f"canonical FactorSpec exceeds {limits.max_canonical_bytes} bytes"
        )
    return _new_bound_factor_spec(
        spec,
        expression,
        canonical_bytes,
        _content_id(FACTOR_SPEC_DOMAIN, canonical_bytes),
    )


def parse_canonical_factor_spec(
    canonical_spec_bytes: bytes,
    expected_factor_spec_id: str,
    canonical_expression_bytes: bytes,
    registry: OperatorRegistry,
    limit_overrides: CanonicalizationLimits | CanonicalizationLimitOverrides | None = None,
) -> CanonicalFactorSpec:
    limits = resolve_canonicalization_limits(limit_overrides)
    if len(canonical_spec_bytes) > limits.max_canonical_bytes:
        raise CanonicalizationError(
            f"canonical FactorSpec exceeds {limits.max_canonical_bytes} bytes"
        )
    try:
        text = canonical_spec_bytes.decode("ascii")
    except UnicodeDecodeError as error:
        raise CanonicalizationError("canonical FactorSpec must contain ASCII only") from error
    try:
        raw = json.loads(
            text,
            object_pairs_hook=_ObjectPairs,
            parse_float=_reject_json_number,
            parse_int=_reject_json_number,
            parse_constant=_reject_json_constant,
        )
        spec = _parse_factor_spec(raw)
    except (json.JSONDecodeError, CanonicalizationError, RecursionError) as error:
        raise CanonicalizationError("invalid canonical FactorSpec JSON") from error
    if _write_factor_spec(spec).encode("ascii") != canonical_spec_bytes:
        raise CanonicalizationError("FactorSpec bytes are valid JSON but not canonical v1 bytes")
    bound = bind_factor_spec(spec, canonical_expression_bytes, registry, limits)
    _verify_content_id(
        expected_factor_spec_id,
        FACTOR_SPEC_DOMAIN,
        canonical_spec_bytes,
        "factor_spec_id",
    )
    return bound


def verify_expression_id(
    expression_id: str,
    canonical_bytes: bytes,
    registry: OperatorRegistry,
    limit_overrides: CanonicalizationLimits | CanonicalizationLimitOverrides | None = None,
) -> CanonicalExpression:
    limits = resolve_canonicalization_limits(limit_overrides)
    ast = parse_canonical_ast(canonical_bytes, registry, limits)
    _verify_content_id(expression_id, EXPRESSION_DOMAIN, canonical_bytes, "expression_id")
    return CanonicalExpression(
        ast=ast,
        canonical_bytes=canonical_bytes,
        expression_id=expression_id,
    )


def verify_factor_spec_id(
    factor_spec_id: str,
    canonical_spec_bytes: bytes,
    canonical_expression_bytes: bytes,
    registry: OperatorRegistry,
    limit_overrides: CanonicalizationLimits | CanonicalizationLimitOverrides | None = None,
) -> CanonicalFactorSpec:
    return parse_canonical_factor_spec(
        canonical_spec_bytes,
        factor_spec_id,
        canonical_expression_bytes,
        registry,
        limit_overrides,
    )


def _new_bound_factor_spec(
    spec: FactorSpec,
    expression: CanonicalExpression,
    canonical_bytes: bytes,
    factor_spec_id: str,
) -> CanonicalFactorSpec:
    bound = object.__new__(CanonicalFactorSpec)
    object.__setattr__(bound, "spec", spec)
    object.__setattr__(bound, "expression", expression)
    object.__setattr__(bound, "canonical_bytes", canonical_bytes)
    object.__setattr__(bound, "factor_spec_id", factor_spec_id)
    return bound


def _normalize_node(
    node: AstNode,
    registry: OperatorRegistry,
    *,
    limits: CanonicalizationLimits,
    depth: int,
    node_counter: list[int],
) -> tuple[AstNode, ValueType]:
    if depth > limits.max_depth:
        raise CanonicalizationError(f"AST depth exceeds {limits.max_depth}")
    node_counter[0] += 1
    if node_counter[0] > limits.max_nodes:
        raise CanonicalizationError(f"AST node count exceeds {limits.max_nodes}")

    if isinstance(node, FieldNode):
        return node, registry.require_field(node.field)
    if isinstance(node, DecimalNode):
        _validate_decimal(node.value)
        return node, ScalarValueType.DECIMAL
    if isinstance(node, BooleanNode):
        if not isinstance(node.value, bool):
            raise CanonicalizationError("boolean literal must be a boolean")
        return node, ScalarValueType.BOOLEAN
    if isinstance(node, EnumNode):
        return node, registry.require_enum(node.enum_type, node.value)
    if not isinstance(node, CallNode):
        raise CanonicalizationError(f"unsupported AST node type: {type(node).__name__}")

    definition = registry.require_operator(node.operator, node.operator_version)
    if len(node.arguments) > limits.max_direct_arguments:
        raise CanonicalizationError(
            f"call has more than {limits.max_direct_arguments} direct arguments"
        )

    resolved_arguments: list[tuple[AstNode, ValueType]] = []
    for index, argument in enumerate(node.arguments):
        normalized, value_type = _normalize_node(
            argument,
            registry,
            limits=limits,
            depth=depth + 1,
            node_counter=node_counter,
        )
        resolved_arguments.append((normalized, value_type))
        _validate_argument(definition, index, normalized, value_type)

    arguments: list[tuple[AstNode, ValueType]] = []
    for normalized, value_type in resolved_arguments:
        if (
            definition.associative
            and isinstance(normalized, CallNode)
            and normalized.operator == node.operator
            and normalized.operator_version == node.operator_version
        ):
            arguments.extend(
                (argument, definition.output_type) for argument in normalized.arguments
            )
        else:
            arguments.append((normalized, value_type))

    if len(arguments) > limits.max_direct_arguments:
        raise CanonicalizationError(
            f"normalized call has more than {limits.max_direct_arguments} direct arguments"
        )
    if not (
        definition.resolved_minimum_arguments
        <= len(arguments)
        <= definition.resolved_maximum_arguments
    ):
        raise CanonicalizationError(
            f"invalid arity for {node.operator}@{node.operator_version}: {len(arguments)}"
        )
    for index, (argument, value_type) in enumerate(arguments):
        _validate_argument(definition, index, argument, value_type)
    if definition.commutative:
        arguments.sort(key=lambda argument: _write_node(argument[0]).encode("ascii"))

    return (
        CallNode(
            node.operator,
            node.operator_version,
            tuple(argument for argument, _ in arguments),
        ),
        definition.output_type,
    )


def _validate_argument(
    definition: OperatorDefinition,
    index: int,
    node: AstNode,
    value_type: ValueType,
) -> None:
    argument = definition.argument_definition(index)
    if argument is None:
        raise CanonicalizationError(
            f"invalid arity for {definition.operator}@{definition.operator_version}"
        )
    if argument.value_type != value_type:
        raise CanonicalizationError(
            f"type mismatch for {definition.operator}@{definition.operator_version} "
            f"argument {index}"
        )
    if argument.literal_only and not _is_literal_of_type(node, argument.value_type):
        raise CanonicalizationError(
            f"argument {index} for {definition.operator}@{definition.operator_version} "
            "must be literal"
        )
    if argument.decimal is not None and isinstance(node, DecimalNode):
        argument.decimal.validate(node.value)


def _is_literal_of_type(node: AstNode, value_type: ValueType) -> bool:
    if value_type == ScalarValueType.DECIMAL:
        return isinstance(node, DecimalNode)
    if value_type == ScalarValueType.BOOLEAN:
        return isinstance(node, BooleanNode)
    if isinstance(value_type, EnumValueType):
        return isinstance(node, EnumNode) and node.enum_type == value_type.enum_type
    return False


class _ObjectPairs(list[tuple[str, Any]]):
    pass


def _parse_node(raw: Any) -> AstNode:
    if not isinstance(raw, _ObjectPairs):
        raise CanonicalizationError("AST node must be an object")
    keys = [key for key, _ in raw]
    values = dict(raw)
    if keys == ["node", "field"] and values["node"] == "field":
        return FieldNode(_require_string(values["field"], "field"))
    if keys == ["node", "value"] and values["node"] == "decimal":
        return DecimalNode(_require_string(values["value"], "decimal value"))
    if keys == ["node", "value"] and values["node"] == "boolean":
        if not isinstance(values["value"], bool):
            raise CanonicalizationError("boolean value must be true or false")
        return BooleanNode(values["value"])
    if keys == ["node", "enum_type", "value"] and values["node"] == "enum":
        return EnumNode(
            _require_string(values["enum_type"], "enum_type"),
            _require_string(values["value"], "enum value"),
        )
    if keys == ["node", "operator", "operator_version", "arguments"] and values["node"] == "call":
        raw_arguments = values["arguments"]
        if not isinstance(raw_arguments, list) or isinstance(raw_arguments, _ObjectPairs):
            raise CanonicalizationError("arguments must be an array")
        return CallNode(
            _require_string(values["operator"], "operator"),
            _require_string(values["operator_version"], "operator_version"),
            tuple(_parse_node(argument) for argument in raw_arguments),
        )
    raise CanonicalizationError(
        "AST object has unknown, duplicate, missing, or out-of-order fields"
    )


def _write_node(node: AstNode) -> str:
    if isinstance(node, FieldNode):
        return f'{{"node":"field","field":"{node.field}"}}'
    if isinstance(node, DecimalNode):
        return f'{{"node":"decimal","value":"{node.value}"}}'
    if isinstance(node, BooleanNode):
        value = "true" if node.value else "false"
        return f'{{"node":"boolean","value":{value}}}'
    if isinstance(node, EnumNode):
        return f'{{"node":"enum","enum_type":"{node.enum_type}","value":"{node.value}"}}'
    if isinstance(node, CallNode):
        arguments = ",".join(_write_node(argument) for argument in node.arguments)
        return (
            f'{{"node":"call","operator":"{node.operator}",'
            f'"operator_version":"{node.operator_version}","arguments":[{arguments}]}}'
        )
    raise CanonicalizationError(f"unsupported AST node type: {type(node).__name__}")


def _write_policy_reference(reference: PolicyReference) -> str:
    return (
        f'{{"policy_id":"{reference.policy_id}","revision":"{reference.revision}",'
        f'"sha256":"{reference.sha256}"}}'
    )


def _parse_policy_reference(raw: Any, label: str) -> PolicyReference:
    if not isinstance(raw, _ObjectPairs):
        raise CanonicalizationError(f"{label} must be an object")
    keys = [key for key, _ in raw]
    if keys != ["policy_id", "revision", "sha256"]:
        raise CanonicalizationError(
            f"{label} has unknown, duplicate, missing, or out-of-order fields"
        )
    values = dict(raw)
    return PolicyReference(
        _require_string(values["policy_id"], f"{label}.policy_id"),
        _require_string(values["revision"], f"{label}.revision"),
        _require_string(values["sha256"], f"{label}.sha256"),
    )


def _parse_factor_spec(raw: Any) -> FactorSpec:
    if not isinstance(raw, _ObjectPairs):
        raise CanonicalizationError("FactorSpec must be an object")
    expected_keys = [
        "schema",
        "expression_id",
        "operator_registry_sha256",
        "direction",
        *_FACTOR_SPEC_POLICY_FIELDS,
    ]
    keys = [key for key, _ in raw]
    if keys != expected_keys:
        raise CanonicalizationError(
            "FactorSpec has unknown, duplicate, missing, or out-of-order fields"
        )
    values = dict(raw)
    if values["schema"] != "loop.factor-spec/v1":
        raise CanonicalizationError("FactorSpec schema must be loop.factor-spec/v1")
    try:
        direction = FactorDirection(_require_string(values["direction"], "direction"))
    except ValueError as error:
        raise CanonicalizationError(
            "direction must be higher_is_better or lower_is_better"
        ) from error
    policies = {
        name: _parse_policy_reference(values[name], name) for name in _FACTOR_SPEC_POLICY_FIELDS
    }
    return FactorSpec(
        expression_id=_require_string(values["expression_id"], "expression_id"),
        operator_registry_sha256=_require_string(
            values["operator_registry_sha256"], "operator_registry_sha256"
        ),
        direction=direction,
        universe_policy=policies["universe_policy"],
        data_policy=policies["data_policy"],
        calendar_policy=policies["calendar_policy"],
        preprocess_policy=policies["preprocess_policy"],
        neutralization_policy=policies["neutralization_policy"],
        portfolio_policy=policies["portfolio_policy"],
        execution_policy=policies["execution_policy"],
        cost_policy=policies["cost_policy"],
        evaluation_policy=policies["evaluation_policy"],
    )


def _write_factor_spec(spec: FactorSpec) -> str:
    _validate_factor_spec(spec)
    fields = (
        ("universe_policy", spec.universe_policy),
        ("data_policy", spec.data_policy),
        ("calendar_policy", spec.calendar_policy),
        ("preprocess_policy", spec.preprocess_policy),
        ("neutralization_policy", spec.neutralization_policy),
        ("portfolio_policy", spec.portfolio_policy),
        ("execution_policy", spec.execution_policy),
        ("cost_policy", spec.cost_policy),
        ("evaluation_policy", spec.evaluation_policy),
    )
    policies = ",".join(
        f'"{name}":{_write_policy_reference(reference)}' for name, reference in fields
    )
    return (
        f'{{"schema":"loop.factor-spec/v1","expression_id":"{spec.expression_id}",'
        f'"operator_registry_sha256":"{spec.operator_registry_sha256}",'
        f'"direction":"{spec.direction.value}",{policies}}}'
    )


def _write_operator_registry(
    fields: dict[str, ValueType],
    enums: dict[str, frozenset[str]],
    operators: dict[tuple[str, str], OperatorDefinition],
) -> str:
    field_json = ",".join(
        f'{{"field":"{field}","outputType":{_write_registry_value_type(output_type)}}}'
        for field, output_type in sorted(fields.items())
    )
    enum_json = ",".join(
        f'{{"enumType":"{enum_type}","values":['
        + ",".join(f'"{value}"' for value in sorted(values))
        + "]}"
        for enum_type, values in sorted(enums.items())
    )
    operator_json = ",".join(
        _write_registry_operator(definition)
        for definition in sorted(
            operators.values(),
            key=lambda definition: (
                definition.operator,
                len(definition.operator_version),
                definition.operator_version,
            ),
        )
    )
    return (
        '{"schema":"loop.operator-registry/v1",'
        f'"fields":[{field_json}],"enums":[{enum_json}],"operators":[{operator_json}]}}'
    )


def _write_registry_value_type(value_type: ValueType) -> str:
    if isinstance(value_type, ScalarValueType):
        return f'"{value_type.value}"'
    return f'{{"enumType":"{value_type.enum_type}"}}'


def _write_registry_argument(argument: ArgumentDefinition) -> str:
    output = f'{{"type":{_write_registry_value_type(argument.value_type)}'
    if argument.literal_only:
        output += ',"literalOnly":true'
    if argument.decimal is not None:
        output += (
            f',"decimal":{{"maxPrecision":"{argument.decimal.max_precision}",'
            f'"maxScale":"{argument.decimal.max_scale}",'
            f'"minimum":"{argument.decimal.minimum}",'
            f'"maximum":"{argument.decimal.maximum}"}}'
        )
    return f"{output}}}"


def _write_registry_operator(definition: OperatorDefinition) -> str:
    parameters = ",".join(_write_registry_argument(argument) for argument in definition.parameters)
    output = (
        f'{{"operator":"{definition.operator}",'
        f'"operatorVersion":"{definition.operator_version}",'
        f'"semanticContractSha256":"{definition.semantic_contract_sha256}",'
        f'"parameters":[{parameters}]'
    )
    if definition.variadic is not None:
        output += (
            f',"variadic":{_write_registry_argument(definition.variadic)},'
            f'"minArguments":"{definition.resolved_minimum_arguments}",'
            f'"maxArguments":"{definition.resolved_maximum_arguments}"'
        )
    associative = "true" if definition.associative else "false"
    commutative = "true" if definition.commutative else "false"
    output += (
        f',"outputType":{_write_registry_value_type(definition.output_type)},'
        f'"associative":{associative},"commutative":{commutative}}}'
    )
    return output


def _write_operator_semantic_contract(contract: OperatorSemanticContract) -> str:
    return (
        f'{{"schema":"{OPERATOR_SEMANTIC_CONTRACT_SCHEMA}",'
        f'"operator":"{contract.operator}",'
        f'"operatorVersion":"{contract.operator_version}",'
        f'"nullPolicy":"{contract.null_policy.value}",'
        f'"windowPolicy":"{contract.window_policy.value}",'
        f'"tiePolicy":"{contract.tie_policy.value}",'
        f'"alignmentPolicy":"{contract.alignment_policy.value}",'
        f'"numericPolicy":"{contract.numeric_policy.value}"}}'
    )


def parse_canonical_operator_semantic_contract(
    canonical_bytes: bytes,
) -> OperatorSemanticContract:
    if not isinstance(canonical_bytes, bytes) or len(canonical_bytes) > 4_096:
        raise CanonicalizationError("semantic contract must be at most 4096 bytes")
    try:
        raw = json.loads(
            canonical_bytes.decode("utf-8", errors="strict"),
            object_pairs_hook=_ObjectPairs,
            parse_int=_reject_json_number,
            parse_float=_reject_json_number,
            parse_constant=_reject_json_number,
        )
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as error:
        raise CanonicalizationError("semantic contract is not valid UTF-8 JSON") from error
    if not isinstance(raw, _ObjectPairs):
        raise CanonicalizationError("semantic contract root must be an object")
    expected_keys = [
        "schema",
        "operator",
        "operatorVersion",
        "nullPolicy",
        "windowPolicy",
        "tiePolicy",
        "alignmentPolicy",
        "numericPolicy",
    ]
    keys = [key for key, _ in raw]
    if keys != expected_keys:
        raise CanonicalizationError(
            "semantic contract fields must be present once in canonical order"
        )
    values = dict(raw)
    if values["schema"] != OPERATOR_SEMANTIC_CONTRACT_SCHEMA:
        raise CanonicalizationError("unsupported semantic contract schema")
    try:
        contract = OperatorSemanticContract(
            operator=_require_string(values["operator"], "semantic contract operator"),
            operator_version=_require_string(
                values["operatorVersion"], "semantic contract operatorVersion"
            ),
            null_policy=NullPolicy(
                _require_string(values["nullPolicy"], "semantic contract nullPolicy")
            ),
            window_policy=WindowPolicy(
                _require_string(values["windowPolicy"], "semantic contract windowPolicy")
            ),
            tie_policy=TiePolicy(
                _require_string(values["tiePolicy"], "semantic contract tiePolicy")
            ),
            alignment_policy=AlignmentPolicy(
                _require_string(values["alignmentPolicy"], "semantic contract alignmentPolicy")
            ),
            numeric_policy=NumericPolicy(
                _require_string(values["numericPolicy"], "semantic contract numericPolicy")
            ),
        )
    except ValueError as error:
        message = "semantic contract contains an unknown policy variant"
        raise CanonicalizationError(message) from error
    if contract.canonical_bytes != canonical_bytes:
        raise CanonicalizationError("semantic contract bytes are not canonical v1 JSON")
    return contract


def semantic_contract_sha256(canonical_bytes: bytes) -> str:
    if not isinstance(canonical_bytes, bytes):
        raise CanonicalizationError("semantic contract content must be bytes")
    return f"sha256:{hashlib.sha256(canonical_bytes).hexdigest()}"


def _validate_identifier(value: str, label: str) -> None:
    if not isinstance(value, str) or not _IDENTIFIER_RE.fullmatch(value):
        raise CanonicalizationError(f"{label} is not a canonical dot-qualified identifier")
    if len(value.encode("ascii")) > MAX_IDENTIFIER_BYTES:
        raise CanonicalizationError(f"{label} exceeds {MAX_IDENTIFIER_BYTES} bytes")


def _validate_policy_id(value: str) -> None:
    if not isinstance(value, str) or not _POLICY_ID_RE.fullmatch(value):
        raise CanonicalizationError("policy_id is not canonical")


def _validate_positive_integer(value: str, label: str) -> None:
    if (
        not isinstance(value, str)
        or len(value) > 20
        or not _POSITIVE_INTEGER_RE.fullmatch(value)
        or int(value) > 18_446_744_073_709_551_615
    ):
        raise CanonicalizationError(
            f"{label} must be canonical decimal in 1..=18446744073709551615"
        )


def _validate_decimal(value: str) -> None:
    if not isinstance(value, str) or value == "-0" or not _DECIMAL_RE.fullmatch(value):
        raise CanonicalizationError("decimal literal is not canonical")


def _decimal_parts(value: str) -> tuple[bool, str, str]:
    negative = value.startswith("-")
    unsigned = value[1:] if negative else value
    integer, separator, fraction = unsigned.partition(".")
    return negative, integer, fraction if separator else ""


def _validate_decimal_shape(value: str, max_precision: int, max_scale: int) -> None:
    _validate_decimal(value)
    _, integer, fraction = _decimal_parts(value)
    precision = len(integer) + len(fraction)
    if precision > max_precision:
        raise CanonicalizationError(f"decimal precision exceeds {max_precision}")
    if len(fraction) > max_scale:
        raise CanonicalizationError(f"decimal scale exceeds {max_scale}")


def _compare_decimals(left: str, right: str) -> int:
    left_negative, left_integer, left_fraction = _decimal_parts(left)
    right_negative, right_integer, right_fraction = _decimal_parts(right)
    if left_negative != right_negative:
        return -1 if left_negative else 1
    width = max(len(left_fraction), len(right_fraction))
    left_magnitude = (len(left_integer), left_integer, left_fraction.ljust(width, "0"))
    right_magnitude = (len(right_integer), right_integer, right_fraction.ljust(width, "0"))
    comparison = (left_magnitude > right_magnitude) - (left_magnitude < right_magnitude)
    return -comparison if left_negative else comparison


def _validate_sha256_id(value: str, label: str) -> None:
    if not isinstance(value, str) or not _SHA256_ID_RE.fullmatch(value):
        raise CanonicalizationError(f"{label} must be sha256: followed by 64 lowercase hex digits")


def _content_id(domain: bytes, canonical_bytes: bytes) -> str:
    return f"sha256:{hashlib.sha256(domain + canonical_bytes).hexdigest()}"


def _verify_content_id(value: str, domain: bytes, canonical_bytes: bytes, label: str) -> None:
    _validate_sha256_id(value, label)
    expected = _content_id(domain, canonical_bytes)
    if not hmac.compare_digest(value, expected):
        raise CanonicalizationError(f"{label} does not match canonical bytes")


def _require_string(value: Any, label: str) -> str:
    if not isinstance(value, str):
        raise CanonicalizationError(f"{label} must be a string")
    return value


def _reject_json_number(value: str) -> Any:
    raise CanonicalizationError(f"JSON number token is forbidden: {value}")


def _reject_json_constant(value: str) -> Any:
    raise CanonicalizationError(f"JSON constant is forbidden: {value}")
