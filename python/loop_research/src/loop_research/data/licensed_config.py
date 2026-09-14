"""Credential references and explicit rights scopes for licensed acquisition."""

from datetime import date, datetime
from typing import Annotated, Literal, Self

from pydantic import Field, TypeAdapter, model_validator

from loop_research.data.fetch_config import AlpacaSymbol, FetchBudget, SecretReference
from loop_research.data.models import Identifier, ImmutableRecord, Instant

type LicensedProvider = Literal["sharadar", "wrds", "databento"]
Digest = Annotated[str, Field(pattern=r"^sha256:[0-9a-f]{64}$")]


class DataLicense(ImmutableRecord):
    """Local owner's rights declaration, not a vendor signature or runtime identity.

    Only already prepaid internal research/storage is supported. No request may
    purchase data, allocate a new subscription slot or infer redistribution rights.
    """

    schema_version: Literal["loop.data-license/v1"] = Field(alias="schema")
    provider: LicensedProvider
    license_id: Identifier
    declared_by: Identifier
    datasets: tuple[Identifier, ...] = Field(min_length=1, max_length=8)
    valid_from: Instant
    expires_at: Instant
    data_start: date
    data_end: date
    purpose: Literal["internal_research"]
    local_storage: Literal[True]
    billing: Literal["prepaid_subscription"]
    current_reference_metadata: bool = False

    @model_validator(mode="after")
    def ordered_scope(self) -> Self:
        if (
            self.valid_from >= self.expires_at
            or self.data_start > self.data_end
            or len(set(self.datasets)) != len(self.datasets)
        ):
            raise ValueError("invalid license scope")
        return self


class LicensedRequest(ImmutableRecord):
    """Dates and budgets are explicit; a matching private license is mandatory."""

    schema_version: Literal["loop.licensed-fetch/v1"] = Field(alias="schema")
    license_sha256: Digest
    start: date
    end: date
    budget: FetchBudget = Field(default_factory=FetchBudget)

    @model_validator(mode="after")
    def ordered_dates(self) -> Self:
        if not date(2005, 1, 1) <= self.start <= self.end <= date(2100, 12, 31):
            raise ValueError("invalid acquisition period")
        return self


class SharadarRequest(LicensedRequest):
    """One explicit ticker selection across native Sharadar Data Link tables.

    This is a bounded acquisition partition, not a historical universe selector.
    SF1 uses as-reported dimensions; datekey remains a source date, not known_at.
    """

    provider: Literal["sharadar"]
    key_reference: SecretReference
    symbols: tuple[AlpacaSymbol, ...] = Field(min_length=1, max_length=8)
    tables: tuple[Literal["SEP", "SF1", "TICKERS", "ACTIONS"], ...] = Field(
        min_length=1, max_length=4
    )
    dimension: Literal["ARQ", "ARY", "ART"] = "ARQ"

    @model_validator(mode="after")
    def unique_selection(self) -> Self:
        if len(set(self.symbols)) != len(self.symbols) or len(set(self.tables)) != len(self.tables):
            raise ValueError("duplicate Sharadar selection")
        return self


class WrdsRequest(LicensedRequest):
    """Separate vendor PostgreSQL access; no caller-provided SQL or hostname."""

    provider: Literal["wrds"]
    username_reference: SecretReference
    password_reference: SecretReference
    profile: Literal["crsp_ciz_daily_v1", "compustat_fundq_v1"]
    identifiers: tuple[Annotated[str, Field(pattern=r"^[0-9]{1,8}$")], ...] = Field(
        min_length=1, max_length=8
    )

    @model_validator(mode="after")
    def unique_identifiers(self) -> Self:
        if self.username_reference == self.password_reference:
            raise ValueError("WRDS credential references must differ")
        if len({int(value) for value in self.identifiers}) != len(self.identifiers):
            raise ValueError("duplicate WRDS identifiers")
        if any(int(value) == 0 for value in self.identifiers):
            raise ValueError("zero is not a WRDS identity")
        if self.profile == "compustat_fundq_v1" and any(
            len(value) != 6 for value in self.identifiers
        ):
            raise ValueError("Compustat selection requires six-digit gvkeys")
        if self.profile == "crsp_ciz_daily_v1" and any(
            value != str(int(value)) for value in self.identifiers
        ):
            raise ValueError("PERMNO selection must use canonical decimal IDs")
        return self


class DatabentoRequest(LicensedRequest):
    """Reference-only reads of already allocated stable listing IDs."""

    provider: Literal["databento"]
    key_reference: SecretReference
    listing_ids: tuple[Annotated[str, Field(pattern=r"^L-[1-9][0-9]{0,15}$")], ...] = Field(
        min_length=1, max_length=8
    )
    datasets: tuple[Literal["security_master", "corporate_actions"], ...] = Field(
        min_length=1, max_length=2
    )
    allocate_isins: Literal[False] = False

    @model_validator(mode="after")
    def unique_selection(self) -> Self:
        if len(set(self.listing_ids)) != len(self.listing_ids) or len(set(self.datasets)) != len(
            self.datasets
        ):
            raise ValueError("duplicate Databento selection")
        return self


type LicensedConfig = SharadarRequest | WrdsRequest | DatabentoRequest
LICENSED_CONFIG: TypeAdapter[LicensedConfig] = TypeAdapter(
    Annotated[LicensedConfig, Field(discriminator="provider")]
)


def requested_datasets(config: LicensedConfig) -> tuple[str, ...]:
    """Return fixed provider namespaces used by both access gates and replay."""
    if isinstance(config, SharadarRequest):
        return tuple("SHARADAR/" + table for table in sorted(config.tables))
    if isinstance(config, WrdsRequest):
        return (
            "crsp.stkdlysecuritydata" if config.profile == "crsp_ciz_daily_v1" else "comp.fundq",
        )
    return tuple("databento/" + name for name in sorted(config.datasets))


def check_license(config: LicensedConfig, license: DataLicense, at: datetime) -> None:
    """Deny absent, expired, mismatched or insufficient declared rights.

    Recheck at receipt publication. Offline replay checks the original operation
    clocks; it does not grant permission for a new download after expiry.
    """
    if (
        license.provider != config.provider
        or not license.valid_from <= at < license.expires_at
        or not license.data_start <= config.start <= config.end <= license.data_end
        or not set(requested_datasets(config)) <= set(license.datasets)
        or (
            isinstance(config, SharadarRequest)
            and "TICKERS" in config.tables
            and not license.current_reference_metadata
        )
    ):
        raise ValueError("license does not cover this operation")
