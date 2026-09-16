"""Frozen administrative portfolio requests; none of these records grant authority."""

from decimal import Decimal
from typing import Literal, Self

from pydantic import Field, model_validator

from loop_research.data.fetch_records import CachedObject
from loop_research.data.models import ExactDecimal, ImmutableRecord
from loop_research.transform_models import PolicyDocument

MAX_REPLAY_CELLS = 100_000
MAX_REPLAY_BYTES = 64 * 1024 * 1024
POLICY_ROLES = (
    "universe_policy",
    "data_policy",
    "calendar_policy",
    "preprocess_policy",
    "neutralization_policy",
    "portfolio_policy",
    "execution_policy",
    "cost_policy",
    "evaluation_policy",
)


def money(value: str, *, positive: bool = False, maximum: str = "1000000000000000") -> Decimal:
    """Require finite bounded USD with at most eight fractional digits."""
    parsed = Decimal(value)
    exponent = parsed.as_tuple().exponent
    if (
        not parsed.is_finite()
        or parsed < 0
        or (positive and parsed == 0)
        or parsed > Decimal(maximum)
        or not isinstance(exponent, int)
        or exponent < -8
    ):
        raise ValueError("USD value exceeds the portfolio decimal bounds")
    return parsed


class PortfolioPolicy(ImmutableRecord):
    """Executable options resolved from the FactorSpec's actual frozen documents."""

    initial_cash_usd: ExactDecimal
    holdings: int = Field(ge=1, le=1000)
    lot_size: int = Field(ge=1, le=10000)
    commission_per_share_usd: ExactDecimal
    minimum_commission_usd: ExactDecimal
    half_spread_bps: int = Field(ge=0, le=1000)

    @model_validator(mode="after")
    def valid_money(self) -> Self:
        money(self.initial_cash_usd, positive=True)
        money(self.commission_per_share_usd, maximum="1000")
        money(self.minimum_commission_usd, maximum="1000000")
        return self


class ExecutionTape(ImmutableRecord):
    """Raw declared observations, distinct from adjusted signals or attested feeds."""

    schema_version: Literal["loop.execution-tape/v1"] = Field(alias="schema")
    quality: Literal["synthetic", "public_development"]
    currency: Literal["USD"]
    price_basis: Literal["raw"]
    corporate_actions: Literal["none_in_sample_declared"]
    observations: CachedObject

    @model_validator(mode="after")
    def bounded_observations(self) -> Self:
        if self.observations.byte_size > MAX_REPLAY_BYTES:
            raise ValueError("execution tape exceeds the byte budget")
        return self


class BacktestRequest(ImmutableRecord):
    """Pin actual evaluation inputs/results and every FactorSpec policy document."""

    schema_version: Literal["loop.portfolio-request/v1"] = Field(
        default="loop.portfolio-request/v1", alias="schema"
    )
    evaluation_work: CachedObject
    evaluation_result: CachedObject
    factor_values: CachedObject
    execution_tape: CachedObject
    policies: dict[str, PolicyDocument] = Field(min_length=9, max_length=9)

    @model_validator(mode="after")
    def bounded_request(self) -> Self:
        if set(self.policies) != set(POLICY_ROLES):
            raise ValueError("portfolio request requires every frozen policy role")
        if (
            any(
                reference.byte_size > 1024 * 1024
                for reference in (self.evaluation_work, self.evaluation_result, self.execution_tape)
            )
            or self.factor_values.byte_size > MAX_REPLAY_BYTES
        ):
            raise ValueError("portfolio request exceeds its input byte budget")
        return self


class LedgerArtifacts(ImmutableRecord):
    """Complete modeled accounting evidence; references never embed data rows."""

    targets: CachedObject
    orders: CachedObject
    fills: CachedObject
    positions: CachedObject
    nav: CachedObject
    returns: CachedObject
    costs: CachedObject


class BacktestReceipt(ImmutableRecord):
    """Immutable administrative replay lineage, not an authorized BacktestResult."""

    schema_version: Literal["loop.portfolio-receipt/v1"] = Field(
        default="loop.portfolio-receipt/v1", alias="schema"
    )
    engine: Literal["long-only-next-open.1"] = "long-only-next-open.1"
    request: BacktestRequest
    factor_spec_id: str = Field(pattern=r"^sha256:[0-9a-f]{64}$")
    source_code_sha256: str = Field(pattern=r"^sha256:[0-9a-f]{64}$")
    environment_sha256: str = Field(pattern=r"^sha256:[0-9a-f]{64}$")
    calendar_version: str = Field(min_length=1, max_length=64)
    quality: Literal["synthetic", "public_development"]
    artifacts: LedgerArtifacts
    sessions: int = Field(ge=2, le=8192)
    orders: int = Field(ge=0, le=MAX_REPLAY_CELLS)
    fills: int = Field(ge=0, le=MAX_REPLAY_CELLS)
    ending_nav_usd: ExactDecimal
    production_eligible: Literal[False] = False


class BacktestReport(ImmutableRecord):
    """Small CLI result; detailed positions and prices remain immutable artifacts."""

    receipt: CachedObject
    artifacts: LedgerArtifacts
    quality: Literal["synthetic", "public_development"]
    sessions: int
    orders: int
    fills: int
    ending_nav_usd: ExactDecimal
    production_eligible: Literal[False] = False
