"""Native source tables with immutable acquisition evidence and explicit limits."""

from typing import Literal, Self

from pydantic import Field, model_validator

from loop_research.data.fetch_records import CachedObject
from loop_research.data.licensed_config import LicensedProvider
from loop_research.data.models import Identifier, ImmutableRecord, Instant


class LicensedCapture(ImmutableRecord):
    """Successful source page with a credential-free, deterministically checked request."""

    dataset: Identifier
    page: int = Field(ge=0, le=31)
    request: str = Field(min_length=1, max_length=8192)
    observed_at: Instant
    # None denotes an exactly zero-byte JSONL response, whose content identity
    # is SHA-256(empty). The generic artifact store intentionally rejects empty
    # objects. No missing nonempty object may use this representation.
    content: CachedObject | None
    request_id: str | None = Field(default=None, max_length=160)


class SourceTable(ImmutableRecord):
    """Validated native columns, exact textual scalars and stable source keys.

    This is a staging table, not an authorized PIT snapshot or an execution-price
    bar. Source clocks and adjustment flags are retained as named native columns.
    """

    dataset: Identifier
    columns: tuple[Identifier, ...] = Field(min_length=1, max_length=200)
    primary_key: tuple[Identifier, ...] = Field(min_length=1, max_length=8)
    rows: tuple[tuple[str | None, ...], ...] = Field(max_length=10_000)
    semantics: tuple[Identifier, ...] = Field(min_length=1, max_length=12)

    @model_validator(mode="after")
    def valid_table(self) -> Self:
        if len(set(self.columns)) != len(self.columns) or not set(self.primary_key) <= set(
            self.columns
        ):
            raise ValueError("invalid table columns or keys")
        indices = tuple(self.columns.index(key) for key in self.primary_key)
        keys: set[tuple[str | None, ...]] = set()
        for row in self.rows:
            if len(row) != len(self.columns) or any(
                value is not None and (len(value) > 4096 or "\x00" in value) for value in row
            ):
                raise ValueError("invalid bounded table row")
            key = tuple(row[index] for index in indices)
            if None in key or "" in key or key in keys:
                raise ValueError("missing or duplicate native key")
            keys.add(key)
        return self


class LicensedBatch(ImmutableRecord):
    """Native staging output awaiting calendar, PIT, master and coverage validation."""

    schema_version: Literal["loop.licensed-batch/v1"] = Field(
        default="loop.licensed-batch/v1", alias="schema"
    )
    provider: LicensedProvider
    tables: tuple[SourceTable, ...] = Field(min_length=1, max_length=4)
    quality: Literal["licensed_source_unverified"] = "licensed_source_unverified"
    historical_pit: Literal["not_verified"] = "not_verified"
    universe_coverage: Literal["selected_identifiers_only"] = "selected_identifiers_only"
    allocation_filter: Literal["not_applicable", "existing_isins_only"]


class LicensedReceipt(ImmutableRecord):
    """Local completion point, never a vendor signature or current access grant."""

    schema_version: Literal["loop.licensed-receipt/v1"] = Field(
        default="loop.licensed-receipt/v1", alias="schema"
    )
    config: CachedObject
    license: CachedObject
    normalized: CachedObject
    captures: tuple[LicensedCapture, ...] = Field(min_length=1, max_length=64)
    started_at: Instant
    completed_at: Instant
    attempts: int = Field(ge=1, le=64)
    bytes_received: int = Field(ge=0, le=128 * 1024 * 1024)

    @model_validator(mode="after")
    def ordered_capture(self) -> Self:
        previous = self.started_at
        if self.completed_at < previous or self.attempts < len(self.captures):
            raise ValueError("invalid receipt clocks or attempts")
        for capture in self.captures:
            if not previous <= capture.observed_at <= self.completed_at:
                raise ValueError("capture clock regression")
            previous = capture.observed_at
        if (
            sum(
                capture.content.byte_size
                for capture in self.captures
                if capture.content is not None
            )
            > self.bytes_received
        ):
            raise ValueError("invalid byte accounting")
        return self


class LicensedReport(ImmutableRecord):
    """Non-secret command result; empty/filtered data is not a coverage certificate."""

    receipt: CachedObject
    provider: LicensedProvider
    result: Literal["records", "empty"]
    row_counts: dict[str, int]
    quality: Literal["licensed_source_unverified"] = "licensed_source_unverified"
    historical_pit: Literal["not_verified"] = "not_verified"
    allocation_filter: Literal["not_applicable", "existing_isins_only"]
