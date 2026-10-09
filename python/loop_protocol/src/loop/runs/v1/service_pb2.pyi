import datetime

from google.protobuf import duration_pb2 as _duration_pb2
from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.discovery.v1 import service_pb2 as _service_pb2
from loop.v1 import common_pb2 as _common_pb2
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class RunStatus(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    RUN_STATUS_UNSPECIFIED: _ClassVar[RunStatus]
    RUN_STATUS_ACTIVE: _ClassVar[RunStatus]
    RUN_STATUS_COMPLETED: _ClassVar[RunStatus]
    RUN_STATUS_BUDGET_EXHAUSTED: _ClassVar[RunStatus]
    RUN_STATUS_INFRASTRUCTURE_FAILED: _ClassVar[RunStatus]
    RUN_STATUS_DEADLINE_EXCEEDED: _ClassVar[RunStatus]
RUN_STATUS_UNSPECIFIED: RunStatus
RUN_STATUS_ACTIVE: RunStatus
RUN_STATUS_COMPLETED: RunStatus
RUN_STATUS_BUDGET_EXHAUSTED: RunStatus
RUN_STATUS_INFRASTRUCTURE_FAILED: RunStatus
RUN_STATUS_DEADLINE_EXCEEDED: RunStatus

class RunBudget(_message.Message):
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

class RunSpecification(_message.Message):
    __slots__ = ("plan", "run_id", "owner", "executor", "discovery", "protocol_selection", "budget", "maximum_rounds")
    PLAN_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    OWNER_FIELD_NUMBER: _ClassVar[int]
    EXECUTOR_FIELD_NUMBER: _ClassVar[int]
    DISCOVERY_FIELD_NUMBER: _ClassVar[int]
    PROTOCOL_SELECTION_FIELD_NUMBER: _ClassVar[int]
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_ROUNDS_FIELD_NUMBER: _ClassVar[int]
    plan: _common_pb2.PolicyReference
    run_id: _common_pb2.RunId
    owner: _common_pb2.Actor
    executor: _common_pb2.Actor
    discovery: _service_pb2.DiscoveryJobInput
    protocol_selection: _common_pb2.ProtocolSelectionSnapshot
    budget: RunBudget
    maximum_rounds: int
    def __init__(self, plan: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., run_id: _Optional[_Union[_common_pb2.RunId, _Mapping]] = ..., owner: _Optional[_Union[_common_pb2.Actor, _Mapping]] = ..., executor: _Optional[_Union[_common_pb2.Actor, _Mapping]] = ..., discovery: _Optional[_Union[_service_pb2.DiscoveryJobInput, _Mapping]] = ..., protocol_selection: _Optional[_Union[_common_pb2.ProtocolSelectionSnapshot, _Mapping]] = ..., budget: _Optional[_Union[RunBudget, _Mapping]] = ..., maximum_rounds: _Optional[int] = ...) -> None: ...

class RunView(_message.Message):
    __slots__ = ("run_id", "status", "revision", "maximum_rounds", "completed_rounds", "current_job", "budget", "reserved_steps", "reserved_input_tokens", "reserved_output_tokens", "reserved_cost", "submitted_at", "updated_at", "deadline", "plan_verified")
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STATUS_FIELD_NUMBER: _ClassVar[int]
    REVISION_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_ROUNDS_FIELD_NUMBER: _ClassVar[int]
    COMPLETED_ROUNDS_FIELD_NUMBER: _ClassVar[int]
    CURRENT_JOB_FIELD_NUMBER: _ClassVar[int]
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    RESERVED_STEPS_FIELD_NUMBER: _ClassVar[int]
    RESERVED_INPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    RESERVED_OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    RESERVED_COST_FIELD_NUMBER: _ClassVar[int]
    SUBMITTED_AT_FIELD_NUMBER: _ClassVar[int]
    UPDATED_AT_FIELD_NUMBER: _ClassVar[int]
    DEADLINE_FIELD_NUMBER: _ClassVar[int]
    PLAN_VERIFIED_FIELD_NUMBER: _ClassVar[int]
    run_id: _common_pb2.RunId
    status: RunStatus
    revision: int
    maximum_rounds: int
    completed_rounds: int
    current_job: _service_pb2.DiscoveryJobHandle
    budget: RunBudget
    reserved_steps: int
    reserved_input_tokens: int
    reserved_output_tokens: int
    reserved_cost: _common_pb2.Money
    submitted_at: _timestamp_pb2.Timestamp
    updated_at: _timestamp_pb2.Timestamp
    deadline: _timestamp_pb2.Timestamp
    plan_verified: bool
    def __init__(self, run_id: _Optional[_Union[_common_pb2.RunId, _Mapping]] = ..., status: _Optional[_Union[RunStatus, str]] = ..., revision: _Optional[int] = ..., maximum_rounds: _Optional[int] = ..., completed_rounds: _Optional[int] = ..., current_job: _Optional[_Union[_service_pb2.DiscoveryJobHandle, _Mapping]] = ..., budget: _Optional[_Union[RunBudget, _Mapping]] = ..., reserved_steps: _Optional[int] = ..., reserved_input_tokens: _Optional[int] = ..., reserved_output_tokens: _Optional[int] = ..., reserved_cost: _Optional[_Union[_common_pb2.Money, _Mapping]] = ..., submitted_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., updated_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., deadline: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., plan_verified: _Optional[bool] = ...) -> None: ...

class StartRunRequest(_message.Message):
    __slots__ = ("context", "plan")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    PLAN_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    plan: _common_pb2.PolicyReference
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., plan: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ...) -> None: ...

class StartRunResponse(_message.Message):
    __slots__ = ("run",)
    RUN_FIELD_NUMBER: _ClassVar[int]
    run: RunView
    def __init__(self, run: _Optional[_Union[RunView, _Mapping]] = ...) -> None: ...

class StepRunRequest(_message.Message):
    __slots__ = ("context", "run_id", "expected_revision")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_REVISION_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    run_id: _common_pb2.RunId
    expected_revision: int
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., run_id: _Optional[_Union[_common_pb2.RunId, _Mapping]] = ..., expected_revision: _Optional[int] = ...) -> None: ...

class StepRunResponse(_message.Message):
    __slots__ = ("run",)
    RUN_FIELD_NUMBER: _ClassVar[int]
    run: RunView
    def __init__(self, run: _Optional[_Union[RunView, _Mapping]] = ...) -> None: ...

class GetRunRequest(_message.Message):
    __slots__ = ("context", "run_id")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    run_id: _common_pb2.RunId
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., run_id: _Optional[_Union[_common_pb2.RunId, _Mapping]] = ...) -> None: ...

class GetRunResponse(_message.Message):
    __slots__ = ("run",)
    RUN_FIELD_NUMBER: _ClassVar[int]
    run: RunView
    def __init__(self, run: _Optional[_Union[RunView, _Mapping]] = ...) -> None: ...
