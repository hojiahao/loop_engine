"""Vendor-neutral, bounded records; a declaration is not data-quality attestation."""

from __future__ import annotations

import re
from collections.abc import Hashable, Iterable
from datetime import UTC, date, datetime
from decimal import Decimal
from typing import Annotated, Literal, Self
from zoneinfo import ZoneInfo

from pydantic import BaseModel, BeforeValidator, ConfigDict, Field, model_validator

MAX_RECORDS = 10_000
Identifier = Annotated[str, Field(min_length=1, max_length=160, pattern=r"^[A-Za-z0-9_.:/-]+$")]
Ticker = Annotated[str, Field(min_length=1, max_length=32, pattern=r"^[A-Z0-9][A-Z0-9./^-]*$")]
Venue = Annotated[str, Field(pattern=r"^[A-Z0-9]{4}$")]
ExactDecimal = Annotated[
    str, Field(max_length=64, pattern=r"^-?(0|[1-9][0-9]{0,24})(\.[0-9]{1,18})?$")
]
_INSTANT = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?(?:Z|[+-]\d{2}:\d{2})\Z")


def parse_instant(value: object) -> datetime:
    """Accept timezone-aware instants, never naive dates or epoch coercions."""
    if isinstance(value, str) and _INSTANT.fullmatch(value):
        value = datetime.fromisoformat(value)
    if not isinstance(value, datetime) or value.utcoffset() is None:
        raise ValueError("timestamps require an explicit RFC3339 timezone")
    if not 1900 <= value.year <= 2100:
        raise ValueError("timestamp exceeds the supported 1900-2100 range")
    result = value.astimezone(UTC)
    if not 1900 <= result.year <= 2100:
        raise ValueError("timestamp exceeds the supported 1900-2100 range")
    return result


Instant = Annotated[datetime, BeforeValidator(parse_instant)]


class ImmutableRecord(BaseModel):
    """Strict records reject unknown fields, coercion and unchecked nested copies."""

    model_config = ConfigDict(
        extra="forbid",
        frozen=True,
        strict=True,
        revalidate_instances="always",
        validate_by_name=True,
        hide_input_in_errors=True,
    )


class SourceEvidence(ImmutableRecord):
    """Trace one observation to captured vendor bytes and an availability rule."""

    source: Identifier
    dataset: Identifier
    revision: Identifier
    record_id: Identifier
    raw_sha256: str = Field(pattern=r"^sha256:[0-9a-f]{64}$")
    availability: Literal["publisher_timestamp", "filing_acceptance", "first_observed", "synthetic"]


class TemporalRecord(ImmutableRecord):
    """Business effective time, public knowledge time and local ingestion time.

    A security event may be announced before becoming effective. Observation
    subclasses impose their stricter end-of-period visibility rules separately.
    These declared times are checked for consistency, not externally attested.
    """

    effective_at: Instant
    known_at: Instant
    ingested_at: Instant
    source: SourceEvidence

    @model_validator(mode="after")
    def valid_clocks(self) -> Self:
        if self.known_at > self.ingested_at:
            raise ValueError("public knowledge cannot follow ingestion")
        if self.source.availability == "first_observed" and self.known_at != self.ingested_at:
            raise ValueError("first-observed records cannot backdate public knowledge")
        return self


class SecurityVersion(TemporalRecord):
    """One security's listing state; issuer IDs never replace security IDs."""

    security_id: Identifier
    issuer_id: Identifier
    ticker: Ticker
    venue: Venue
    kind: Literal["common_stock", "adr", "etf", "fund", "preferred", "spac", "other", "unknown"]
    listed: bool
    effective_until: Instant | None = None

    @model_validator(mode="after")
    def valid_interval(self) -> Self:
        if self.effective_until is not None and self.effective_until <= self.effective_at:
            raise ValueError("security interval must have an exclusive later end")
        return self


class RawBar(TemporalRecord):
    """Unadjusted OHLCV for an explicit interval ending at effective_at.

    Corporate actions, total returns and execution prices are separate products.
    This schema checks interval/date consistency, not exchange-session completeness
    or feed coverage. Vendor adapters must preserve the interval/feed meaning.
    """

    security_id: Identifier
    session: date
    interval_start: Instant
    currency: str = Field(pattern=r"^[A-Z]{3}$")
    price_basis: Literal["raw"]
    open: ExactDecimal
    high: ExactDecimal
    low: ExactDecimal
    close: ExactDecimal
    volume: int = Field(ge=0, le=2**63 - 1)

    @model_validator(mode="after")
    def valid_bar(self) -> Self:
        timezone = ZoneInfo("America/New_York")
        if not self.interval_start < self.effective_at <= self.known_at:
            raise ValueError("bar must close before it becomes publicly available")
        if any(
            instant.astimezone(timezone).date() != self.session
            for instant in (self.interval_start, self.effective_at)
        ):
            raise ValueError("bar interval must belong to its New York session date")
        opening, high, low, close = map(Decimal, (self.open, self.high, self.low, self.close))
        if not 0 < low <= min(opening, close) <= max(opening, close) <= high:
            raise ValueError("raw OHLC prices must be positive and within low/high")
        return self


