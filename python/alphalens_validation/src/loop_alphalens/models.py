"""Small versioned boundary; no primary research or provider imports."""

from datetime import date
from typing import Literal, Self

from pydantic import BaseModel, ConfigDict, Field, model_validator

HASH = r"^sha256:[0-9a-f]{64}$"


class Record(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, populate_by_name=True)


class Reference(Record):
    sha256: str = Field(pattern=HASH)
    byte_size: int = Field(ge=1, le=64 * 1024 * 1024, strict=True)


class Inputs(Record):
    schema_version: Literal["loop.alphalens-input/v1"] = Field(alias="schema")
    profile: Literal["alphalens-statistics.1"]
    primary_statistics: Reference
    primary_backtest: Reference
    source_code_sha256: str = Field(pattern=HASH)
    environment_sha256: str = Field(pattern=HASH)
    observations: Reference
    primary_cross_sections: Reference
    sessions: tuple[date, ...] = Field(min_length=2, max_length=8192)
    securities: tuple[str, ...] = Field(min_length=1, max_length=4096)
    direction: Literal["higher_is_better", "lower_is_better"]
    groups: int = Field(ge=2, le=10, strict=True)
    minimum_cross_section: int = Field(ge=3, le=1000, strict=True)
    minimum_sessions: int = Field(ge=8, le=8192, strict=True)
    production_eligible: Literal[False]

    @model_validator(mode="after")
    def bounded_axes(self) -> Self:
        if self.sessions != tuple(sorted(set(self.sessions))) or self.securities != tuple(
            sorted(set(self.securities))
        ):
            raise ValueError("independent axes must be sorted and unique")
        if len(self.sessions) * len(self.securities) > 100_000:
            raise ValueError("independent cell budget")
        if self.groups > self.minimum_cross_section:
            raise ValueError("independent group coverage")
        if any(day.weekday() > 4 or not 2007 <= day.year <= 2020 for day in self.sessions):
            raise ValueError("only development exchange sessions are supported")
        if any(not value.isascii() or not value or len(value) > 128 for value in self.securities):
            raise ValueError("invalid security identity")
        return self


class Receipt(Record):
    schema_version: Literal["loop.alphalens-receipt/v1"] = Field(
        default="loop.alphalens-receipt/v1", alias="schema"
    )
    inputs: Reference
    build: Reference
    cross_sections: Reference
    turnover: Reference
    differences: Reference
    summary: Reference
    disposition: Literal["accepted", "rejected", "unavailable"]
    production_eligible: Literal[False] = False
