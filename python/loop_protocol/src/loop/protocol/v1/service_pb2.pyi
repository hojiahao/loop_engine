from loop.v1 import common_pb2 as _common_pb2
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class GetProtocolInfoRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class GetProtocolInfoResponse(_message.Message):
    __slots__ = ("protocol",)
    PROTOCOL_FIELD_NUMBER: _ClassVar[int]
    protocol: _common_pb2.ProtocolInfo
    def __init__(self, protocol: _Optional[_Union[_common_pb2.ProtocolInfo, _Mapping]] = ...) -> None: ...
