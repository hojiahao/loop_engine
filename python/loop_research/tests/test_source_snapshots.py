"""Actual receipt-to-Parquet workflows; all source fixtures are invented."""

import asyncio
import json
import selectors
import subprocess
import sys
from datetime import date
from pathlib import Path

import httpx
import pyarrow as pa
import pyarrow.parquet as pq
import pytest
from data_helpers import (
    ENVIRONMENT,
    OBSERVED,
    AdvancingClock,
    alpaca_config,
    fixture_handler,
    response,
    sec_config,
)
from licensed_helpers import (
    DB_KEY,
    ENV,
    SEP_ROW,
    cache,
    jsonl,
    license_config,
    reference_row,
    table_bytes,
)

from loop_research.data.fetch_cache import publish, read_cached
from loop_research.data.ingestion import fetch_data
from loop_research.data.licensed_ingestion import fetch_licensed
from loop_research.data.parquet_io import Row, encode_parquet, verify_parquet
from loop_research.data.snapshot_models import SnapshotManifest, SnapshotRequest
from loop_research.data.snapshot_sources import load_acquisition, timestamp_ns
from loop_research.data.snapshots import build_snapshot, read_snapshot, validate_snapshot


def acquire(store: Path, provider: str, directory: Path) -> str:
    if provider in {"sec", "alpaca"}:
        requests: list[httpx.Request] = []
        report = asyncio.run(
            fetch_data(
                sec_config() if provider == "sec" else alpaca_config(),
                store,
                transport=httpx.MockTransport(fixture_handler(requests)),
                environment=ENVIRONMENT,
                now=lambda: OBSERVED,
                monotonic=AdvancingClock(),
            )
        )
        return report.receipt.sha256
    config, license = license_config(directory, provider)
    payload = table_bytes() if provider == "sharadar" else jsonl(reference_row())
    native = asyncio.run(
        fetch_licensed(
            config,
            store,
            license,
            transport=httpx.MockTransport(lambda _: response(payload)),
            environment=ENV if provider == "sharadar" else {"LOOP_TEST_KEY": DB_KEY},
            now=lambda: OBSERVED,
            monotonic=AdvancingClock(),
        )
    )
    return native.receipt.sha256


def request(
    *receipts: str, start: date = date(2026, 1, 1), through: date = date(2026, 8, 31)
) -> SnapshotRequest:
    return SnapshotRequest(receipts=tuple(sorted(receipts)), start=start, through=through)


@pytest.mark.parametrize(
    "provider,rows", [("sec", 4), ("alpaca", 2), ("sharadar", 1), ("databento", 1)]
)
def test_receipts_produce_replayable_parquet(tmp_path: Path, provider: str, rows: int) -> None:
    store = cache(tmp_path)
    digest = acquire(store, provider, tmp_path)
    config = request(digest)
    first = build_snapshot(store, config)
    assert first.row_count == rows and not first.production_eligible
    assert first.excluded_rows == (1 if provider == "alpaca" else 0)
    before = {path.name: path.stat().st_mtime_ns for path in store.iterdir()}
    assert build_snapshot(store, config) == first
    assert validate_snapshot(store, first.snapshot.sha256) == first
    assert before == {path.name: path.stat().st_mtime_ns for path in store.iterdir()}
    _, manifest = read_snapshot(store, first.snapshot.sha256)
    assert manifest.access_scope == "private_source_only"
    for part in manifest.parts:
        content = read_cached(store, part.parquet)
        actual = pq.ParquetFile(pa.BufferReader(content)).read(use_threads=False)
        assert actual.num_rows == part.row_count
        assert actual.schema.field("_loop_observation_date").type == pa.date32()
        assert actual.schema.field("_loop_known_at_ns").type == pa.int64()
        quality = json.loads(read_cached(store, part.quality_report))
        assert quality["historical_pit"] == "not_certified"
        if provider == "sharadar" and part.row_count:
            assert actual["closeunadj"].to_pylist() == ["44"]
            assert actual["close"].to_pylist() == ["11"]
            coverage = quality["selected_identifier_coverage"][0]
            assert coverage["missing_sessions"] > 0
            assert coverage["observed_sessions"] == 1
        if provider == "databento":
            assert actual["_loop_known_at_ns"].to_pylist() == [1788004800123456789]
            assert actual["ts_record"].to_pylist() == ["2026-08-29T12:00:00.123456789Z"]


