import datetime

from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.v1 import artifact_pb2 as _artifact_pb2
from loop.v1 import common_pb2 as _common_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class SampleRole(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    SAMPLE_ROLE_UNSPECIFIED: _ClassVar[SampleRole]
    SAMPLE_ROLE_OPERATOR_WARMUP: _ClassVar[SampleRole]
    SAMPLE_ROLE_IN_SAMPLE: _ClassVar[SampleRole]
    SAMPLE_ROLE_DEVELOPMENT_VALIDATION: _ClassVar[SampleRole]
    SAMPLE_ROLE_FIRST_LOCKED_CONFIRMATION: _ClassVar[SampleRole]
    SAMPLE_ROLE_SECOND_LOCKED_HISTORICAL_HOLDOUT: _ClassVar[SampleRole]
    SAMPLE_ROLE_TEMPORAL_ISOLATION: _ClassVar[SampleRole]
    SAMPLE_ROLE_PROSPECTIVE_OBSERVATION: _ClassVar[SampleRole]

class DataQualityLevel(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    DATA_QUALITY_LEVEL_UNSPECIFIED: _ClassVar[DataQualityLevel]
    DATA_QUALITY_LEVEL_SYNTHETIC: _ClassVar[DataQualityLevel]
    DATA_QUALITY_LEVEL_PUBLIC_DEVELOPMENT: _ClassVar[DataQualityLevel]
    DATA_QUALITY_LEVEL_LICENSED_RESEARCH: _ClassVar[DataQualityLevel]
    DATA_QUALITY_LEVEL_PRODUCTION: _ClassVar[DataQualityLevel]
SAMPLE_ROLE_UNSPECIFIED: SampleRole
SAMPLE_ROLE_OPERATOR_WARMUP: SampleRole
SAMPLE_ROLE_IN_SAMPLE: SampleRole
SAMPLE_ROLE_DEVELOPMENT_VALIDATION: SampleRole
SAMPLE_ROLE_FIRST_LOCKED_CONFIRMATION: SampleRole
SAMPLE_ROLE_SECOND_LOCKED_HISTORICAL_HOLDOUT: SampleRole
SAMPLE_ROLE_TEMPORAL_ISOLATION: SampleRole
SAMPLE_ROLE_PROSPECTIVE_OBSERVATION: SampleRole
DATA_QUALITY_LEVEL_UNSPECIFIED: DataQualityLevel
DATA_QUALITY_LEVEL_SYNTHETIC: DataQualityLevel
DATA_QUALITY_LEVEL_PUBLIC_DEVELOPMENT: DataQualityLevel
DATA_QUALITY_LEVEL_LICENSED_RESEARCH: DataQualityLevel
DATA_QUALITY_LEVEL_PRODUCTION: DataQualityLevel

class SampleWindow(_message.Message):
    __slots__ = ("role", "start_inclusive", "end_inclusive")
    ROLE_FIELD_NUMBER: _ClassVar[int]
    START_INCLUSIVE_FIELD_NUMBER: _ClassVar[int]
    END_INCLUSIVE_FIELD_NUMBER: _ClassVar[int]
    role: SampleRole
    start_inclusive: _common_pb2.CivilDate
    end_inclusive: _common_pb2.CivilDate
    def __init__(self, role: _Optional[_Union[SampleRole, str]] = ..., start_inclusive: _Optional[_Union[_common_pb2.CivilDate, _Mapping]] = ..., end_inclusive: _Optional[_Union[_common_pb2.CivilDate, _Mapping]] = ...) -> None: ...

class SecurityReference(_message.Message):
    __slots__ = ("security_id", "ticker", "listing_venue", "ticker_effective_from", "ticker_effective_through")
    SECURITY_ID_FIELD_NUMBER: _ClassVar[int]
    TICKER_FIELD_NUMBER: _ClassVar[int]
    LISTING_VENUE_FIELD_NUMBER: _ClassVar[int]
    TICKER_EFFECTIVE_FROM_FIELD_NUMBER: _ClassVar[int]
    TICKER_EFFECTIVE_THROUGH_FIELD_NUMBER: _ClassVar[int]
    security_id: _common_pb2.SecurityId
    ticker: str
    listing_venue: str
    ticker_effective_from: _common_pb2.CivilDate
    ticker_effective_through: _common_pb2.CivilDate
    def __init__(self, security_id: _Optional[_Union[_common_pb2.SecurityId, _Mapping]] = ..., ticker: _Optional[str] = ..., listing_venue: _Optional[str] = ..., ticker_effective_from: _Optional[_Union[_common_pb2.CivilDate, _Mapping]] = ..., ticker_effective_through: _Optional[_Union[_common_pb2.CivilDate, _Mapping]] = ...) -> None: ...

class BitemporalInterval(_message.Message):
    __slots__ = ("effective_at", "known_at", "ingested_at")
    EFFECTIVE_AT_FIELD_NUMBER: _ClassVar[int]
    KNOWN_AT_FIELD_NUMBER: _ClassVar[int]
    INGESTED_AT_FIELD_NUMBER: _ClassVar[int]
    effective_at: _timestamp_pb2.Timestamp
    known_at: _timestamp_pb2.Timestamp
    ingested_at: _timestamp_pb2.Timestamp
    def __init__(self, effective_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., known_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., ingested_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class DataSourceReference(_message.Message):
    __slots__ = ("source_name", "dataset_name", "entitlement_level", "source_revision", "extraction_configuration_sha256")
    SOURCE_NAME_FIELD_NUMBER: _ClassVar[int]
    DATASET_NAME_FIELD_NUMBER: _ClassVar[int]
    ENTITLEMENT_LEVEL_FIELD_NUMBER: _ClassVar[int]
    SOURCE_REVISION_FIELD_NUMBER: _ClassVar[int]
    EXTRACTION_CONFIGURATION_SHA256_FIELD_NUMBER: _ClassVar[int]
    source_name: str
    dataset_name: str
    entitlement_level: str
    source_revision: str
    extraction_configuration_sha256: _common_pb2.Sha256Digest
    def __init__(self, source_name: _Optional[str] = ..., dataset_name: _Optional[str] = ..., entitlement_level: _Optional[str] = ..., source_revision: _Optional[str] = ..., extraction_configuration_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class DataSnapshot(_message.Message):
    __slots__ = ("snapshot_id", "schema_version", "source", "created_at", "known_through", "window", "quality_level", "artifacts", "manifest_sha256", "trading_calendar_sha256", "parent_snapshot_ids")
    SNAPSHOT_ID_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_VERSION_FIELD_NUMBER: _ClassVar[int]
    SOURCE_FIELD_NUMBER: _ClassVar[int]
    CREATED_AT_FIELD_NUMBER: _ClassVar[int]
    KNOWN_THROUGH_FIELD_NUMBER: _ClassVar[int]
    WINDOW_FIELD_NUMBER: _ClassVar[int]
    QUALITY_LEVEL_FIELD_NUMBER: _ClassVar[int]
    ARTIFACTS_FIELD_NUMBER: _ClassVar[int]
    MANIFEST_SHA256_FIELD_NUMBER: _ClassVar[int]
    TRADING_CALENDAR_SHA256_FIELD_NUMBER: _ClassVar[int]
    PARENT_SNAPSHOT_IDS_FIELD_NUMBER: _ClassVar[int]
    snapshot_id: _common_pb2.SnapshotId
    schema_version: int
    source: DataSourceReference
    created_at: _timestamp_pb2.Timestamp
    known_through: _timestamp_pb2.Timestamp
    window: SampleWindow
    quality_level: DataQualityLevel
    artifacts: _containers.RepeatedCompositeFieldContainer[_artifact_pb2.ArtifactRef]
    manifest_sha256: _common_pb2.Sha256Digest
    trading_calendar_sha256: _common_pb2.Sha256Digest
    parent_snapshot_ids: _containers.RepeatedCompositeFieldContainer[_common_pb2.SnapshotId]
    def __init__(self, snapshot_id: _Optional[_Union[_common_pb2.SnapshotId, _Mapping]] = ..., schema_version: _Optional[int] = ..., source: _Optional[_Union[DataSourceReference, _Mapping]] = ..., created_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., known_through: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., window: _Optional[_Union[SampleWindow, _Mapping]] = ..., quality_level: _Optional[_Union[DataQualityLevel, str]] = ..., artifacts: _Optional[_Iterable[_Union[_artifact_pb2.ArtifactRef, _Mapping]]] = ..., manifest_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., trading_calendar_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., parent_snapshot_ids: _Optional[_Iterable[_Union[_common_pb2.SnapshotId, _Mapping]]] = ...) -> None: ...
