"""Source snapshot contracts; none of these records grant research data access."""

from datetime import date
from typing import Annotated, Literal, Self

from pydantic import Field, model_validator

from loop_research.data.fetch_config import AlpacaRequest, SecRequest
from loop_research.data.fetch_records import CachedObject
from loop_research.data.licensed_config import DatabentoRequest, SharadarRequest, WrdsRequest
from loop_research.data.models import Identifier, ImmutableRecord

FIRST_OBSERVATION = date(2005, 1, 1)
LAST_OBSERVATION = date(2026, 8, 31)
MAX_ROWS = 100_000
MAX_SOURCE_BYTES = 512 * 1024 * 1024
type SamplePeriod = Literal["warmup", "in_sample", "development", "confirmation", "recent_holdout"]
PERIODS: tuple[tuple[SamplePeriod, date, date], ...] = (
    ("warmup", date(2005, 1, 1), date(2006, 12, 31)),
    ("in_sample", date(2007, 1, 1), date(2016, 12, 31)),
    ("development", date(2017, 1, 1), date(2020, 12, 31)),
    ("confirmation", date(2021, 1, 1), date(2024, 12, 31)),
    ("recent_holdout", date(2025, 1, 1), date(2026, 8, 31)),
)
Digest = Annotated[str, Field(pattern=r"^sha256:[0-9a-f]{64}$")]
SourceRequest = Annotated[
    SecRequest | AlpacaRequest | SharadarRequest | WrdsRequest | DatabentoRequest,
    Field(discriminator="provider"),
]


class SnapshotRequest(ImmutableRecord):
    """Explicit source receipt set and inclusive business-date selection."""

    schema_version: Literal["loop.source-snapshot-request/v1"] = Field(
        default="loop.source-snapshot-request/v1", alias="schema"
    )
    receipts: tuple[Digest, ...] = Field(min_length=1, max_length=32)
    start: date
    through: date

    @model_validator(mode="after")
    def valid_request(self) -> Self:
        if not FIRST_OBSERVATION <= self.start <= self.through <= LAST_OBSERVATION:
            raise ValueError("source snapshots require an explicit 2005-2026-08-31 interval")
        if tuple(sorted(set(self.receipts))) != self.receipts:
            raise ValueError("source receipts must be sorted and unique")
        return self


class SnapshotPart(ImmutableRecord):
    """An immutable source table partition, not an executable factor panel."""

    source_receipt: CachedObject
    normalized_source: CachedObject
    provider: Identifier
    dataset: Identifier
    period: SamplePeriod
    start: date
    through: date
    row_count: int = Field(ge=0, le=10_000)
    parquet: CachedObject
    table_schema: CachedObject
    quality_report: CachedObject


class SnapshotManifest(ImmutableRecord):
    """Administrative source artifacts stay separate from authorized dataset schemas."""

    schema_version: Literal["loop.source-snapshot/v1"] = Field(
        default="loop.source-snapshot/v1", alias="schema"
    )
    request: SnapshotRequest
    access_scope: Literal["private_source_only"] = "private_source_only"
    historical_pit: Literal["not_certified"] = "not_certified"
    universe_coverage: Literal["selected_identifiers_only"] = "selected_identifiers_only"
    production_eligible: Literal[False] = False
    calendar: CachedObject
    writer: Identifier
    parts: tuple[SnapshotPart, ...] = Field(max_length=512)
    total_rows: int = Field(ge=0, le=MAX_ROWS)
    excluded_rows: int = Field(ge=0, le=MAX_ROWS)


class SnapshotReport(ImmutableRecord):
    """Small CLI output with immutable identity and honest readiness status."""

    snapshot: CachedObject
    start: date
    through: date
    parts: int
    row_count: int
    excluded_rows: int
    historical_pit: Literal["not_certified"] = "not_certified"
    production_eligible: Literal[False] = False


class SyncPlan(ImmutableRecord):
    """A bounded sequential batch; each request keeps its original supplier limits."""

    schema_version: Literal["loop.data-sync-plan/v1"] = Field(
        default="loop.data-sync-plan/v1", alias="schema"
    )
    start: date
    through: date
    requests: tuple[SourceRequest, ...] = Field(min_length=1, max_length=32)
    max_requests: int = Field(ge=1, le=512)
    max_source_bytes: int = Field(ge=1024, le=MAX_SOURCE_BYTES)
    max_records: int = Field(ge=1, le=MAX_ROWS)
    timeout_seconds: int = Field(ge=1, le=1800)

    @model_validator(mode="after")
    def valid_plan(self) -> Self:
        if not FIRST_OBSERVATION <= self.start <= self.through <= LAST_OBSERVATION:
            raise ValueError("sync requires a bounded historical observation range")
        if any(not self.start <= item.start <= item.end <= self.through for item in self.requests):
            raise ValueError("source request exceeds the synchronization range")
        if len({item.model_dump_json(by_alias=True) for item in self.requests}) != len(
            self.requests
        ):
            raise ValueError("duplicate source request")
        for actual, maximum in (
            (sum(item.budget.requests for item in self.requests), self.max_requests),
            (sum(item.budget.total_bytes for item in self.requests), self.max_source_bytes),
            (sum(item.budget.records for item in self.requests), self.max_records),
            (sum(item.budget.timeout_seconds for item in self.requests), self.timeout_seconds),
        ):
            if actual > maximum:
                raise ValueError("source request reservations exceed the whole-plan budget")
        return self


class SyncProgress(ImmutableRecord):
    """Immutable completed prefix; retry verifies it, never infers a moving latest."""

    schema_version: Literal["loop.data-sync-progress/v1"] = Field(
        default="loop.data-sync-progress/v1", alias="schema"
    )
    plan: CachedObject
    receipts: tuple[CachedObject, ...] = Field(max_length=32)
