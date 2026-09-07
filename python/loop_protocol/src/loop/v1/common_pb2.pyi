import datetime

from google.protobuf import timestamp_pb2 as _timestamp_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class ActorKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    ACTOR_KIND_UNSPECIFIED: _ClassVar[ActorKind]
    ACTOR_KIND_HUMAN: _ClassVar[ActorKind]
    ACTOR_KIND_SERVICE: _ClassVar[ActorKind]
    ACTOR_KIND_AGENT: _ClassVar[ActorKind]
    ACTOR_KIND_SCHEDULER: _ClassVar[ActorKind]

class ErrorCategory(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    ERROR_CATEGORY_UNSPECIFIED: _ClassVar[ErrorCategory]
    ERROR_CATEGORY_VALIDATION: _ClassVar[ErrorCategory]
    ERROR_CATEGORY_AUTHENTICATION: _ClassVar[ErrorCategory]
    ERROR_CATEGORY_AUTHORIZATION: _ClassVar[ErrorCategory]
    ERROR_CATEGORY_NOT_FOUND: _ClassVar[ErrorCategory]
    ERROR_CATEGORY_CONFLICT: _ClassVar[ErrorCategory]
    ERROR_CATEGORY_RATE_LIMIT: _ClassVar[ErrorCategory]
    ERROR_CATEGORY_TIMEOUT: _ClassVar[ErrorCategory]
    ERROR_CATEGORY_CANCELLED: _ClassVar[ErrorCategory]
    ERROR_CATEGORY_DEPENDENCY: _ClassVar[ErrorCategory]
    ERROR_CATEGORY_INTERNAL: _ClassVar[ErrorCategory]
    ERROR_CATEGORY_BUDGET_EXHAUSTED: _ClassVar[ErrorCategory]

class VcsObjectAlgorithm(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    VCS_OBJECT_ALGORITHM_UNSPECIFIED: _ClassVar[VcsObjectAlgorithm]
    VCS_OBJECT_ALGORITHM_SHA1: _ClassVar[VcsObjectAlgorithm]
    VCS_OBJECT_ALGORITHM_SHA256: _ClassVar[VcsObjectAlgorithm]
ACTOR_KIND_UNSPECIFIED: ActorKind
ACTOR_KIND_HUMAN: ActorKind
ACTOR_KIND_SERVICE: ActorKind
ACTOR_KIND_AGENT: ActorKind
ACTOR_KIND_SCHEDULER: ActorKind
ERROR_CATEGORY_UNSPECIFIED: ErrorCategory
ERROR_CATEGORY_VALIDATION: ErrorCategory
ERROR_CATEGORY_AUTHENTICATION: ErrorCategory
ERROR_CATEGORY_AUTHORIZATION: ErrorCategory
ERROR_CATEGORY_NOT_FOUND: ErrorCategory
ERROR_CATEGORY_CONFLICT: ErrorCategory
ERROR_CATEGORY_RATE_LIMIT: ErrorCategory
ERROR_CATEGORY_TIMEOUT: ErrorCategory
ERROR_CATEGORY_CANCELLED: ErrorCategory
ERROR_CATEGORY_DEPENDENCY: ErrorCategory
ERROR_CATEGORY_INTERNAL: ErrorCategory
ERROR_CATEGORY_BUDGET_EXHAUSTED: ErrorCategory
VCS_OBJECT_ALGORITHM_UNSPECIFIED: VcsObjectAlgorithm
VCS_OBJECT_ALGORITHM_SHA1: VcsObjectAlgorithm
VCS_OBJECT_ALGORITHM_SHA256: VcsObjectAlgorithm

class ActorId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class ArtifactId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class AuditEventId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class AuditLedgerId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class BacktestId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class CorrelationId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class CausationId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class ExperimentId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class FactorExpressionId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class FactorSpecId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class HoldoutGrantId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class HoldoutEvaluationPlanId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class HoldoutApprovalRecordId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class HoldoutPeriodId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class IdempotencyKey(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class JobId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class JobBatchId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class LeaseId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class ModelId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class ModelResolutionId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class PolicyId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class PageToken(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class ProviderId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class ProviderContinuationId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class RequestId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class RunId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class SecurityId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class SnapshotId(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class Sha256Digest(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: bytes
    def __init__(self, value: _Optional[bytes] = ...) -> None: ...

class VcsObjectId(_message.Message):
    __slots__ = ("algorithm", "value")
    ALGORITHM_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    algorithm: VcsObjectAlgorithm
    value: bytes
    def __init__(self, algorithm: _Optional[_Union[VcsObjectAlgorithm, str]] = ..., value: _Optional[bytes] = ...) -> None: ...

class ExactDecimal(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class Money(_message.Message):
    __slots__ = ("amount", "currency_code")
    AMOUNT_FIELD_NUMBER: _ClassVar[int]
    CURRENCY_CODE_FIELD_NUMBER: _ClassVar[int]
    amount: ExactDecimal
    currency_code: str
    def __init__(self, amount: _Optional[_Union[ExactDecimal, _Mapping]] = ..., currency_code: _Optional[str] = ...) -> None: ...

class CivilDate(_message.Message):
    __slots__ = ("year", "month", "day")
    YEAR_FIELD_NUMBER: _ClassVar[int]
    MONTH_FIELD_NUMBER: _ClassVar[int]
    DAY_FIELD_NUMBER: _ClassVar[int]
    year: int
    month: int
    day: int
    def __init__(self, year: _Optional[int] = ..., month: _Optional[int] = ..., day: _Optional[int] = ...) -> None: ...

class Actor(_message.Message):
    __slots__ = ("actor_id", "kind", "display_name", "authenticated_subject")
    ACTOR_ID_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    DISPLAY_NAME_FIELD_NUMBER: _ClassVar[int]
    AUTHENTICATED_SUBJECT_FIELD_NUMBER: _ClassVar[int]
    actor_id: ActorId
    kind: ActorKind
    display_name: str
    authenticated_subject: str
    def __init__(self, actor_id: _Optional[_Union[ActorId, _Mapping]] = ..., kind: _Optional[_Union[ActorKind, str]] = ..., display_name: _Optional[str] = ..., authenticated_subject: _Optional[str] = ...) -> None: ...

class PolicyReference(_message.Message):
    __slots__ = ("policy_id", "revision", "sha256")
    POLICY_ID_FIELD_NUMBER: _ClassVar[int]
    REVISION_FIELD_NUMBER: _ClassVar[int]
    SHA256_FIELD_NUMBER: _ClassVar[int]
    policy_id: PolicyId
    revision: str
    sha256: Sha256Digest
    def __init__(self, policy_id: _Optional[_Union[PolicyId, _Mapping]] = ..., revision: _Optional[str] = ..., sha256: _Optional[_Union[Sha256Digest, _Mapping]] = ...) -> None: ...

class PageRequest(_message.Message):
    __slots__ = ("page_size", "page_token")
    PAGE_SIZE_FIELD_NUMBER: _ClassVar[int]
    PAGE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    page_size: int
    page_token: PageToken
    def __init__(self, page_size: _Optional[int] = ..., page_token: _Optional[_Union[PageToken, _Mapping]] = ...) -> None: ...

class PageInfo(_message.Message):
    __slots__ = ("next_page_token",)
    NEXT_PAGE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    next_page_token: PageToken
    def __init__(self, next_page_token: _Optional[_Union[PageToken, _Mapping]] = ...) -> None: ...

class ProtocolLimits(_message.Message):
    __slots__ = ("maximum_unary_bytes", "maximum_stream_event_bytes", "maximum_canonical_ast_bytes", "maximum_ast_nodes", "maximum_ast_depth", "maximum_page_records", "maximum_identity_bytes", "maximum_artifact_uri_bytes")
    MAXIMUM_UNARY_BYTES_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_STREAM_EVENT_BYTES_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_CANONICAL_AST_BYTES_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_AST_NODES_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_AST_DEPTH_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_PAGE_RECORDS_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_IDENTITY_BYTES_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_ARTIFACT_URI_BYTES_FIELD_NUMBER: _ClassVar[int]
    maximum_unary_bytes: int
    maximum_stream_event_bytes: int
    maximum_canonical_ast_bytes: int
    maximum_ast_nodes: int
    maximum_ast_depth: int
    maximum_page_records: int
    maximum_identity_bytes: int
    maximum_artifact_uri_bytes: int
    def __init__(self, maximum_unary_bytes: _Optional[int] = ..., maximum_stream_event_bytes: _Optional[int] = ..., maximum_canonical_ast_bytes: _Optional[int] = ..., maximum_ast_nodes: _Optional[int] = ..., maximum_ast_depth: _Optional[int] = ..., maximum_page_records: _Optional[int] = ..., maximum_identity_bytes: _Optional[int] = ..., maximum_artifact_uri_bytes: _Optional[int] = ...) -> None: ...

class ProtocolInfo(_message.Message):
    __slots__ = ("supported_packages", "features", "limits", "build_version", "build_sha256")
    SUPPORTED_PACKAGES_FIELD_NUMBER: _ClassVar[int]
    FEATURES_FIELD_NUMBER: _ClassVar[int]
    LIMITS_FIELD_NUMBER: _ClassVar[int]
    BUILD_VERSION_FIELD_NUMBER: _ClassVar[int]
    BUILD_SHA256_FIELD_NUMBER: _ClassVar[int]
    supported_packages: _containers.RepeatedScalarFieldContainer[str]
    features: _containers.RepeatedScalarFieldContainer[str]
    limits: ProtocolLimits
    build_version: str
    build_sha256: Sha256Digest
    def __init__(self, supported_packages: _Optional[_Iterable[str]] = ..., features: _Optional[_Iterable[str]] = ..., limits: _Optional[_Union[ProtocolLimits, _Mapping]] = ..., build_version: _Optional[str] = ..., build_sha256: _Optional[_Union[Sha256Digest, _Mapping]] = ...) -> None: ...

class ProtocolSelectionSnapshot(_message.Message):
    __slots__ = ("selected_package", "enabled_features", "effective_limits", "server_build_version", "server_build_sha256", "schema_descriptor_sha256", "selection_sha256", "selected_at", "client_build_version", "client_build_sha256")
    SELECTED_PACKAGE_FIELD_NUMBER: _ClassVar[int]
    ENABLED_FEATURES_FIELD_NUMBER: _ClassVar[int]
    EFFECTIVE_LIMITS_FIELD_NUMBER: _ClassVar[int]
    SERVER_BUILD_VERSION_FIELD_NUMBER: _ClassVar[int]
    SERVER_BUILD_SHA256_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_DESCRIPTOR_SHA256_FIELD_NUMBER: _ClassVar[int]
    SELECTION_SHA256_FIELD_NUMBER: _ClassVar[int]
    SELECTED_AT_FIELD_NUMBER: _ClassVar[int]
    CLIENT_BUILD_VERSION_FIELD_NUMBER: _ClassVar[int]
    CLIENT_BUILD_SHA256_FIELD_NUMBER: _ClassVar[int]
    selected_package: str
    enabled_features: _containers.RepeatedScalarFieldContainer[str]
    effective_limits: ProtocolLimits
    server_build_version: str
    server_build_sha256: Sha256Digest
    schema_descriptor_sha256: Sha256Digest
    selection_sha256: Sha256Digest
    selected_at: _timestamp_pb2.Timestamp
    client_build_version: str
    client_build_sha256: Sha256Digest
    def __init__(self, selected_package: _Optional[str] = ..., enabled_features: _Optional[_Iterable[str]] = ..., effective_limits: _Optional[_Union[ProtocolLimits, _Mapping]] = ..., server_build_version: _Optional[str] = ..., server_build_sha256: _Optional[_Union[Sha256Digest, _Mapping]] = ..., schema_descriptor_sha256: _Optional[_Union[Sha256Digest, _Mapping]] = ..., selection_sha256: _Optional[_Union[Sha256Digest, _Mapping]] = ..., selected_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., client_build_version: _Optional[str] = ..., client_build_sha256: _Optional[_Union[Sha256Digest, _Mapping]] = ...) -> None: ...

class CommandContext(_message.Message):
    __slots__ = ("request_id", "correlation_id", "causation_id", "idempotency_key", "actor", "requested_at")
    REQUEST_ID_FIELD_NUMBER: _ClassVar[int]
    CORRELATION_ID_FIELD_NUMBER: _ClassVar[int]
    CAUSATION_ID_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    ACTOR_FIELD_NUMBER: _ClassVar[int]
    REQUESTED_AT_FIELD_NUMBER: _ClassVar[int]
    request_id: RequestId
    correlation_id: CorrelationId
    causation_id: CausationId
    idempotency_key: IdempotencyKey
    actor: Actor
    requested_at: _timestamp_pb2.Timestamp
    def __init__(self, request_id: _Optional[_Union[RequestId, _Mapping]] = ..., correlation_id: _Optional[_Union[CorrelationId, _Mapping]] = ..., causation_id: _Optional[_Union[CausationId, _Mapping]] = ..., idempotency_key: _Optional[_Union[IdempotencyKey, _Mapping]] = ..., actor: _Optional[_Union[Actor, _Mapping]] = ..., requested_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class ErrorDetail(_message.Message):
    __slots__ = ("field_path", "code", "message")
    FIELD_PATH_FIELD_NUMBER: _ClassVar[int]
    CODE_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_FIELD_NUMBER: _ClassVar[int]
    field_path: str
    code: str
    message: str
    def __init__(self, field_path: _Optional[str] = ..., code: _Optional[str] = ..., message: _Optional[str] = ...) -> None: ...

class ServiceError(_message.Message):
    __slots__ = ("category", "code", "message", "retryable", "details")
    CATEGORY_FIELD_NUMBER: _ClassVar[int]
    CODE_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_FIELD_NUMBER: _ClassVar[int]
    RETRYABLE_FIELD_NUMBER: _ClassVar[int]
    DETAILS_FIELD_NUMBER: _ClassVar[int]
    category: ErrorCategory
    code: str
    message: str
    retryable: bool
    details: _containers.RepeatedCompositeFieldContainer[ErrorDetail]
    def __init__(self, category: _Optional[_Union[ErrorCategory, str]] = ..., code: _Optional[str] = ..., message: _Optional[str] = ..., retryable: _Optional[bool] = ..., details: _Optional[_Iterable[_Union[ErrorDetail, _Mapping]]] = ...) -> None: ...
