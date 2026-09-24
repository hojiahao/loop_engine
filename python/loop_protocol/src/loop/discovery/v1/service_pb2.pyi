import datetime

from google.protobuf import duration_pb2 as _duration_pb2
from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.v1 import common_pb2 as _common_pb2
from loop.v1 import development_data_pb2 as _development_data_pb2
from loop.v1 import model_pb2 as _model_pb2
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class DiscoveryJobStatus(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    DISCOVERY_JOB_STATUS_UNSPECIFIED: _ClassVar[DiscoveryJobStatus]
    DISCOVERY_JOB_STATUS_QUEUED: _ClassVar[DiscoveryJobStatus]
    DISCOVERY_JOB_STATUS_LEASED: _ClassVar[DiscoveryJobStatus]
    DISCOVERY_JOB_STATUS_RUNNING: _ClassVar[DiscoveryJobStatus]
    DISCOVERY_JOB_STATUS_SUCCEEDED: _ClassVar[DiscoveryJobStatus]
    DISCOVERY_JOB_STATUS_FACTOR_REJECTED: _ClassVar[DiscoveryJobStatus]
    DISCOVERY_JOB_STATUS_INFRASTRUCTURE_FAILED: _ClassVar[DiscoveryJobStatus]
    DISCOVERY_JOB_STATUS_CANCELLED: _ClassVar[DiscoveryJobStatus]
    DISCOVERY_JOB_STATUS_BUDGET_EXHAUSTED: _ClassVar[DiscoveryJobStatus]
DISCOVERY_JOB_STATUS_UNSPECIFIED: DiscoveryJobStatus
DISCOVERY_JOB_STATUS_QUEUED: DiscoveryJobStatus
DISCOVERY_JOB_STATUS_LEASED: DiscoveryJobStatus
DISCOVERY_JOB_STATUS_RUNNING: DiscoveryJobStatus
DISCOVERY_JOB_STATUS_SUCCEEDED: DiscoveryJobStatus
DISCOVERY_JOB_STATUS_FACTOR_REJECTED: DiscoveryJobStatus
DISCOVERY_JOB_STATUS_INFRASTRUCTURE_FAILED: DiscoveryJobStatus
DISCOVERY_JOB_STATUS_CANCELLED: DiscoveryJobStatus
DISCOVERY_JOB_STATUS_BUDGET_EXHAUSTED: DiscoveryJobStatus

class DiscoveryJobBudget(_message.Message):
    __slots__ = ("maximum_steps", "maximum_input_tokens", "maximum_output_tokens", "maximum_cost", "maximum_wall_time")
    MAXIMUM_STEPS_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_INPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_COST_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_WALL_TIME_FIELD_NUMBER: _ClassVar[int]
    maximum_steps: int
    maximum_input_tokens: int
    maximum_output_tokens: int
    maximum_cost: _common_pb2.Money
    maximum_wall_time: _duration_pb2.Duration
    def __init__(self, maximum_steps: _Optional[int] = ..., maximum_input_tokens: _Optional[int] = ..., maximum_output_tokens: _Optional[int] = ..., maximum_cost: _Optional[_Union[_common_pb2.Money, _Mapping]] = ..., maximum_wall_time: _Optional[_Union[datetime.timedelta, _duration_pb2.Duration, _Mapping]] = ...) -> None: ...

class DiscoveryJobInput(_message.Message):
    __slots__ = ("dataset", "research_policy", "maker_model", "checker_model", "budget", "maximum_candidates")
    DATASET_FIELD_NUMBER: _ClassVar[int]
    RESEARCH_POLICY_FIELD_NUMBER: _ClassVar[int]
    MAKER_MODEL_FIELD_NUMBER: _ClassVar[int]
    CHECKER_MODEL_FIELD_NUMBER: _ClassVar[int]
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_CANDIDATES_FIELD_NUMBER: _ClassVar[int]
    dataset: _development_data_pb2.DevelopmentDatasetReference
    research_policy: _common_pb2.PolicyReference
    maker_model: _model_pb2.ModelResolutionSnapshot
    checker_model: _model_pb2.ModelResolutionSnapshot
    budget: DiscoveryJobBudget
    maximum_candidates: int
    def __init__(self, dataset: _Optional[_Union[_development_data_pb2.DevelopmentDatasetReference, _Mapping]] = ..., research_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., maker_model: _Optional[_Union[_model_pb2.ModelResolutionSnapshot, _Mapping]] = ..., checker_model: _Optional[_Union[_model_pb2.ModelResolutionSnapshot, _Mapping]] = ..., budget: _Optional[_Union[DiscoveryJobBudget, _Mapping]] = ..., maximum_candidates: _Optional[int] = ...) -> None: ...

class StartDiscoveryRequest(_message.Message):
    __slots__ = ("context", "discovery")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    DISCOVERY_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    discovery: DiscoveryJobInput
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., discovery: _Optional[_Union[DiscoveryJobInput, _Mapping]] = ...) -> None: ...

class DiscoveryJobHandle(_message.Message):
    __slots__ = ("job_id", "status", "revision", "submitted_at", "updated_at")
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    STATUS_FIELD_NUMBER: _ClassVar[int]
    REVISION_FIELD_NUMBER: _ClassVar[int]
    SUBMITTED_AT_FIELD_NUMBER: _ClassVar[int]
    UPDATED_AT_FIELD_NUMBER: _ClassVar[int]
    job_id: _common_pb2.JobId
    status: DiscoveryJobStatus
    revision: int
    submitted_at: _timestamp_pb2.Timestamp
    updated_at: _timestamp_pb2.Timestamp
    def __init__(self, job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., status: _Optional[_Union[DiscoveryJobStatus, str]] = ..., revision: _Optional[int] = ..., submitted_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., updated_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class StartDiscoveryResponse(_message.Message):
    __slots__ = ("job",)
    JOB_FIELD_NUMBER: _ClassVar[int]
    job: DiscoveryJobHandle
    def __init__(self, job: _Optional[_Union[DiscoveryJobHandle, _Mapping]] = ...) -> None: ...
