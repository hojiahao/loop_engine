from loop.v1 import common_pb2 as _common_pb2
from loop.v1 import development_data_pb2 as _development_data_pb2
from loop.v1 import research_common_pb2 as _research_common_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class PerturbationReason(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    PERTURBATION_REASON_UNSPECIFIED: _ClassVar[PerturbationReason]
    PERTURBATION_REASON_EXPLORATION: _ClassVar[PerturbationReason]
    PERTURBATION_REASON_GRADIENT: _ClassVar[PerturbationReason]
    PERTURBATION_REASON_EXHAUSTED: _ClassVar[PerturbationReason]
PERTURBATION_REASON_UNSPECIFIED: PerturbationReason
PERTURBATION_REASON_EXPLORATION: PerturbationReason
PERTURBATION_REASON_GRADIENT: PerturbationReason
PERTURBATION_REASON_EXHAUSTED: PerturbationReason

class PerturbationSpace(_message.Message):
    __slots__ = ("algorithm", "dataset", "provenance", "backtest_seed", "random_seed", "candidates")
    ALGORITHM_FIELD_NUMBER: _ClassVar[int]
    DATASET_FIELD_NUMBER: _ClassVar[int]
    PROVENANCE_FIELD_NUMBER: _ClassVar[int]
    BACKTEST_SEED_FIELD_NUMBER: _ClassVar[int]
    RANDOM_SEED_FIELD_NUMBER: _ClassVar[int]
    CANDIDATES_FIELD_NUMBER: _ClassVar[int]
    algorithm: str
    dataset: _development_data_pb2.DevelopmentDatasetReference
    provenance: _research_common_pb2.ResearchProvenanceFingerprint
    backtest_seed: _common_pb2.Sha256Digest
    random_seed: _common_pb2.Sha256Digest
    candidates: _containers.RepeatedCompositeFieldContainer[WindowCandidate]
    def __init__(self, algorithm: _Optional[str] = ..., dataset: _Optional[_Union[_development_data_pb2.DevelopmentDatasetReference, _Mapping]] = ..., provenance: _Optional[_Union[_research_common_pb2.ResearchProvenanceFingerprint, _Mapping]] = ..., backtest_seed: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., random_seed: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., candidates: _Optional[_Iterable[_Union[WindowCandidate, _Mapping]]] = ...) -> None: ...

class WindowCandidate(_message.Message):
    __slots__ = ("window", "factor_spec_id")
    WINDOW_FIELD_NUMBER: _ClassVar[int]
    FACTOR_SPEC_ID_FIELD_NUMBER: _ClassVar[int]
    window: int
    factor_spec_id: _common_pb2.FactorSpecId
    def __init__(self, window: _Optional[int] = ..., factor_spec_id: _Optional[_Union[_common_pb2.FactorSpecId, _Mapping]] = ...) -> None: ...

class WindowObservation(_message.Message):
    __slots__ = ("source_job_id", "candidate", "net_sharpe")
    SOURCE_JOB_ID_FIELD_NUMBER: _ClassVar[int]
    CANDIDATE_FIELD_NUMBER: _ClassVar[int]
    NET_SHARPE_FIELD_NUMBER: _ClassVar[int]
    source_job_id: _common_pb2.JobId
    candidate: WindowCandidate
    net_sharpe: float
    def __init__(self, source_job_id: _Optional[_Union[_common_pb2.JobId, _Mapping]] = ..., candidate: _Optional[_Union[WindowCandidate, _Mapping]] = ..., net_sharpe: _Optional[float] = ...) -> None: ...

class PerturbationState(_message.Message):
    __slots__ = ("version", "random_seed", "random_draws", "history", "momentum", "second_moment", "proposed_factor_ids")
    VERSION_FIELD_NUMBER: _ClassVar[int]
    RANDOM_SEED_FIELD_NUMBER: _ClassVar[int]
    RANDOM_DRAWS_FIELD_NUMBER: _ClassVar[int]
    HISTORY_FIELD_NUMBER: _ClassVar[int]
    MOMENTUM_FIELD_NUMBER: _ClassVar[int]
    SECOND_MOMENT_FIELD_NUMBER: _ClassVar[int]
    PROPOSED_FACTOR_IDS_FIELD_NUMBER: _ClassVar[int]
    version: int
    random_seed: _common_pb2.Sha256Digest
    random_draws: int
    history: _containers.RepeatedCompositeFieldContainer[WindowObservation]
    momentum: float
    second_moment: float
    proposed_factor_ids: _containers.RepeatedCompositeFieldContainer[_common_pb2.FactorSpecId]
    def __init__(self, version: _Optional[int] = ..., random_seed: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., random_draws: _Optional[int] = ..., history: _Optional[_Iterable[_Union[WindowObservation, _Mapping]]] = ..., momentum: _Optional[float] = ..., second_moment: _Optional[float] = ..., proposed_factor_ids: _Optional[_Iterable[_Union[_common_pb2.FactorSpecId, _Mapping]]] = ...) -> None: ...

class PerturbationWork(_message.Message):
    __slots__ = ("state", "candidates", "failed_factor_ids", "observation", "current_window")
    STATE_FIELD_NUMBER: _ClassVar[int]
    CANDIDATES_FIELD_NUMBER: _ClassVar[int]
    FAILED_FACTOR_IDS_FIELD_NUMBER: _ClassVar[int]
    OBSERVATION_FIELD_NUMBER: _ClassVar[int]
    CURRENT_WINDOW_FIELD_NUMBER: _ClassVar[int]
    state: PerturbationState
    candidates: _containers.RepeatedCompositeFieldContainer[WindowCandidate]
    failed_factor_ids: _containers.RepeatedCompositeFieldContainer[_common_pb2.FactorSpecId]
    observation: WindowObservation
    current_window: int
    def __init__(self, state: _Optional[_Union[PerturbationState, _Mapping]] = ..., candidates: _Optional[_Iterable[_Union[WindowCandidate, _Mapping]]] = ..., failed_factor_ids: _Optional[_Iterable[_Union[_common_pb2.FactorSpecId, _Mapping]]] = ..., observation: _Optional[_Union[WindowObservation, _Mapping]] = ..., current_window: _Optional[int] = ...) -> None: ...

class PerturbationStep(_message.Message):
    __slots__ = ("state", "candidate", "reason")
    STATE_FIELD_NUMBER: _ClassVar[int]
    CANDIDATE_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    state: PerturbationState
    candidate: WindowCandidate
    reason: PerturbationReason
    def __init__(self, state: _Optional[_Union[PerturbationState, _Mapping]] = ..., candidate: _Optional[_Union[WindowCandidate, _Mapping]] = ..., reason: _Optional[_Union[PerturbationReason, str]] = ...) -> None: ...
