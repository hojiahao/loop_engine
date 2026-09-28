import datetime

from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.v1 import common_pb2 as _common_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class FactorDirection(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    FACTOR_DIRECTION_UNSPECIFIED: _ClassVar[FactorDirection]
    FACTOR_DIRECTION_HIGHER_IS_BETTER: _ClassVar[FactorDirection]
    FACTOR_DIRECTION_LOWER_IS_BETTER: _ClassVar[FactorDirection]
FACTOR_DIRECTION_UNSPECIFIED: FactorDirection
FACTOR_DIRECTION_HIGHER_IS_BETTER: FactorDirection
FACTOR_DIRECTION_LOWER_IS_BETTER: FactorDirection

class FieldReference(_message.Message):
    __slots__ = ("field",)
    FIELD_FIELD_NUMBER: _ClassVar[int]
    field: str
    def __init__(self, field: _Optional[str] = ...) -> None: ...

class OperatorReference(_message.Message):
    __slots__ = ("operator", "operator_version")
    OPERATOR_FIELD_NUMBER: _ClassVar[int]
    OPERATOR_VERSION_FIELD_NUMBER: _ClassVar[int]
    operator: str
    operator_version: str
    def __init__(self, operator: _Optional[str] = ..., operator_version: _Optional[str] = ...) -> None: ...

class OperatorCall(_message.Message):
    __slots__ = ("operator", "arguments")
    OPERATOR_FIELD_NUMBER: _ClassVar[int]
    ARGUMENTS_FIELD_NUMBER: _ClassVar[int]
    operator: OperatorReference
    arguments: _containers.RepeatedCompositeFieldContainer[FactorAstNode]
    def __init__(self, operator: _Optional[_Union[OperatorReference, _Mapping]] = ..., arguments: _Optional[_Iterable[_Union[FactorAstNode, _Mapping]]] = ...) -> None: ...

class VersionedEnumLiteral(_message.Message):
    __slots__ = ("enum_type", "value")
    ENUM_TYPE_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    enum_type: str
    value: str
    def __init__(self, enum_type: _Optional[str] = ..., value: _Optional[str] = ...) -> None: ...

class FactorLiteral(_message.Message):
    __slots__ = ("decimal", "boolean", "enumeration")
    DECIMAL_FIELD_NUMBER: _ClassVar[int]
    BOOLEAN_FIELD_NUMBER: _ClassVar[int]
    ENUMERATION_FIELD_NUMBER: _ClassVar[int]
    decimal: _common_pb2.ExactDecimal
    boolean: bool
    enumeration: VersionedEnumLiteral
    def __init__(self, decimal: _Optional[_Union[_common_pb2.ExactDecimal, _Mapping]] = ..., boolean: _Optional[bool] = ..., enumeration: _Optional[_Union[VersionedEnumLiteral, _Mapping]] = ...) -> None: ...

class FactorAstNode(_message.Message):
    __slots__ = ("field", "literal", "call")
    FIELD_FIELD_NUMBER: _ClassVar[int]
    LITERAL_FIELD_NUMBER: _ClassVar[int]
    CALL_FIELD_NUMBER: _ClassVar[int]
    field: FieldReference
    literal: FactorLiteral
    call: OperatorCall
    def __init__(self, field: _Optional[_Union[FieldReference, _Mapping]] = ..., literal: _Optional[_Union[FactorLiteral, _Mapping]] = ..., call: _Optional[_Union[OperatorCall, _Mapping]] = ...) -> None: ...

class FactorAst(_message.Message):
    __slots__ = ("schema_version", "root")
    SCHEMA_VERSION_FIELD_NUMBER: _ClassVar[int]
    ROOT_FIELD_NUMBER: _ClassVar[int]
    schema_version: int
    root: FactorAstNode
    def __init__(self, schema_version: _Optional[int] = ..., root: _Optional[_Union[FactorAstNode, _Mapping]] = ...) -> None: ...

class CanonicalFactorAst(_message.Message):
    __slots__ = ("expression_id", "ast", "canonicalization_profile", "canonical_json")
    EXPRESSION_ID_FIELD_NUMBER: _ClassVar[int]
    AST_FIELD_NUMBER: _ClassVar[int]
    CANONICALIZATION_PROFILE_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_JSON_FIELD_NUMBER: _ClassVar[int]
    expression_id: _common_pb2.FactorExpressionId
    ast: FactorAst
    canonicalization_profile: str
    canonical_json: bytes
    def __init__(self, expression_id: _Optional[_Union[_common_pb2.FactorExpressionId, _Mapping]] = ..., ast: _Optional[_Union[FactorAst, _Mapping]] = ..., canonicalization_profile: _Optional[str] = ..., canonical_json: _Optional[bytes] = ...) -> None: ...

class FrozenResearchPolicyReference(_message.Message):
    __slots__ = ("universe_policy", "data_policy", "calendar_policy", "preprocess_policy", "neutralization_policy", "portfolio_policy", "execution_policy", "cost_policy", "evaluation_policy")
    UNIVERSE_POLICY_FIELD_NUMBER: _ClassVar[int]
    DATA_POLICY_FIELD_NUMBER: _ClassVar[int]
    CALENDAR_POLICY_FIELD_NUMBER: _ClassVar[int]
    PREPROCESS_POLICY_FIELD_NUMBER: _ClassVar[int]
    NEUTRALIZATION_POLICY_FIELD_NUMBER: _ClassVar[int]
    PORTFOLIO_POLICY_FIELD_NUMBER: _ClassVar[int]
    EXECUTION_POLICY_FIELD_NUMBER: _ClassVar[int]
    COST_POLICY_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_POLICY_FIELD_NUMBER: _ClassVar[int]
    universe_policy: _common_pb2.PolicyReference
    data_policy: _common_pb2.PolicyReference
    calendar_policy: _common_pb2.PolicyReference
    preprocess_policy: _common_pb2.PolicyReference
    neutralization_policy: _common_pb2.PolicyReference
    portfolio_policy: _common_pb2.PolicyReference
    execution_policy: _common_pb2.PolicyReference
    cost_policy: _common_pb2.PolicyReference
    evaluation_policy: _common_pb2.PolicyReference
    def __init__(self, universe_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., data_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., calendar_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., preprocess_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., neutralization_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., portfolio_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., execution_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., cost_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., evaluation_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ...) -> None: ...

class FactorSpec(_message.Message):
    __slots__ = ("factor_spec_id", "expression_id", "expression", "direction", "frozen_policy", "operator_registry_sha256")
    FACTOR_SPEC_ID_FIELD_NUMBER: _ClassVar[int]
    EXPRESSION_ID_FIELD_NUMBER: _ClassVar[int]
    EXPRESSION_FIELD_NUMBER: _ClassVar[int]
    DIRECTION_FIELD_NUMBER: _ClassVar[int]
    FROZEN_POLICY_FIELD_NUMBER: _ClassVar[int]
    OPERATOR_REGISTRY_SHA256_FIELD_NUMBER: _ClassVar[int]
    factor_spec_id: _common_pb2.FactorSpecId
    expression_id: _common_pb2.FactorExpressionId
    expression: CanonicalFactorAst
    direction: FactorDirection
    frozen_policy: FrozenResearchPolicyReference
    operator_registry_sha256: _common_pb2.Sha256Digest
    def __init__(self, factor_spec_id: _Optional[_Union[_common_pb2.FactorSpecId, _Mapping]] = ..., expression_id: _Optional[_Union[_common_pb2.FactorExpressionId, _Mapping]] = ..., expression: _Optional[_Union[CanonicalFactorAst, _Mapping]] = ..., direction: _Optional[_Union[FactorDirection, str]] = ..., frozen_policy: _Optional[_Union[FrozenResearchPolicyReference, _Mapping]] = ..., operator_registry_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class FactorMetadata(_message.Message):
    __slots__ = ("factor_spec_id", "expression_id", "name", "thesis", "created_at", "created_by", "frozen_at", "frozen_by")
    FACTOR_SPEC_ID_FIELD_NUMBER: _ClassVar[int]
    EXPRESSION_ID_FIELD_NUMBER: _ClassVar[int]
    NAME_FIELD_NUMBER: _ClassVar[int]
    THESIS_FIELD_NUMBER: _ClassVar[int]
    CREATED_AT_FIELD_NUMBER: _ClassVar[int]
    CREATED_BY_FIELD_NUMBER: _ClassVar[int]
    FROZEN_AT_FIELD_NUMBER: _ClassVar[int]
    FROZEN_BY_FIELD_NUMBER: _ClassVar[int]
    factor_spec_id: _common_pb2.FactorSpecId
    expression_id: _common_pb2.FactorExpressionId
    name: str
    thesis: str
    created_at: _timestamp_pb2.Timestamp
    created_by: _common_pb2.Actor
    frozen_at: _timestamp_pb2.Timestamp
    frozen_by: _common_pb2.Actor
    def __init__(self, factor_spec_id: _Optional[_Union[_common_pb2.FactorSpecId, _Mapping]] = ..., expression_id: _Optional[_Union[_common_pb2.FactorExpressionId, _Mapping]] = ..., name: _Optional[str] = ..., thesis: _Optional[str] = ..., created_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., created_by: _Optional[_Union[_common_pb2.Actor, _Mapping]] = ..., frozen_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., frozen_by: _Optional[_Union[_common_pb2.Actor, _Mapping]] = ...) -> None: ...
