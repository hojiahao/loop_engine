"""Read verified, bounded development panels from one authorized artifact leaf."""

from __future__ import annotations

import csv
import hashlib
import io
import math
import os
import stat
from dataclasses import dataclass
from datetime import date
from pathlib import Path
from typing import Literal

import numpy as np
from pydantic import BaseModel, ConfigDict, Field

from loop_research.calendar import require_session_decisions
from loop_research.evaluator import MAX_CELLS, Panel
from loop_research.operators import FIELDS

MAX_PANEL_BYTES = 64 * 1024 * 1024
MAX_METADATA_BYTES = 1024 * 1024


class ContentRef(BaseModel):
    """A relative CAS identity, not a caller-selectable filesystem location."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)
    sha256: str = Field(pattern=r"^sha256:[0-9a-f]{64}$")
    byte_size: int = Field(gt=0, le=MAX_PANEL_BYTES)


class PanelManifest(BaseModel):
    """Versioned development input declaration bound to exact CSV bytes."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)
    schema_version: Literal["loop.factor-panel/v1"] = Field(alias="schema")
    quality: Literal["synthetic", "public_development"]
    sessions: tuple[str, ...] = Field(min_length=1, max_length=8192)
    securities: tuple[str, ...] = Field(min_length=1, max_length=10000)
    fields: tuple[str, ...] = Field(min_length=1, max_length=6)
    decision_times_ms: tuple[int, ...] = Field(min_length=1, max_length=8192)
    evaluation_start: str
    values: ContentRef


def _version(metadata: os.stat_result) -> tuple[int, int, int, int, int]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


@dataclass(frozen=True, slots=True)
class _FileGuard:
    view: Path
    identity: tuple[int, int]
    reference: ContentRef
    version: tuple[int, int, int, int, int]

    def check(self) -> None:
        root = self.view.stat(follow_symlinks=False)
        if (
            not stat.S_ISDIR(root.st_mode)
            or root.st_mode & 0o222
            or (root.st_dev, root.st_ino) != self.identity
        ):
            raise ValueError("authorized data view changed")
        metadata = (self.view / self.reference.sha256[7:]).stat(follow_symlinks=False)
        if not stat.S_ISREG(metadata.st_mode) or _version(metadata) != self.version:
            raise ValueError("authorized input changed after verification")


