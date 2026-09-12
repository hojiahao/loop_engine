import datetime

from google.protobuf import duration_pb2 as _duration_pb2
from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.v1 import artifact_pb2 as _artifact_pb2
from loop.v1 import common_pb2 as _common_pb2
from loop.v1 import job_pb2 as _job_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class GetJobRequest(_message.Message):
    __slots__ = ("job_id",)
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    job_id: _common_pb2.JobId
    def __init__(self, job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ...) -> None: ...

class GetJobResponse(_message.Message):
    __slots__ = ("job",)
    JOB_FIELD_NUMBER: _ClassVar[int]
    job: _job_pb2.JobRecord
    def __init__(self, job: _Optional[_Union[_job_pb2.JobRecord, _Mapping]] = ...) -> None: ...

class AcquireJobLeaseRequest(_message.Message):
    __slots__ = ("context", "job_id", "expected_revision", "requested_duration")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_REVISION_FIELD_NUMBER: _ClassVar[int]
    REQUESTED_DURATION_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    job_id: _common_pb2.JobId
    expected_revision: int
    requested_duration: _duration_pb2.Duration
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., expected_revision: _Optional[int] = ..., requested_duration: _Optional[_Union[datetime.timedelta, _duration_pb2.Duration, _Mapping]] = ...) -> None: ...

class AcquireJobLeaseResponse(_message.Message):
    __slots__ = ("job",)
    JOB_FIELD_NUMBER: _ClassVar[int]
    job: _job_pb2.JobRecord
    def __init__(self, job: _Optional[_Union[_job_pb2.JobRecord, _Mapping]] = ...) -> None: ...

class HeartbeatJobLeaseRequest(_message.Message):
    __slots__ = ("context", "job_id", "lease_id", "expected_revision", "requested_extension")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    LEASE_ID_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_REVISION_FIELD_NUMBER: _ClassVar[int]
    REQUESTED_EXTENSION_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    job_id: _common_pb2.JobId
    lease_id: _common_pb2.LeaseId
    expected_revision: int
    requested_extension: _duration_pb2.Duration
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., lease_id: _Optional[_Union[_common_pb2.LeaseId, _Mapping]] = ..., expected_revision: _Optional[int] = ..., requested_extension: _Optional[_Union[datetime.timedelta, _duration_pb2.Duration, _Mapping]] = ...) -> None: ...

class HeartbeatJobLeaseResponse(_message.Message):
    __slots__ = ("job",)
    JOB_FIELD_NUMBER: _ClassVar[int]
    job: _job_pb2.JobRecord
    def __init__(self, job: _Optional[_Union[_job_pb2.JobRecord, _Mapping]] = ...) -> None: ...

class CompleteJobRequest(_message.Message):
    __slots__ = ("context", "job_id", "lease_id", "expected_revision", "outcome")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    LEASE_ID_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_REVISION_FIELD_NUMBER: _ClassVar[int]
    OUTCOME_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    job_id: _common_pb2.JobId
    lease_id: _common_pb2.LeaseId
    expected_revision: int
    outcome: _job_pb2.JobOutcome
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., lease_id: _Optional[_Union[_common_pb2.LeaseId, _Mapping]] = ..., expected_revision: _Optional[int] = ..., outcome: _Optional[_Union[_job_pb2.JobOutcome, _Mapping]] = ...) -> None: ...

class CompleteJobResponse(_message.Message):
    __slots__ = ("job",)
    JOB_FIELD_NUMBER: _ClassVar[int]
    job: _job_pb2.JobRecord
    def __init__(self, job: _Optional[_Union[_job_pb2.JobRecord, _Mapping]] = ...) -> None: ...

class CancelJobRequest(_message.Message):
    __slots__ = ("context", "job_id", "expected_revision", "reason")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_REVISION_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    job_id: _common_pb2.JobId
    expected_revision: int
    reason: str
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., expected_revision: _Optional[int] = ..., reason: _Optional[str] = ...) -> None: ...

class CancelJobResponse(_message.Message):
    __slots__ = ("job",)
    JOB_FIELD_NUMBER: _ClassVar[int]
    job: _job_pb2.JobRecord
    def __init__(self, job: _Optional[_Union[_job_pb2.JobRecord, _Mapping]] = ...) -> None: ...

class PrepareJobArtifactsRequest(_message.Message):
    __slots__ = ("context", "job_id", "lease_id", "expected_revision")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    LEASE_ID_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_REVISION_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    job_id: _common_pb2.JobId
    lease_id: _common_pb2.LeaseId
    expected_revision: int
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., lease_id: _Optional[_Union[_common_pb2.LeaseId, _Mapping]] = ..., expected_revision: _Optional[int] = ...) -> None: ...

class PrepareJobArtifactsResponse(_message.Message):
    __slots__ = ("job_id", "lease_id", "data_manifest_sha256", "artifacts", "expires_at", "view_id")
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    LEASE_ID_FIELD_NUMBER: _ClassVar[int]
    DATA_MANIFEST_SHA256_FIELD_NUMBER: _ClassVar[int]
    ARTIFACTS_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_FIELD_NUMBER: _ClassVar[int]
    VIEW_ID_FIELD_NUMBER: _ClassVar[int]
    job_id: _common_pb2.JobId
    lease_id: _common_pb2.LeaseId
    data_manifest_sha256: _common_pb2.Sha256Digest
    artifacts: _containers.RepeatedCompositeFieldContainer[_artifact_pb2.ArtifactRef]
    expires_at: _timestamp_pb2.Timestamp
    view_id: str
    def __init__(self, job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., lease_id: _Optional[_Union[_common_pb2.LeaseId, _Mapping]] = ..., data_manifest_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., artifacts: _Optional[_Iterable[_Union[_artifact_pb2.ArtifactRef, _Mapping]]] = ..., expires_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., view_id: _Optional[str] = ...) -> None: ...
