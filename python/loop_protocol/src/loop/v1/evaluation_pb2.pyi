import datetime

from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.v1 import artifact_pb2 as _artifact_pb2
from loop.v1 import common_pb2 as _common_pb2
from loop.v1 import factor_pb2 as _factor_pb2
from loop.v1 import research_common_pb2 as _research_common_pb2
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class FactorEvaluationWork(_message.Message):
    __slots__ = ("job_id", "lease_id", "factor", "panel_manifest", "sample_start", "sample_end", "provenance", "deterministic_seed")
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    LEASE_ID_FIELD_NUMBER: _ClassVar[int]
    FACTOR_FIELD_NUMBER: _ClassVar[int]
    PANEL_MANIFEST_FIELD_NUMBER: _ClassVar[int]
    SAMPLE_START_FIELD_NUMBER: _ClassVar[int]
    SAMPLE_END_FIELD_NUMBER: _ClassVar[int]
    PROVENANCE_FIELD_NUMBER: _ClassVar[int]
    DETERMINISTIC_SEED_FIELD_NUMBER: _ClassVar[int]
    job_id: _common_pb2.JobId
    lease_id: _common_pb2.LeaseId
    factor: _factor_pb2.FactorSpec
    panel_manifest: _artifact_pb2.ArtifactRef
    sample_start: _common_pb2.CivilDate
    sample_end: _common_pb2.CivilDate
    provenance: _research_common_pb2.ResearchProvenanceFingerprint
    deterministic_seed: _common_pb2.Sha256Digest
    def __init__(self, job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., lease_id: _Optional[_Union[_common_pb2.LeaseId, _Mapping]] = ..., factor: _Optional[_Union[_factor_pb2.FactorSpec, _Mapping]] = ..., panel_manifest: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., sample_start: _Optional[_Union[_common_pb2.CivilDate, _Mapping]] = ..., sample_end: _Optional[_Union[_common_pb2.CivilDate, _Mapping]] = ..., provenance: _Optional[_Union[_research_common_pb2.ResearchProvenanceFingerprint, _Mapping]] = ..., deterministic_seed: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class FactorEvaluationResult(_message.Message):
    __slots__ = ("job_id", "lease_id", "factor_spec_id", "expression_id", "provenance", "deterministic_seed", "values", "eligible_observations", "valid_observations", "work_units", "sample_start", "sample_end", "completed_at", "manifest")
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    LEASE_ID_FIELD_NUMBER: _ClassVar[int]
    FACTOR_SPEC_ID_FIELD_NUMBER: _ClassVar[int]
    EXPRESSION_ID_FIELD_NUMBER: _ClassVar[int]
    PROVENANCE_FIELD_NUMBER: _ClassVar[int]
    DETERMINISTIC_SEED_FIELD_NUMBER: _ClassVar[int]
    VALUES_FIELD_NUMBER: _ClassVar[int]
    ELIGIBLE_OBSERVATIONS_FIELD_NUMBER: _ClassVar[int]
    VALID_OBSERVATIONS_FIELD_NUMBER: _ClassVar[int]
    WORK_UNITS_FIELD_NUMBER: _ClassVar[int]
    SAMPLE_START_FIELD_NUMBER: _ClassVar[int]
    SAMPLE_END_FIELD_NUMBER: _ClassVar[int]
    COMPLETED_AT_FIELD_NUMBER: _ClassVar[int]
    MANIFEST_FIELD_NUMBER: _ClassVar[int]
    job_id: _common_pb2.JobId
    lease_id: _common_pb2.LeaseId
    factor_spec_id: _common_pb2.FactorSpecId
    expression_id: _common_pb2.FactorExpressionId
    provenance: _research_common_pb2.ResearchProvenanceFingerprint
    deterministic_seed: _common_pb2.Sha256Digest
    values: _artifact_pb2.ArtifactRef
    eligible_observations: int
    valid_observations: int
    work_units: int
    sample_start: _common_pb2.CivilDate
    sample_end: _common_pb2.CivilDate
    completed_at: _timestamp_pb2.Timestamp
    manifest: _artifact_pb2.ArtifactRef
    def __init__(self, job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., lease_id: _Optional[_Union[_common_pb2.LeaseId, _Mapping]] = ..., factor_spec_id: _Optional[_Union[_common_pb2.FactorSpecId, _Mapping]] = ..., expression_id: _Optional[_Union[_common_pb2.FactorExpressionId, _Mapping]] = ..., provenance: _Optional[_Union[_research_common_pb2.ResearchProvenanceFingerprint, _Mapping]] = ..., deterministic_seed: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., values: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., eligible_observations: _Optional[int] = ..., valid_observations: _Optional[int] = ..., work_units: _Optional[int] = ..., sample_start: _Optional[_Union[_common_pb2.CivilDate, _Mapping]] = ..., sample_end: _Optional[_Union[_common_pb2.CivilDate, _Mapping]] = ..., completed_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., manifest: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ...) -> None: ...
