from loop.v1 import common_pb2 as _common_pb2
from loop.v1 import model_pb2 as _model_pb2
from loop.v1 import stream_pb2 as _stream_pb2
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

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
