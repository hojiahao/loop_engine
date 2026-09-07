"""Fail-closed domain validation for immutable artifact references."""

from dataclasses import dataclass
from typing import Final, NoReturn, Protocol

from loop.v1.artifact_pb2 import ArtifactRef
from loop.v1.common_pb2 import Sha256Digest

MAX_ARTIFACT_URI_BYTES: Final = 2_048
_CONTENT_ADDRESS_PREFIX: Final = "artifact://sha256/"
_MIN_TIMESTAMP_SECONDS: Final = -62_135_596_800
_MAX_TIMESTAMP_SECONDS: Final = 253_402_300_799


class _Timestamp(Protocol):
    seconds: int
    nanos: int


class ArtifactValidationError(ValueError):
    """An ArtifactRef was unsafe or incomplete."""

    def __init__(self, code: str, field: str) -> None:
        self.code = code
        self.field = field
        super().__init__(f"{field} failed artifact validation ({code})")


@dataclass(frozen=True, slots=True)
class ValidatedArtifactRef:
    artifact_id: str
    uri: str
    sha256: bytes
    schema_name: str
    schema_version: int
    schema_sha256: bytes
    media_type: str
    byte_size: int
    row_count: int | None
    created_at_seconds: int
    created_at_nanos: int
    manifest_sha256: bytes | None


def validate_artifact_ref(reference: ArtifactRef) -> ValidatedArtifactRef:
    """Validate and detach a wire DTO before domain or storage use."""

    digest = _require_digest(reference.sha256 if reference.HasField("sha256") else None, "sha256")
    digest_hex = digest.hex()

    if len(reference.uri.encode("utf-8")) > MAX_ARTIFACT_URI_BYTES:
        _fail("uri_too_long", "uri")
    if not _is_strict_content_address(reference.uri):
        _fail("invalid_locator", "uri")
    if reference.uri != f"{_CONTENT_ADDRESS_PREFIX}{digest_hex}":
        _fail("identity_mismatch", "uri")

    if not reference.HasField("artifact_id"):
        _fail("missing_field", "artifact_id")
    artifact_id = reference.artifact_id.value
    if artifact_id != f"sha256:{digest_hex}":
        _fail("identity_mismatch", "artifact_id")

    if not reference.HasField("schema"):
        _fail("missing_field", "schema")
    schema = reference.schema
    if schema.version < 1 or not _is_identifier(schema.name):
        _fail("invalid_schema", "schema")
    schema_digest = _require_digest(
        schema.schema_sha256 if schema.HasField("schema_sha256") else None,
        "schema.schema_sha256",
    )
    if not _is_media_type(reference.media_type):
        _fail("invalid_media_type", "media_type")
    created_at_seconds, created_at_nanos = _require_timestamp(
        reference.created_at if reference.HasField("created_at") else None,
        "created_at",
    )
    manifest_digest = (
        _require_digest(reference.manifest_sha256, "manifest_sha256")
        if reference.HasField("manifest_sha256")
        else None
    )

    return ValidatedArtifactRef(
        artifact_id=artifact_id,
        uri=reference.uri,
        sha256=digest,
        schema_name=schema.name,
        schema_version=schema.version,
        schema_sha256=schema_digest,
        media_type=reference.media_type,
        byte_size=reference.byte_size,
        row_count=reference.row_count if reference.HasField("row_count") else None,
        created_at_seconds=created_at_seconds,
        created_at_nanos=created_at_nanos,
        manifest_sha256=manifest_digest,
    )


def _require_digest(digest: Sha256Digest | None, field: str) -> bytes:
    if digest is None:
        _fail("missing_field", field)
    value = bytes(digest.value)
    if len(value) != 32:
        _fail("invalid_digest", field)
    return value


def _require_timestamp(timestamp: _Timestamp | None, field: str) -> tuple[int, int]:
    if timestamp is None:
        _fail("missing_field", field)
    if (
        not _MIN_TIMESTAMP_SECONDS <= timestamp.seconds <= _MAX_TIMESTAMP_SECONDS
        or not 0 <= timestamp.nanos < 1_000_000_000
    ):
        _fail("invalid_timestamp", field)
    return timestamp.seconds, timestamp.nanos


def _is_strict_content_address(uri: str) -> bool:
    if not uri.startswith(_CONTENT_ADDRESS_PREFIX):
        return False
    digest = uri.removeprefix(_CONTENT_ADDRESS_PREFIX)
    return len(digest) == 64 and all(character in "0123456789abcdef" for character in digest)


def _is_identifier(value: str) -> bool:
    if not 0 < len(value.encode("utf-8")) <= 128:
        return False
    return all(
        segment
        and "a" <= segment[0] <= "z"
        and all(
            character.isascii() and (character.islower() or character.isdigit() or character == "_")
            for character in segment
        )
        for segment in value.split(".")
    )


def _is_media_type(value: str) -> bool:
    if not value.isascii() or not 0 < len(value.encode("ascii")) <= 255 or value.count("/") != 1:
        return False
    kind, subtype = value.split("/", maxsplit=1)
    return bool(kind and subtype) and all(32 < ord(character) < 127 for character in value)


def _fail(code: str, field: str) -> NoReturn:
    raise ArtifactValidationError(code, field)
