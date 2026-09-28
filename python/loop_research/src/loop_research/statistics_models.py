"""Frozen, bounded administrative statistics contracts (ADR 0031)."""

import re
from typing import Literal, Self

from pydantic import Field, model_validator

from loop_research.data.fetch_records import CachedObject
from loop_research.data.models import ImmutableRecord
from loop_research.transform_models import PolicyDocument

HASH = r"^sha256:[0-9a-f]{64}$"
IDENTIFIER = r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$"


class Statistic(ImmutableRecord):
    """A finite numerical result or an explicit reason that it is undefined."""

    status: Literal["available", "unavailable"]
    value: float | None = Field(default=None, allow_inf_nan=False)
    reason: str | None = None
    observations: int = Field(ge=0)

    @model_validator(mode="after")
    def valid_outcome(self) -> Self:
        if (self.status == "available") != (self.value is not None and self.reason is None):
            raise ValueError("statistic status does not match its value")
        if self.status == "unavailable" and (self.value is not None or not self.reason):
            raise ValueError("unavailable statistic requires a reason and no value")
        return self


def available(value: float, count: int) -> Statistic:
    """Create a finite result; NaN/Infinity never become JSON metrics."""
    return Statistic(status="available", value=float(value), observations=count)


def unavailable(reason: str, count: int = 0) -> Statistic:
    """Retain missingness rather than guessing a metric or test result."""
    return Statistic(status="unavailable", reason=reason, observations=count)


class StatisticsPolicy(ImmutableRecord):
    """All selection/uncertainty settings are frozen in the FactorSpec policy."""

    groups: int = Field(ge=2, le=10)
    minimum_cross_section: int = Field(ge=3, le=1000)
    minimum_sessions: int = Field(ge=8, le=8192)
    hac_lags: int = Field(ge=0, le=60)
    pbo_blocks: int = Field(ge=4, le=10)
    experiment_plan: str | None = Field(default=None, pattern=HASH)
    trial_id: str | None = Field(default=None, pattern=IDENTIFIER)

    @model_validator(mode="after")
    def valid_settings(self) -> Self:
        if (
            self.minimum_cross_section < self.groups
            or self.hac_lags >= self.minimum_sessions
            or self.pbo_blocks % 2
            or (self.experiment_plan is None) != (self.trial_id is None)
        ):
            raise ValueError("inconsistent frozen statistical settings")
        return self


def resolve_statistics(document: PolicyDocument) -> StatisticsPolicy | None:
    """Recognize only coverage v1 or the complete opt-in statistics profile."""
    settings = document.settings
    if set(settings) == {"minimum_coverage_bps"}:
        return None
    integers = {"groups", "minimum_cross_section", "minimum_sessions", "hac_lags", "pbo_blocks"}
    required = integers | {"minimum_coverage_bps", "statistics_profile"}
    extra = {"experiment_plan", "trial_id"}
    if set(settings) not in (required, required | extra):
        raise ValueError("unsupported evaluation statistics policy")
    if settings["statistics_profile"] != "daily-statistics.1":
        raise ValueError("unsupported statistics profile")
    if any(not re.fullmatch(r"0|[1-9][0-9]{0,4}", settings[key]) for key in integers):
        raise ValueError("statistical settings require bounded canonical integers")
    values: dict[str, object] = {key: int(settings[key]) for key in integers}
    if extra <= set(settings):
        values.update(
            experiment_plan="sha256:" + settings["experiment_plan"], trial_id=settings["trial_id"]
        )
    return StatisticsPolicy.model_validate(values)


class PlannedTrial(ImmutableRecord):
    """Predeclared candidate/work identity, excluding the circular evaluation policy."""

    trial_id: str = Field(pattern=IDENTIFIER)
    binding_sha256: str = Field(pattern=HASH)


class ExperimentPlan(ImmutableRecord):
    """Completeness is scoped to this explicit finite family, not all prior research."""

    schema_version: Literal["loop.experiment-plan/v1"] = Field(
        default="loop.experiment-plan/v1", alias="schema"
    )
    family_id: str = Field(pattern=IDENTIFIER)
    trials: tuple[PlannedTrial, ...] = Field(min_length=2, max_length=64)

    @model_validator(mode="after")
    def unique_trials(self) -> Self:
        if len({trial.trial_id for trial in self.trials}) != len(self.trials) or len(
            {trial.binding_sha256 for trial in self.trials}
        ) != len(self.trials):
            raise ValueError("experiment trials and bindings must be unique")
        return self


class TrialOutcome(ImmutableRecord):
    """One outcome per planned trial; unsuccessful attempts remain in the family."""

    trial_id: str = Field(pattern=IDENTIFIER)
    backtest: CachedObject | None = None
    failure: CachedObject | None = None

    @model_validator(mode="after")
    def one_outcome(self) -> Self:
        if (self.backtest is None) == (self.failure is None):
            raise ValueError("trial requires exactly one outcome")
        return self


class TrialFailure(ImmutableRecord):
    """Content-bound administrative failure, not authenticated runtime testimony."""

    schema_version: Literal["loop.trial-failure/v1"] = Field(
        default="loop.trial-failure/v1", alias="schema"
    )
    trial_id: str = Field(pattern=IDENTIFIER)
    binding_sha256: str = Field(pattern=HASH)
    kind: Literal["rejected", "infrastructure_failure", "cancelled"]
    reason: str = Field(pattern=IDENTIFIER)


class ExperimentEvidence(ImmutableRecord):
    """Full ordered outcomes for the predeclared plan, including unsuccessful trials."""

    schema_version: Literal["loop.experiment-evidence/v1"] = Field(
        default="loop.experiment-evidence/v1", alias="schema"
    )
    plan: CachedObject
    outcomes: tuple[TrialOutcome, ...] = Field(min_length=2, max_length=64)


class StatisticsRequest(ImmutableRecord):
    """References to actual portfolio receipts; all data stays in existing stores."""

    schema_version: Literal["loop.statistics-request/v1"] = Field(
        default="loop.statistics-request/v1", alias="schema"
    )
    backtest: CachedObject
    experiment: CachedObject | None = None


class StatisticsReceipt(ImmutableRecord):
    """Immutable result references. These diagnostics cannot authorize admission."""

    schema_version: Literal["loop.statistics-receipt/v1"] = Field(
        default="loop.statistics-receipt/v1", alias="schema"
    )
    request: StatisticsRequest
    summary: CachedObject
    cross_sections: CachedObject
    portfolio: CachedObject
    exposures: CachedObject
    multiple_testing: CachedObject
    source_code_sha256: str = Field(pattern=HASH)
    environment_sha256: str = Field(pattern=HASH)
    production_eligible: Literal[False] = False


class StatisticsReport(ImmutableRecord):
    """Small CLI handle for a replayable statistics receipt."""

    receipt: CachedObject
    artifacts: StatisticsReceipt
