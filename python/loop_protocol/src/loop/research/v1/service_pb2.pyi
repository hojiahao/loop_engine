import datetime

from google.protobuf import duration_pb2 as _duration_pb2
from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.v1 import common_pb2 as _common_pb2
from loop.v1 import development_data_pb2 as _development_data_pb2
from loop.v1 import factor_pb2 as _factor_pb2
from loop.v1 import research_common_pb2 as _research_common_pb2
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class ResearchJobStatus(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    RESEARCH_JOB_STATUS_UNSPECIFIED: _ClassVar[ResearchJobStatus]
    RESEARCH_JOB_STATUS_QUEUED: _ClassVar[ResearchJobStatus]
    RESEARCH_JOB_STATUS_LEASED: _ClassVar[ResearchJobStatus]
    RESEARCH_JOB_STATUS_RUNNING: _ClassVar[ResearchJobStatus]
    RESEARCH_JOB_STATUS_SUCCEEDED: _ClassVar[ResearchJobStatus]
    RESEARCH_JOB_STATUS_FACTOR_REJECTED: _ClassVar[ResearchJobStatus]
    RESEARCH_JOB_STATUS_INFRASTRUCTURE_FAILED: _ClassVar[ResearchJobStatus]
    RESEARCH_JOB_STATUS_CANCELLED: _ClassVar[ResearchJobStatus]
    RESEARCH_JOB_STATUS_BUDGET_EXHAUSTED: _ClassVar[ResearchJobStatus]
RESEARCH_JOB_STATUS_UNSPECIFIED: ResearchJobStatus
RESEARCH_JOB_STATUS_QUEUED: ResearchJobStatus
RESEARCH_JOB_STATUS_LEASED: ResearchJobStatus
RESEARCH_JOB_STATUS_RUNNING: ResearchJobStatus
RESEARCH_JOB_STATUS_SUCCEEDED: ResearchJobStatus
RESEARCH_JOB_STATUS_FACTOR_REJECTED: ResearchJobStatus
RESEARCH_JOB_STATUS_INFRASTRUCTURE_FAILED: ResearchJobStatus
RESEARCH_JOB_STATUS_CANCELLED: ResearchJobStatus
RESEARCH_JOB_STATUS_BUDGET_EXHAUSTED: ResearchJobStatus

class ResearchJobBudget(_message.Message):
    __slots__ = ("maximum_steps", "maximum_input_tokens", "maximum_output_tokens", "maximum_cost", "maximum_wall_time")
    MAXIMUM_STEPS_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_INPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_COST_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_WALL_TIME_FIELD_NUMBER: _ClassVar[int]
    maximum_steps: int
    maximum_input_tokens: int
    maximum_output_tokens: int
    maximum_cost: _common_pb2.Money
    maximum_wall_time: _duration_pb2.Duration
    def __init__(self, maximum_steps: _Optional[int] = ..., maximum_input_tokens: _Optional[int] = ..., maximum_output_tokens: _Optional[int] = ..., maximum_cost: _Optional[_Union[_common_pb2.Money, _Mapping]] = ..., maximum_wall_time: _Optional[_Union[datetime.timedelta, _duration_pb2.Duration, _Mapping]] = ...) -> None: ...

class FactorEvaluationInput(_message.Message):
    __slots__ = ("factor", "dataset", "budget")
    FACTOR_FIELD_NUMBER: _ClassVar[int]
    DATASET_FIELD_NUMBER: _ClassVar[int]
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    factor: _factor_pb2.FactorSpec
    dataset: _development_data_pb2.DevelopmentDatasetReference
    budget: ResearchJobBudget
    def __init__(self, factor: _Optional[_Union[_factor_pb2.FactorSpec, _Mapping]] = ..., dataset: _Optional[_Union[_development_data_pb2.DevelopmentDatasetReference, _Mapping]] = ..., budget: _Optional[_Union[ResearchJobBudget, _Mapping]] = ...) -> None: ...

class BacktestInput(_message.Message):
    __slots__ = ("budget", "factor_spec_id", "dataset", "return_definition", "provenance", "deterministic_seed")
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    FACTOR_SPEC_ID_FIELD_NUMBER: _ClassVar[int]
    DATASET_FIELD_NUMBER: _ClassVar[int]
    RETURN_DEFINITION_FIELD_NUMBER: _ClassVar[int]
    PROVENANCE_FIELD_NUMBER: _ClassVar[int]
    DETERMINISTIC_SEED_FIELD_NUMBER: _ClassVar[int]
    budget: ResearchJobBudget
    factor_spec_id: _common_pb2.FactorSpecId
    dataset: _development_data_pb2.DevelopmentDatasetReference
    return_definition: _research_common_pb2.ReturnDefinition
    provenance: _research_common_pb2.ResearchProvenanceFingerprint
    deterministic_seed: _common_pb2.Sha256Digest
    def __init__(self, budget: _Optional[_Union[ResearchJobBudget, _Mapping]] = ..., factor_spec_id: _Optional[_Union[_common_pb2.FactorSpecId, _Mapping]] = ..., dataset: _Optional[_Union[_development_data_pb2.DevelopmentDatasetReference, _Mapping]] = ..., return_definition: _Optional[_Union[_research_common_pb2.ReturnDefinition, str]] = ..., provenance: _Optional[_Union[_research_common_pb2.ResearchProvenanceFingerprint, _Mapping]] = ..., deterministic_seed: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class ReconciliationInput(_message.Message):
    __slots__ = ("primary_backtest_id", "independent_backtest_id", "reconciliation_policy", "budget")
    PRIMARY_BACKTEST_ID_FIELD_NUMBER: _ClassVar[int]
    INDEPENDENT_BACKTEST_ID_FIELD_NUMBER: _ClassVar[int]
    RECONCILIATION_POLICY_FIELD_NUMBER: _ClassVar[int]
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    primary_backtest_id: _common_pb2.BacktestId
    independent_backtest_id: _common_pb2.BacktestId
    reconciliation_policy: _common_pb2.PolicyReference
    budget: ResearchJobBudget
    def __init__(self, primary_backtest_id: _Optional[_Union[_common_pb2.BacktestId, _Mapping]] = ..., independent_backtest_id: _Optional[_Union[_common_pb2.BacktestId, _Mapping]] = ..., reconciliation_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., budget: _Optional[_Union[ResearchJobBudget, _Mapping]] = ...) -> None: ...

class EnqueueFactorEvaluationRequest(_message.Message):
    __slots__ = ("context", "input")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    INPUT_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    input: FactorEvaluationInput
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., input: _Optional[_Union[FactorEvaluationInput, _Mapping]] = ...) -> None: ...

class EnqueueFactorEvaluationResponse(_message.Message):
    __slots__ = ("job",)
    JOB_FIELD_NUMBER: _ClassVar[int]
    job: ResearchJobHandle
    def __init__(self, job: _Optional[_Union[ResearchJobHandle, _Mapping]] = ...) -> None: ...

class EnqueueBacktestRequest(_message.Message):
    __slots__ = ("context", "input")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    INPUT_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    input: BacktestInput
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., input: _Optional[_Union[BacktestInput, _Mapping]] = ...) -> None: ...

