from loop.v1 import common_pb2 as _common_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class DevelopmentDatasetReference(_message.Message):
    __slots__ = ("snapshot_ids", "manifest_sha256")
    SNAPSHOT_IDS_FIELD_NUMBER: _ClassVar[int]
    MANIFEST_SHA256_FIELD_NUMBER: _ClassVar[int]
    snapshot_ids: _containers.RepeatedCompositeFieldContainer[_common_pb2.SnapshotId]
    manifest_sha256: _common_pb2.Sha256Digest
    def __init__(self, snapshot_ids: _Optional[_Iterable[_Union[_common_pb2.SnapshotId, _Mapping]]] = ..., manifest_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...