def test_installed_snapshot_commands(tmp_path: Path) -> None:
    store = cache(tmp_path)
    digest = acquire(store, "sec", tmp_path)
    completed = subprocess.run(
        [
            sys.executable,
            "-I",
            "-m",
            "loop_research.cli",
            "data-snapshot",
            "--store",
            str(store),
            "--receipt",
            digest,
            "--start",
            "2026-01-01",
            "--through",
            "2026-08-31",
        ],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert completed.returncode == 0, completed.stderr
    report = json.loads(completed.stdout)
    replay = subprocess.run(
        [
            sys.executable,
            "-I",
            "-m",
            "loop_research.cli",
            "data-validate",
            "--store",
            str(store),
            "--snapshot",
            report["snapshot"]["sha256"],
        ],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert replay.returncode == 0, replay.stderr
    assert json.loads(replay.stdout) == report


def test_full_history_partitions_have_fixed_boundaries(tmp_path: Path) -> None:
    store = cache(tmp_path)
    dates = [
        "2006-12-29",
        "2007-01-03",
        "2016-12-30",
        "2017-01-03",
        "2020-12-31",
        "2021-01-04",
        "2024-12-31",
        "2025-01-02",
        "2026-08-31",
    ]
    config, license = license_config(tmp_path, start="2005-01-01")
    payload = table_bytes(rows=[[SEP_ROW[0], day, *SEP_ROW[2:]] for day in dates])
    receipt = asyncio.run(
        fetch_licensed(
            config,
            store,
            license,
            transport=httpx.MockTransport(lambda _: response(payload)),
            environment=ENV,
            now=lambda: OBSERVED,
        )
    ).receipt.sha256
    result = build_snapshot(store, request(receipt, start=date(2005, 1, 1)))
    _, manifest = read_snapshot(store, result.snapshot.sha256)
    assert [part.period for part in manifest.parts] == [
        "warmup",
        "in_sample",
        "development",
        "confirmation",
        "recent_holdout",
    ]
    assert [part.row_count for part in manifest.parts] == [1, 2, 2, 2, 2]
    calendar = json.loads(read_cached(store, manifest.calendar))
    assert calendar["sessions"][0] == "2005-01-03"
    assert calendar["sessions"][-1] == "2026-08-31"
    assert len(calendar["sessions"]) > 5400
    assert validate_snapshot(store, result.snapshot.sha256) == result


def test_source_revisions_create_new_snapshots(tmp_path: Path) -> None:
    store = cache(tmp_path)
    first_receipt = acquire(store, "sharadar", tmp_path)
    original = build_snapshot(store, request(first_receipt))
    original_bytes = read_cached(store, original.snapshot)
    config, license = license_config(tmp_path)
    corrected = [*SEP_ROW]
    corrected[7] = 10.5
    second_receipt = asyncio.run(
        fetch_licensed(
            config,
            store,
            license,
            transport=httpx.MockTransport(lambda _: response(table_bytes(rows=[corrected]))),
            environment=ENV,
            now=lambda: OBSERVED,
        )
    ).receipt.sha256
    revised = build_snapshot(store, request(second_receipt))
    assert original.snapshot.sha256 != revised.snapshot.sha256
    assert read_cached(store, original.snapshot) == original_bytes
    assert validate_snapshot(store, original.snapshot.sha256) == original


@pytest.mark.parametrize(
    "target",
    [
        "parquet",
        "table_schema",
        "quality_report",
        "calendar",
        "normalized_source",
        "source_receipt",
    ],
)
def test_corrupt_snapshot_graph_fails(tmp_path: Path, target: str) -> None:
    store = cache(tmp_path)
    result = build_snapshot(store, request(acquire(store, "sharadar", tmp_path)))
    _, manifest = read_snapshot(store, result.snapshot.sha256)
    reference = manifest.calendar if target == "calendar" else getattr(manifest.parts[0], target)
    path = store / reference.sha256[7:]
    path.write_bytes(b"x" * reference.byte_size)
    with pytest.raises(ValueError):
        validate_snapshot(store, result.snapshot.sha256)


@pytest.mark.parametrize(
    "field,value", [("total_rows", 9), ("excluded_rows", 9), ("writer", "other:writer")]
)
def test_rehashed_false_manifest_is_rejected(tmp_path: Path, field: str, value: object) -> None:
    store = cache(tmp_path)
    result = build_snapshot(store, request(acquire(store, "sharadar", tmp_path)))
    document = json.loads(read_cached(store, result.snapshot))
    document[field] = value
    changed = SnapshotManifest.model_validate_json(json.dumps(document))
    forged = publish(store, changed.model_dump_json(by_alias=True).encode())
    with pytest.raises(ValueError, match="declarations"):
        validate_snapshot(store, forged.sha256)


@pytest.mark.parametrize("changed", ["rows", "schema", "footer"])
def test_parquet_semantics_and_footer_are_verified(changed: str) -> None:
    rows: tuple[Row, ...] = (
        (date(2026, 8, 31), 1, 2, "synthetic", ("0.123456789012345678", None)),
    )
    columns = ("price", "absent")
    data = encode_parquet(columns, rows)
    verify_parquet(data, columns, rows)
    if changed == "rows":
        data = encode_parquet(columns, ((*rows[0][:4], ("7", None)),))
    elif changed == "schema":
        data = encode_parquet(("wrong", "absent"), rows)
    else:
        data = data[:-8] + (4 * 1024 * 1024).to_bytes(4, "little") + b"PAR1"
    with pytest.raises(ValueError):
        verify_parquet(data, columns, rows)


def test_all_null_empty_and_exact_text_columns() -> None:
    columns = ("leading_zero_id", "nullable")
    rows: tuple[Row, ...] = ((date(2026, 8, 31), 1, 2, "synthetic", ("000123", None)),)
    for values in (rows, ()):
        encoded = encode_parquet(columns, values)
        verify_parquet(encoded, columns, values)
        table = pq.ParquetFile(pa.BufferReader(encoded)).read(use_threads=False)
        assert table.schema.field("nullable").type == pa.string()
        if values:
            assert table["leading_zero_id"].to_pylist() == ["000123"]


@pytest.mark.parametrize(
    "start,through",
    [("2004-12-31", "2026-08-31"), ("2005-01-01", "2026-09-01"), ("2026-08-31", "2026-08-01")],
)
def test_snapshot_dates_fail_closed(start: str, through: str) -> None:
    with pytest.raises(ValueError):
        SnapshotRequest.model_validate_json(
            json.dumps(
                {
                    "receipts": ["sha256:" + "a" * 64],
                    "start": start,
                    "through": through,
                }
            )
        )


def test_nanosecond_offsets_preserve_identity() -> None:
    assert timestamp_ns("2026-08-29T08:00:00.123456789-04:00") == 1788004800123456789
    assert timestamp_ns("2026-08-29T12:00:00.123456789Z") == 1788004800123456789


def test_source_budget_denies_before_raw_replay(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from loop_research.data import snapshot_sources

    store = cache(tmp_path)
    digest = acquire(store, "sec", tmp_path)
    monkeypatch.setattr(
        snapshot_sources,
        "replay_data",
        lambda *_: pytest.fail("source budget was checked after IO"),
    )
    with pytest.raises(ValueError, match="source byte budget"):
        load_acquisition(store, digest, max_source_bytes=0)


@pytest.mark.parametrize("ticks", [(100.0, 99.0), (100.0, 102.0), (100.0, float("nan"))])
def test_deadline_and_clock_regression_deny(ticks: tuple[float, float]) -> None:
    from loop_research.data.snapshots import _Deadline

    readings = iter(ticks)
    budget = _Deadline(1, lambda: next(readings))
    with pytest.raises(ValueError, match="deadline or clock regression"):
        budget.remaining()


def test_nontrading_daily_observation_is_not_publishable(tmp_path: Path) -> None:
    store = cache(tmp_path)
    config, license = license_config(tmp_path)
    weekend = [*SEP_ROW]
    weekend[1] = "2026-08-29"
    receipt = asyncio.run(
        fetch_licensed(
            config,
            store,
            license,
            environment=ENV,
            now=lambda: OBSERVED,
            transport=httpx.MockTransport(lambda _: response(table_bytes(rows=[weekend]))),
        )
    ).receipt.sha256
    with pytest.raises(ValueError, match="XNYS sessions"):
        build_snapshot(store, request(receipt))
    assert all(
        not path.read_bytes().startswith(b'{"schema":"loop.source-snapshot/v1"')
        for path in store.iterdir()
    )


def test_empty_market_response_reports_all_requested_sessions_missing(tmp_path: Path) -> None:
    store = cache(tmp_path)
    config, license = license_config(tmp_path)
    receipt = asyncio.run(
        fetch_licensed(
            config,
            store,
            license,
            environment=ENV,
            now=lambda: OBSERVED,
            transport=httpx.MockTransport(lambda _: response(table_bytes(rows=[]))),
        )
    ).receipt.sha256
    result = build_snapshot(store, request(receipt))
    assert result.row_count == 0
    _, manifest = read_snapshot(store, result.snapshot.sha256)
    report = json.loads(read_cached(store, manifest.parts[0].quality_report))
    coverage = report["selected_identifier_coverage"][0]
    assert coverage["missing_sessions"] == coverage["expected_sessions"] == 21
    assert validate_snapshot(store, result.snapshot.sha256) == result


def test_failed_manifest_publication_can_retry(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from loop_research.data import snapshots

    store = cache(tmp_path)
    config = request(acquire(store, "sec", tmp_path))
    original = snapshots.publish

    def fail_manifest(path: Path, content: bytes):
        if content.startswith(b'{"schema":"loop.source-snapshot/v1"'):
            raise OSError("synthetic disk failure")
        return original(path, content)

    monkeypatch.setattr(snapshots, "publish", fail_manifest)
    with pytest.raises(OSError):
        build_snapshot(store, config)
    assert all(
        not path.read_bytes().startswith(b'{"schema":"loop.source-snapshot/v1"')
        for path in store.iterdir()
    )
    monkeypatch.setattr(snapshots, "publish", original)
    result = build_snapshot(store, config)
    assert validate_snapshot(store, result.snapshot.sha256) == result


@pytest.mark.parametrize("writers", [2, 4, 8])
def test_independent_snapshot_writers(tmp_path: Path, writers: int) -> None:
    store = cache(tmp_path)
    digest = acquire(store, "sec", tmp_path)
    command = [
        sys.executable,
        "-I",
        "-m",
        "loop_research.cli",
        "data-snapshot",
        "--store",
        str(store),
        "--receipt",
        digest,
        "--start",
        "2026-01-01",
        "--through",
        "2026-08-31",
    ]
    processes = [
        subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        for _ in range(writers)
    ]
    try:
        results = []
        for process in processes:
            stdout, stderr = process.communicate(timeout=90)
            assert process.returncode == 0, stderr
            results.append(json.loads(stdout)["snapshot"]["sha256"])
        assert len(set(results)) == 1
        assert validate_snapshot(store, results[0]).row_count == 4
    finally:
        for process in processes:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=10)


@pytest.mark.parametrize("boundary", ["before", "after"])
def test_hard_kill_at_manifest_commit_recovers(tmp_path: Path, boundary: str) -> None:
    store = cache(tmp_path)
    digest = acquire(store, "sec", tmp_path)
    script = """
import hashlib, json, sys, threading
from datetime import date
from pathlib import Path
from loop_research.data import snapshots
from loop_research.data.snapshot_models import SnapshotRequest
original = snapshots.publish
def intercepted(path, content):
    if content.startswith(b'{"schema":"loop.source-snapshot/v1"'):
        if sys.argv[3] == "after":
            original(path, content)
        print("sha256:" + hashlib.sha256(content).hexdigest(), flush=True)
        threading.Event().wait()
    return original(path, content)
snapshots.publish = intercepted
snapshots.build_snapshot(Path(sys.argv[1]), SnapshotRequest(
    receipts=(sys.argv[2],), start=date(2026, 1, 1), through=date(2026, 8, 31)))
"""
    process = subprocess.Popen(
        [sys.executable, "-I", "-c", script, str(store), digest, boundary],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        assert process.stdout is not None
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            assert selector.select(timeout=45), "snapshot writer did not reach commit boundary"
            manifest_digest = process.stdout.readline().strip()
        assert manifest_digest.startswith("sha256:")
        process.kill()
        process.wait(timeout=10)
        assert (store / manifest_digest[7:]).exists() == (boundary == "after")
        recovered = build_snapshot(store, request(digest))
        assert recovered.snapshot.sha256 == manifest_digest
        assert validate_snapshot(store, manifest_digest) == recovered
    finally:
        if process.poll() is None:
            process.kill()
        process.communicate(timeout=10)
