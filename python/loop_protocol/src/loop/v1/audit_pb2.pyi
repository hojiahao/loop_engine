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

class AuditAction(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    AUDIT_ACTION_UNSPECIFIED: _ClassVar[AuditAction]
    AUDIT_ACTION_COMMAND_ACCEPTED: _ClassVar[AuditAction]
    AUDIT_ACTION_STATE_TRANSITIONED: _ClassVar[AuditAction]
    AUDIT_ACTION_FACTOR_ADMITTED: _ClassVar[AuditAction]
    AUDIT_ACTION_FACTOR_REJECTED: _ClassVar[AuditAction]
    AUDIT_ACTION_OVERRIDE_AUTHORIZED: _ClassVar[AuditAction]
    AUDIT_ACTION_READMISSION_REQUESTED: _ClassVar[AuditAction]
    AUDIT_ACTION_READMISSION_DECIDED: _ClassVar[AuditAction]
    AUDIT_ACTION_HOLDOUT_GRANT_ISSUED: _ClassVar[AuditAction]
    AUDIT_ACTION_HOLDOUT_GRANT_CONSUMED: _ClassVar[AuditAction]
    AUDIT_ACTION_ARTIFACT_EXPORTED: _ClassVar[AuditAction]
    AUDIT_ACTION_HOLDOUT_APPROVAL_RECORDED: _ClassVar[AuditAction]

class OverrideKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    OVERRIDE_KIND_UNSPECIFIED: _ClassVar[OverrideKind]
    OVERRIDE_KIND_FORCE_ADMISSION: _ClassVar[OverrideKind]
    OVERRIDE_KIND_READMISSION: _ClassVar[OverrideKind]
    OVERRIDE_KIND_POLICY_EXCEPTION: _ClassVar[OverrideKind]

class ReadmissionDisposition(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    READMISSION_DISPOSITION_UNSPECIFIED: _ClassVar[ReadmissionDisposition]
    READMISSION_DISPOSITION_ADMITTED: _ClassVar[ReadmissionDisposition]
    READMISSION_DISPOSITION_REJECTED: _ClassVar[ReadmissionDisposition]
    READMISSION_DISPOSITION_QUARANTINED: _ClassVar[ReadmissionDisposition]
AUDIT_ACTION_UNSPECIFIED: AuditAction
AUDIT_ACTION_COMMAND_ACCEPTED: AuditAction
AUDIT_ACTION_STATE_TRANSITIONED: AuditAction
AUDIT_ACTION_FACTOR_ADMITTED: AuditAction
AUDIT_ACTION_FACTOR_REJECTED: AuditAction
AUDIT_ACTION_OVERRIDE_AUTHORIZED: AuditAction
AUDIT_ACTION_READMISSION_REQUESTED: AuditAction
AUDIT_ACTION_READMISSION_DECIDED: AuditAction
AUDIT_ACTION_HOLDOUT_GRANT_ISSUED: AuditAction
AUDIT_ACTION_HOLDOUT_GRANT_CONSUMED: AuditAction
AUDIT_ACTION_ARTIFACT_EXPORTED: AuditAction
AUDIT_ACTION_HOLDOUT_APPROVAL_RECORDED: AuditAction
OVERRIDE_KIND_UNSPECIFIED: OverrideKind
OVERRIDE_KIND_FORCE_ADMISSION: OverrideKind
OVERRIDE_KIND_READMISSION: OverrideKind
OVERRIDE_KIND_POLICY_EXCEPTION: OverrideKind
READMISSION_DISPOSITION_UNSPECIFIED: ReadmissionDisposition
READMISSION_DISPOSITION_ADMITTED: ReadmissionDisposition
READMISSION_DISPOSITION_REJECTED: ReadmissionDisposition
READMISSION_DISPOSITION_QUARANTINED: ReadmissionDisposition

class AuditTarget(_message.Message):
    __slots__ = ("run_id", "job_id", "factor_spec_id", "backtest_id", "snapshot_id", "holdout_grant_id", "artifact_id", "holdout_approval_record_id")
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    FACTOR_SPEC_ID_FIELD_NUMBER: _ClassVar[int]
    BACKTEST_ID_FIELD_NUMBER: _ClassVar[int]
    SNAPSHOT_ID_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_GRANT_ID_FIELD_NUMBER: _ClassVar[int]
    ARTIFACT_ID_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_APPROVAL_RECORD_ID_FIELD_NUMBER: _ClassVar[int]
    run_id: _common_pb2.RunId
    job_id: _common_pb2.JobId
    factor_spec_id: _common_pb2.FactorSpecId
    backtest_id: _common_pb2.BacktestId
    snapshot_id: _common_pb2.SnapshotId
    holdout_grant_id: _common_pb2.HoldoutGrantId
    artifact_id: _common_pb2.ArtifactId
    holdout_approval_record_id: _common_pb2.HoldoutApprovalRecordId
    def __init__(self, run_id: _Optional[_Union[_common_pb2.RunId, _Mapping]] = ..., job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., factor_spec_id: _Optional[_Union[_common_pb2.FactorSpecId, _Mapping]] = ..., backtest_id: _Optional[_Union[_common_pb2.BacktestId, _Mapping]] = ..., snapshot_id: _Optional[_Union[_common_pb2.SnapshotId, _Mapping]] = ..., holdout_grant_id: _Optional[_Union[_common_pb2.HoldoutGrantId, _Mapping]] = ..., artifact_id: _Optional[_Union[_common_pb2.ArtifactId, _Mapping]] = ..., holdout_approval_record_id: _Optional[_Union[_common_pb2.HoldoutApprovalRecordId, _Mapping]] = ...) -> None: ...

class AuditPayload(_message.Message):
    __slots__ = ("schema_name", "schema_version", "canonical_json", "payload_sha256")
    SCHEMA_NAME_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_VERSION_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_JSON_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_SHA256_FIELD_NUMBER: _ClassVar[int]
    schema_name: str
    schema_version: int
    canonical_json: bytes
    payload_sha256: _common_pb2.Sha256Digest
    def __init__(self, schema_name: _Optional[str] = ..., schema_version: _Optional[int] = ..., canonical_json: _Optional[bytes] = ..., payload_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class AuditEvent(_message.Message):
    __slots__ = ("audit_event_id", "sequence", "previous_event_sha256", "event_sha256", "occurred_at", "correlation_id", "actor", "action", "target", "payload", "causation_id", "audit_ledger_id")
    AUDIT_EVENT_ID_FIELD_NUMBER: _ClassVar[int]
    SEQUENCE_FIELD_NUMBER: _ClassVar[int]
    PREVIOUS_EVENT_SHA256_FIELD_NUMBER: _ClassVar[int]
    EVENT_SHA256_FIELD_NUMBER: _ClassVar[int]
    OCCURRED_AT_FIELD_NUMBER: _ClassVar[int]
    CORRELATION_ID_FIELD_NUMBER: _ClassVar[int]
    ACTOR_FIELD_NUMBER: _ClassVar[int]
    ACTION_FIELD_NUMBER: _ClassVar[int]
    TARGET_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    CAUSATION_ID_FIELD_NUMBER: _ClassVar[int]
    AUDIT_LEDGER_ID_FIELD_NUMBER: _ClassVar[int]
    audit_event_id: _common_pb2.AuditEventId
    sequence: int
    previous_event_sha256: _common_pb2.Sha256Digest
    event_sha256: _common_pb2.Sha256Digest
    occurred_at: _timestamp_pb2.Timestamp
    correlation_id: _common_pb2.CorrelationId
    actor: _common_pb2.Actor
    action: AuditAction
    target: AuditTarget
    payload: AuditPayload
    causation_id: _common_pb2.CausationId
    audit_ledger_id: _common_pb2.AuditLedgerId
    def __init__(self, audit_event_id: _Optional[_Union[_common_pb2.AuditEventId, _Mapping]] = ..., sequence: _Optional[int] = ..., previous_event_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., event_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., occurred_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., correlation_id: _Optional[_Union[_common_pb2.CorrelationId, _Mapping]] = ..., actor: _Optional[_Union[_common_pb2.Actor, _Mapping]] = ..., action: _Optional[_Union[AuditAction, str]] = ..., target: _Optional[_Union[AuditTarget, _Mapping]] = ..., payload: _Optional[_Union[AuditPayload, _Mapping]] = ..., causation_id: _Optional[_Union[_common_pb2.CausationId, _Mapping]] = ..., audit_ledger_id: _Optional[_Union[_common_pb2.AuditLedgerId, _Mapping]] = ...) -> None: ...

class OverrideAuthorization(_message.Message):
    __slots__ = ("kind", "authorized_by", "reason", "approval_reference", "evidence", "authorized_at")
    KIND_FIELD_NUMBER: _ClassVar[int]
    AUTHORIZED_BY_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    APPROVAL_REFERENCE_FIELD_NUMBER: _ClassVar[int]
    EVIDENCE_FIELD_NUMBER: _ClassVar[int]
    AUTHORIZED_AT_FIELD_NUMBER: _ClassVar[int]
    kind: OverrideKind
    authorized_by: _common_pb2.Actor
    reason: str
    approval_reference: str
    evidence: _containers.RepeatedCompositeFieldContainer[_artifact_pb2.ArtifactRef]
    authorized_at: _timestamp_pb2.Timestamp
    def __init__(self, kind: _Optional[_Union[OverrideKind, str]] = ..., authorized_by: _Optional[_Union[_common_pb2.Actor, _Mapping]] = ..., reason: _Optional[str] = ..., approval_reference: _Optional[str] = ..., evidence: _Optional[_Iterable[_Union[_artifact_pb2.ArtifactRef, _Mapping]]] = ..., authorized_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class ReadmissionRecord(_message.Message):
    __slots__ = ("factor_spec_id", "original_rejection_event_id", "authorization", "disposition", "decision_event_id", "decision_reason")
    FACTOR_SPEC_ID_FIELD_NUMBER: _ClassVar[int]
    ORIGINAL_REJECTION_EVENT_ID_FIELD_NUMBER: _ClassVar[int]
    AUTHORIZATION_FIELD_NUMBER: _ClassVar[int]
    DISPOSITION_FIELD_NUMBER: _ClassVar[int]
    DECISION_EVENT_ID_FIELD_NUMBER: _ClassVar[int]
    DECISION_REASON_FIELD_NUMBER: _ClassVar[int]
    factor_spec_id: _common_pb2.FactorSpecId
    original_rejection_event_id: _common_pb2.AuditEventId
    authorization: OverrideAuthorization
    disposition: ReadmissionDisposition
    decision_event_id: _common_pb2.AuditEventId
    decision_reason: str
    def __init__(self, factor_spec_id: _Optional[_Union[_common_pb2.FactorSpecId, _Mapping]] = ..., original_rejection_event_id: _Optional[_Union[_common_pb2.AuditEventId, _Mapping]] = ..., authorization: _Optional[_Union[OverrideAuthorization, _Mapping]] = ..., disposition: _Optional[_Union[ReadmissionDisposition, str]] = ..., decision_event_id: _Optional[_Union[_common_pb2.AuditEventId, _Mapping]] = ..., decision_reason: _Optional[str] = ...) -> None: ...
