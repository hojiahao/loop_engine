"""Private causal exposure construction and read-only worker artifact parsing."""

import csv
import io
from collections.abc import Callable
from datetime import UTC, date, datetime, timedelta
from pathlib import Path
from typing import Literal

import numpy as np

from loop_research.cross_section import Exposures
from loop_research.data.fetch_cache import read_cached
from loop_research.data.fetch_json import decode_object
from loop_research.data.fetch_records import CachedObject
from loop_research.exposure_models import ExposureCapture, ExposureVersion
from loop_research.panel_io import (
    MAX_METADATA_BYTES,
    MAX_PANEL_BYTES,
    ContentRef,
    _FileGuard,
    _read,
    _value,
)
from loop_research.panel_sources import check_raw_references
from loop_research.transform_models import TransformRequest, resolve_policy

COLUMNS = ["session", "security_id", "known_at_ms", "industry", "market_cap", "beta"]


def prepare_exposures(
    sources: Path,
    reference: CachedObject,
    *,
    quality: Literal["synthetic", "public_development"],
    captured_at: datetime,
    sessions: tuple[date, ...],
    securities: tuple[str, ...],
    decisions: tuple[int, ...],
    check: Callable[[], float],
    charge: Callable[[int], None],
) -> tuple[TransformRequest, bytes | None]:
    """Resolve private policies and select visible exposure revisions without publication."""
    if reference.byte_size > MAX_METADATA_BYTES:
        raise ValueError("transformation metadata byte budget")
    check()
    content = read_cached(sources, reference)
    decode_object(content)
    request = TransformRequest.model_validate_json(content)
    policy = resolve_policy(request.preprocess, request.neutralization)
    if policy.needs_exposures != (request.exposure_capture is not None):
        raise ValueError("transformation exposure capture differs from the policy")
    if request.exposure_capture is None:
        return request, None
    if request.exposure_capture.byte_size > 8 * 1024 * 1024:
        raise ValueError("exposure capture byte budget")
    content = read_cached(sources, request.exposure_capture)
    decode_object(content)
    capture = ExposureCapture.model_validate_json(content)
    if capture.quality != quality or capture.captured_at != captured_at:
        raise ValueError("exposure quality or ingestion cutoff differs from price capture")
    if any(record.security_id not in securities for record in capture.records):
        raise ValueError("exposure capture contains an unselected security")
    check_raw_references(
        sources, tuple(record.source.raw_sha256 for record in capture.records), check
    )
    indexed: dict[tuple[str, date], list[ExposureVersion]] = {}
    for record in capture.records:
        indexed.setdefault((record.security_id, record.session), []).append(record)
    stream = io.StringIO(newline="")
    writer = csv.writer(stream, lineterminator="\n")
    writer.writerow(COLUMNS)
    for day, decision_ms in zip(sessions, decisions, strict=True):
        charge(len(capture.records) + len(securities))
        decision = datetime(1970, 1, 1, tzinfo=UTC) + timedelta(milliseconds=decision_ms)
        for security in securities:
            selected = max(
                (
                    record
                    for record in indexed.get((security, day), ())
                    if record.effective_at <= decision and record.known_at <= decision
                ),
                key=lambda record: record.known_at,
                default=None,
            )
            known = ""
            if selected is not None:
                delta = selected.known_at - datetime(1970, 1, 1, tzinfo=UTC)
                known = str(
                    (delta.days * 86400 + delta.seconds) * 1000 + (delta.microseconds + 999) // 1000
                )
            writer.writerow(
                [
                    day.isoformat(),
                    security,
                    known,
                    selected.industry or "" if selected else "",
                    selected.market_cap or "" if selected else "",
                    selected.beta or "" if selected else "",
                ]
            )
        if stream.tell() > MAX_PANEL_BYTES:
            raise ValueError("exposure CSV byte budget")
    check()
    return request, stream.getvalue().encode("ascii")


def load_exposures(
    view: Path,
    reference: CachedObject,
    *,
    sessions: tuple[date, ...],
    securities: tuple[str, ...],
    decisions: tuple[int, ...],
) -> tuple[Exposures, _FileGuard]:
    """Load only a verified artifact from an already authorized read-only leaf."""
    content, guard = _read(view, ContentRef(**reference.model_dump()), MAX_PANEL_BYTES)
    reader = csv.reader(io.StringIO(content.decode("ascii")), strict=True)
    if next(reader, None) != COLUMNS:
        raise ValueError("exposure CSV header mismatch")
    shape = len(sessions), len(securities)
    industries: list[tuple[str | None, ...]] = []
    size, beta = (np.full(shape, np.nan, dtype=np.float64) for _ in range(2))
    for index, day in enumerate(sessions):
        industry: list[str | None] = []
        for column, security in enumerate(securities):
            row = next(reader, None)
            if row is None or len(row) != len(COLUMNS) or row[:2] != [day.isoformat(), security]:
                raise ValueError("exposure rows must match the complete ordered panel grid")
            if row[2]:
                if len(row[2]) > 16 or not row[2].isascii() or not row[2].isdecimal():
                    raise ValueError("exposure requires a canonical knowledge timestamp")
                known = int(row[2])
                if str(known) != row[2] or not 0 < known <= decisions[index]:
                    raise ValueError("exposure was unavailable at the decision")
            elif any(row[3:]):
                raise ValueError("observed exposure requires a knowledge timestamp")
            industry.append(row[3] or None)
            size[index, column], beta[index, column] = _value(row[4]), _value(row[5])
        industries.append(tuple(industry))
    if next(reader, None) is not None:
        raise ValueError("exposure rows exceed the authorized grid")
    result = Exposures(sessions, securities, tuple(industries), size, beta)
    guard.check()
    return result, guard
