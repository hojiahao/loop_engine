import datetime

from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.v1 import artifact_pb2 as _artifact_pb2
from loop.v1 import common_pb2 as _common_pb2
from loop.v1 import holdout_pb2 as _holdout_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class RecordHoldoutApprovalRequest(_message.Message):
    __slots__ = ("context", "holdout_period_id", "freeze_manifest_sha256", "reason", "evidence", "canonical_period_sha256", "holdout_evaluation_plan_id", "evaluation_plan_sha256", "evaluation_plan_entry_count")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_PERIOD_ID_FIELD_NUMBER: _ClassVar[int]
    FREEZE_MANIFEST_SHA256_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    EVIDENCE_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_PERIOD_SHA256_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_EVALUATION_PLAN_ID_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_SHA256_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_ENTRY_COUNT_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    holdout_period_id: _common_pb2.HoldoutPeriodId
    freeze_manifest_sha256: _common_pb2.Sha256Digest
    reason: str
    evidence: _containers.RepeatedCompositeFieldContainer[_artifact_pb2.ArtifactRef]
    canonical_period_sha256: _common_pb2.Sha256Digest
    holdout_evaluation_plan_id: _common_pb2.HoldoutEvaluationPlanId
    evaluation_plan_sha256: _common_pb2.Sha256Digest
    evaluation_plan_entry_count: int
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., holdout_period_id: _Optional[_Union[_common_pb2.HoldoutPeriodId, _Mapping]] = ..., freeze_manifest_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., reason: _Optional[str] = ..., evidence: _Optional[_Iterable[_Union[_artifact_pb2.ArtifactRef, _Mapping]]] = ..., canonical_period_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., holdout_evaluation_plan_id: _Optional[_Union[_common_pb2.HoldoutEvaluationPlanId, _Mapping]] = ..., evaluation_plan_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., evaluation_plan_entry_count: _Optional[int] = ...) -> None: ...

class RecordHoldoutApprovalResponse(_message.Message):
    __slots__ = ("approval_record",)
    APPROVAL_RECORD_FIELD_NUMBER: _ClassVar[int]
    approval_record: _holdout_pb2.HoldoutApprovalRecord
    def __init__(self, approval_record: _Optional[_Union[_holdout_pb2.HoldoutApprovalRecord, _Mapping]] = ...) -> None: ...

class RequestHoldoutGrantRequest(_message.Message):
    __slots__ = ("context", "freeze_manifest", "approval_record_ids", "holdout_period_id", "expected_period_revision", "canonical_period_sha256")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    FREEZE_MANIFEST_FIELD_NUMBER: _ClassVar[int]
    APPROVAL_RECORD_IDS_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_PERIOD_ID_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_PERIOD_REVISION_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_PERIOD_SHA256_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    freeze_manifest: _holdout_pb2.FreezeManifestReference
    approval_record_ids: _containers.RepeatedCompositeFieldContainer[_common_pb2.HoldoutApprovalRecordId]
    holdout_period_id: _common_pb2.HoldoutPeriodId
    expected_period_revision: int
    canonical_period_sha256: _common_pb2.Sha256Digest
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., freeze_manifest: _Optional[_Union[_holdout_pb2.FreezeManifestReference, _Mapping]] = ..., approval_record_ids: _Optional[_Iterable[_Union[_common_pb2.HoldoutApprovalRecordId, _Mapping]]] = ..., holdout_period_id: _Optional[_Union[_common_pb2.HoldoutPeriodId, _Mapping]] = ..., expected_period_revision: _Optional[int] = ..., canonical_period_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class RequestHoldoutGrantResponse(_message.Message):
    __slots__ = ("grant", "period_record")
    GRANT_FIELD_NUMBER: _ClassVar[int]
    PERIOD_RECORD_FIELD_NUMBER: _ClassVar[int]
    grant: _holdout_pb2.HoldoutGrantRecord
    period_record: _holdout_pb2.HoldoutPeriodRecord
    def __init__(self, grant: _Optional[_Union[_holdout_pb2.HoldoutGrantRecord, _Mapping]] = ..., period_record: _Optional[_Union[_holdout_pb2.HoldoutPeriodRecord, _Mapping]] = ...) -> None: ...

class ConsumeGrantAndEnqueueBacktestRequest(_message.Message):
    __slots__ = ("context", "grant_reference", "expected_grant_revision", "expected_period_revision")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    GRANT_REFERENCE_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_GRANT_REVISION_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_PERIOD_REVISION_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    grant_reference: _holdout_pb2.HoldoutGrantReference
    expected_grant_revision: int
    expected_period_revision: int
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., grant_reference: _Optional[_Union[_holdout_pb2.HoldoutGrantReference, _Mapping]] = ..., expected_grant_revision: _Optional[int] = ..., expected_period_revision: _Optional[int] = ...) -> None: ...

