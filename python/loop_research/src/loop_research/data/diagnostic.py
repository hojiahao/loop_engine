"""Read-only local PIT reports; no runtime capability or quality certification."""

import hashlib
import json
import os
import stat
from pathlib import Path
from typing import Literal

from pydantic import Field

from loop_research.data.models import ImmutableRecord, PitInput, PitQuery
from loop_research.data.query import PitResult, query_capture

MAX_INPUT_BYTES = 8 * 1024 * 1024


class PitReport(ImmutableRecord):
    """Byte-bound diagnostic evidence, never a registered/admitted research result."""

    schema_version: Literal["loop.pit-diagnostic/v1"] = Field(
        default="loop.pit-diagnostic/v1", alias="schema"
    )
    input_sha256: str
    result_sha256: str
    declared_quality: Literal["synthetic", "public_development"]
    quality_verification: Literal["not_attested"] = "not_attested"
    calendar_validation: Literal["not_performed"] = "not_performed"
    result: PitResult


def _unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON field")
        result[key] = value
    return result


def _version(metadata: os.stat_result) -> tuple[int, int, int, int, int]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def _read(path: Path) -> bytes:
    descriptor = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as source:
        before = os.fstat(source.fileno())
        if not stat.S_ISREG(before.st_mode):
            raise ValueError("PIT input must be a regular file")
        if not 0 < before.st_size <= MAX_INPUT_BYTES:
            raise ValueError("PIT input exceeds the nonempty 8 MiB byte budget")
        content = source.read(MAX_INPUT_BYTES + 1)
        if (
            len(content) != before.st_size
            or _version(before) != _version(os.fstat(source.fileno()))
            or _version(before) != _version(path.stat(follow_symlinks=False))
        ):
            raise ValueError("PIT input changed during read")
    return content


def query_file(path: Path, query: PitQuery) -> PitReport:
    """Validate all input bytes and emit a deterministic historical selection.

    Bounded regular-file reads reject symlinks, devices, FIFOs and concurrent
    mutation. Duplicate JSON keys and invalid records fail before any report.
    Validation/file errors raise ValueError/OSError; nothing is written. Paths
    and data are local developer inputs, never an authorization mechanism.
    """
    content = _read(path)
    try:
        # Check ambiguous keys before Pydantic's strict JSON-mode date parsing.
        json.loads(content.decode("utf-8"), object_pairs_hook=_unique_object)
        capture = PitInput.model_validate_json(content)
    except (RecursionError, UnicodeError) as error:
        raise ValueError("PIT input must be bounded UTF-8 JSON") from error
    result = query_capture(capture, query)
    encoded = result.model_dump_json(by_alias=True).encode("utf-8")
    return PitReport(
        input_sha256="sha256:" + hashlib.sha256(content).hexdigest(),
        result_sha256="sha256:" + hashlib.sha256(encoded).hexdigest(),
        declared_quality=capture.quality,
        result=result,
    )
