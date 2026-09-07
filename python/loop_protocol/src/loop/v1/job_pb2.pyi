import datetime

from google.protobuf import duration_pb2 as _duration_pb2
from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.v1 import artifact_pb2 as _artifact_pb2
from loop.v1 import backtest_pb2 as _backtest_pb2
from loop.v1 import common_pb2 as _common_pb2
from loop.v1 import development_data_pb2 as _development_data_pb2
from loop.v1 import factor_pb2 as _factor_pb2
from loop.v1 import holdout_pb2 as _holdout_pb2
from loop.v1 import model_pb2 as _model_pb2
from loop.v1 import research_common_pb2 as _research_common_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class JobKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    JOB_KIND_UNSPECIFIED: _ClassVar[JobKind]
    JOB_KIND_DISCOVERY: _ClassVar[JobKind]
    JOB_KIND_FACTOR_EVALUATION: _ClassVar[JobKind]
    JOB_KIND_BACKTEST: _ClassVar[JobKind]
    JOB_KIND_INDEPENDENT_RECONCILIATION: _ClassVar[JobKind]
    JOB_KIND_REPORT: _ClassVar[JobKind]
    JOB_KIND_PROSPECTIVE_OBSERVATION: _ClassVar[JobKind]
    JOB_KIND_HOLDOUT_BACKTEST: _ClassVar[JobKind]

