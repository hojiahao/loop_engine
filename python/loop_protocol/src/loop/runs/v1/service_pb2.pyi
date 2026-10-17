import datetime

from google.protobuf import duration_pb2 as _duration_pb2
from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.discovery.v1 import service_pb2 as _service_pb2
from loop.research.v1 import service_pb2 as _service_pb2_1
from loop.v1 import common_pb2 as _common_pb2
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class RunStatus(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    RUN_STATUS_UNSPECIFIED: _ClassVar[RunStatus]
    RUN_STATUS_ACTIVE: _ClassVar[RunStatus]
    RUN_STATUS_COMPLETED: _ClassVar[RunStatus]
    RUN_STATUS_BUDGET_EXHAUSTED: _ClassVar[RunStatus]
    RUN_STATUS_INFRASTRUCTURE_FAILED: _ClassVar[RunStatus]
    RUN_STATUS_DEADLINE_EXCEEDED: _ClassVar[RunStatus]
    RUN_STATUS_SEARCH_EXHAUSTED: _ClassVar[RunStatus]

class RunStage(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    RUN_STAGE_UNSPECIFIED: _ClassVar[RunStage]
    RUN_STAGE_DISCOVERY: _ClassVar[RunStage]
    RUN_STAGE_EVALUATION: _ClassVar[RunStage]
    RUN_STAGE_PORTFOLIO: _ClassVar[RunStage]
    RUN_STAGE_PERTURBATION: _ClassVar[RunStage]

class RoundDisposition(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    ROUND_DISPOSITION_UNSPECIFIED: _ClassVar[RoundDisposition]
    ROUND_DISPOSITION_EVALUATED: _ClassVar[RoundDisposition]
    ROUND_DISPOSITION_DUPLICATE: _ClassVar[RoundDisposition]
    ROUND_DISPOSITION_PREVIOUSLY_REJECTED: _ClassVar[RoundDisposition]
    ROUND_DISPOSITION_INSUFFICIENT_COVERAGE: _ClassVar[RoundDisposition]
    ROUND_DISPOSITION_EXHAUSTED: _ClassVar[RoundDisposition]
RUN_STATUS_UNSPECIFIED: RunStatus
RUN_STATUS_ACTIVE: RunStatus
RUN_STATUS_COMPLETED: RunStatus
RUN_STATUS_BUDGET_EXHAUSTED: RunStatus
RUN_STATUS_INFRASTRUCTURE_FAILED: RunStatus
RUN_STATUS_DEADLINE_EXCEEDED: RunStatus
RUN_STATUS_SEARCH_EXHAUSTED: RunStatus
RUN_STAGE_UNSPECIFIED: RunStage
RUN_STAGE_DISCOVERY: RunStage
RUN_STAGE_EVALUATION: RunStage
RUN_STAGE_PORTFOLIO: RunStage
RUN_STAGE_PERTURBATION: RunStage
ROUND_DISPOSITION_UNSPECIFIED: RoundDisposition
ROUND_DISPOSITION_EVALUATED: RoundDisposition
ROUND_DISPOSITION_DUPLICATE: RoundDisposition
ROUND_DISPOSITION_PREVIOUSLY_REJECTED: RoundDisposition
ROUND_DISPOSITION_INSUFFICIENT_COVERAGE: RoundDisposition
ROUND_DISPOSITION_EXHAUSTED: RoundDisposition

class RunBudget(_message.Message):
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

class RunSpecification(_message.Message):
    __slots__ = ("plan", "run_id", "owner", "executor", "discovery", "protocol_selection", "budget", "maximum_rounds", "numerical", "feedback_policy")
    PLAN_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    OWNER_FIELD_NUMBER: _ClassVar[int]
    EXECUTOR_FIELD_NUMBER: _ClassVar[int]
    DISCOVERY_FIELD_NUMBER: _ClassVar[int]
    PROTOCOL_SELECTION_FIELD_NUMBER: _ClassVar[int]
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_ROUNDS_FIELD_NUMBER: _ClassVar[int]
    NUMERICAL_FIELD_NUMBER: _ClassVar[int]
    FEEDBACK_POLICY_FIELD_NUMBER: _ClassVar[int]
    plan: _common_pb2.PolicyReference
    run_id: _common_pb2.RunId
    owner: _common_pb2.Actor
    executor: _common_pb2.Actor
    discovery: _service_pb2.DiscoveryJobInput
    protocol_selection: _common_pb2.ProtocolSelectionSnapshot
    budget: RunBudget
    maximum_rounds: int
    numerical: NumericalWorkflow
    feedback_policy: _common_pb2.PolicyReference
    def __init__(self, plan: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., run_id: _Optional[_Union[_common_pb2.RunId, _Mapping]] = ..., owner: _Optional[_Union[_common_pb2.Actor, _Mapping]] = ..., executor: _Optional[_Union[_common_pb2.Actor, _Mapping]] = ..., discovery: _Optional[_Union[_service_pb2.DiscoveryJobInput, _Mapping]] = ..., protocol_selection: _Optional[_Union[_common_pb2.ProtocolSelectionSnapshot, _Mapping]] = ..., budget: _Optional[_Union[RunBudget, _Mapping]] = ..., maximum_rounds: _Optional[int] = ..., numerical: _Optional[_Union[NumericalWorkflow, _Mapping]] = ..., feedback_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ...) -> None: ...

class NumericalWorkflow(_message.Message):
    __slots__ = ("research_executor", "recipe", "evaluation_budget", "portfolio_budget", "protocol_selection", "perturbation_budget")
    RESEARCH_EXECUTOR_FIELD_NUMBER: _ClassVar[int]
    RECIPE_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_BUDGET_FIELD_NUMBER: _ClassVar[int]
    PORTFOLIO_BUDGET_FIELD_NUMBER: _ClassVar[int]
    PROTOCOL_SELECTION_FIELD_NUMBER: _ClassVar[int]
    PERTURBATION_BUDGET_FIELD_NUMBER: _ClassVar[int]
    research_executor: _common_pb2.Actor
    recipe: _common_pb2.PolicyReference
    evaluation_budget: _service_pb2_1.ResearchJobBudget
    portfolio_budget: _service_pb2_1.ResearchJobBudget
    protocol_selection: _common_pb2.ProtocolSelectionSnapshot
    perturbation_budget: _service_pb2_1.ResearchJobBudget
    def __init__(self, research_executor: _Optional[_Union[_common_pb2.Actor, _Mapping]] = ..., recipe: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., evaluation_budget: _Optional[_Union[_service_pb2_1.ResearchJobBudget, _Mapping]] = ..., portfolio_budget: _Optional[_Union[_service_pb2_1.ResearchJobBudget, _Mapping]] = ..., protocol_selection: _Optional[_Union[_common_pb2.ProtocolSelectionSnapshot, _Mapping]] = ..., perturbation_budget: _Optional[_Union[_service_pb2_1.ResearchJobBudget, _Mapping]] = ...) -> None: ...

class ResearchRunView(_message.Message):
    __slots__ = ("round", "stage", "discovery", "research", "discovery_job_id", "evaluation_job_id", "expression_id", "factor_spec_id", "binding_sha256", "disposition", "perturbation_job_id")
    ROUND_FIELD_NUMBER: _ClassVar[int]
    STAGE_FIELD_NUMBER: _ClassVar[int]
    DISCOVERY_FIELD_NUMBER: _ClassVar[int]
    RESEARCH_FIELD_NUMBER: _ClassVar[int]
    DISCOVERY_JOB_ID_FIELD_NUMBER: _ClassVar[int]
    EVALUATION_JOB_ID_FIELD_NUMBER: _ClassVar[int]
    EXPRESSION_ID_FIELD_NUMBER: _ClassVar[int]
    FACTOR_SPEC_ID_FIELD_NUMBER: _ClassVar[int]
    BINDING_SHA256_FIELD_NUMBER: _ClassVar[int]
    DISPOSITION_FIELD_NUMBER: _ClassVar[int]
    PERTURBATION_JOB_ID_FIELD_NUMBER: _ClassVar[int]
    round: int
    stage: RunStage
    discovery: _service_pb2.DiscoveryJobHandle
    research: _service_pb2_1.ResearchJobHandle
    discovery_job_id: _common_pb2.JobId
    evaluation_job_id: _common_pb2.JobId
    expression_id: _common_pb2.FactorExpressionId
    factor_spec_id: _common_pb2.FactorSpecId
    binding_sha256: _common_pb2.Sha256Digest
    disposition: RoundDisposition
    perturbation_job_id: _common_pb2.JobId
    def __init__(self, round: _Optional[int] = ..., stage: _Optional[_Union[RunStage, str]] = ..., discovery: _Optional[_Union[_service_pb2.DiscoveryJobHandle, _Mapping]] = ..., research: _Optional[_Union[_service_pb2_1.ResearchJobHandle, _Mapping]] = ..., discovery_job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., evaluation_job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., expression_id: _Optional[_Union[_common_pb2.FactorExpressionId, _Mapping]] = ..., factor_spec_id: _Optional[_Union[_common_pb2.FactorSpecId, _Mapping]] = ..., binding_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., disposition: _Optional[_Union[RoundDisposition, str]] = ..., perturbation_job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ...) -> None: ...

class AdaptiveRunView(_message.Message):
    __slots__ = ("feedback_policy", "source_count", "feedback_sha256", "model_context_sha256", "family_sha256", "random_draws", "state_sha256")
    FEEDBACK_POLICY_FIELD_NUMBER: _ClassVar[int]
    SOURCE_COUNT_FIELD_NUMBER: _ClassVar[int]
    FEEDBACK_SHA256_FIELD_NUMBER: _ClassVar[int]
    MODEL_CONTEXT_SHA256_FIELD_NUMBER: _ClassVar[int]
    FAMILY_SHA256_FIELD_NUMBER: _ClassVar[int]
    RANDOM_DRAWS_FIELD_NUMBER: _ClassVar[int]
    STATE_SHA256_FIELD_NUMBER: _ClassVar[int]
    feedback_policy: _common_pb2.PolicyReference
    source_count: int
    feedback_sha256: _common_pb2.Sha256Digest
    model_context_sha256: _common_pb2.Sha256Digest
    family_sha256: _common_pb2.Sha256Digest
    random_draws: int
    state_sha256: _common_pb2.Sha256Digest
    def __init__(self, feedback_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., source_count: _Optional[int] = ..., feedback_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., model_context_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., family_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., random_draws: _Optional[int] = ..., state_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class RunView(_message.Message):
    __slots__ = ("run_id", "status", "revision", "maximum_rounds", "completed_rounds", "current_job", "budget", "reserved_steps", "reserved_input_tokens", "reserved_output_tokens", "reserved_cost", "submitted_at", "updated_at", "deadline", "plan_verified", "research", "adaptive")
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STATUS_FIELD_NUMBER: _ClassVar[int]
    REVISION_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_ROUNDS_FIELD_NUMBER: _ClassVar[int]
    COMPLETED_ROUNDS_FIELD_NUMBER: _ClassVar[int]
    CURRENT_JOB_FIELD_NUMBER: _ClassVar[int]
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    RESERVED_STEPS_FIELD_NUMBER: _ClassVar[int]
    RESERVED_INPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    RESERVED_OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    RESERVED_COST_FIELD_NUMBER: _ClassVar[int]
    SUBMITTED_AT_FIELD_NUMBER: _ClassVar[int]
    UPDATED_AT_FIELD_NUMBER: _ClassVar[int]
    DEADLINE_FIELD_NUMBER: _ClassVar[int]
    PLAN_VERIFIED_FIELD_NUMBER: _ClassVar[int]
    RESEARCH_FIELD_NUMBER: _ClassVar[int]
    ADAPTIVE_FIELD_NUMBER: _ClassVar[int]
    run_id: _common_pb2.RunId
    status: RunStatus
    revision: int
    maximum_rounds: int
    completed_rounds: int
    current_job: _service_pb2.DiscoveryJobHandle
    budget: RunBudget
    reserved_steps: int
    reserved_input_tokens: int
    reserved_output_tokens: int
    reserved_cost: _common_pb2.Money
    submitted_at: _timestamp_pb2.Timestamp
    updated_at: _timestamp_pb2.Timestamp
    deadline: _timestamp_pb2.Timestamp
    plan_verified: bool
    research: ResearchRunView
    adaptive: AdaptiveRunView
    def __init__(self, run_id: _Optional[_Union[_common_pb2.RunId, _Mapping]] = ..., status: _Optional[_Union[RunStatus, str]] = ..., revision: _Optional[int] = ..., maximum_rounds: _Optional[int] = ..., completed_rounds: _Optional[int] = ..., current_job: _Optional[_Union[_service_pb2.DiscoveryJobHandle, _Mapping]] = ..., budget: _Optional[_Union[RunBudget, _Mapping]] = ..., reserved_steps: _Optional[int] = ..., reserved_input_tokens: _Optional[int] = ..., reserved_output_tokens: _Optional[int] = ..., reserved_cost: _Optional[_Union[_common_pb2.Money, _Mapping]] = ..., submitted_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., updated_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., deadline: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., plan_verified: _Optional[bool] = ..., research: _Optional[_Union[ResearchRunView, _Mapping]] = ..., adaptive: _Optional[_Union[AdaptiveRunView, _Mapping]] = ...) -> None: ...

class StartRunRequest(_message.Message):
    __slots__ = ("context", "plan", "workflow_version")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    PLAN_FIELD_NUMBER: _ClassVar[int]
    WORKFLOW_VERSION_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    plan: _common_pb2.PolicyReference
    workflow_version: int
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., plan: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ..., workflow_version: _Optional[int] = ...) -> None: ...

class StartRunResponse(_message.Message):
    __slots__ = ("run",)
    RUN_FIELD_NUMBER: _ClassVar[int]
    run: RunView
    def __init__(self, run: _Optional[_Union[RunView, _Mapping]] = ...) -> None: ...

class StepRunRequest(_message.Message):
    __slots__ = ("context", "run_id", "expected_revision", "workflow_version")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_REVISION_FIELD_NUMBER: _ClassVar[int]
    WORKFLOW_VERSION_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    run_id: _common_pb2.RunId
    expected_revision: int
    workflow_version: int
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., run_id: _Optional[_Union[_common_pb2.RunId, _Mapping]] = ..., expected_revision: _Optional[int] = ..., workflow_version: _Optional[int] = ...) -> None: ...

class StepRunResponse(_message.Message):
    __slots__ = ("run",)
    RUN_FIELD_NUMBER: _ClassVar[int]
    run: RunView
    def __init__(self, run: _Optional[_Union[RunView, _Mapping]] = ...) -> None: ...

class GetRunRequest(_message.Message):
    __slots__ = ("context", "run_id", "workflow_version")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    WORKFLOW_VERSION_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    run_id: _common_pb2.RunId
    workflow_version: int
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., run_id: _Optional[_Union[_common_pb2.RunId, _Mapping]] = ..., workflow_version: _Optional[int] = ...) -> None: ...

class GetRunResponse(_message.Message):
    __slots__ = ("run",)
    RUN_FIELD_NUMBER: _ClassVar[int]
    run: RunView
    def __init__(self, run: _Optional[_Union[RunView, _Mapping]] = ...) -> None: ...
