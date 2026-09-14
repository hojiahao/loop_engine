"""Immutable source snapshots, quality reports and executable offline verification."""

import hashlib
import math
import os
import time
from collections.abc import Callable
from datetime import date
from importlib.metadata import version
from pathlib import Path

from loop_research.build_identity import canonical_bytes
from loop_research.calendar import xnys_session_dates
from loop_research.data.fetch_cache import private_directory, publish, read_cached
from loop_research.data.fetch_records import CachedObject
from loop_research.data.licensed_records import SourceTable
from loop_research.data.parquet_io import Row, encode_parquet, verify_parquet
from loop_research.data.snapshot_models import (
    MAX_ROWS,
    MAX_SOURCE_BYTES,
    PERIODS,
    SnapshotManifest,
    SnapshotPart,
    SnapshotReport,
    SnapshotRequest,
)
from loop_research.data.snapshot_sources import (
    Acquisition,
    daily_identifiers,
    load_acquisition,
    row_clocks,
)


class _Deadline:
    """One operation's bounded monotonic budget, including regression rejection."""

    def __init__(self, seconds: float, clock: Callable[[], float] = time.monotonic) -> None:
        if not math.isfinite(seconds) or not 0 < seconds <= 1800:
            raise ValueError("invalid source operation time budget")
        self.clock = clock
        self.previous = self.started = clock()
        if not math.isfinite(self.started):
            raise ValueError("invalid source operation clock")
        self.seconds = seconds

    def remaining(self) -> float:
        current = self.clock()
        remaining = self.seconds - (current - self.started)
        if not math.isfinite(current) or current < self.previous or remaining <= 0:
            raise ValueError("source operation deadline or clock regression")
        self.previous = current
        return remaining


def read_snapshot(store: Path, digest: str) -> tuple[CachedObject, SnapshotManifest]:
    """Load one bounded, checksum-named manifest; this does not verify its graph."""
    reference = CachedObject(sha256=digest, byte_size=1)
    directory = private_directory(store)
    try:
        size = os.stat(digest[7:], dir_fd=directory, follow_symlinks=False).st_size
    finally:
        os.close(directory)
    if not 0 < size <= 1024 * 1024:
        raise ValueError("source snapshot manifest byte budget")
    reference = CachedObject(sha256=reference.sha256, byte_size=size)
    content = read_cached(store, reference)
    manifest = SnapshotManifest.model_validate_json(content)
    if manifest.model_dump_json(by_alias=True).encode() != content:
        raise ValueError("source snapshot must be canonical JSON")
    return reference, manifest


def _calendar(request: SnapshotRequest) -> tuple[bytes, tuple[date, ...]]:
    sessions = xnys_session_dates(request.start, request.through)
    content = canonical_bytes(
        {
            "schema": "loop.source-calendar/v1",
            "name": "XNYS",
            "timezone": "America/New_York",
            "package_version": version("exchange-calendars"),
            "start": request.start.isoformat(),
            "through": request.through.isoformat(),
            "sessions": [day.isoformat() for day in sessions],
        }
    )
    return content, sessions


def _quality(
    source: Acquisition,
    table: SourceTable,
    rows: tuple[Row, ...],
    start: date,
    through: date,
    sessions: tuple[date, ...],
) -> bytes:
    expected = tuple(
        day
        for day in sessions
        if max(start, source.config.start) <= day <= min(through, source.config.end)
    )
    coverage = []
    daily = daily_identifiers(source, table)
    if daily:
        field, identifiers = daily
        expected_set = set(expected)
        column = table.columns.index(field) if rows else None
        for identifier in identifiers:
            actual = {row[0] for row in rows if column is not None and row[4][column] == identifier}
            if actual - expected_set:
                raise ValueError("daily source row is outside its requested XNYS sessions")
            missing = tuple(day for day in expected if day not in actual)
            coverage.append(
                {
                    "identifier": identifier,
                    "expected_sessions": len(expected),
                    "observed_sessions": len(actual),
                    "missing_sessions": len(missing),
                    "first_missing": missing[0].isoformat() if missing else None,
                    "last_missing": missing[-1].isoformat() if missing else None,
                    "missing_sha256": "sha256:"
                    + hashlib.sha256(
                        "".join(day.isoformat() + "\n" for day in missing).encode()
                    ).hexdigest(),
                }
            )
    return canonical_bytes(
        {
            "schema": "loop.source-quality/v1",
            "provider": source.config.provider,
            "dataset": table.dataset,
            "row_count": len(rows),
            "native_semantics": list(table.semantics),
            "calendar": "XNYS",
            "calendar_check": "daily_session_dates" if daily else "not_applicable",
            "selected_identifier_coverage": coverage,
            "null_counts": {
                column: sum(row[4][index] is None for row in rows)
                for index, column in enumerate(table.columns)
            },
            "known_through_ns": max((row[1] for row in rows), default=None),
            "ingested_through_ns": max((row[2] for row in rows), default=None),
            "historical_pit": "not_certified",
            "historical_universe": "not_certified",
            "delisting_coverage": "not_certified",
            "production_eligible": False,
            "entitlement": "public_development"
            if source.config.provider in {"sec", "alpaca"}
            else "declared_subscription_at_acquisition",
        }
    )


