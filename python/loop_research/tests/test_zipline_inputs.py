import json
import os
import subprocess
import sys
from pathlib import Path

import pytest
from test_backtest_workflow import Case
from test_backtest_workflow import build as build
from test_backtest_workflow import case as case
from test_backtest_workflow import prepared as prepared
from test_market_workflow import market_case as market_case

from loop_research.data.fetch_cache import read_cached
from loop_research.data.fetch_records import CachedObject
from loop_research.zipline_inputs import prepare_zipline


def launch(case: Case, command: str, digest: str) -> subprocess.CompletedProcess[str]:
    script = Path(__file__).parents[3] / "scripts/uv-zipline.sh"
    return subprocess.run(
        [
            "bash",
            str(script),
            "run",
            "--locked",
            "--offline",
            "loop-zipline",
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


@pytest.mark.parametrize("fixture", ["case", "market_case"])
def test_accounting_replay(request: pytest.FixtureRequest, fixture: str) -> None:
    scenario: Case = request.getfixturevalue(fixture)
    result = scenario.run()
    export = subprocess.run(
        [
            sys.executable,
            "-m",
            "loop_research.cli",
            "zipline-prepare",
            "--backtest",
            result.receipt.sha256,
            "--evidence",
            str(scenario.evidence),
            "--view",
            str(scenario.view),
            "--store",
            str(scenario.store),
        ],
        capture_output=True,
        text=True,
        timeout=200,
        check=False,
        env={**os.environ, "OPENBLAS_NUM_THREADS": "1", "OMP_NUM_THREADS": "1"},
    )
    assert export.returncode == 0, export.stderr
    reference = CachedObject.model_validate_json(export.stdout)
    document = json.loads(read_cached(scenario.store, reference))
    raw = read_cached(scenario.store, CachedObject(**document["observations"]))
    assert b"nav_usd" not in raw and b"filled_shares" not in raw
    process = launch(scenario, "run", reference.sha256)
    assert process.returncode == 0, process.stderr + process.stdout
    report = json.loads(process.stdout)
    assert report["artifacts"]["disposition"] == "accepted"
    summary = json.loads(
        read_cached(scenario.store, CachedObject(**report["artifacts"]["summary"]))
    )
    assert summary["differences"] == 0 and summary["production_eligible"] is False
    before = {
        path.name: (path.stat().st_mtime_ns, path.read_bytes()) for path in scenario.store.iterdir()
    }
    replay = launch(scenario, "validate", report["receipt"]["sha256"])
    assert replay.returncode == 0, replay.stderr
    assert json.loads(replay.stdout) == report
    assert before == {
        path.name: (path.stat().st_mtime_ns, path.read_bytes()) for path in scenario.store.iterdir()
    }


def test_corrupt_portfolio(case: Case) -> None:
    result = case.run()
    (case.store / result.receipt.sha256[7:]).write_bytes(b"corrupt")
    before = set(case.store.iterdir())
    with pytest.raises(ValueError):
        prepare_zipline(case.evidence, case.view, case.store, result.receipt.sha256)
    assert before == set(case.store.iterdir())


def test_export_deadline(case: Case) -> None:
    result = case.run()
    ticks = iter((0.0, 0.0, 181.0))
    before = set(case.store.iterdir())
    with pytest.raises(TimeoutError):
        prepare_zipline(
            case.evidence, case.view, case.store, result.receipt.sha256, clock=lambda: next(ticks)
        )
    assert before == set(case.store.iterdir())