class Fundamental(TemporalRecord):
    """An entity-wide filing fact; no implicit share-class or fiscal-frame join.

    effective_at denotes the completed accounting period, not its publication.
    Instant facts have no period_start. Values remain exact decimal strings;
    standard concepts and units must match before comparing revisions.
    """

    issuer_id: Identifier
    concept: Identifier
    unit: Identifier
    period_start: date | None
    period_end: date
    filing_id: Identifier
    value: ExactDecimal

    @model_validator(mode="after")
    def valid_period(self) -> Self:
        if self.period_start is not None and self.period_start > self.period_end:
            raise ValueError("fundamental period endpoints must be ordered")
        if self.effective_at.date() != self.period_end or self.effective_at > self.known_at:
            raise ValueError("fundamental effective time must end its period before publication")
        return self


class PitInput(ImmutableRecord):
    """Small immutable development capture, not a production snapshot manifest."""

    schema_version: Literal["loop.pit-input/v1"] = Field(alias="schema")
    quality: Literal["synthetic", "public_development"]
    captured_at: Instant
    securities: tuple[SecurityVersion, ...] = Field(min_length=1, max_length=MAX_RECORDS)
    bars: tuple[RawBar, ...] = Field(max_length=MAX_RECORDS)
    fundamentals: tuple[Fundamental, ...] = Field(max_length=MAX_RECORDS)

    @model_validator(mode="after")
    def valid_capture(self) -> Self:
        records: tuple[TemporalRecord, ...] = (*self.securities, *self.bars, *self.fundamentals)
        if len(records) > MAX_RECORDS:
            raise ValueError("capture exceeds the total record budget")
        for record in records:
            if record.ingested_at > self.captured_at:
                raise ValueError("record was ingested after the capture")
            if (record.source.availability == "synthetic") != (self.quality == "synthetic"):
                raise ValueError("synthetic sources and capture quality must agree")
        security_ids = {record.security_id for record in self.securities}
        issuer_ids = {record.issuer_id for record in self.securities}
        if any(record.security_id not in security_ids for record in self.bars):
            raise ValueError("bar references an unresolved security")
        if any(record.issuer_id not in issuer_ids for record in self.fundamentals):
            raise ValueError("fundamental references an unresolved issuer")
        _unique_versions(
            (record.security_id, record.effective_at, record.known_at) for record in self.securities
        )
        _unique_versions(
            (record.security_id, record.session, record.known_at) for record in self.bars
        )
        _unique_versions((*fact_key(record), record.known_at) for record in self.fundamentals)
        _consistent_sources(
            ((record.security_id, record.effective_at), record) for record in self.securities
        )
        _consistent_sources(((record.security_id, record.session), record) for record in self.bars)
        _consistent_sources((fact_key(record), record) for record in self.fundamentals)
        currencies: dict[tuple[str, date], str] = {}
        for record in self.bars:
            identity = record.security_id, record.session
            if currencies.setdefault(identity, record.currency) != record.currency:
                raise ValueError("bar revisions cannot silently change currency")
        return self


def fact_key(record: Fundamental) -> tuple[str, str, str, date | None, date]:
    """Keep concept/unit and instant/duration periods distinct across revisions."""
    return record.issuer_id, record.concept, record.unit, record.period_start, record.period_end


def _unique_versions(keys: Iterable[tuple[Hashable, ...]]) -> None:
    seen: set[tuple[Hashable, ...]] = set()
    for key in keys:
        if key in seen:
            raise ValueError("duplicate or conflicting observation version")
        seen.add(key)


def _consistent_sources(
    records: Iterable[tuple[tuple[Hashable, ...], TemporalRecord]],
) -> None:
    sources: dict[tuple[Hashable, ...], tuple[str, str]] = {}
    for identity, record in records:
        source = record.source.source, record.source.dataset
        if sources.setdefault(identity, source) != source:
            raise ValueError("observation revisions require one explicit source dataset")


class PitQuery(ImmutableRecord):
    """Explicit decision clocks and optional security/ticker selector.

    No selector means the visible listed common-stock universe on XNYS/XNAS/XASE.
    An explicit ID can inspect an excluded/delisted security without making it
    universe-eligible. Ticker selection always requires an explicit venue.
    """

    market_at: Instant
    known_at: Instant
    ingested_at: Instant
    security_id: Identifier | None = None
    ticker: Ticker | None = None
    venue: Venue | None = None

    @model_validator(mode="after")
    def valid_query(self) -> Self:
        if not self.market_at <= self.known_at <= self.ingested_at:
            raise ValueError("query clocks require market <= known <= ingested")
        if (self.ticker is None) != (self.venue is None):
            raise ValueError("ticker selection requires ticker and venue together")
        if self.security_id is not None and self.ticker is not None:
            raise ValueError("select either security ID or venue/ticker")
        return self