def _read(view: Path, reference: ContentRef, maximum: int) -> tuple[bytes, _FileGuard]:
    if not view.is_absolute() or view.resolve(strict=True) != view:
        raise ValueError("authorized data view must be an absolute canonical leaf")
    if reference.byte_size > maximum:
        raise ValueError("authorized input exceeds the byte budget")
    directory = os.open(view, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        root = os.fstat(directory)
        if root.st_mode & 0o222:
            raise ValueError("worker data view must be read-only")
        descriptor = os.open(
            reference.sha256[7:], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory
        )
        with os.fdopen(descriptor, "rb") as stream:
            before = os.fstat(stream.fileno())
            if (
                not stat.S_ISREG(before.st_mode)
                or before.st_mode & 0o222
                or before.st_size != reference.byte_size
            ):
                raise ValueError("worker input must be a bounded read-only regular file")
            content = stream.read(reference.byte_size + 1)
            if _version(before) != _version(os.fstat(stream.fileno())):
                raise ValueError("worker input changed during read")
        if (
            len(content) != reference.byte_size
            or "sha256:" + hashlib.sha256(content).hexdigest() != reference.sha256
        ):
            raise ValueError("worker input checksum mismatch")
        guard = _FileGuard(view, (root.st_dev, root.st_ino), reference, _version(before))
        guard.check()
        return content, guard
    finally:
        os.close(directory)


@dataclass(frozen=True, slots=True)
class PanelInput:
    """Parsed data with live file guards; not a reusable access capability."""

    panel: Panel
    manifest: PanelManifest
    guards: tuple[_FileGuard, ...]

    def check(self) -> None:
        """Reject file/view replacement before releasing numerical outputs."""
        for guard in self.guards:
            guard.check()


def _date(value: str) -> date:
    parsed = date.fromisoformat(value)
    if parsed.isoformat() != value:
        raise ValueError("panel dates must use the canonical ISO format")
    return parsed


def _value(value: str) -> float:
    if value == "":
        return math.nan
    if len(value) > 64 or any(character not in "0123456789eE+-." for character in value):
        raise ValueError("panel observation is not a bounded decimal")
    parsed = float(value)
    if not math.isfinite(parsed):
        raise ValueError("panel observation is not finite")
    return parsed


def load_panel(
    view: Path, reference: ContentRef, *, sample_start: date, sample_end: date
) -> PanelInput:
    """Resolve only hash-named files inside the runtime-prepared leaf view.

    The caller must obtain this leaf and sample from authenticated, lease-bound
    runtime preparation. Supplied paths, dates or a PanelInput are not authority.
    This version denies protected date ranges and production quality claims.
    """
    if (
        type(sample_start) is not date
        or type(sample_end) is not date
        or sample_start > sample_end
        or not (
            date(2007, 1, 1) <= sample_start <= sample_end <= date(2016, 12, 31)
            or date(2017, 1, 1) <= sample_start <= sample_end <= date(2020, 12, 31)
        )
    ):
        raise ValueError("factor evaluation requires one authorized development sample")
    metadata, metadata_guard = _read(view, reference, MAX_METADATA_BYTES)
    manifest = PanelManifest.model_validate_json(metadata)
    if manifest.model_dump_json(by_alias=True).encode("utf8") != metadata:
        raise ValueError("factor panel manifest must be canonical JSON")
    sessions = tuple(_date(value) for value in manifest.sessions)
    start = _date(manifest.evaluation_start)
    if (
        len(sessions) * len(manifest.securities) > MAX_CELLS
        or not date(2005, 1, 1) <= sessions[0] <= start
        or sessions[-1] > sample_end
        or start not in sessions
        or not sample_start <= start <= sample_end
        or tuple(sorted(set(manifest.fields))) != manifest.fields
        or not set(manifest.fields) <= set(FIELDS)
    ):
        raise ValueError("panel axes, fields or evaluation window mismatch")
    require_session_decisions(sessions, manifest.decision_times_ms)
    # Exact session endpoints prevent trimming either end of a frozen sample.
    from loop_research.calendar import xnys_session_dates

    expected = xnys_session_dates(sample_start, sample_end)
    if not expected or sessions[sessions.index(start) :] != expected:
        raise ValueError("panel does not cover the complete frozen evaluation sample")
    content, values_guard = _read(view, manifest.values, MAX_PANEL_BYTES)
    shape = (len(sessions), len(manifest.securities))
    fields = {name: np.full(shape, np.nan, dtype=np.float64) for name in manifest.fields}
    eligible = np.zeros(shape, dtype=np.bool_)
    reader = csv.reader(io.StringIO(content.decode("ascii"), newline=""), strict=True)
    expected_header = ["session", "security_id", "eligible", "known_at_ms", *manifest.fields]
    if next(reader, None) != expected_header:
        raise ValueError("panel CSV schema mismatch")
    for row_index, session in enumerate(manifest.sessions):
        for column, security in enumerate(manifest.securities):
            row = next(reader, None)
            if (
                row is None
                or len(row) != len(expected_header)
                or row[:2] != [session, security]
                or row[2] not in ("0", "1")
            ):
                raise ValueError("panel rows must cover the exact ordered grid")
            eligible[row_index, column] = row[2] == "1"
            parsed = [_value(value) for value in row[4:]]
            if row[3] == "":
                if any(math.isfinite(value) for value in parsed):
                    raise ValueError("observed values require a visibility timestamp")
            else:
                known = int(row[3])
                if str(known) != row[3] or not 0 < known <= manifest.decision_times_ms[row_index]:
                    raise ValueError("observation was unavailable at the decision time")
            for name, value in zip(manifest.fields, parsed, strict=True):
                fields[name][row_index, column] = value
    if next(reader, None) is not None:
        raise ValueError("panel contains rows outside the authorized grid")
    result = PanelInput(
        Panel(sessions, manifest.securities, fields, eligible),
        manifest,
        (metadata_guard, values_guard),
    )
    result.check()
    return result
