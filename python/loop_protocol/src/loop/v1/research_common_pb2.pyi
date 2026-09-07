from loop.v1 import common_pb2 as _common_pb2
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class ReturnDefinition(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    RETURN_DEFINITION_UNSPECIFIED: _ClassVar[ReturnDefinition]
    RETURN_DEFINITION_SIMPLE_NAV_RETURN: _ClassVar[ReturnDefinition]
RETURN_DEFINITION_UNSPECIFIED: ReturnDefinition
RETURN_DEFINITION_SIMPLE_NAV_RETURN: ReturnDefinition

class ResearchProvenanceFingerprint(_message.Message):
    __slots__ = ("source_code_sha256", "operator_registry_sha256", "configuration_sha256", "data_manifest_sha256", "trading_calendar_sha256", "environment_sha256")
    SOURCE_CODE_SHA256_FIELD_NUMBER: _ClassVar[int]
    OPERATOR_REGISTRY_SHA256_FIELD_NUMBER: _ClassVar[int]
    CONFIGURATION_SHA256_FIELD_NUMBER: _ClassVar[int]
    DATA_MANIFEST_SHA256_FIELD_NUMBER: _ClassVar[int]
    TRADING_CALENDAR_SHA256_FIELD_NUMBER: _ClassVar[int]
    ENVIRONMENT_SHA256_FIELD_NUMBER: _ClassVar[int]
    source_code_sha256: _common_pb2.Sha256Digest
    operator_registry_sha256: _common_pb2.Sha256Digest
    configuration_sha256: _common_pb2.Sha256Digest
    data_manifest_sha256: _common_pb2.Sha256Digest
    trading_calendar_sha256: _common_pb2.Sha256Digest
    environment_sha256: _common_pb2.Sha256Digest
    def __init__(self, source_code_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., operator_registry_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., configuration_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., data_manifest_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., trading_calendar_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., environment_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...
