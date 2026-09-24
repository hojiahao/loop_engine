"""Versioned development execution evidence and explicit accounting assumptions."""

from datetime import date
from typing import Annotated, Literal, Self

from pydantic import Field, model_validator

from loop_research.backtest_models import MAX_REPLAY_BYTES, PortfolioPolicy, money
from loop_research.data.fetch_records import CachedObject
from loop_research.data.models import ExactDecimal, ImmutableRecord, Instant, TemporalRecord
from loop_research.panel_models import PanelSecurity


class MarketPolicy(PortfolioPolicy):
    """Frozen research model, not a broker margin/settlement or compliance model."""

    long_weight_bps: int = Field(ge=0, le=20000)
    short_weight_bps: int = Field(ge=0, le=20000)
    initial_margin_bps: int = Field(ge=5000, le=10000)
    maintenance_margin_bps: int = Field(ge=2500, le=10000)
    participation_bps: int = Field(ge=1, le=10000)
    impact_bps: int = Field(ge=0, le=5000)
    short_collateral_bps: int = Field(ge=10000, le=30000)
    cash_debit_bps: int = Field(ge=0, le=100000)
    cash_credit_bps: int = Field(ge=0, le=10000)
    day_count: Literal[360, 365]

    @model_validator(mode="after")
    def valid_exposures(self) -> Self:
        gross = self.long_weight_bps + self.short_weight_bps
        if not 0 < gross <= 20000 or gross * self.initial_margin_bps > 100_000_000:
            raise ValueError("target gross exposure exceeds initial margin policy")
        if self.maintenance_margin_bps > self.initial_margin_bps:
            raise ValueError("maintenance margin exceeds initial margin")
        return self


class ExecutionPrice(TemporalRecord):
    """Raw opening-auction print or closing mark, with public revision clocks.

    Auction volume belongs to the same observed event, never an end-of-day bar.
    A modeled fill at its dissemination time is not a guarantee of auction access.
    """

    security_id: PanelSecurity
    session: date
    kind: Literal["open", "close"]
    price_usd: ExactDecimal
    auction_volume: int | None = Field(default=None, ge=0, le=10**12)

    @model_validator(mode="after")
    def valid_price(self) -> Self:
        if money(self.price_usd, positive=True, maximum="1000000000") < money("0.000001"):
            raise ValueError("execution price is below the supported bound")
        if (self.kind == "open") != (self.auction_volume is not None):
            raise ValueError("only opening prints require explicit auction volume")
        if self.known_at < self.effective_at:
            raise ValueError("a price cannot be public before its observed event")
        return self


class ExecutionTerms(TemporalRecord):
    """Time-limited trading/borrow declaration; availability is an absolute limit.

    A zero limit or recall forces existing shorts to be reduced. A short-sale
    restriction blocks new shorts but does not, by itself, recall existing loans.
    These declarations do not certify Regulation SHO or broker authorization.
    """

    security_id: PanelSecurity
    session: date
    valid_until: Instant
    tradable: bool
    short_allowed: bool
    borrow_limit: int = Field(ge=0, le=10**12)
    borrow_rate_bps: int = Field(ge=0, le=100000)
    recalled: bool
    sec_fee_usd_per_million: ExactDecimal
    taf_fee_usd_per_share: ExactDecimal
    taf_fee_cap_usd: ExactDecimal

    @model_validator(mode="after")
    def valid_interval(self) -> Self:
        if self.valid_until <= self.effective_at:
            raise ValueError("execution terms require a positive validity interval")
        money(self.sec_fee_usd_per_million, maximum="1000")
        rate = money(self.taf_fee_usd_per_share, maximum="1")
        cap = money(self.taf_fee_cap_usd, maximum="1000000")
        if rate and not cap:
            raise ValueError("a nonzero TAF pass-through rate requires an explicit cap")
        if "." in self.taf_fee_cap_usd and len(self.taf_fee_cap_usd.split(".")[1].rstrip("0")) > 2:
            raise ValueError("TAF pass-through cap requires whole cents")
        return self


class Action(TemporalRecord):
    """One source-backed effective event; revisions never rewrite earlier decisions."""

    event_id: PanelSecurity
    security_id: PanelSecurity
    session: date


class Split(Action):
    """Whole-share conversion; a residual requires already known cash-in-lieu terms."""

    kind: Literal["split"]
    numerator: int = Field(ge=1, le=1_000_000)
    denominator: int = Field(ge=1, le=1_000_000)
    fraction_price_usd: ExactDecimal | None = None
    pay_at: Instant | None = None

    @model_validator(mode="after")
    def valid_split(self) -> Self:
        if self.numerator == self.denominator:
            raise ValueError("a split must change the share ratio")
        if (self.fraction_price_usd is None) != (self.pay_at is None):
            raise ValueError("cash-in-lieu price and payment time must be paired")
        if self.fraction_price_usd is not None:
            money(self.fraction_price_usd, positive=True, maximum="1000000000")
        if self.pay_at is not None and self.pay_at < self.effective_at:
            raise ValueError("cash-in-lieu cannot precede the split")
        return self


class CashAction(Action):
    """Ordinary cash dividend or final cash-only delisting; other actions reject."""

    kind: Literal["dividend", "delisting"]
    amount_per_share_usd: ExactDecimal
    pay_at: Instant

    @model_validator(mode="after")
    def valid_payment(self) -> Self:
        money(self.amount_per_share_usd, maximum="1000000000", positive=self.kind == "dividend")
        if self.pay_at < self.effective_at:
            raise ValueError("action payment cannot precede entitlement")
        return self


CorporateAction = Annotated[Split | CashAction, Field(discriminator="kind")]


class ExecutionCapture(ImmutableRecord):
    """Bounded declared history backed by immutable raw source objects.

    Source bytes are integrity evidence, not vendor-attested historical coverage.
    No daily-bar adapter can manufacture opening auction or borrow observations.
    """

    schema_version: Literal["loop.execution-capture/v1"] = Field(alias="schema")
    captured_at: Instant
    prices: tuple[ExecutionPrice, ...] = Field(max_length=200_000)
    terms: tuple[ExecutionTerms, ...] = Field(max_length=100_000)
    actions: tuple[CorporateAction, ...] = Field(max_length=10000)
    sources: tuple[CachedObject, ...] = Field(min_length=1, max_length=128)

    @model_validator(mode="after")
    def valid_sources(self) -> Self:
        digests = [source.sha256 for source in self.sources]
        if (
            digests != sorted(set(digests))
            or sum(s.byte_size for s in self.sources) > MAX_REPLAY_BYTES
        ):
            raise ValueError("execution source references must be unique, sorted and bounded")
        referenced = set()
        for record in (*self.prices, *self.terms, *self.actions):
            if record.ingested_at > self.captured_at:
                raise ValueError("execution capture precedes record ingestion")
            referenced.add(record.source.raw_sha256)
        if referenced != set(digests):
            raise ValueError("execution source set differs from record lineage")
        return self


class MarketTape(ImmutableRecord):
    """Separate opt-in model; v1 retains its original no-action semantics."""

    schema_version: Literal["loop.execution-tape/v2"] = Field(alias="schema")
    quality: Literal["synthetic", "public_development"]
    currency: Literal["USD"]
    price_basis: Literal["raw"]
    coverage: Literal["explicit_development_declaration"]
    capture: CachedObject

    @model_validator(mode="after")
    def bounded_capture(self) -> Self:
        if self.capture.byte_size > MAX_REPLAY_BYTES:
            raise ValueError("execution capture exceeds the byte budget")
        return self
