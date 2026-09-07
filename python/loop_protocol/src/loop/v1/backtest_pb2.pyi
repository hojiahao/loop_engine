import datetime

from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.v1 import artifact_pb2 as _artifact_pb2
from loop.v1 import common_pb2 as _common_pb2
from loop.v1 import data_pb2 as _data_pb2
from loop.v1 import research_common_pb2 as _research_common_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class BacktestEngineKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    BACKTEST_ENGINE_KIND_UNSPECIFIED: _ClassVar[BacktestEngineKind]
    BACKTEST_ENGINE_KIND_PRIMARY_CROSS_SECTIONAL: _ClassVar[BacktestEngineKind]
    BACKTEST_ENGINE_KIND_ALPHALENS_VALIDATION: _ClassVar[BacktestEngineKind]
    BACKTEST_ENGINE_KIND_ZIPLINE_VALIDATION: _ClassVar[BacktestEngineKind]

class ReconciliationDisposition(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    RECONCILIATION_DISPOSITION_UNSPECIFIED: _ClassVar[ReconciliationDisposition]
    RECONCILIATION_DISPOSITION_ACCEPTED: _ClassVar[ReconciliationDisposition]
    RECONCILIATION_DISPOSITION_REJECTED: _ClassVar[ReconciliationDisposition]
BACKTEST_ENGINE_KIND_UNSPECIFIED: BacktestEngineKind
BACKTEST_ENGINE_KIND_PRIMARY_CROSS_SECTIONAL: BacktestEngineKind
BACKTEST_ENGINE_KIND_ALPHALENS_VALIDATION: BacktestEngineKind
BACKTEST_ENGINE_KIND_ZIPLINE_VALIDATION: BacktestEngineKind
RECONCILIATION_DISPOSITION_UNSPECIFIED: ReconciliationDisposition
RECONCILIATION_DISPOSITION_ACCEPTED: ReconciliationDisposition
RECONCILIATION_DISPOSITION_REJECTED: ReconciliationDisposition

class BacktestSpec(_message.Message):
    __slots__ = ("backtest_id", "schema_version", "factor_spec_id", "snapshot_ids", "sample", "return_definition", "provenance", "canonical_spec_sha256", "created_at", "deterministic_seed")
    BACKTEST_ID_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_VERSION_FIELD_NUMBER: _ClassVar[int]
    FACTOR_SPEC_ID_FIELD_NUMBER: _ClassVar[int]
    SNAPSHOT_IDS_FIELD_NUMBER: _ClassVar[int]
    SAMPLE_FIELD_NUMBER: _ClassVar[int]
    RETURN_DEFINITION_FIELD_NUMBER: _ClassVar[int]
    PROVENANCE_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_SPEC_SHA256_FIELD_NUMBER: _ClassVar[int]
    CREATED_AT_FIELD_NUMBER: _ClassVar[int]
    DETERMINISTIC_SEED_FIELD_NUMBER: _ClassVar[int]
    backtest_id: _common_pb2.BacktestId
    schema_version: int
    factor_spec_id: _common_pb2.FactorSpecId
    snapshot_ids: _containers.RepeatedCompositeFieldContainer[_common_pb2.SnapshotId]
    sample: _data_pb2.SampleWindow
    return_definition: _research_common_pb2.ReturnDefinition
    provenance: _research_common_pb2.ResearchProvenanceFingerprint
    canonical_spec_sha256: _common_pb2.Sha256Digest
    created_at: _timestamp_pb2.Timestamp
    deterministic_seed: _common_pb2.Sha256Digest
    def __init__(self, backtest_id: _Optional[_Union[_common_pb2.BacktestId, _Mapping]] = ..., schema_version: _Optional[int] = ..., factor_spec_id: _Optional[_Union[_common_pb2.FactorSpecId, _Mapping]] = ..., snapshot_ids: _Optional[_Iterable[_Union[_common_pb2.SnapshotId, _Mapping]]] = ..., sample: _Optional[_Union[_data_pb2.SampleWindow, _Mapping]] = ..., return_definition: _Optional[_Union[_research_common_pb2.ReturnDefinition, str]] = ..., provenance: _Optional[_Union[_research_common_pb2.ResearchProvenanceFingerprint, _Mapping]] = ..., canonical_spec_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., created_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., deterministic_seed: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class BacktestMetric(_message.Message):
    __slots__ = ("name", "value", "unit", "estimator")
    NAME_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    UNIT_FIELD_NUMBER: _ClassVar[int]
    ESTIMATOR_FIELD_NUMBER: _ClassVar[int]
    name: str
    value: _common_pb2.ExactDecimal
    unit: str
    estimator: str
    def __init__(self, name: _Optional[str] = ..., value: _Optional[_Union[_common_pb2.ExactDecimal, _Mapping]] = ..., unit: _Optional[str] = ..., estimator: _Optional[str] = ...) -> None: ...

class BacktestArtifacts(_message.Message):
    __slots__ = ("factor_values", "target_positions", "orders", "fills", "nav", "simple_returns", "risk_exposures", "cost_ledger")
    FACTOR_VALUES_FIELD_NUMBER: _ClassVar[int]
    TARGET_POSITIONS_FIELD_NUMBER: _ClassVar[int]
    ORDERS_FIELD_NUMBER: _ClassVar[int]
    FILLS_FIELD_NUMBER: _ClassVar[int]
    NAV_FIELD_NUMBER: _ClassVar[int]
    SIMPLE_RETURNS_FIELD_NUMBER: _ClassVar[int]
    RISK_EXPOSURES_FIELD_NUMBER: _ClassVar[int]
    COST_LEDGER_FIELD_NUMBER: _ClassVar[int]
    factor_values: _artifact_pb2.ArtifactRef
    target_positions: _artifact_pb2.ArtifactRef
    orders: _artifact_pb2.ArtifactRef
    fills: _artifact_pb2.ArtifactRef
    nav: _artifact_pb2.ArtifactRef
    simple_returns: _artifact_pb2.ArtifactRef
    risk_exposures: _artifact_pb2.ArtifactRef
    cost_ledger: _artifact_pb2.ArtifactRef
    def __init__(self, factor_values: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., target_positions: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., orders: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., fills: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., nav: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., simple_returns: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., risk_exposures: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., cost_ledger: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ...) -> None: ...

class BacktestResult(_message.Message):
    __slots__ = ("backtest_id", "engine", "engine_version", "provenance", "metrics", "artifacts", "result_manifest_sha256", "completed_at")
    BACKTEST_ID_FIELD_NUMBER: _ClassVar[int]
    ENGINE_FIELD_NUMBER: _ClassVar[int]
    ENGINE_VERSION_FIELD_NUMBER: _ClassVar[int]
    PROVENANCE_FIELD_NUMBER: _ClassVar[int]
    METRICS_FIELD_NUMBER: _ClassVar[int]
    ARTIFACTS_FIELD_NUMBER: _ClassVar[int]
    RESULT_MANIFEST_SHA256_FIELD_NUMBER: _ClassVar[int]
    COMPLETED_AT_FIELD_NUMBER: _ClassVar[int]
    backtest_id: _common_pb2.BacktestId
    engine: BacktestEngineKind
    engine_version: str
    provenance: _research_common_pb2.ResearchProvenanceFingerprint
    metrics: _containers.RepeatedCompositeFieldContainer[BacktestMetric]
    artifacts: BacktestArtifacts
    result_manifest_sha256: _common_pb2.Sha256Digest
    completed_at: _timestamp_pb2.Timestamp
    def __init__(self, backtest_id: _Optional[_Union[_common_pb2.BacktestId, _Mapping]] = ..., engine: _Optional[_Union[BacktestEngineKind, str]] = ..., engine_version: _Optional[str] = ..., provenance: _Optional[_Union[_research_common_pb2.ResearchProvenanceFingerprint, _Mapping]] = ..., metrics: _Optional[_Iterable[_Union[BacktestMetric, _Mapping]]] = ..., artifacts: _Optional[_Union[BacktestArtifacts, _Mapping]] = ..., result_manifest_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., completed_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class ReconciliationDifference(_message.Message):
    __slots__ = ("field_path", "primary_value", "independent_value", "absolute_tolerance", "explanation")
    FIELD_PATH_FIELD_NUMBER: _ClassVar[int]
    PRIMARY_VALUE_FIELD_NUMBER: _ClassVar[int]
    INDEPENDENT_VALUE_FIELD_NUMBER: _ClassVar[int]
    ABSOLUTE_TOLERANCE_FIELD_NUMBER: _ClassVar[int]
    EXPLANATION_FIELD_NUMBER: _ClassVar[int]
    field_path: str
    primary_value: _common_pb2.ExactDecimal
    independent_value: _common_pb2.ExactDecimal
    absolute_tolerance: _common_pb2.ExactDecimal
    explanation: str
    def __init__(self, field_path: _Optional[str] = ..., primary_value: _Optional[_Union[_common_pb2.ExactDecimal, _Mapping]] = ..., independent_value: _Optional[_Union[_common_pb2.ExactDecimal, _Mapping]] = ..., absolute_tolerance: _Optional[_Union[_common_pb2.ExactDecimal, _Mapping]] = ..., explanation: _Optional[str] = ...) -> None: ...

class BacktestReconciliation(_message.Message):
    __slots__ = ("primary_backtest_id", "independent_backtest_id", "differences", "detailed_report", "reconciliation_sha256", "disposition")
    PRIMARY_BACKTEST_ID_FIELD_NUMBER: _ClassVar[int]
    INDEPENDENT_BACKTEST_ID_FIELD_NUMBER: _ClassVar[int]
    DIFFERENCES_FIELD_NUMBER: _ClassVar[int]
    DETAILED_REPORT_FIELD_NUMBER: _ClassVar[int]
    RECONCILIATION_SHA256_FIELD_NUMBER: _ClassVar[int]
    DISPOSITION_FIELD_NUMBER: _ClassVar[int]
    primary_backtest_id: _common_pb2.BacktestId
    independent_backtest_id: _common_pb2.BacktestId
    differences: _containers.RepeatedCompositeFieldContainer[ReconciliationDifference]
    detailed_report: _artifact_pb2.ArtifactRef
    reconciliation_sha256: _common_pb2.Sha256Digest
    disposition: ReconciliationDisposition
    def __init__(self, primary_backtest_id: _Optional[_Union[_common_pb2.BacktestId, _Mapping]] = ..., independent_backtest_id: _Optional[_Union[_common_pb2.BacktestId, _Mapping]] = ..., differences: _Optional[_Iterable[_Union[ReconciliationDifference, _Mapping]]] = ..., detailed_report: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., reconciliation_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., disposition: _Optional[_Union[ReconciliationDisposition, str]] = ...) -> None: ...
