import json
import os
import subprocess
import sys
from pathlib import Path

import pytest
from test_backtest_workflow import Case
from test_statistics_workflow import build as build
from test_statistics_workflow import case as case
from test_statistics_workflow import extended as extended
from test_statistics_workflow import freeze
from test_statistics_workflow import prepared as prepared

from loop_research.alphalens_inputs import prepare_alphalens
from loop_research.data.fetch_cache import read_cached
from loop_research.data.fetch_records import CachedObject
from loop_research.statistics_models import StatisticsRequest
from loop_research.statistics_workflow import run_statistics


def launch(case: Case, command: str, digest: str) -> subprocess.CompletedProcess[str]:
    script = Path(__file__).parents[3] / "scripts/uv-alphalens.sh"
    return subprocess.run(
        [
            "bash",
            str(script),
            "run",
            "--locked",
            "--offline",
            "loop-alphalens",
            command,
            "--store",
            str(case.store),
            "--input" if command == "run" else "--receipt",
            digest,
        ],
        capture_output=True,
        text=True,
        timeout=200,
        check=False,
        env={**os.environ, "OPENBLAS_NUM_THREADS": "1", "OMP_NUM_THREADS": "1"},
    )


def statistics(case: Case) -> str:
    freeze(case)
    primary = case.run()
    return run_statistics(
        case.evidence, case.view, case.store, StatisticsRequest(backtest=primary.receipt)
    ).receipt.sha256


def test_independent_roundtrip(extended: Case) -> None:
    digest = statistics(extended)
    export = subprocess.run(
        [
            sys.executable,
            "-m",
            "loop_research.cli",
            "alphalens-prepare",
            "--statistics",
            digest,
            "--evidence",
            str(extended.evidence),
            "--view",
            str(extended.view),
            "--store",
            str(extended.store),
        ],
        capture_output=True,
        text=True,
        timeout=200,
        check=False,
        env={**os.environ, "OPENBLAS_NUM_THREADS": "1", "OMP_NUM_THREADS": "1"},
    )
    assert export.returncode == 0, export.stderr
    inputs = CachedObject.model_validate_json(export.stdout)
    document = json.loads(read_cached(extended.store, inputs))
    raw = read_cached(extended.store, CachedObject(**document["observations"]))
    assert b"rank_ic" not in raw and b"group_1" not in raw
    process = launch(extended, "run", inputs.sha256)
    assert process.returncode == 0, process.stderr
    report = json.loads(process.stdout)
    assert report["artifacts"]["disposition"] == "accepted"
    summary = json.loads(
        read_cached(extended.store, CachedObject(**report["artifacts"]["summary"]))
    )
    assert summary["available_sessions"] == 16 and summary["differences"] == 0
    before = {
        path.name: (path.stat().st_mtime_ns, path.read_bytes()) for path in extended.store.iterdir()
    }
    replay = launch(extended, "validate", report["receipt"]["sha256"])
    assert replay.returncode == 0, replay.stderr
    assert json.loads(replay.stdout) == report
    assert before == {
        path.name: (path.stat().st_mtime_ns, path.read_bytes()) for path in extended.store.iterdir()
    }


def test_corrupt_statistics(extended: Case) -> None:
    digest = statistics(extended)
    (extended.store / digest[7:]).write_bytes(b"corrupt")
    before = set(extended.store.iterdir())
    with pytest.raises(ValueError):
        prepare_alphalens(extended.evidence, extended.view, extended.store, digest)
    assert before == set(extended.store.iterdir())


def test_export_deadline(extended: Case) -> None:
    digest = statistics(extended)
    ticks = iter((0.0, 0.0, 181.0))
    before = set(extended.store.iterdir())
    with pytest.raises(TimeoutError):
        prepare_alphalens(
            extended.evidence, extended.view, extended.store, digest, clock=lambda: next(ticks)
        )
    assert before == set(extended.store.iterdir())
