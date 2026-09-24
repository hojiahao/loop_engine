"""Development ingestion artifacts, distinct from authorized PIT snapshots."""

from datetime import date
from typing import Literal, Self

from pydantic import Field, model_validator

from loop_research.data.models import (
    Fundamental,
    Identifier,
    ImmutableRecord,
    Instant,
    RawBar,
    SourceEvidence,
    Ticker,
)


class CachedObject(ImmutableRecord):
    """Content identity; filenames are derived locally, never accepted as URIs."""

    sha256: str = Field(pattern=r"^sha256:[0-9a-f]{64}$")
    byte_size: int = Field(gt=0, le=128 * 1024 * 1024)


class CapturedResponse(ImmutableRecord):
    """A successful response; URL contains query parameters, never credentials."""

    kind: Literal["sec_facts", "sec_submissions", "alpaca_asset", "alpaca_bars", "alpaca_sip_probe"]
    key: Identifier
    url: str = Field(min_length=1, max_length=8192)
    observed_at: Instant
    request_id: str | None = Field(default=None, max_length=160)
    content: CachedObject


class ObservedAsset(ImmutableRecord):
    """Current vendor identity, without invented issuer or historical listing."""

    security_id: Identifier
    symbol: Ticker
    exchange: Identifier
    status: Literal["active", "inactive"]
    asset_class: Literal["us_equity"]
    observed_at: Instant
    source: SourceEvidence


class DevelopmentBatch(ImmutableRecord):
    """First-observed source records requiring a verified master/calendar join."""

    schema_version: Literal["loop.development-batch/v1"] = Field(
        default="loop.development-batch/v1", alias="schema"
    )
    provider: Literal["sec", "alpaca"]
    start: date
    end: date
    quality: Literal["public_development"] = "public_development"
    historical_pit: Literal["not_verified"] = "not_verified"
    universe_coverage: Literal["not_verified"] = "not_verified"
    calendar_validation: Literal["not_performed"] = "not_performed"
    assets: tuple[ObservedAsset, ...] = Field(default=(), max_length=8)
    bars: tuple[RawBar, ...] = Field(default=(), max_length=10_000)
    fundamentals: tuple[Fundamental, ...] = Field(default=(), max_length=10_000)
    missing: tuple[Identifier, ...] = Field(default=(), max_length=8)
    symbol_asof: date | None = None
    feed: Literal["iex", "sip"] | None = None

    @model_validator(mode="after")
    def consistent_records(self) -> Self:
        if self.start > self.end or len(self.bars) + len(self.fundamentals) > 10_000:
            raise ValueError("invalid development batch bounds")
        if self.provider == "sec":
            if self.assets or self.bars or self.feed is not None or self.symbol_asof is not None:
                raise ValueError("SEC batch cannot assert market identity")
        elif self.fundamentals or self.feed is None or self.symbol_asof is None or not self.assets:
            raise ValueError("Alpaca batch requires explicit feed and current asset identity")
        assets = {asset.security_id for asset in self.assets}
        if len(assets) != len(self.assets) or len({a.symbol for a in self.assets}) != len(
            self.assets
        ):
            raise ValueError("ambiguous current asset identity")
        if any(bar.security_id not in assets for bar in self.bars):
            raise ValueError("unresolved bar identity")
        if any(not self.start <= bar.session <= self.end for bar in self.bars):
            raise ValueError("bar outside configured period")
        if any(not self.start <= fact.period_end <= self.end for fact in self.fundamentals):
            raise ValueError("fundamental outside configured period")
        return self


class FetchReceipt(ImmutableRecord):
    """Completed local acquisition; not a signed supplier entitlement certificate."""

    schema_version: Literal["loop.development-receipt/v1"] = Field(
        default="loop.development-receipt/v1", alias="schema"
    )
    config: CachedObject
    normalized: CachedObject
    responses: tuple[CapturedResponse, ...] = Field(min_length=1, max_length=42)
    started_at: Instant
    completed_at: Instant
    attempts: int = Field(ge=1, le=64)
    bytes_received: int = Field(ge=1, le=128 * 1024 * 1024)
    recent_sip: Literal["not_requested", "response_permitted", "forbidden"] = "not_requested"

    @model_validator(mode="after")
    def valid_completion(self) -> Self:
        if self.started_at > self.completed_at or self.attempts < len(self.responses):
            raise ValueError("invalid completion clocks or attempts")
        previous = self.started_at
        for response in self.responses:
            if not previous <= response.observed_at <= self.completed_at:
                raise ValueError("response observation clock regression")
            previous = response.observed_at
        if sum(response.content.byte_size for response in self.responses) > self.bytes_received:
            raise ValueError("response bytes exceed receipt accounting")
        return self


class FetchReport(ImmutableRecord):
    """Safe CLI summary; an empty response remains distinguishable from denial."""

    receipt: CachedObject
    provider: Literal["sec", "alpaca"]
    result: Literal["records", "empty"]
    bar_count: int
    fundamental_count: int
    missing: tuple[Identifier, ...]
    recent_sip: Literal["not_requested", "response_permitted", "forbidden"]
    historical_pit: Literal["not_verified"] = "not_verified"
    universe_coverage: Literal["not_verified"] = "not_verified"
