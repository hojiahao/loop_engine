"""Explicit development risk exposures; no vendor-quality claim or implicit estimation."""

from datetime import date
from decimal import Decimal
from typing import Literal, Self

from pydantic import Field, model_validator

from loop_research.data.models import (
    MAX_RECORDS,
    ExactDecimal,
    Identifier,
    ImmutableRecord,
    Instant,
    TemporalRecord,
)


class ExposureVersion(TemporalRecord):
    """One version of a declared security/session exposure vector."""

    security_id: Identifier
    session: date
    currency: Literal["USD"]
    industry: str | None = Field(default=None, pattern=r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$")
    market_cap: ExactDecimal | None = None
    beta: ExactDecimal | None = None

    @model_validator(mode="after")
    def positive_size(self) -> Self:
        if self.market_cap is not None and Decimal(self.market_cap) <= 0:
            raise ValueError("market capitalization must be positive USD")
        return self


class ExposureCapture(ImmutableRecord):
    """Bounded source-backed revisions; supplied exposures remain development inputs."""

    schema_version: Literal["loop.exposure-input/v1"] = Field(alias="schema")
    quality: Literal["synthetic", "public_development"]
    captured_at: Instant
    records: tuple[ExposureVersion, ...] = Field(max_length=MAX_RECORDS)

    @model_validator(mode="after")
    def valid_records(self) -> Self:
        versions = set()
        sources: dict[tuple[str, date], tuple[str, str]] = {}
        for record in self.records:
            key = record.security_id, record.session
            version = *key, record.known_at
            source = record.source.source, record.source.dataset
            if (
                record.ingested_at > self.captured_at
                or (record.source.availability == "synthetic") != (self.quality == "synthetic")
                or version in versions
                or sources.setdefault(key, source) != source
            ):
                raise ValueError("inconsistent exposure quality, revision, source or capture time")
            versions.add(version)
        return self
