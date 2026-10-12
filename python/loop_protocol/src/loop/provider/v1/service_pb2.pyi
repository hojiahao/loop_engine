from loop.v1 import common_pb2 as _common_pb2
from loop.v1 import model_pb2 as _model_pb2
from loop.v1 import stream_pb2 as _stream_pb2
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class InvocationState(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    INVOCATION_STATE_UNSPECIFIED: _ClassVar[InvocationState]
    INVOCATION_STATE_ABSENT: _ClassVar[InvocationState]
    INVOCATION_STATE_AMBIGUOUS: _ClassVar[InvocationState]
    INVOCATION_STATE_COMPLETED: _ClassVar[InvocationState]
INVOCATION_STATE_UNSPECIFIED: InvocationState
INVOCATION_STATE_ABSENT: InvocationState
INVOCATION_STATE_AMBIGUOUS: InvocationState
INVOCATION_STATE_COMPLETED: InvocationState

class InvokeModelRequest(_message.Message):
    __slots__ = ("context", "invocation")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    INVOCATION_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    invocation: _model_pb2.ModelInvocation
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., invocation: _Optional[_Union[_model_pb2.ModelInvocation, _Mapping]] = ...) -> None: ...

class InvokeModelResponse(_message.Message):
    __slots__ = ("response",)
    RESPONSE_FIELD_NUMBER: _ClassVar[int]
    response: _model_pb2.ModelResponse
    def __init__(self, response: _Optional[_Union[_model_pb2.ModelResponse, _Mapping]] = ...) -> None: ...

class StreamModelRequest(_message.Message):
    __slots__ = ("context", "invocation")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    INVOCATION_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    invocation: _model_pb2.ModelInvocation
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., invocation: _Optional[_Union[_model_pb2.ModelInvocation, _Mapping]] = ...) -> None: ...

class StreamModelResponse(_message.Message):
    __slots__ = ("event",)
    EVENT_FIELD_NUMBER: _ClassVar[int]
    event: _stream_pb2.ModelStreamEvent
    def __init__(self, event: _Optional[_Union[_stream_pb2.ModelStreamEvent, _Mapping]] = ...) -> None: ...

class LookupInvocationRequest(_message.Message):
    __slots__ = ("context", "original_request_id", "original_idempotency_key", "request_sha256")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    ORIGINAL_REQUEST_ID_FIELD_NUMBER: _ClassVar[int]
    ORIGINAL_IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    REQUEST_SHA256_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    original_request_id: _common_pb2.RequestId
    original_idempotency_key: _common_pb2.IdempotencyKey
    request_sha256: _common_pb2.Sha256Digest
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., original_request_id: _Optional[_Union[_common_pb2.RequestId, _Mapping]] = ..., original_idempotency_key: _Optional[_Union[_common_pb2.IdempotencyKey, _Mapping]] = ..., request_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class LookupInvocationResponse(_message.Message):
    __slots__ = ("state", "response", "reserved_cost")
    STATE_FIELD_NUMBER: _ClassVar[int]
    RESPONSE_FIELD_NUMBER: _ClassVar[int]
    RESERVED_COST_FIELD_NUMBER: _ClassVar[int]
    state: InvocationState
    response: _model_pb2.ModelResponse
    reserved_cost: _common_pb2.Money
    def __init__(self, state: _Optional[_Union[InvocationState, str]] = ..., response: _Optional[_Union[_model_pb2.ModelResponse, _Mapping]] = ..., reserved_cost: _Optional[_Union[_common_pb2.Money, _Mapping]] = ...) -> None: ...
