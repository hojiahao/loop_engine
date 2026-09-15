"""Administrative causal panel requests and private construction receipts."""

from datetime import date
from typing import Annotated, Literal, Self

from pydantic import Field, model_validator

from loop_research.data.fetch_records import CachedObject
from loop_research.data.models import ImmutableRecord

PanelSecurity = Annotated[str, Field(pattern=r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$")]
RawField = Literal["market.open", "market.high", "market.low", "market.close", "market.volume"]


class PanelRequest(ImmutableRecord):
    """Freeze a development selection; references never grant data authority."""

    schema_version: Literal["loop.panel-build-request/v1"] = Field(
        default="loop.panel-build-request/v1", alias="schema"
    )
    capture: CachedObject
    source_snapshot: CachedObject | None = None
    securities: tuple[PanelSecurity, ...] = Field(min_length=1, max_length=10000)
    fields: tuple[RawField, ...] = Field(min_length=1, max_length=5)
    warmup_start: date
    sample_start: date
    sample_end: date
    close_delay_ms: int = Field(default=300_000, ge=0, le=7_200_000)

    @model_validator(mode="after")
    def valid_selection(self) -> Self:
        if not (
            date(2005, 1, 1) <= self.warmup_start <= self.sample_start <= self.sample_end
            and (
                date(2007, 1, 1) <= self.sample_start <= self.sample_end <= date(2016, 12, 31)
                or date(2017, 1, 1) <= self.sample_start <= self.sample_end <= date(2020, 12, 31)
            )
        ):
            raise ValueError("panel construction requires one IS/development sample")
        if self.capture.byte_size > 8 * 1024 * 1024 or (
            self.source_snapshot is not None and self.source_snapshot.byte_size > 1024 * 1024
        ):
            raise ValueError("panel source metadata byte budget")
        if tuple(sorted(set(self.securities))) != self.securities:
            raise ValueError("panel security selection must be sorted and unique")
        if tuple(sorted(set(self.fields))) != self.fields:
            raise ValueError("panel fields must be sorted and unique")
        return self


class PanelReceipt(ImmutableRecord):
    """Private input/output lineage; no worker receives the source capture."""

    schema_version: Literal["loop.panel-build-receipt/v1"] = Field(
        default="loop.panel-build-receipt/v1", alias="schema"
    )
    request: PanelRequest
    builder: Literal["causal-raw-ohlcv.1"] = "causal-raw-ohlcv.1"
    source_code_sha256: str = Field(pattern=r"^sha256:[0-9a-f]{64}$")
    calendar_version: str = Field(min_length=1, max_length=64)
    quality: Literal["synthetic", "public_development"]
    panel: CachedObject
    values: CachedObject
    calendar: CachedObject
    dataset: CachedObject
    rows: int = Field(ge=1, le=2_000_000)
    eligible_rows: int = Field(ge=0, le=2_000_000)
    observed_rows: int = Field(ge=0, le=2_000_000)
    evaluation_eligible_rows: int = Field(ge=0, le=2_000_000)
    evaluation_observed_rows: int = Field(ge=0, le=2_000_000)


class PanelReport(ImmutableRecord):
    """Bounded command output; successful construction is not factor admission."""

    receipt: CachedObject
    panel: CachedObject
    calendar: CachedObject
    dataset: CachedObject
    quality: Literal["synthetic", "public_development"]
    rows: int
    eligible_rows: int
    observed_rows: int
    evaluation_eligible_rows: int
    evaluation_observed_rows: int
    production_eligible: Literal[False] = False
