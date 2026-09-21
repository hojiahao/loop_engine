"""Bounded global-report inputs; only the runtime authenticates their population."""

from typing import Literal, Self

from pydantic import Field, model_validator

from loop_research.data.fetch_records import CachedObject
from loop_research.data.models import Identifier, ImmutableRecord
from loop_research.portfolio_worker import PortfolioWork, TrialLedger


class GlobalPolicy(ImmutableRecord):
    """Frozen whole-registry analysis settings, independent of reported performance."""

    schema_version: Literal["loop.global-statistics-policy/v1"] = Field(alias="schema")
    policy_id: Identifier
    revision: str = Field(pattern=r"^[1-9][0-9]{0,19}$")
    scope: Literal["all-database-development-trials"]
    minimum_sessions: int = Field(ge=8, le=8192, strict=True)
    hac_lags: int = Field(ge=0, le=60, strict=True)
    pbo_blocks: int = Field(ge=4, le=10, strict=True)

    @model_validator(mode="after")
    def valid_settings(self) -> Self:
        if (
            int(self.revision) >= 2**64
            or self.hac_lags >= self.minimum_sessions
            or self.pbo_blocks % 2
        ):
            raise ValueError("inconsistent global statistical settings")
        return self


class TrialState(ImmutableRecord):
    """Exact current revision, state and acquired attempt from the durable job."""

    job_id: Identifier
    revision: int = Field(ge=1, lt=2**63, strict=True)
    kind: Literal["factor_evaluation", "backtest"]
    state: int = Field(ge=1, le=8, strict=True)
    attempt: int = Field(ge=0, le=65536, strict=True)


class GlobalSnapshot(ImmutableRecord):
    """All trial identities and states, in the same exact order as the registry."""

    schema_version: Literal["loop.global-snapshot/v1"] = Field(
        default="loop.global-snapshot/v1", alias="schema"
    )
    ledger: TrialLedger
    states: tuple[TrialState, ...] = Field(min_length=1, max_length=4096)

    @model_validator(mode="after")
    def complete_states(self) -> Self:
        if tuple(item.job_id for item in self.states) != tuple(
            item.job_id for item in self.ledger.entries
        ):
            raise ValueError("global state population differs from the registry")
        if any(
            max(state.attempt, 1) != entry.attempts
            for state, entry in zip(self.states, self.ledger.entries, strict=True)
        ):
            raise ValueError("global attempt projection differs")
        return self


class GlobalPortfolio(ImmutableRecord):
    """One actual registered portfolio and its checked numerical predecessor."""

    job_id: Identifier
    evaluation_job_id: Identifier
    lease_id: Identifier
    specification: CachedObject
    request: CachedObject
    manifest: CachedObject

    def work(self, ledger: TrialLedger) -> PortfolioWork:
        """Reconstruct the existing producer request without repeating the ledger."""
        return PortfolioWork(
            schema="loop.portfolio-work/v1",
            job_id=self.job_id,
            lease_id=self.lease_id,
            specification=self.specification,
            request=self.request,
            manifest=self.manifest,
            trials=ledger,
        )


class GlobalWork(ImmutableRecord):
    """Reference-only installed-worker envelope, not caller-controlled authority."""

    schema_version: Literal["loop.global-statistics-work/v1"] = Field(alias="schema")
    job_id: Identifier
    lease_id: Identifier
    started_at_ms: int = Field(ge=0, lt=2**63, strict=True)
    policy: CachedObject
    snapshot: GlobalSnapshot
    portfolios: tuple[GlobalPortfolio, ...] = Field(max_length=64)
    manifest: CachedObject | None = None

    @model_validator(mode="after")
    def complete_sources(self) -> Self:
        states = {item.job_id: item for item in self.snapshot.states}
        expected = tuple(
            item.job_id
            for item in self.snapshot.states
            if item.kind == "backtest" and item.state == 4
        )
        if self.job_id in states or tuple(item.job_id for item in self.portfolios) != expected:
            raise ValueError("global portfolios differ from all successful registered backtests")
        for source in self.portfolios:
            predecessor = states.get(source.evaluation_job_id)
            if predecessor is None or predecessor.kind != "factor_evaluation":
                raise ValueError("global portfolio predecessor is outside the registry")
        return self