def _schema(table: SourceTable) -> bytes:
    return canonical_bytes(
        {
            "schema": "loop.source-table-schema/v1",
            "dataset": table.dataset,
            "envelope": {
                "_loop_observation_date": "date32",
                "_loop_known_at_ns": "int64",
                "_loop_ingested_at_ns": "int64",
                "_loop_availability": "utf8",
            },
            "native_columns": list(table.columns),
            "native_type": "nullable_utf8",
            "native_primary_key": list(table.primary_key),
            "semantics": list(table.semantics),
        }
    )


def _reference(content: bytes) -> CachedObject:
    return CachedObject(
        sha256="sha256:" + hashlib.sha256(content).hexdigest(), byte_size=len(content)
    )


def _materialize(
    store: Path,
    request: SnapshotRequest,
    deadline: _Deadline,
    expected: SnapshotManifest | None = None,
) -> SnapshotManifest:
    deadline.remaining()
    request = SnapshotRequest.model_validate(request)
    os.close(private_directory(store))
    calendar_bytes, sessions = _calendar(request)
    parts: list[SnapshotPart] = []
    total = excluded = source_bytes = input_rows = 0

    def object_reference(content: bytes) -> CachedObject:
        deadline.remaining()
        reference = _reference(content)
        if expected is None:
            return publish(store, content)
        if read_cached(store, reference) != content:
            raise ValueError("source snapshot object differs from replay")
        return reference

    calendar = object_reference(calendar_bytes)
    for digest in request.receipts:
        deadline.remaining()
        source = load_acquisition(store, digest, max_source_bytes=MAX_SOURCE_BYTES - source_bytes)
        deadline.remaining()
        source_bytes += source.source_bytes
        input_rows += sum(len(table.rows) for table in source.tables)
        if source_bytes > MAX_SOURCE_BYTES or input_rows > MAX_ROWS:
            raise ValueError("aggregate source snapshot budget")
        for table in sorted(source.tables, key=lambda item: item.dataset):
            selected: list[Row] = []
            for row in table.rows:
                clocks = row_clocks(source, table, dict(zip(table.columns, row, strict=True)))
                if (
                    max(request.start, source.config.start)
                    <= clocks[0]
                    <= min(request.through, source.config.end)
                ):
                    selected.append((*clocks, row))
                else:
                    excluded += 1
            selected.sort(key=lambda row: (row[0], row[1], tuple(value or "" for value in row[4])))
            for period, lower, upper in PERIODS:
                start, through = (
                    max(lower, request.start, source.config.start),
                    min(upper, request.through, source.config.end),
                )
                if start > through:
                    continue
                rows = tuple(row for row in selected if start <= row[0] <= through)
                encoded = encode_parquet(table.columns, rows)
                parquet = object_reference(encoded)
                verify_parquet(read_cached(store, parquet), table.columns, rows)
                # Reports and schema are independently content addressed; an object
                # existing on disk is never enough to publish a success manifest.
                parts.append(
                    SnapshotPart(
                        source_receipt=source.reference,
                        normalized_source=source.normalized,
                        provider=source.config.provider,
                        dataset=table.dataset,
                        period=period,
                        start=start,
                        through=through,
                        row_count=len(rows),
                        parquet=parquet,
                        table_schema=object_reference(_schema(table)),
                        quality_report=object_reference(
                            _quality(source, table, rows, start, through, sessions)
                        ),
                    )
                )
                total += len(rows)
                deadline.remaining()
                if len(parts) > 512:
                    raise ValueError("source snapshot part budget")
    manifest = SnapshotManifest(
        request=request,
        calendar=calendar,
        writer="pyarrow-" + version("pyarrow") + ":source-v1",
        parts=tuple(parts),
        total_rows=total,
        excluded_rows=excluded,
    )
    if expected is not None and manifest != expected:
        raise ValueError("snapshot declarations differ from replayed evidence")
    return manifest


def build_snapshot(
    store: Path,
    request: SnapshotRequest,
    *,
    timeout_seconds: float = 180,
) -> SnapshotReport:
    """Publish source Parquet and reports, then atomically publish the final manifest.

    Failures can leave immutable intermediate objects, never a successful final
    manifest. Retrying checks existing bytes without replacing them. This is an
    administrator's local workflow, not a runtime registration or holdout unlock.
    """
    deadline = _Deadline(min(timeout_seconds, 180))
    manifest = _materialize(store, request, deadline)
    content = manifest.model_dump_json(by_alias=True).encode()
    if len(content) > 1024 * 1024:
        raise ValueError("source snapshot manifest byte budget")
    deadline.remaining()
    reference = publish(store, content)
    return _report(reference, manifest)


def validate_snapshot(store: Path, digest: str) -> SnapshotReport:
    """Replay all sources and verify actual Parquet values; write nothing."""
    reference, manifest = read_snapshot(store, digest)
    _materialize(store, manifest.request, _Deadline(180), expected=manifest)
    return _report(reference, manifest)


def _report(reference: CachedObject, manifest: SnapshotManifest) -> SnapshotReport:
    return SnapshotReport(
        snapshot=reference,
        start=manifest.request.start,
        through=manifest.request.through,
        parts=len(manifest.parts),
        row_count=manifest.total_rows,
        excluded_rows=manifest.excluded_rows,
    )