class EnqueueBacktestResponse(_message.Message):
    __slots__ = ("job",)
    JOB_FIELD_NUMBER: _ClassVar[int]
    job: ResearchJobHandle
    def __init__(self, job: _Optional[_Union[ResearchJobHandle, _Mapping]] = ...) -> None: ...

class EnqueueReconciliationRequest(_message.Message):
    __slots__ = ("context", "input")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    INPUT_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    input: ReconciliationInput
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., input: _Optional[_Union[ReconciliationInput, _Mapping]] = ...) -> None: ...

class EnqueueReconciliationResponse(_message.Message):
    __slots__ = ("job",)
    JOB_FIELD_NUMBER: _ClassVar[int]
    job: ResearchJobHandle
    def __init__(self, job: _Optional[_Union[ResearchJobHandle, _Mapping]] = ...) -> None: ...

class ResearchJobHandle(_message.Message):
    __slots__ = ("job_id", "status", "revision", "submitted_at", "updated_at")
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    STATUS_FIELD_NUMBER: _ClassVar[int]
    REVISION_FIELD_NUMBER: _ClassVar[int]
    SUBMITTED_AT_FIELD_NUMBER: _ClassVar[int]
    UPDATED_AT_FIELD_NUMBER: _ClassVar[int]
    job_id: _common_pb2.JobId
    status: ResearchJobStatus
    revision: int
    submitted_at: _timestamp_pb2.Timestamp
    updated_at: _timestamp_pb2.Timestamp
    def __init__(self, job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., status: _Optional[_Union[ResearchJobStatus, str]] = ..., revision: _Optional[int] = ..., submitted_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., updated_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...