class JobBatchHandle(_message.Message):
    __slots__ = ("job_batch_id", "holdout_grant_id", "holdout_evaluation_plan_id", "evaluation_plan_sha256", "evaluation_plan_entry_count", "job_count", "job_ids", "revision", "created_at")
    JOB_BATCH_ID_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_GRANT_ID_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_EVALUATION_PLAN_ID_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_SHA256_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_ENTRY_COUNT_FIELD_NUMBER: _ClassVar[int]
    JOB_COUNT_FIELD_NUMBER: _ClassVar[int]
    JOB_IDS_FIELD_NUMBER: _ClassVar[int]
    REVISION_FIELD_NUMBER: _ClassVar[int]
    CREATED_AT_FIELD_NUMBER: _ClassVar[int]
    job_batch_id: _common_pb2.JobBatchId
    holdout_grant_id: _common_pb2.HoldoutGrantId
    holdout_evaluation_plan_id: _common_pb2.HoldoutEvaluationPlanId
    evaluation_plan_sha256: _common_pb2.Sha256Digest
    evaluation_plan_entry_count: int
    job_count: int
    job_ids: _containers.RepeatedCompositeFieldContainer[_common_pb2.JobId]
    revision: int
    created_at: _timestamp_pb2.Timestamp
    def __init__(self, job_batch_id: _Optional[_Union[_common_pb2.JobBatchId, _Mapping]] = ..., holdout_grant_id: _Optional[_Union[_common_pb2.HoldoutGrantId, _Mapping]] = ..., holdout_evaluation_plan_id: _Optional[_Union[_common_pb2.HoldoutEvaluationPlanId, _Mapping]] = ..., evaluation_plan_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., evaluation_plan_entry_count: _Optional[int] = ..., job_count: _Optional[int] = ..., job_ids: _Optional[_Iterable[_Union[_common_pb2.JobId, _Mapping]]] = ..., revision: _Optional[int] = ..., created_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class ConsumeGrantAndEnqueueBacktestResponse(_message.Message):
    __slots__ = ("consumed_grant", "job_batch", "period_record")
    CONSUMED_GRANT_FIELD_NUMBER: _ClassVar[int]
    JOB_BATCH_FIELD_NUMBER: _ClassVar[int]
    PERIOD_RECORD_FIELD_NUMBER: _ClassVar[int]
    consumed_grant: _holdout_pb2.HoldoutGrantReference
    job_batch: JobBatchHandle
    period_record: _holdout_pb2.HoldoutPeriodRecord
    def __init__(self, consumed_grant: _Optional[_Union[_holdout_pb2.HoldoutGrantReference, _Mapping]] = ..., job_batch: _Optional[_Union[JobBatchHandle, _Mapping]] = ..., period_record: _Optional[_Union[_holdout_pb2.HoldoutPeriodRecord, _Mapping]] = ...) -> None: ...

class GetHoldoutPeriodRequest(_message.Message):
    __slots__ = ("holdout_period_id", "canonical_period_sha256")
    HOLDOUT_PERIOD_ID_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_PERIOD_SHA256_FIELD_NUMBER: _ClassVar[int]
    holdout_period_id: _common_pb2.HoldoutPeriodId
    canonical_period_sha256: _common_pb2.Sha256Digest
    def __init__(self, holdout_period_id: _Optional[_Union[_common_pb2.HoldoutPeriodId, _Mapping]] = ..., canonical_period_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class GetHoldoutPeriodResponse(_message.Message):
    __slots__ = ("period_record",)
    PERIOD_RECORD_FIELD_NUMBER: _ClassVar[int]
    period_record: _holdout_pb2.HoldoutPeriodRecord
    def __init__(self, period_record: _Optional[_Union[_holdout_pb2.HoldoutPeriodRecord, _Mapping]] = ...) -> None: ...

class GetHoldoutApprovalRecordRequest(_message.Message):
    __slots__ = ("holdout_approval_record_id",)
    HOLDOUT_APPROVAL_RECORD_ID_FIELD_NUMBER: _ClassVar[int]
    holdout_approval_record_id: _common_pb2.HoldoutApprovalRecordId
    def __init__(self, holdout_approval_record_id: _Optional[_Union[_common_pb2.HoldoutApprovalRecordId, _Mapping]] = ...) -> None: ...

class GetHoldoutApprovalRecordResponse(_message.Message):
    __slots__ = ("approval_record",)
    APPROVAL_RECORD_FIELD_NUMBER: _ClassVar[int]
    approval_record: _holdout_pb2.HoldoutApprovalRecord
    def __init__(self, approval_record: _Optional[_Union[_holdout_pb2.HoldoutApprovalRecord, _Mapping]] = ...) -> None: ...

class GetHoldoutGrantRequest(_message.Message):
    __slots__ = ("holdout_grant_id",)
    HOLDOUT_GRANT_ID_FIELD_NUMBER: _ClassVar[int]
    holdout_grant_id: _common_pb2.HoldoutGrantId
    def __init__(self, holdout_grant_id: _Optional[_Union[_common_pb2.HoldoutGrantId, _Mapping]] = ...) -> None: ...

class GetHoldoutGrantResponse(_message.Message):
    __slots__ = ("grant",)
    GRANT_FIELD_NUMBER: _ClassVar[int]
    grant: _holdout_pb2.HoldoutGrantRecord
    def __init__(self, grant: _Optional[_Union[_holdout_pb2.HoldoutGrantRecord, _Mapping]] = ...) -> None: ...
