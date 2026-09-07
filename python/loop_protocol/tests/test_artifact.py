from pathlib import Path

import pytest

from loop.v1.artifact_pb2 import ArtifactRef, ArtifactSchemaReference
from loop.v1.common_pb2 import ArtifactId, Sha256Digest
from loop_protocol import ArtifactValidationError, validate_artifact_ref

_VECTORS = Path(__file__).parents[3] / "tests/contracts/artifact_ref_vectors.tsv"


def test_shared_artifact_vectors_fail_closed() -> None:
    for name, expected, raw_uri, raw_digest, raw_artifact_id, created_at in _rows():
        digest_hex = _token(raw_digest, "")
        digest = bytes.fromhex(digest_hex)
        uri = _token(raw_uri, digest_hex if len(digest_hex) == 64 else "")
        artifact_id = f"sha256:{digest_hex}" if raw_artifact_id == "@matching" else raw_artifact_id
        reference = ArtifactRef(
            artifact_id=ArtifactId(value=artifact_id),
            uri=uri,
            sha256=Sha256Digest(value=digest),
            schema=ArtifactSchemaReference(
                name="table.factor_values",
                version=1,
                schema_sha256=Sha256Digest(value=bytes([2]) * 32),
            ),
            media_type="application/vnd.apache.parquet",
            byte_size=42,
            row_count=1,
        )
        _set_created_at(reference, created_at)

        if expected == "accept":
            validated = validate_artifact_ref(reference)
            assert validated.uri == uri, name
            assert validated.artifact_id == artifact_id, name
            assert (validated.created_at_seconds, validated.created_at_nanos) == (
                reference.created_at.seconds,
                reference.created_at.nanos,
            ), name
        else:
            with pytest.raises(ArtifactValidationError) as error:
                validate_artifact_ref(reference)
            assert error.value.code == expected, name


def test_generated_artifact_ref_rejects_an_inline_payload_field() -> None:
    with pytest.raises(
        ValueError, match='Protocol message ArtifactRef has no "inline_bytes" field'
    ):
        ArtifactRef(inline_bytes=b"licensed dataset bytes")


def _rows() -> list[tuple[str, str, str, str, str, str]]:
    rows: list[tuple[str, str, str, str, str, str]] = []
    for line in _VECTORS.read_text(encoding="ascii").splitlines():
        if not line or line.startswith("#"):
            continue
        columns = line.split("\t")
        if len(columns) != 6:
            raise AssertionError("invalid shared artifact vector")
        rows.append((columns[0], columns[1], columns[2], columns[3], columns[4], columns[5]))
    return rows


def _set_created_at(reference: ArtifactRef, value: str) -> None:
    timestamps = {
        "valid": (1, 0),
        "zero": (0, 0),
        "before_minimum": (-62_135_596_801, 0),
        "after_maximum": (253_402_300_800, 0),
        "negative_nanos": (1, -1),
        "nanos_overflow": (1, 1_000_000_000),
    }
    if value == "missing":
        return
    seconds, nanos = timestamps[value]
    reference.created_at.seconds = seconds
    reference.created_at.nanos = nanos


def _token(value: str, digest: str) -> str:
    return (
        value.replace("@digest", digest)
        .replace("@zero32", "00" * 32)
        .replace("@zero31", "00" * 31)
        .replace("@one32", "11" * 32)
        .replace("@upperdigest", "AA" * 32)
        .replace("@overlong", f"artifact://sha256/{'0' * 2_100}")
    )
