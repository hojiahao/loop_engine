import datetime

from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.v1 import common_pb2 as _common_pb2
from loop.v1 import model_pb2 as _model_pb2
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class StreamStarted(_message.Message):
    __slots__ = ("resolution_id",)
    RESOLUTION_ID_FIELD_NUMBER: _ClassVar[int]
    resolution_id: _common_pb2.ModelResolutionId
    def __init__(self, resolution_id: _Optional[_Union[_common_pb2.ModelResolutionId, _Mapping]] = ...) -> None: ...

class TextDelta(_message.Message):
    __slots__ = ("text",)
    TEXT_FIELD_NUMBER: _ClassVar[int]
    text: str
    def __init__(self, text: _Optional[str] = ...) -> None: ...

class ReasoningDelta(_message.Message):
    __slots__ = ("text",)
    TEXT_FIELD_NUMBER: _ClassVar[int]
    text: str
    def __init__(self, text: _Optional[str] = ...) -> None: ...

class ToolCallDelta(_message.Message):
    __slots__ = ("tool_call_id", "tool_name", "arguments_json_fragment")
    TOOL_CALL_ID_FIELD_NUMBER: _ClassVar[int]
    TOOL_NAME_FIELD_NUMBER: _ClassVar[int]
    ARGUMENTS_JSON_FRAGMENT_FIELD_NUMBER: _ClassVar[int]
    tool_call_id: str
    tool_name: str
    arguments_json_fragment: bytes
    def __init__(self, tool_call_id: _Optional[str] = ..., tool_name: _Optional[str] = ..., arguments_json_fragment: _Optional[bytes] = ...) -> None: ...

class ContentDelta(_message.Message):
    __slots__ = ("content_index", "text", "reasoning", "tool_call")
    CONTENT_INDEX_FIELD_NUMBER: _ClassVar[int]
    TEXT_FIELD_NUMBER: _ClassVar[int]
    REASONING_FIELD_NUMBER: _ClassVar[int]
    TOOL_CALL_FIELD_NUMBER: _ClassVar[int]
    content_index: int
    text: TextDelta
    reasoning: ReasoningDelta
    tool_call: ToolCallDelta
    def __init__(self, content_index: _Optional[int] = ..., text: _Optional[_Union[TextDelta, _Mapping]] = ..., reasoning: _Optional[_Union[ReasoningDelta, _Mapping]] = ..., tool_call: _Optional[_Union[ToolCallDelta, _Mapping]] = ...) -> None: ...

class UsageUpdate(_message.Message):
    __slots__ = ("usage",)
    USAGE_FIELD_NUMBER: _ClassVar[int]
    usage: _model_pb2.ModelUsage
    def __init__(self, usage: _Optional[_Union[_model_pb2.ModelUsage, _Mapping]] = ...) -> None: ...

class StreamCompleted(_message.Message):
    __slots__ = ("response",)
    RESPONSE_FIELD_NUMBER: _ClassVar[int]
    response: _model_pb2.ModelResponse
    def __init__(self, response: _Optional[_Union[_model_pb2.ModelResponse, _Mapping]] = ...) -> None: ...

class ModelStreamEvent(_message.Message):
    __slots__ = ("request_id", "sequence", "emitted_at", "started", "content_delta", "usage_update", "completed")
    REQUEST_ID_FIELD_NUMBER: _ClassVar[int]
    SEQUENCE_FIELD_NUMBER: _ClassVar[int]
    EMITTED_AT_FIELD_NUMBER: _ClassVar[int]
    STARTED_FIELD_NUMBER: _ClassVar[int]
    CONTENT_DELTA_FIELD_NUMBER: _ClassVar[int]
    USAGE_UPDATE_FIELD_NUMBER: _ClassVar[int]
    COMPLETED_FIELD_NUMBER: _ClassVar[int]
    request_id: _common_pb2.RequestId
    sequence: int
    emitted_at: _timestamp_pb2.Timestamp
    started: StreamStarted
    content_delta: ContentDelta
    usage_update: UsageUpdate
    completed: StreamCompleted
    def __init__(self, request_id: _Optional[_Union[_common_pb2.RequestId, _Mapping]] = ..., sequence: _Optional[int] = ..., emitted_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., started: _Optional[_Union[StreamStarted, _Mapping]] = ..., content_delta: _Optional[_Union[ContentDelta, _Mapping]] = ..., usage_update: _Optional[_Union[UsageUpdate, _Mapping]] = ..., completed: _Optional[_Union[StreamCompleted, _Mapping]] = ...) -> None: ...
