"""Resolve private byte-backed inputs before any derived panel publication."""

import os
from collections.abc import Callable
from datetime import date, datetime
from pathlib import Path

from loop_research.data.fetch_cache import private_directory, read_cached, read_receipt
from loop_research.data.fetch_config import FETCH_CONFIG
from loop_research.data.fetch_json import decode_object
from loop_research.data.fetch_records import CachedObject, DevelopmentBatch, FetchReceipt
from loop_research.data.models import MAX_RECORDS, PitInput, RawBar
from loop_research.data.snapshot_sources import load_acquisition
from loop_research.data.snapshots import read_snapshot, validate_snapshot
from loop_research.panel_models import PanelRequest

MAX_SOURCE_BYTES = 512 * 1024 * 1024
MAX_SOURCE_FILES = 512


def _check_raw(store: Path, capture: PitInput, check: Callable[[], float]) -> None:
    references = sorted(
        {record.source.raw_sha256 for record in (*capture.securities, *capture.bars)}
    )
    if len(references) > MAX_SOURCE_FILES:
        raise ValueError("panel source file budget")
    total = 0
    for digest in references:
        check()
        directory = private_directory(store)
        try:
            size = os.stat(digest[7:], dir_fd=directory, follow_symlinks=False).st_size
        finally:
            os.close(directory)
        total += size
        if total > MAX_SOURCE_BYTES:
            raise ValueError("panel source byte budget")
        read_cached(store, CachedObject(sha256=digest, byte_size=size))


def _development_bars(
    store: Path, request: PanelRequest, check: Callable[[], float]
) -> tuple[tuple[RawBar, ...], datetime]:
    if request.source_snapshot is None:
        raise ValueError("public development panels require a verified source snapshot")
    reference, snapshot = read_snapshot(store, request.source_snapshot.sha256)
    if reference != request.source_snapshot or not (
        request.warmup_start
        <= snapshot.request.start
        <= snapshot.request.through
        <= request.sample_end
    ):
        raise ValueError("source snapshot exceeds the selected development range")
    if any(part.provider not in {"sec", "alpaca"} for part in snapshot.parts):
        raise ValueError("licensed source panels require a separate quality gate")
    for digest in snapshot.request.receipts:
        check()
        _, content = read_receipt(store, digest)
        receipt = FetchReceipt.model_validate_json(content)
        config = FETCH_CONFIG.validate_json(read_cached(store, receipt.config))
        if not request.warmup_start <= config.start <= config.end <= request.sample_end:
            raise ValueError("source acquisition exceeds the selected development range")
    # This replays and checks original bytes, normalized tables and actual Parquet.
    validate_snapshot(store, reference.sha256, timeout_seconds=check())
    check()
    bars: dict[tuple[str, date, datetime], RawBar] = {}
    completed: datetime | None = None
    total = 0
    selected = set(request.securities)
    for digest in snapshot.request.receipts:
        check()
        acquisition = load_acquisition(store, digest, max_source_bytes=MAX_SOURCE_BYTES - total)
        total += acquisition.source_bytes
        if acquisition.config.provider not in {"sec", "alpaca"}:
            raise ValueError("unsupported source protocol for development panels")
        batch = DevelopmentBatch.model_validate_json(read_cached(store, acquisition.normalized))
        completed = max(completed or acquisition.completed_at, acquisition.completed_at)
        for bar in batch.bars:
            if bar.security_id not in selected or not (
                snapshot.request.start <= bar.session <= snapshot.request.through
            ):
                continue
            key = bar.security_id, bar.session, bar.known_at
            previous = bars.setdefault(key, bar)
            if previous != bar:
                raise ValueError("source snapshots disagree on a bar version")
            if len(bars) > MAX_RECORDS:
                raise ValueError("panel source record budget")
    if completed is None:
        raise ValueError("source snapshot has no acquisition evidence")
    return tuple(bars[key] for key in sorted(bars)), completed


def load_capture(store: Path, request: PanelRequest, check: Callable[[], float]) -> PitInput:
    """Read an immutable capture and resolve all raw source references.

    Synthetic captures supply invented prices explicitly. Public captures supply
    only an administrative security history; their prices are derived from a
    verified source snapshot. The latter never gains production/PIT certification.
    File, validation and source replay failures raise; no output is published.
    """
    check()
    content = read_cached(store, request.capture)
    decode_object(content)
    capture = PitInput.model_validate_json(content)
    if not set(request.securities) <= {record.security_id for record in capture.securities}:
        raise ValueError("requested security has no explicit history")
    if capture.fundamentals:
        raise ValueError("raw OHLCV construction requires a capture without fundamentals")
    _check_raw(store, capture, check)
    if capture.quality == "synthetic":
        if request.source_snapshot is not None:
            raise ValueError("synthetic captures cannot import public source data")
        return capture
    if capture.bars:
        raise ValueError("public prices must come from replayed source snapshots")
    bars, completed = _development_bars(store, request, check)
    if completed > capture.captured_at:
        raise ValueError("source acquisition follows the security capture cutoff")
    values = capture.model_dump()
    values["bars"] = bars
    return PitInput.model_validate(values)
