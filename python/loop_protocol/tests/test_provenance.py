import csv
from dataclasses import FrozenInstanceError
from pathlib import Path

import pytest

from loop.v1.research_common_pb2 import ResearchProvenanceFingerprint
from loop_protocol.provenance import (
    PROVENANCE_COMPONENTS,
    ProvenanceError,
    ProvenanceSnapshot,
    assess_provenance,
)

VECTORS = Path(__file__).parents[3] / "tests/contracts/provenance_vectors.tsv"
with VECTORS.open(encoding="ascii", newline="") as vector_file:
    CASES = list(csv.DictReader(vector_file, delimiter="\t"))


def fingerprint(mask: str = "000000") -> ResearchProvenanceFingerprint:
    value = ResearchProvenanceFingerprint()
    for index, (component, marker) in enumerate(zip(PROVENANCE_COMPONENTS, mask, strict=True)):
        assert marker in "01"
        digest = bytearray([index + 1] * 32)
        if marker == "1":
            digest[0] ^= 0xFF
        getattr(value, f"{component}_sha256").value = bytes(digest)
    return value


@pytest.mark.parametrize("case", CASES, ids=lambda value: value["name"])
def test_shared_freshness_vectors(case: dict[str, str]) -> None:
    recorded = ProvenanceSnapshot.from_wire(fingerprint(case["recorded"]))
    frozen = ProvenanceSnapshot.from_wire(fingerprint(case["frozen"]))
    current = (
        None
        if case["current"] == "-"
        else ProvenanceSnapshot.from_wire(fingerprint(case["current"]))
    )
    status: str
    try:
        assessment = assess_provenance(recorded, frozen, current)
        status, changed = assessment.status, assessment.changed
    except ProvenanceError as error:
        status, changed = error.code, error.changed
    assert status == case["status"]
    assert (",".join(changed) or "-") == case["changed"]


@pytest.mark.parametrize("component", PROVENANCE_COMPONENTS)
@pytest.mark.parametrize("size", [None, 0, 31, 33, 1024])
def test_every_digest_is_required_and_fixed_width(component: str, size: int | None) -> None:
    value = fingerprint()
    field = f"{component}_sha256"
    if size is None:
        value.ClearField(field)
    else:
        getattr(value, field).value = b"x" * size
    with pytest.raises(ProvenanceError) as failure:
        ProvenanceSnapshot.from_wire(value)
    assert failure.value.code == "invalid_digest"
    assert failure.value.changed == (component,)


def test_snapshot_does_not_alias_wire_bytes() -> None:
    wire = fingerprint()
    snapshot = ProvenanceSnapshot.from_wire(wire)
    wire.source_code_sha256.value = b"z" * 32
    assert snapshot == ProvenanceSnapshot.from_wire(fingerprint())
    assert snapshot.differences(ProvenanceSnapshot.from_wire(wire)) == ("source_code",)


def test_snapshot_is_immutable() -> None:
    snapshot = ProvenanceSnapshot.from_wire(fingerprint())
    with pytest.raises(FrozenInstanceError):
        snapshot._digests = ()  # type: ignore[misc]


@pytest.mark.parametrize("digests", [[], (), (b"x" * 32,), tuple(bytearray(32) for _ in range(6))])
def test_constructor_cannot_bypass_validation(digests: object) -> None:
    with pytest.raises(ProvenanceError):
        ProvenanceSnapshot(digests)  # type: ignore[arg-type]


@pytest.mark.parametrize(
    "mask,status", [("000000", "current"), ("000010", "stale"), ("-", "unresolved_current")]
)
def test_only_current_metrics_pass_the_gate(mask: str, status: str) -> None:
    frozen = ProvenanceSnapshot.from_wire(fingerprint())
    current = None if mask == "-" else ProvenanceSnapshot.from_wire(fingerprint(mask))
    assessment = assess_provenance(frozen, frozen, current)
    if status == "current":
        assessment.require_current()
    else:
        with pytest.raises(ProvenanceError) as failure:
            assessment.require_current()
        assert failure.value.code == status
