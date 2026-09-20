import json
from pathlib import Path

import pytest
from test_backtest_workflow import Case
from test_backtest_workflow import build as build
from test_backtest_workflow import case as case
from test_backtest_workflow import prepared as prepared
from test_portfolio_worker import prepare as primary_work
from test_portfolio_worker import run
from test_statistics_workflow import extended as extended

from loop_research.data.fetch_cache import publish, read_cached
from loop_research.data.fetch_records import CachedObject
from loop_research.reconciliation_worker import ValidationWork, prepare


def setup(case: Case) -> tuple[ValidationWork, Path]:
    work = primary_work(case)
    artifact = run(case, work)
    primary = work.model_copy(update={"manifest": CachedObject.model_validate(artifact["object"])})
    policy = publish(case.evidence, b'{"profile":"alphalens-zipline-development.1"}')
    output = case.store.parent / "validation"
    output.mkdir(mode=0o700)
    return ValidationWork(
        schema="loop.validation-work/v1",
        job_id="job.validation",
        lease_id="lease.validation",
        primary_revision=3,
        primary=primary,
        policy=policy,
    ), output


def export(case: Case, work: ValidationWork, output: Path) -> dict[str, object]:
    return prepare(
        work, evidence=case.evidence, view=case.view, primary_store=case.store, output=output
    )


def mtimes(directory: Path) -> dict[str, int]:
    return {path.name: path.stat().st_mtime_ns for path in directory.iterdir()}


def test_export_replay(extended: Case) -> None:
    work, output = setup(extended)
    result = export(extended, work, output)
    reference = CachedObject.model_validate(result["reference"])
    inputs = json.loads(read_cached(output, reference))
    assert inputs["production_eligible"] is False
    assert inputs["primary_job_id"] == work.primary.job_id
    before = mtimes(output), mtimes(extended.store)
    replay = work.model_copy(update={"prepared": reference})
    assert export(extended, replay, output) == result
    assert (mtimes(output), mtimes(extended.store)) == before


@pytest.mark.parametrize("change", ["source", "lease", "revision", "missing", "corrupt"])
def test_replay_denial(extended: Case, change: str) -> None:
    work, output = setup(extended)
    result = export(extended, work, output)
    reference = CachedObject.model_validate(result["reference"])
    work = work.model_copy(update={"prepared": reference})
    if change == "source":
        assert work.primary.manifest is not None
        path = extended.store / work.primary.manifest.sha256[7:]
        path.write_bytes(b"corrupted")
    elif change in {"lease", "revision"}:
        update = {"lease_id": "lease.changed"} if change == "lease" else {"primary_revision": 4}
        work = work.model_copy(update=update)
    else:
        inputs = json.loads(read_cached(output, reference))
        path = output / inputs["alphalens"]["sha256"][7:]
        if change == "missing":
            path.unlink()
        else:
            path.write_bytes(b"corrupted")
    before = mtimes(output), mtimes(extended.store)
    with pytest.raises((ValueError, OSError)):
        export(extended, work, output)
    assert (mtimes(output), mtimes(extended.store)) == before
