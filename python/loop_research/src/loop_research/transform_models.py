"""Closed, versioned transformation policies and derived exposure references."""

import hashlib
from dataclasses import dataclass
from typing import Literal, Self

from pydantic import Field, model_validator

from loop_research.build_identity import canonical_bytes
from loop_research.data.fetch_records import CachedObject
from loop_research.data.models import ImmutableRecord


class PolicyDocument(ImmutableRecord):
    """The exact canonical policy document already addressed by a FactorSpec."""

    schema_version: Literal["loop.research-policy/v1"] = Field(alias="schema")
    policy_id: str = Field(pattern=r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$")
    revision: str = Field(pattern=r"^[1-9][0-9]{0,18}$")
    settings: dict[str, str] = Field(max_length=16)

    @model_validator(mode="after")
    def bounded_settings(self) -> Self:
        if list(self.settings) != sorted(self.settings) or any(
            not 1 <= len(key) <= 64 or not 1 <= len(value) <= 64
            for key, value in self.settings.items()
        ):
            raise ValueError("transformation policy settings must be bounded and ordered")
        return self

    def digest(self) -> str:
        """Compute the same field-ordered policy identity as the Rust materializer."""
        content = canonical_bytes(self.model_dump(mode="json", by_alias=True))
        return "sha256:" + hashlib.sha256(content).hexdigest()


class TransformRequest(ImmutableRecord):
    """Private panel-builder recipe; an exposure capture is not a worker artifact."""

    schema_version: Literal["loop.panel-transform-request/v1"] = Field(alias="schema")
    preprocess: PolicyDocument
    neutralization: PolicyDocument
    exposure_capture: CachedObject | None


class PanelTransform(ImmutableRecord):
    """Policy documents and the exact derived, session-aligned exposure CSV."""

    preprocess: PolicyDocument
    neutralization: PolicyDocument
    exposures: CachedObject | None


class TransformEvidence(ImmutableRecord):
    """Bounded outcome details attached to a version-2 factor result."""

    profile: Literal["cross-section.1"] = "cross-section.1"
    preprocess_sha256: str = Field(pattern=r"^sha256:[0-9a-f]{64}$")
    neutralization_sha256: str = Field(pattern=r"^sha256:[0-9a-f]{64}$")
    exposures_sha256: str | None = Field(pattern=r"^sha256:[0-9a-f]{64}$")
    raw_valid_observations: int = Field(ge=0, le=2_000_000)
    outcomes: tuple[Literal["ok", "insufficient", "rank_deficient", "constant"], ...] = Field(
        min_length=1, max_length=8192
    )


@dataclass(frozen=True, slots=True)
class TransformPolicy:
    """Validated executable options; identity remains in the source documents."""

    winsor_tail_bps: int
    standardize: bool
    minimum_observations: int
    industry: bool
    log_size: bool
    beta: bool

    def __post_init__(self) -> None:
        if (
            type(self.winsor_tail_bps) is not int
            or not 0 <= self.winsor_tail_bps <= 2500
            or type(self.minimum_observations) is not int
            or not 2 <= self.minimum_observations <= 10000
            or any(
                type(value) is not bool
                for value in (self.standardize, self.industry, self.log_size, self.beta)
            )
        ):
            raise ValueError("invalid executable transformation options")

    @property
    def needs_exposures(self) -> bool:
        return self.industry or self.log_size or self.beta


def _integer(value: str, lower: int, upper: int) -> int:
    if not value.isascii() or not value.isdecimal():
        raise ValueError("transformation policy requires a canonical integer")
    result = int(value)
    if str(result) != value or not lower <= result <= upper:
        raise ValueError("transformation policy integer bounds")
    return result


def resolve_policy(preprocess: PolicyDocument, neutralization: PolicyDocument) -> TransformPolicy:
    """Reject unknown settings and algorithms; never silently fall back to raw values."""
    preprocess = PolicyDocument.model_validate(preprocess)
    neutralization = PolicyDocument.model_validate(neutralization)
    first, second = preprocess.settings, neutralization.settings
    if (
        set(first) != {"algorithm", "minimum_observations", "standardize", "winsor_tail_bps"}
        or first["algorithm"] != "cross-section.1"
        or first["standardize"] not in {"none", "zscore"}
    ):
        raise ValueError("unsupported preprocessing policy")
    flags = (False, False, False)
    if second != {"algorithm": "none"}:
        if (
            set(second) != {"algorithm", "beta", "industry", "log_size"}
            or second["algorithm"] != "ols.1"
            or any(second[key] not in {"true", "false"} for key in ("industry", "log_size", "beta"))
        ):
            raise ValueError("unsupported neutralization policy")
        flags = second["industry"] == "true", second["log_size"] == "true", second["beta"] == "true"
        if not any(flags):
            raise ValueError("OLS requires at least one declared exposure")
    return TransformPolicy(
        _integer(first["winsor_tail_bps"], 0, 2500),
        first["standardize"] == "zscore",
        _integer(first["minimum_observations"], 2, 10000),
        *flags,
    )