class JobState(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    JOB_STATE_UNSPECIFIED: _ClassVar[JobState]
    JOB_STATE_QUEUED: _ClassVar[JobState]
    JOB_STATE_LEASED: _ClassVar[JobState]
    JOB_STATE_RUNNING: _ClassVar[JobState]
    JOB_STATE_SUCCEEDED: _ClassVar[JobState]
    JOB_STATE_FACTOR_REJECTED: _ClassVar[JobState]
    JOB_STATE_INFRASTRUCTURE_FAILED: _ClassVar[JobState]
    JOB_STATE_CANCELLED: _ClassVar[JobState]
    JOB_STATE_BUDGET_EXHAUSTED: _ClassVar[JobState]

class FactorRejectionCode(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    FACTOR_REJECTION_CODE_UNSPECIFIED: _ClassVar[FactorRejectionCode]
    FACTOR_REJECTION_CODE_DUPLICATE: _ClassVar[FactorRejectionCode]
    FACTOR_REJECTION_CODE_PREVIOUSLY_FAILED: _ClassVar[FactorRejectionCode]
    FACTOR_REJECTION_CODE_INSUFFICIENT_COVERAGE: _ClassVar[FactorRejectionCode]
    FACTOR_REJECTION_CODE_DETERMINISTIC_FILTER: _ClassVar[FactorRejectionCode]
    FACTOR_REJECTION_CODE_PERFORMANCE: _ClassVar[FactorRejectionCode]
    FACTOR_REJECTION_CODE_CORRELATION: _ClassVar[FactorRejectionCode]
    FACTOR_REJECTION_CODE_SEMANTIC_REVIEW: _ClassVar[FactorRejectionCode]
    FACTOR_REJECTION_CODE_POLICY: _ClassVar[FactorRejectionCode]
JOB_KIND_UNSPECIFIED: JobKind
JOB_KIND_DISCOVERY: JobKind
JOB_KIND_FACTOR_EVALUATION: JobKind
JOB_KIND_BACKTEST: JobKind
JOB_KIND_INDEPENDENT_RECONCILIATION: JobKind
JOB_KIND_REPORT: JobKind
JOB_KIND_PROSPECTIVE_OBSERVATION: JobKind
JOB_KIND_HOLDOUT_BACKTEST: JobKind
JOB_STATE_UNSPECIFIED: JobState
JOB_STATE_QUEUED: JobState
JOB_STATE_LEASED: JobState
JOB_STATE_RUNNING: JobState
JOB_STATE_SUCCEEDED: JobState
JOB_STATE_FACTOR_REJECTED: JobState
JOB_STATE_INFRASTRUCTURE_FAILED: JobState
JOB_STATE_CANCELLED: JobState
JOB_STATE_BUDGET_EXHAUSTED: JobState
FACTOR_REJECTION_CODE_UNSPECIFIED: FactorRejectionCode
FACTOR_REJECTION_CODE_DUPLICATE: FactorRejectionCode
FACTOR_REJECTION_CODE_PREVIOUSLY_FAILED: FactorRejectionCode
FACTOR_REJECTION_CODE_INSUFFICIENT_COVERAGE: FactorRejectionCode
FACTOR_REJECTION_CODE_DETERMINISTIC_FILTER: FactorRejectionCode
FACTOR_REJECTION_CODE_PERFORMANCE: FactorRejectionCode
FACTOR_REJECTION_CODE_CORRELATION: FactorRejectionCode
FACTOR_REJECTION_CODE_SEMANTIC_REVIEW: FactorRejectionCode
FACTOR_REJECTION_CODE_POLICY: FactorRejectionCode

class JobBudget(_message.Message):
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

class DiscoveryJobInput(_message.Message):
    __slots__ = ("dataset", "research_policy", "maker_model", "checker_model", "budget", "maximum_candidates")
    DATASET_FIELD_NUMBER: _ClassVar[int]
    RESEARCH_POLICY_FIELD_NUMBER: _ClassVar[int]
    MAKER_MODEL_FIELD_NUMBER: _ClassVar[int]
    CHECKER_MODEL_FIELD_NUMBER: _ClassVar[int]
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_CANDIDATES_FIELD_NUMBER: _ClassVar[int]
    dataset: _development_data_pb2.DevelopmentDatasetReference
    research_policy: _common_pb2.PolicyReference
    maker_model: _model_pb2.ModelResolutionSnapshot
    checker_model: _model_pb2.ModelResolutionSnapshot
    budget: JobBudget
    maximum_candidates: int
    def __init__(self, dataset: _Optional[_Union[_development_data_pb2.DevelopmentDatasetReference, _Mapping]] = ..., research_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., maker_model: _Optional[_Union[_model_pb2.ModelResolutionSnapshot, _Mapping]] = ..., checker_model: _Optional[_Union[_model_pb2.ModelResolutionSnapshot, _Mapping]] = ..., budget: _Optional[_Union[JobBudget, _Mapping]] = ..., maximum_candidates: _Optional[int] = ...) -> None: ...

class FactorEvaluationJobInput(_message.Message):
    __slots__ = ("factor", "dataset", "budget")
    FACTOR_FIELD_NUMBER: _ClassVar[int]
    DATASET_FIELD_NUMBER: _ClassVar[int]
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    factor: _factor_pb2.FactorSpec
    dataset: _development_data_pb2.DevelopmentDatasetReference
    budget: JobBudget
    def __init__(self, factor: _Optional[_Union[_factor_pb2.FactorSpec, _Mapping]] = ..., dataset: _Optional[_Union[_development_data_pb2.DevelopmentDatasetReference, _Mapping]] = ..., budget: _Optional[_Union[JobBudget, _Mapping]] = ...) -> None: ...

class BacktestJobInput(_message.Message):
    __slots__ = ("budget", "factor_spec_id", "dataset", "return_definition", "provenance", "deterministic_seed")
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    FACTOR_SPEC_ID_FIELD_NUMBER: _ClassVar[int]
    DATASET_FIELD_NUMBER: _ClassVar[int]
    RETURN_DEFINITION_FIELD_NUMBER: _ClassVar[int]
    PROVENANCE_FIELD_NUMBER: _ClassVar[int]
    DETERMINISTIC_SEED_FIELD_NUMBER: _ClassVar[int]
    budget: JobBudget
    factor_spec_id: _common_pb2.FactorSpecId
    dataset: _development_data_pb2.DevelopmentDatasetReference
    return_definition: _research_common_pb2.ReturnDefinition
    provenance: _research_common_pb2.ResearchProvenanceFingerprint
    deterministic_seed: _common_pb2.Sha256Digest
    def __init__(self, budget: _Optional[_Union[JobBudget, _Mapping]] = ..., factor_spec_id: _Optional[_Union[_common_pb2.FactorSpecId, _Mapping]] = ..., dataset: _Optional[_Union[_development_data_pb2.DevelopmentDatasetReference, _Mapping]] = ..., return_definition: _Optional[_Union[_research_common_pb2.ReturnDefinition, str]] = ..., provenance: _Optional[_Union[_research_common_pb2.ResearchProvenanceFingerprint, _Mapping]] = ..., deterministic_seed: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class ReconciliationJobInput(_message.Message):
    __slots__ = ("primary_backtest_id", "independent_backtest_id", "reconciliation_policy", "budget")
    PRIMARY_BACKTEST_ID_FIELD_NUMBER: _ClassVar[int]
    INDEPENDENT_BACKTEST_ID_FIELD_NUMBER: _ClassVar[int]
    RECONCILIATION_POLICY_FIELD_NUMBER: _ClassVar[int]
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    primary_backtest_id: _common_pb2.BacktestId
    independent_backtest_id: _common_pb2.BacktestId
    reconciliation_policy: _common_pb2.PolicyReference
    budget: JobBudget
    def __init__(self, primary_backtest_id: _Optional[_Union[_common_pb2.BacktestId, _Mapping]] = ..., independent_backtest_id: _Optional[_Union[_common_pb2.BacktestId, _Mapping]] = ..., reconciliation_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., budget: _Optional[_Union[JobBudget, _Mapping]] = ...) -> None: ...

class HoldoutBacktestJobInput(_message.Message):
    __slots__ = ("consumed_grant", "consumed_grant_revision", "frozen_backtest_spec", "budget", "job_batch_id", "holdout_evaluation_plan_id", "evaluation_plan_sha256", "evaluation_plan_entry_index")
    CONSUMED_GRANT_FIELD_NUMBER: _ClassVar[int]
    CONSUMED_GRANT_REVISION_FIELD_NUMBER: _ClassVar[int]
    FROZEN_BACKTEST_SPEC_FIELD_NUMBER: _ClassVar[int]
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    JOB_BATCH_ID_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_EVALUATION_PLAN_ID_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_SHA256_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_PLAN_ENTRY_INDEX_FIELD_NUMBER: _ClassVar[int]
    consumed_grant: _holdout_pb2.HoldoutGrantReference
    consumed_grant_revision: int
    frozen_backtest_spec: _backtest_pb2.BacktestSpec
    budget: JobBudget
    job_batch_id: _common_pb2.JobBatchId
    holdout_evaluation_plan_id: _common_pb2.HoldoutEvaluationPlanId
    evaluation_plan_sha256: _common_pb2.Sha256Digest
    evaluation_plan_entry_index: int
    def __init__(self, consumed_grant: _Optional[_Union[_holdout_pb2.HoldoutGrantReference, _Mapping]] = ..., consumed_grant_revision: _Optional[int] = ..., frozen_backtest_spec: _Optional[_Union[_backtest_pb2.BacktestSpec, _Mapping]] = ..., budget: _Optional[_Union[JobBudget, _Mapping]] = ..., job_batch_id: _Optional[_Union[_common_pb2.JobBatchId, _Mapping]] = ..., holdout_evaluation_plan_id: _Optional[_Union[_common_pb2.HoldoutEvaluationPlanId, _Mapping]] = ..., evaluation_plan_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., evaluation_plan_entry_index: _Optional[int] = ...) -> None: ...

class ArtifactJobInput(_message.Message):
    __slots__ = ("input", "policy", "budget")
    INPUT_FIELD_NUMBER: _ClassVar[int]
    POLICY_FIELD_NUMBER: _ClassVar[int]
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    input: _artifact_pb2.ArtifactRef
    policy: _common_pb2.PolicyReference
    budget: JobBudget
    def __init__(self, input: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., budget: _Optional[_Union[JobBudget, _Mapping]] = ...) -> None: ...

class JobSpecification(_message.Message):
    __slots__ = ("job_id", "run_id", "kind", "discovery", "factor_evaluation", "backtest", "reconciliation", "holdout_backtest", "artifact", "submitted_at", "submitted_by", "idempotency_key", "correlation_id", "causation_id", "protocol_selection")
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    DISCOVERY_FIELD_NUMBER: _ClassVar[int]
    FACTOR_EVALUATION_FIELD_NUMBER: _ClassVar[int]
    BACKTEST_FIELD_NUMBER: _ClassVar[int]
    RECONCILIATION_FIELD_NUMBER: _ClassVar[int]
    HOLDOUT_BACKTEST_FIELD_NUMBER: _ClassVar[int]
    ARTIFACT_FIELD_NUMBER: _ClassVar[int]
    SUBMITTED_AT_FIELD_NUMBER: _ClassVar[int]
    SUBMITTED_BY_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    CORRELATION_ID_FIELD_NUMBER: _ClassVar[int]
    CAUSATION_ID_FIELD_NUMBER: _ClassVar[int]
    PROTOCOL_SELECTION_FIELD_NUMBER: _ClassVar[int]
    job_id: _common_pb2.JobId
    run_id: _common_pb2.RunId
    kind: JobKind
    discovery: DiscoveryJobInput
    factor_evaluation: FactorEvaluationJobInput
    backtest: BacktestJobInput
    reconciliation: ReconciliationJobInput
    holdout_backtest: HoldoutBacktestJobInput
    artifact: ArtifactJobInput
    submitted_at: _timestamp_pb2.Timestamp
    submitted_by: _common_pb2.Actor
    idempotency_key: _common_pb2.IdempotencyKey
    correlation_id: _common_pb2.CorrelationId
    causation_id: _common_pb2.CausationId
    protocol_selection: _common_pb2.ProtocolSelectionSnapshot
    def __init__(self, job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., run_id: _Optional[_Union[_common_pb2.RunId, _Mapping]] = ..., kind: _Optional[_Union[JobKind, str]] = ..., discovery: _Optional[_Union[DiscoveryJobInput, _Mapping]] = ..., factor_evaluation: _Optional[_Union[FactorEvaluationJobInput, _Mapping]] = ..., backtest: _Optional[_Union[BacktestJobInput, _Mapping]] = ..., reconciliation: _Optional[_Union[ReconciliationJobInput, _Mapping]] = ..., holdout_backtest: _Optional[_Union[HoldoutBacktestJobInput, _Mapping]] = ..., artifact: _Optional[_Union[ArtifactJobInput, _Mapping]] = ..., submitted_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., submitted_by: _Optional[_Union[_common_pb2.Actor, _Mapping]] = ..., idempotency_key: _Optional[_Union[_common_pb2.IdempotencyKey, _Mapping]] = ..., correlation_id: _Optional[_Union[_common_pb2.CorrelationId, _Mapping]] = ..., causation_id: _Optional[_Union[_common_pb2.CausationId, _Mapping]] = ..., protocol_selection: _Optional[_Union[_common_pb2.ProtocolSelectionSnapshot, _Mapping]] = ...) -> None: ...

class JobLease(_message.Message):
    __slots__ = ("lease_id", "job_id", "owner", "acquired_revision", "issued_at", "heartbeat_at", "expires_at")
    LEASE_ID_FIELD_NUMBER: _ClassVar[int]
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    OWNER_FIELD_NUMBER: _ClassVar[int]
    ACQUIRED_REVISION_FIELD_NUMBER: _ClassVar[int]
    ISSUED_AT_FIELD_NUMBER: _ClassVar[int]
    HEARTBEAT_AT_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_FIELD_NUMBER: _ClassVar[int]
    lease_id: _common_pb2.LeaseId
    job_id: _common_pb2.JobId
    owner: _common_pb2.Actor
    acquired_revision: int
    issued_at: _timestamp_pb2.Timestamp
    heartbeat_at: _timestamp_pb2.Timestamp
    expires_at: _timestamp_pb2.Timestamp
    def __init__(self, lease_id: _Optional[_Union[_common_pb2.LeaseId, _Mapping]] = ..., job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., owner: _Optional[_Union[_common_pb2.Actor, _Mapping]] = ..., acquired_revision: _Optional[int] = ..., issued_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., heartbeat_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., expires_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class JobSuccess(_message.Message):
    __slots__ = ("outputs",)
    OUTPUTS_FIELD_NUMBER: _ClassVar[int]
    outputs: _containers.RepeatedCompositeFieldContainer[_artifact_pb2.ArtifactRef]
    def __init__(self, outputs: _Optional[_Iterable[_Union[_artifact_pb2.ArtifactRef, _Mapping]]] = ...) -> None: ...

class FactorRejection(_message.Message):
    __slots__ = ("factor_spec_id", "code", "reason", "evidence", "rejected_at")
    FACTOR_SPEC_ID_FIELD_NUMBER: _ClassVar[int]
    CODE_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    EVIDENCE_FIELD_NUMBER: _ClassVar[int]
    REJECTED_AT_FIELD_NUMBER: _ClassVar[int]
    factor_spec_id: _common_pb2.FactorSpecId
    code: FactorRejectionCode
    reason: str
    evidence: _containers.RepeatedCompositeFieldContainer[_artifact_pb2.ArtifactRef]
    rejected_at: _timestamp_pb2.Timestamp
    def __init__(self, factor_spec_id: _Optional[_Union[_common_pb2.FactorSpecId, _Mapping]] = ..., code: _Optional[_Union[FactorRejectionCode, str]] = ..., reason: _Optional[str] = ..., evidence: _Optional[_Iterable[_Union[_artifact_pb2.ArtifactRef, _Mapping]]] = ..., rejected_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class InfrastructureFailure(_message.Message):
    __slots__ = ("error", "attempt", "failed_at")
    ERROR_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    FAILED_AT_FIELD_NUMBER: _ClassVar[int]
    error: _common_pb2.ServiceError
    attempt: int
    failed_at: _timestamp_pb2.Timestamp
    def __init__(self, error: _Optional[_Union[_common_pb2.ServiceError, _Mapping]] = ..., attempt: _Optional[int] = ..., failed_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class JobCancellation(_message.Message):
    __slots__ = ("reason", "cancelled_by", "cancelled_at")
    REASON_FIELD_NUMBER: _ClassVar[int]
    CANCELLED_BY_FIELD_NUMBER: _ClassVar[int]
    CANCELLED_AT_FIELD_NUMBER: _ClassVar[int]
    reason: str
    cancelled_by: _common_pb2.Actor
    cancelled_at: _timestamp_pb2.Timestamp
    def __init__(self, reason: _Optional[str] = ..., cancelled_by: _Optional[_Union[_common_pb2.Actor, _Mapping]] = ..., cancelled_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class BudgetExhaustion(_message.Message):
    __slots__ = ("exhausted_limit", "enforced_budget", "exhausted_at")
    EXHAUSTED_LIMIT_FIELD_NUMBER: _ClassVar[int]
    ENFORCED_BUDGET_FIELD_NUMBER: _ClassVar[int]
    EXHAUSTED_AT_FIELD_NUMBER: _ClassVar[int]
    exhausted_limit: str
    enforced_budget: JobBudget
    exhausted_at: _timestamp_pb2.Timestamp
    def __init__(self, exhausted_limit: _Optional[str] = ..., enforced_budget: _Optional[_Union[JobBudget, _Mapping]] = ..., exhausted_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class JobOutcome(_message.Message):
    __slots__ = ("success", "factor_rejection", "infrastructure_failure", "cancellation", "budget_exhaustion")
    SUCCESS_FIELD_NUMBER: _ClassVar[int]
    FACTOR_REJECTION_FIELD_NUMBER: _ClassVar[int]
    INFRASTRUCTURE_FAILURE_FIELD_NUMBER: _ClassVar[int]
    CANCELLATION_FIELD_NUMBER: _ClassVar[int]
    BUDGET_EXHAUSTION_FIELD_NUMBER: _ClassVar[int]
    success: JobSuccess
    factor_rejection: FactorRejection
    infrastructure_failure: InfrastructureFailure
    cancellation: JobCancellation
    budget_exhaustion: BudgetExhaustion
    def __init__(self, success: _Optional[_Union[JobSuccess, _Mapping]] = ..., factor_rejection: _Optional[_Union[FactorRejection, _Mapping]] = ..., infrastructure_failure: _Optional[_Union[InfrastructureFailure, _Mapping]] = ..., cancellation: _Optional[_Union[JobCancellation, _Mapping]] = ..., budget_exhaustion: _Optional[_Union[BudgetExhaustion, _Mapping]] = ...) -> None: ...

class JobRecord(_message.Message):
    __slots__ = ("specification", "state", "revision", "attempt", "active_lease", "outcome", "updated_at")
    SPECIFICATION_FIELD_NUMBER: _ClassVar[int]
    STATE_FIELD_NUMBER: _ClassVar[int]
    REVISION_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    ACTIVE_LEASE_FIELD_NUMBER: _ClassVar[int]
    OUTCOME_FIELD_NUMBER: _ClassVar[int]
    UPDATED_AT_FIELD_NUMBER: _ClassVar[int]
    specification: JobSpecification
    state: JobState
    revision: int
    attempt: int
    active_lease: JobLease
    outcome: JobOutcome
    updated_at: _timestamp_pb2.Timestamp
    def __init__(self, specification: _Optional[_Union[JobSpecification, _Mapping]] = ..., state: _Optional[_Union[JobState, str]] = ..., revision: _Optional[int] = ..., attempt: _Optional[int] = ..., active_lease: _Optional[_Union[JobLease, _Mapping]] = ..., outcome: _Optional[_Union[JobOutcome, _Mapping]] = ..., updated_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...
