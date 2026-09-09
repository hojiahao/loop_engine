"""Immutable research fingerprints; equality is not authority or proof of execution."""

from dataclasses import dataclass
from typing import Literal

from loop.v1.research_common_pb2 import ResearchProvenanceFingerprint

PROVENANCE_COMPONENTS = (
    "source_code",
    "operator_registry",
    "configuration",
    "data_manifest",
    "trading_calendar",
    "environment",
)


class ProvenanceError(ValueError):
    """Integrity/freshness failure, distinct from deterministic factor rejection."""

    def __init__(self, code: str, changed: tuple[str, ...] = ()) -> None:
        self.code = code
        self.changed = changed
        super().__init__(f"research provenance failed: {code}: {','.join(changed)}")


@dataclass(frozen=True, slots=True)
class ProvenanceSnapshot:
    """Validated, owned digest bytes in protocol field order."""

    _digests: tuple[bytes, ...]

    def __post_init__(self) -> None:
        if not isinstance(self._digests, tuple) or len(self._digests) != 6:
            raise ProvenanceError("invalid_snapshot")
        for component, digest in zip(PROVENANCE_COMPONENTS, self._digests, strict=True):
            if not isinstance(digest, bytes) or len(digest) != 32:
                raise ProvenanceError("invalid_digest", (component,))

    @classmethod
    def from_wire(cls, value: ResearchProvenanceFingerprint) -> ProvenanceSnapshot:
        """Reject absent or malformed digests and detach from the mutable DTO."""
        digests = []
        for component in PROVENANCE_COMPONENTS:
            field = f"{component}_sha256"
            if not value.HasField(field):
                raise ProvenanceError("invalid_digest", (component,))
            digests.append(bytes(getattr(value, field).value))
        return cls(tuple(digests))

    def differences(self, other: ProvenanceSnapshot) -> tuple[str, ...]:
        """Return all changed fields in stable order."""
        return tuple(
            component
            for component, left, right in zip(
                PROVENANCE_COMPONENTS, self._digests, other._digests, strict=True
            )
            if left != right
        )


@dataclass(frozen=True, slots=True)
class ProvenanceAssessment:
    """Metadata freshness, never a data capability or factor admission."""

    status: Literal["current", "stale", "unresolved"]
    changed: tuple[str, ...] = ()

    def require_current(self) -> None:
        """Reject stale or unresolved metrics at a current-result boundary."""
        if self.status == "stale":
            raise ProvenanceError("stale", self.changed)
        if self.status != "current":
            raise ProvenanceError("unresolved_current")


def assess_provenance(
    recorded: ProvenanceSnapshot,
    frozen: ProvenanceSnapshot,
    current: ProvenanceSnapshot | None,
) -> ProvenanceAssessment:
    """Check original-run integrity before freshness, without rewriting evidence.

    The owner must independently resolve factor, backtest, sample, seed, frozen
    inputs and the current context; caller metadata alone cannot establish them.
    A missing current context is never a match. No I/O is performed here.
    """
    if mismatch := recorded.differences(frozen):
        raise ProvenanceError("recording_mismatch", mismatch)
    if current is None:
        return ProvenanceAssessment("unresolved")
    changed = recorded.differences(current)
    return ProvenanceAssessment("stale" if changed else "current", changed)
