import datetime

from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.v1 import artifact_pb2 as _artifact_pb2
from loop.v1 import common_pb2 as _common_pb2
from loop.v1 import data_pb2 as _data_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class HoldoutGrantState(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    HOLDOUT_GRANT_STATE_UNSPECIFIED: _ClassVar[HoldoutGrantState]
    HOLDOUT_GRANT_STATE_ISSUED: _ClassVar[HoldoutGrantState]
    HOLDOUT_GRANT_STATE_CONSUMED: _ClassVar[HoldoutGrantState]
    HOLDOUT_GRANT_STATE_EXPIRED: _ClassVar[HoldoutGrantState]
    HOLDOUT_GRANT_STATE_REVOKED: _ClassVar[HoldoutGrantState]

class HoldoutPeriodState(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    HOLDOUT_PERIOD_STATE_UNSPECIFIED: _ClassVar[HoldoutPeriodState]
    HOLDOUT_PERIOD_STATE_SEALED: _ClassVar[HoldoutPeriodState]
    HOLDOUT_PERIOD_STATE_GRANT_ISSUED: _ClassVar[HoldoutPeriodState]
    HOLDOUT_PERIOD_STATE_CONSUMED: _ClassVar[HoldoutPeriodState]
    HOLDOUT_PERIOD_STATE_CLOSED: _ClassVar[HoldoutPeriodState]
HOLDOUT_GRANT_STATE_UNSPECIFIED: HoldoutGrantState
HOLDOUT_GRANT_STATE_ISSUED: HoldoutGrantState
HOLDOUT_GRANT_STATE_CONSUMED: HoldoutGrantState
HOLDOUT_GRANT_STATE_EXPIRED: HoldoutGrantState
HOLDOUT_GRANT_STATE_REVOKED: HoldoutGrantState
HOLDOUT_PERIOD_STATE_UNSPECIFIED: HoldoutPeriodState
HOLDOUT_PERIOD_STATE_SEALED: HoldoutPeriodState
HOLDOUT_PERIOD_STATE_GRANT_ISSUED: HoldoutPeriodState
HOLDOUT_PERIOD_STATE_CONSUMED: HoldoutPeriodState
HOLDOUT_PERIOD_STATE_CLOSED: HoldoutPeriodState

class HoldoutEvaluationPlanReference(_message.Message):
    __slots__ = ("holdout_evaluation_plan_id", "canonical_plan", "plan_sha256", "entry_count", "holdout_period_id", "canonical_period_sha256")
    HOLDOUT_EVALUATION_PLAN_ID_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_PLAN_FIELD_NUMBER: _ClassVar[int]
    PLAN_SHA256_FIELD_NUMBER: _ClassVar[int]
    ENTRY_COUNT_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_PERIOD_ID_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_PERIOD_SHA256_FIELD_NUMBER: _ClassVar[int]
    holdout_evaluation_plan_id: _common_pb2.HoldoutEvaluationPlanId
    canonical_plan: _artifact_pb2.ArtifactRef
    plan_sha256: _common_pb2.Sha256Digest
    entry_count: int
    holdout_period_id: _common_pb2.HoldoutPeriodId
    canonical_period_sha256: _common_pb2.Sha256Digest
    def __init__(self, holdout_evaluation_plan_id: _Optional[_Union[_common_pb2.HoldoutEvaluationPlanId, _Mapping]] = ..., canonical_plan: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., plan_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., entry_count: _Optional[int] = ..., holdout_period_id: _Optional[_Union[_common_pb2.HoldoutPeriodId, _Mapping]] = ..., canonical_period_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class FreezeManifestReference(_message.Message):
    __slots__ = ("manifest", "configuration_sha256", "model_catalog_sha256", "data_manifest_sha256", "source_commit", "source_tree_sha256", "holdout_approval_policy", "holdout_evaluation_plan")
    MANIFEST_FIELD_NUMBER: _ClassVar[int]
    CONFIGURATION_SHA256_FIELD_NUMBER: _ClassVar[int]
    MODEL_CATALOG_SHA256_FIELD_NUMBER: _ClassVar[int]
    DATA_MANIFEST_SHA256_FIELD_NUMBER: _ClassVar[int]
    SOURCE_COMMIT_FIELD_NUMBER: _ClassVar[int]
    SOURCE_TREE_SHA256_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_APPROVAL_POLICY_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_EVALUATION_PLAN_FIELD_NUMBER: _ClassVar[int]
    manifest: _artifact_pb2.ArtifactRef
    configuration_sha256: _common_pb2.Sha256Digest
    model_catalog_sha256: _common_pb2.Sha256Digest
    data_manifest_sha256: _common_pb2.Sha256Digest
    source_commit: _common_pb2.VcsObjectId
    source_tree_sha256: _common_pb2.Sha256Digest
    holdout_approval_policy: _common_pb2.PolicyReference
    holdout_evaluation_plan: HoldoutEvaluationPlanReference
    def __init__(self, manifest: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., configuration_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., model_catalog_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., data_manifest_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., source_commit: _Optional[_Union[_common_pb2.VcsObjectId, _Mapping]] = ..., source_tree_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., holdout_approval_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., holdout_evaluation_plan: _Optional[_Union[HoldoutEvaluationPlanReference, _Mapping]] = ...) -> None: ...

class HoldoutPeriod(_message.Message):
    __slots__ = ("holdout_period_id", "sample", "snapshot_ids", "snapshot_manifest_sha256", "canonical_period_sha256")
    HOLDOUT_PERIOD_ID_FIELD_NUMBER: _ClassVar[int]
    SAMPLE_FIELD_NUMBER: _ClassVar[int]
    SNAPSHOT_IDS_FIELD_NUMBER: _ClassVar[int]
    SNAPSHOT_MANIFEST_SHA256_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_PERIOD_SHA256_FIELD_NUMBER: _ClassVar[int]
    holdout_period_id: _common_pb2.HoldoutPeriodId
    sample: _data_pb2.SampleWindow
    snapshot_ids: _containers.RepeatedCompositeFieldContainer[_common_pb2.SnapshotId]
    snapshot_manifest_sha256: _common_pb2.Sha256Digest
    canonical_period_sha256: _common_pb2.Sha256Digest
    def __init__(self, holdout_period_id: _Optional[_Union[_common_pb2.HoldoutPeriodId, _Mapping]] = ..., sample: _Optional[_Union[_data_pb2.SampleWindow, _Mapping]] = ..., snapshot_ids: _Optional[_Iterable[_Union[_common_pb2.SnapshotId, _Mapping]]] = ..., snapshot_manifest_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., canonical_period_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class HoldoutPeriodRecord(_message.Message):
    __slots__ = ("period", "state", "revision", "issued_grant_id", "grant_issued_at", "terminal_at")
    PERIOD_FIELD_NUMBER: _ClassVar[int]
    STATE_FIELD_NUMBER: _ClassVar[int]
    REVISION_FIELD_NUMBER: _ClassVar[int]
    ISSUED_GRANT_ID_FIELD_NUMBER: _ClassVar[int]
    GRANT_ISSUED_AT_FIELD_NUMBER: _ClassVar[int]
    TERMINAL_AT_FIELD_NUMBER: _ClassVar[int]
    period: HoldoutPeriod
    state: HoldoutPeriodState
    revision: int
    issued_grant_id: _common_pb2.HoldoutGrantId
    grant_issued_at: _timestamp_pb2.Timestamp
    terminal_at: _timestamp_pb2.Timestamp
    def __init__(self, period: _Optional[_Union[HoldoutPeriod, _Mapping]] = ..., state: _Optional[_Union[HoldoutPeriodState, str]] = ..., revision: _Optional[int] = ..., issued_grant_id: _Optional[_Union[_common_pb2.HoldoutGrantId, _Mapping]] = ..., grant_issued_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., terminal_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class HoldoutApprovalRecord(_message.Message):
    __slots__ = ("holdout_approval_record_id", "holdout_period_id", "freeze_manifest_sha256", "approved_by", "reason", "evidence", "approved_at", "expires_at", "approval_record_sha256", "holdout_evaluation_plan_id", "evaluation_plan_sha256", "evaluation_plan_entry_count", "canonical_period_sha256")
    HOLDOUT_APPROVAL_RECORD_ID_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_PERIOD_ID_FIELD_NUMBER: _ClassVar[int]
    FREEZE_MANIFEST_SHA256_FIELD_NUMBER: _ClassVar[int]
    APPROVED_BY_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    EVIDENCE_FIELD_NUMBER: _ClassVar[int]
    APPROVED_AT_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_FIELD_NUMBER: _ClassVar[int]
    APPROVAL_RECORD_SHA256_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_EVALUATION_PLAN_ID_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_SHA256_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_ENTRY_COUNT_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_PERIOD_SHA256_FIELD_NUMBER: _ClassVar[int]
    holdout_approval_record_id: _common_pb2.HoldoutApprovalRecordId
    holdout_period_id: _common_pb2.HoldoutPeriodId
    freeze_manifest_sha256: _common_pb2.Sha256Digest
    approved_by: _common_pb2.Actor
    reason: str
    evidence: _containers.RepeatedCompositeFieldContainer[_artifact_pb2.ArtifactRef]
    approved_at: _timestamp_pb2.Timestamp
    expires_at: _timestamp_pb2.Timestamp
    approval_record_sha256: _common_pb2.Sha256Digest
    holdout_evaluation_plan_id: _common_pb2.HoldoutEvaluationPlanId
    evaluation_plan_sha256: _common_pb2.Sha256Digest
    evaluation_plan_entry_count: int
    canonical_period_sha256: _common_pb2.Sha256Digest
    def __init__(self, holdout_approval_record_id: _Optional[_Union[_common_pb2.HoldoutApprovalRecordId, _Mapping]] = ..., holdout_period_id: _Optional[_Union[_common_pb2.HoldoutPeriodId, _Mapping]] = ..., freeze_manifest_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., approved_by: _Optional[_Union[_common_pb2.Actor, _Mapping]] = ..., reason: _Optional[str] = ..., evidence: _Optional[_Iterable[_Union[_artifact_pb2.ArtifactRef, _Mapping]]] = ..., approved_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., expires_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., approval_record_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., holdout_evaluation_plan_id: _Optional[_Union[_common_pb2.HoldoutEvaluationPlanId, _Mapping]] = ..., evaluation_plan_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., evaluation_plan_entry_count: _Optional[int] = ..., canonical_period_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class HoldoutApprovalRecordReference(_message.Message):
    __slots__ = ("holdout_approval_record_id", "holdout_period_id", "freeze_manifest_sha256", "approved_by_actor_id", "approved_at", "expires_at", "approval_record_sha256", "holdout_evaluation_plan_id", "evaluation_plan_sha256", "evaluation_plan_entry_count", "canonical_period_sha256")
    HOLDOUT_APPROVAL_RECORD_ID_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_PERIOD_ID_FIELD_NUMBER: _ClassVar[int]
    FREEZE_MANIFEST_SHA256_FIELD_NUMBER: _ClassVar[int]
    APPROVED_BY_ACTOR_ID_FIELD_NUMBER: _ClassVar[int]
    APPROVED_AT_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_FIELD_NUMBER: _ClassVar[int]
    APPROVAL_RECORD_SHA256_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_EVALUATION_PLAN_ID_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_SHA256_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_ENTRY_COUNT_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_PERIOD_SHA256_FIELD_NUMBER: _ClassVar[int]
    holdout_approval_record_id: _common_pb2.HoldoutApprovalRecordId
    holdout_period_id: _common_pb2.HoldoutPeriodId
    freeze_manifest_sha256: _common_pb2.Sha256Digest
    approved_by_actor_id: _common_pb2.ActorId
    approved_at: _timestamp_pb2.Timestamp
    expires_at: _timestamp_pb2.Timestamp
    approval_record_sha256: _common_pb2.Sha256Digest
    holdout_evaluation_plan_id: _common_pb2.HoldoutEvaluationPlanId
    evaluation_plan_sha256: _common_pb2.Sha256Digest
    evaluation_plan_entry_count: int
    canonical_period_sha256: _common_pb2.Sha256Digest
    def __init__(self, holdout_approval_record_id: _Optional[_Union[_common_pb2.HoldoutApprovalRecordId, _Mapping]] = ..., holdout_period_id: _Optional[_Union[_common_pb2.HoldoutPeriodId, _Mapping]] = ..., freeze_manifest_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., approved_by_actor_id: _Optional[_Union[_common_pb2.ActorId, _Mapping]] = ..., approved_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., expires_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., approval_record_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., holdout_evaluation_plan_id: _Optional[_Union[_common_pb2.HoldoutEvaluationPlanId, _Mapping]] = ..., evaluation_plan_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., evaluation_plan_entry_count: _Optional[int] = ..., canonical_period_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class HoldoutGrantReference(_message.Message):
    __slots__ = ("holdout_grant_id", "holdout_period_id", "freeze_manifest_sha256", "issued_at", "expires_at", "holdout_evaluation_plan_id", "evaluation_plan_sha256", "evaluation_plan_entry_count", "canonical_period_sha256")
    HOLDOUT_GRANT_ID_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_PERIOD_ID_FIELD_NUMBER: _ClassVar[int]
    FREEZE_MANIFEST_SHA256_FIELD_NUMBER: _ClassVar[int]
    ISSUED_AT_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_EVALUATION_PLAN_ID_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_SHA256_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_ENTRY_COUNT_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_PERIOD_SHA256_FIELD_NUMBER: _ClassVar[int]
    holdout_grant_id: _common_pb2.HoldoutGrantId
    holdout_period_id: _common_pb2.HoldoutPeriodId
    freeze_manifest_sha256: _common_pb2.Sha256Digest
    issued_at: _timestamp_pb2.Timestamp
    expires_at: _timestamp_pb2.Timestamp
    holdout_evaluation_plan_id: _common_pb2.HoldoutEvaluationPlanId
    evaluation_plan_sha256: _common_pb2.Sha256Digest
    evaluation_plan_entry_count: int
    canonical_period_sha256: _common_pb2.Sha256Digest
    def __init__(self, holdout_grant_id: _Optional[_Union[_common_pb2.HoldoutGrantId, _Mapping]] = ..., holdout_period_id: _Optional[_Union[_common_pb2.HoldoutPeriodId, _Mapping]] = ..., freeze_manifest_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., issued_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., expires_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., holdout_evaluation_plan_id: _Optional[_Union[_common_pb2.HoldoutEvaluationPlanId, _Mapping]] = ..., evaluation_plan_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., evaluation_plan_entry_count: _Optional[int] = ..., canonical_period_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class HoldoutGrantRecord(_message.Message):
    __slots__ = ("reference", "state", "revision", "approval_records", "consumed_at", "approval_policy", "holdout_evaluation_plan_id", "evaluation_plan_sha256", "evaluation_plan_entry_count", "canonical_period_sha256")
    REFERENCE_FIELD_NUMBER: _ClassVar[int]
    STATE_FIELD_NUMBER: _ClassVar[int]
    REVISION_FIELD_NUMBER: _ClassVar[int]
    APPROVAL_RECORDS_FIELD_NUMBER: _ClassVar[int]
    CONSUMED_AT_FIELD_NUMBER: _ClassVar[int]
    APPROVAL_POLICY_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_EVALUATION_PLAN_ID_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_SHA256_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_ENTRY_COUNT_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_PERIOD_SHA256_FIELD_NUMBER: _ClassVar[int]
    reference: HoldoutGrantReference
    state: HoldoutGrantState
    revision: int
    approval_records: _containers.RepeatedCompositeFieldContainer[HoldoutApprovalRecordReference]
    consumed_at: _timestamp_pb2.Timestamp
    approval_policy: _common_pb2.PolicyReference
    holdout_evaluation_plan_id: _common_pb2.HoldoutEvaluationPlanId
    evaluation_plan_sha256: _common_pb2.Sha256Digest
    evaluation_plan_entry_count: int
    canonical_period_sha256: _common_pb2.Sha256Digest
    def __init__(self, reference: _Optional[_Union[HoldoutGrantReference, _Mapping]] = ..., state: _Optional[_Union[HoldoutGrantState, str]] = ..., revision: _Optional[int] = ..., approval_records: _Optional[_Iterable[_Union[HoldoutApprovalRecordReference, _Mapping]]] = ..., consumed_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., approval_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., holdout_evaluation_plan_id: _Optional[_Union[_common_pb2.HoldoutEvaluationPlanId, _Mapping]] = ..., evaluation_plan_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., evaluation_plan_entry_count: _Optional[int] = ..., canonical_period_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...
