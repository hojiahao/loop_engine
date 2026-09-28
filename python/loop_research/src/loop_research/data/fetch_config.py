"""Explicit, bounded development ingestion requests; credentials are references."""

from datetime import date
from typing import Annotated, Literal, Self

from pydantic import Field, TypeAdapter, model_validator

from loop_research.data.models import Identifier, ImmutableRecord


class FetchBudget(ImmutableRecord):
    """One sequential operation's request, decoded-byte, row and time budgets."""

    requests: int = Field(default=24, ge=1, le=64)
    pages: int = Field(default=8, ge=1, le=32)
    response_bytes: int = Field(default=16 * 1024 * 1024, ge=1024, le=32 * 1024 * 1024)
    total_bytes: int = Field(default=64 * 1024 * 1024, ge=1024, le=128 * 1024 * 1024)
    records: int = Field(default=10_000, ge=1, le=10_000)
    timeout_seconds: int = Field(default=90, ge=1, le=180)
    retries: int = Field(default=1, ge=0, le=2)
    interval_seconds: float = Field(default=0.5, ge=0.5, le=5, allow_inf_nan=False)

    @model_validator(mode="after")
    def consistent_limits(self) -> Self:
        if self.response_bytes > self.total_bytes:
            raise ValueError("response byte budget exceeds the total budget")
        return self


class FetchRequest(ImmutableRecord):
    """Common request shape; dates never default to an upstream server clock."""

    schema_version: Literal["loop.development-fetch/v1"] = Field(alias="schema")
    start: date
    end: date
    budget: FetchBudget = Field(default_factory=FetchBudget)

    @model_validator(mode="after")
    def valid_dates(self) -> Self:
        if not date(2005, 1, 1) <= self.start <= self.end <= date(2100, 12, 31):
            raise ValueError("fetch dates must be ordered within 2005-2100")
        return self


class SecRequest(FetchRequest):
    """Selected numeric SEC company facts plus the current submission metadata."""

    provider: Literal["sec"]
    cik: str = Field(pattern=r"^[0-9]{10}$")
    concepts: tuple[Identifier, ...] = Field(min_length=1, max_length=8)
    contact_email: str = Field(
        max_length=160, pattern=r"^[A-Za-z0-9_.+%-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$"
    )

    @model_validator(mode="after")
    def valid_selection(self) -> Self:
        if int(self.cik) == 0 or len(set(self.concepts)) != len(self.concepts):
            raise ValueError("SEC selection requires one CIK and unique concepts")
        for concept in self.concepts:
            parts = concept.split(":")
            if len(parts) != 2 or parts[0] not in {"us-gaap", "ifrs-full", "dei", "srt"}:
                raise ValueError("SEC concepts require an explicit standard taxonomy")
        return self


SecretReference = Annotated[str, Field(max_length=96, pattern=r"^LOOP_[A-Z][A-Z0-9_]+$")]
AlpacaSymbol = Annotated[str, Field(pattern=r"^[A-Z0-9][A-Z0-9.-]{0,31}$")]


class AlpacaRequest(FetchRequest):
    """Current asset identity with explicit historical feed and raw daily bars.

    The operation pins symbol asof to its observed New York date and records
    that choice in the receipt. Current asset metadata is never a historical
    issuer/share-class or survivorship attestation.
    """

    provider: Literal["alpaca"]
    symbols: tuple[AlpacaSymbol, ...] = Field(min_length=1, max_length=8)
    feed: Literal["iex", "sip"]
    identity_basis: Literal["current_asset"]
    key_id_reference: SecretReference
    secret_key_reference: SecretReference
    paper: bool = True
    probe_recent_sip: bool = False

    @model_validator(mode="after")
    def unique_symbols(self) -> Self:
        if len(set(self.symbols)) != len(self.symbols):
            raise ValueError("Alpaca symbols must be unique")
        if self.key_id_reference == self.secret_key_reference:
            raise ValueError("Alpaca credential references must be distinct")
        return self


type FetchConfig = SecRequest | AlpacaRequest
FETCH_CONFIG: TypeAdapter[FetchConfig] = TypeAdapter(
    Annotated[FetchConfig, Field(discriminator="provider")]
)
