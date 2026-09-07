import datetime

from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.v1 import common_pb2 as _common_pb2
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class ArtifactSchemaReference(_message.Message):
    __slots__ = ("name", "version", "schema_sha256")
    NAME_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_SHA256_FIELD_NUMBER: _ClassVar[int]
    name: str
    version: int
    schema_sha256: _common_pb2.Sha256Digest
    def __init__(self, name: _Optional[str] = ..., version: _Optional[int] = ..., schema_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class ArtifactRef(_message.Message):
    __slots__ = ("artifact_id", "uri", "sha256", "schema", "media_type", "byte_size", "row_count", "created_at", "manifest_sha256")
    ARTIFACT_ID_FIELD_NUMBER: _ClassVar[int]
    URI_FIELD_NUMBER: _ClassVar[int]
    SHA256_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_FIELD_NUMBER: _ClassVar[int]
    MEDIA_TYPE_FIELD_NUMBER: _ClassVar[int]
    BYTE_SIZE_FIELD_NUMBER: _ClassVar[int]
    ROW_COUNT_FIELD_NUMBER: _ClassVar[int]
    CREATED_AT_FIELD_NUMBER: _ClassVar[int]
    MANIFEST_SHA256_FIELD_NUMBER: _ClassVar[int]
    artifact_id: _common_pb2.ArtifactId
    uri: str
    sha256: _common_pb2.Sha256Digest
    schema: ArtifactSchemaReference
    media_type: str
    byte_size: int
    row_count: int
    created_at: _timestamp_pb2.Timestamp
    manifest_sha256: _common_pb2.Sha256Digest
    def __init__(self, artifact_id: _Optional[_Union[_common_pb2.ArtifactId, _Mapping]] = ..., uri: _Optional[str] = ..., sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., schema: _Optional[_Union[ArtifactSchemaReference, _Mapping]] = ..., media_type: _Optional[str] = ..., byte_size: _Optional[int] = ..., row_count: _Optional[int] = ..., created_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., manifest_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...
