"""Closed, bounded execution inputs; these records carry no research authority."""

from datetime import UTC, date, datetime, timedelta
from fractions import Fraction
from typing import Annotated, Literal, Self
from zoneinfo import ZoneInfo

from pydantic import BaseModel, ConfigDict, Field, model_validator

HASH = r"^sha256:[0-9a-f]{64}$"
Security = Annotated[str, Field(pattern=r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$")]
Money = Annotated[str, Field(pattern=r"^(0|[1-9][0-9]{0,15})(\.[0-9]{1,8})?$")]
Clock = Annotated[int, Field(gt=0, lt=2**53, strict=True)]


class Record(BaseModel):
    model_config = ConfigDict(
        extra="forbid", frozen=True, populate_by_name=True, allow_inf_nan=False
    )


class Reference(Record):
    sha256: str = Field(pattern=HASH)
    byte_size: int = Field(ge=1, le=64 * 1024 * 1024, strict=True)


class Policy(Record):
    initial_cash_usd: Money
    holdings: int = Field(ge=1, le=1000, strict=True)
    lot_size: int = Field(ge=1, le=10000, strict=True)
    commission_per_share_usd: Money
    minimum_commission_usd: Money
    half_spread_bps: int = Field(ge=0, le=1000, strict=True)
    long_weight_bps: int = Field(default=10000, ge=0, le=20000, strict=True)
    short_weight_bps: int = Field(default=0, ge=0, le=20000, strict=True)
    initial_margin_bps: int = Field(default=10000, ge=5000, le=10000, strict=True)
    maintenance_margin_bps: int = Field(default=10000, ge=2500, le=10000, strict=True)
    participation_bps: int = Field(default=10000, ge=1, le=10000, strict=True)
    impact_bps: int = Field(default=0, ge=0, le=5000, strict=True)
    short_collateral_bps: int = Field(default=10000, ge=10000, le=30000, strict=True)
    cash_debit_bps: int = Field(default=0, ge=0, le=100000, strict=True)
    cash_credit_bps: int = Field(default=0, ge=0, le=10000, strict=True)
    day_count: Literal[360, 365] = 365

    @model_validator(mode="after")
    def valid_budget(self) -> Self:
        gross = self.long_weight_bps + self.short_weight_bps
        if not 0 < Fraction(self.initial_cash_usd) <= 10**15:
            raise ValueError("initial cash bound")
        if not 0 < gross <= 20000 or gross * self.initial_margin_bps > 10**8:
            raise ValueError("gross target exceeds margin")
        if self.maintenance_margin_bps > self.initial_margin_bps:
            raise ValueError("maintenance exceeds initial margin")
        return self


class Terms(Record):
    tradable: bool = Field(strict=True)
    short_allowed: bool = Field(strict=True)
    borrow_limit: int = Field(ge=0, le=10**12, strict=True)
    borrow_rate_bps: int = Field(ge=0, le=100000, strict=True)
    recalled: bool = Field(strict=True)
    sec_fee_usd_per_million: Money
    taf_fee_usd_per_share: Money
    taf_fee_cap_usd: Money

    @model_validator(mode="after")
    def fee_bounds(self) -> Self:
        rate, cap = Fraction(self.taf_fee_usd_per_share), Fraction(self.taf_fee_cap_usd)
        if Fraction(self.sec_fee_usd_per_million) > 1000 or rate > 1 or cap > 10**6:
            raise ValueError("dated fee bounds")
        if (rate and not cap) or (cap * 100).denominator != 1:
            raise ValueError("TAF cap requires whole cents")
        return self


class Row(Record):
    security_id: Security
    eligible: bool = Field(strict=True)
    factor: float | None
    open_ms: Clock | None
    opening: Money | None
    close_ms: Clock | None
    closing: Money | None
    auction_volume: int | None = Field(ge=0, le=10**12, strict=True)
    opening_terms: Terms | None
    closing_terms: Terms | None

    @model_validator(mode="after")
    def paired_quotes(self) -> Self:
        for price, clock in ((self.opening, self.open_ms), (self.closing, self.close_ms)):
            if (price is None) != (clock is None):
                raise ValueError("price and clock must be paired")
            if price is not None and not Fraction(1, 10**6) <= Fraction(price) <= 10**9:
                raise ValueError("raw quote bounds")
        return self


class Event(Record):
    event_id: Security
    security_id: Security
    at_ms: Clock
    pay_ms: Clock | None
    kind: Literal["split", "dividend", "delisting"]
    numerator: int | None = Field(default=None, ge=1, le=10**6, strict=True)
    denominator: int | None = Field(default=None, ge=1, le=10**6, strict=True)
    fraction_price_usd: Money | None = None
    amount_per_share_usd: Money | None = None

    @model_validator(mode="after")
    def valid_action(self) -> Self:
        if self.pay_ms is not None and self.pay_ms < self.at_ms:
            raise ValueError("payment precedes entitlement")
        for value in (self.fraction_price_usd, self.amount_per_share_usd):
            if value is not None and Fraction(value) > 10**9:
                raise ValueError("action amount bound")
        if self.fraction_price_usd is not None and not Fraction(self.fraction_price_usd):
            raise ValueError("fractional settlement price must be positive")
        if (
            self.kind == "dividend"
            and self.amount_per_share_usd is not None
            and not Fraction(self.amount_per_share_usd)
        ):
            raise ValueError("ordinary dividend amount must be positive")
        if self.kind == "split":
            if (
                self.numerator is None
                or self.denominator is None
                or self.numerator == self.denominator
            ):
                raise ValueError("split ratio required")
            if self.amount_per_share_usd is not None or (
                (self.fraction_price_usd is None) != (self.pay_ms is None)
            ):
                raise ValueError("split fields differ")
        elif (
            self.pay_ms is None
            or self.amount_per_share_usd is None
            or any(
                value is not None
                for value in (self.numerator, self.denominator, self.fraction_price_usd)
            )
        ):
            raise ValueError("cash action fields differ")
        return self


class Session(Record):
    day: date
    decision_ms: Clock
    scheduled_open_ms: Clock
    rows: tuple[Row, ...] = Field(min_length=1, max_length=4096)
    actions: tuple[Event, ...] = Field(max_length=4096)


class Tape(Record):
    schema_version: Literal["loop.zipline-observations/v1"] = Field(alias="schema")
    sessions: tuple[Session, ...] = Field(min_length=2, max_length=8192)

    @model_validator(mode="after")
    def causal_axes(self) -> Self:
        ids = tuple(row.security_id for row in self.sessions[0].rows)
        days = tuple(session.day for session in self.sessions)
        if ids != tuple(sorted(set(ids))) or days != tuple(sorted(set(days))):
            raise ValueError("canonical session/security axes required")
        if len(ids) * len(days) > 100_000 or sum(len(s.actions) for s in self.sessions) > 10_000:
            raise ValueError("execution cell bound")
        if any(not 2007 <= day.year <= 2020 for day in days):
            raise ValueError("only development sessions are supported")
        import exchange_calendars as calendars  # type: ignore[import-untyped]

        calendar = calendars.get_calendar(
            "XNYS", start=days[0] - timedelta(days=7), end=days[-1] + timedelta(days=7)
        )
        expected = tuple(value.date() for value in calendar.sessions_in_range(days[0], days[-1]))
        if days != expected:
            raise ValueError("incomplete XNYS session sequence")
        previous = 0
        identities: set[str] = set()
        for session in self.sessions:
            if not 2007 <= session.day.year <= 2020 or session.day.weekday() > 4:
                raise ValueError("only development sessions are supported")
            if tuple(row.security_id for row in session.rows) != ids:
                raise ValueError("incomplete execution grid")
            if session.decision_ms <= previous:
                raise ValueError("decision clock regression")
            decision = datetime.fromtimestamp(session.decision_ms / 1000, UTC)
            scheduled = int(calendar.session_open(session.day.isoformat()).value // 1_000_000)
            market = any(row.auction_volume is not None for row in session.rows)
            if session.scheduled_open_ms != (scheduled if market else session.decision_ms):
                raise ValueError("scheduled opening differs from the exchange calendar")
            if decision < calendar.session_close(session.day.isoformat()).to_pydatetime() or (
                decision.astimezone(ZoneInfo("America/New_York")).date() != session.day
            ):
                raise ValueError("decision precedes exchange close or uses another date")
            for row in session.rows:
                if row.open_ms is not None and not previous < row.open_ms < session.decision_ms:
                    raise ValueError("opening outside execution interval")
                if row.close_ms is not None and row.close_ms > session.decision_ms:
                    raise ValueError("future closing quote")
            seen = set()
            for event in session.actions:
                if (
                    event.security_id not in ids
                    or event.security_id in seen
                    or event.event_id in identities
                ):
                    raise ValueError("duplicate or unknown corporate action")
                if not previous < event.at_ms <= session.decision_ms:
                    raise ValueError("action outside session")
                if datetime.fromtimestamp(event.at_ms / 1000, UTC).date() != session.day:
                    raise ValueError("action date differs")
                seen.add(event.security_id)
                identities.add(event.event_id)
            previous = session.decision_ms
        return self


class Artifacts(Record):
    targets: Reference
    orders: Reference
    fills: Reference
    positions: Reference
    nav: Reference
    returns: Reference
    costs: Reference


class Inputs(Record):
    schema_version: Literal["loop.zipline-input/v1"] = Field(alias="schema")
    profile: Literal["zipline-accounting.1"]
    primary_backtest: Reference
    observations: Reference
    engine: Literal["long-only-next-open.1", "pit-actions-long-short.1"]
    source_code_sha256: str = Field(pattern=HASH)
    environment_sha256: str = Field(pattern=HASH)
    direction: Literal["higher_is_better", "lower_is_better"]
    policy: Policy
    primary_artifacts: Artifacts
    production_eligible: Literal[False]


class Receipt(Record):
    schema_version: Literal["loop.zipline-receipt/v1"] = Field(
        default="loop.zipline-receipt/v1", alias="schema"
    )
    inputs: Reference
    build: Reference
    ledgers: Artifacts
    bridge: Reference
    differences: Reference
    summary: Reference
    disposition: Literal["accepted", "rejected", "unavailable"]
    production_eligible: Literal[False] = False
