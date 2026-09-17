import copy
import json
import os
from pathlib import Path

import pytest
from test_backtest_workflow import Case
from test_backtest_workflow import build as build
from test_backtest_workflow import case as case
from test_backtest_workflow import prepared as prepared
from test_market_workflow import MarketCase
from test_market_workflow import market_case as market_case
from test_statistics_workflow import extended as extended
from test_statistics_workflow import freeze

from loop_research.build_identity import canonical_bytes
from loop_research.data.fetch_cache import publish, read_cached
from loop_research.data.fetch_records import CachedObject
from loop_research.portfolio_worker import (
    PortfolioWork,
    TrialLedger,
    execute,
    global_statistics,
)
from loop_research.statistics_kernels import fdr_by


def ledger(count: int = 3) -> TrialLedger:
    return TrialLedger.model_validate_json(
        canonical_bytes(
            {
                "schema": "loop.global-trials/v1",
                "entries": [
                    {
                        "job_id": f"job.{index}",
                        "run_id": f"run.{index}",
                        "factor_spec_id": "sha256:" + "11" * 32,
                        "specification_sha256": "sha256:" + "22" * 32,
                        "attempts": 1,
                    }
                    for index in range(count)
                ],
            }
        )
    )


@pytest.mark.parametrize("probability", [0.0, 0.001, 0.05, 0.5, 1.0])
def test_global_correction(probability: float) -> None:
    actual = json.loads(global_statistics(ledger(), "job.0", probability))
    assert actual["fdr_adjusted"] == pytest.approx(fdr_by([probability, 1.0, 1.0])[0])
    assert actual["registered_jobs"] == actual["counted_attempts"] == 3
    assert actual["dsr"]["status"] == actual["pbo"]["status"] == "unavailable"


def test_retry_count() -> None:
    value = ledger().model_dump(mode="json", by_alias=True)
    value["entries"][1]["attempts"] = 4
    actual = json.loads(
        global_statistics(TrialLedger.model_validate_json(canonical_bytes(value)), "job.0", 0.001)
    )
    assert actual["registered_jobs"] == 3
    assert actual["counted_attempts"] == 6
    assert actual["fdr_adjusted"] == pytest.approx(fdr_by([0.001, *([1.0] * 5)])[0])


@pytest.mark.parametrize("probability", [None, 1.0])
def test_no_discovery(probability: float | None) -> None:
    value = json.loads(global_statistics(ledger(), "job.0", probability))
    assert value["fdr_rejected"] is False
    assert value["production_eligible"] is False


@pytest.mark.parametrize("probability", [float("nan"), float("inf"), -0.1, 1.1])
def test_invalid_probability(probability: float) -> None:
    with pytest.raises(ValueError, match="p-value"):
        global_statistics(ledger(), "job.0", probability)


def test_missing_trial() -> None:
    with pytest.raises(ValueError, match="absent"):
        global_statistics(ledger(), "job.hidden", 0.001)


@pytest.mark.parametrize("change", ["duplicate", "unsorted", "empty", "overflow", "zero"])
def test_invalid_ledger(change: str) -> None:
    value = ledger().model_dump(mode="json", by_alias=True)
    entries = value["entries"]
    if change == "duplicate":
        entries.append(copy.deepcopy(entries[0]))
    elif change == "unsorted":
        entries.reverse()
    elif change == "empty":
        entries.clear()
    else:
        entries[0]["attempts"] = 65536 if change == "overflow" else 0
    with pytest.raises(ValueError):
        TrialLedger.model_validate_json(canonical_bytes(value))


def prepare(case: Case) -> PortfolioWork:
    freeze(case)
    # Only the market objects explicitly published by the broker are readable.
    references = [case.request.execution_tape]
    tape = json.loads(read_cached(case.evidence, case.request.execution_tape))
    if tape["schema"] == "loop.execution-tape/v2":
        capture = CachedObject.model_validate(tape["capture"])
        references.append(capture)
        records = json.loads(read_cached(case.evidence, capture))
        references.extend(CachedObject.model_validate(source) for source in records["sources"])
    else:
        references.append(CachedObject.model_validate(tape["observations"]))
    os.chmod(case.view, 0o700)
    for reference in references:
        path = case.view / reference.sha256[7:]
        path.write_bytes(read_cached(case.evidence, reference))
        path.chmod(0o444)
    case.view.chmod(0o555)
    request = publish(
        case.evidence, canonical_bytes(case.request.model_dump(mode="json", by_alias=True))
    )
    return PortfolioWork.model_validate_json(
        canonical_bytes(
            {
                "schema": "loop.portfolio-work/v1",
                "job_id": "job.0",
                "lease_id": "lease.portfolio",
                "specification": {"sha256": "sha256:" + "44" * 32, "byte_size": 1},
                "request": request.model_dump(),
                "trials": ledger().model_dump(mode="json", by_alias=True),
                "manifest": None,
            }
        )
    )


def run(case: Case, work: PortfolioWork) -> dict[str, object]:
    return execute(work, evidence=case.evidence, view=case.view, output=case.store)


def test_worker_replay(extended: Case) -> None:
    work = prepare(extended)
    artifact = run(extended, work)
    reference = CachedObject.model_validate(artifact["object"])
    document = json.loads(read_cached(extended.store, reference))
    assert document["result"]["engine_version"] == "authorized-portfolio.1"
    assert len(document["result"]["artifacts"]) == 8
    assert len(document["supplementary"]) == 6
    before = {path.name: path.stat().st_mtime_ns for path in extended.store.iterdir()}
    replay = work.model_copy(update={"manifest": reference})
    assert run(extended, replay) == artifact
    assert {path.name: path.stat().st_mtime_ns for path in extended.store.iterdir()} == before


def test_readonly_market(extended: Case) -> None:
    work = prepare(extended)
    path = extended.view / extended.request.execution_tape.sha256[7:]
    path.chmod(0o644)
    with pytest.raises(ValueError, match="read-only"):
        run(extended, work)
    assert not list(extended.store.iterdir())


def test_missing_market(extended: Case) -> None:
    work = prepare(extended)
    extended.view.chmod(0o755)
    (extended.view / extended.request.execution_tape.sha256[7:]).unlink()
    extended.view.chmod(0o555)
    # The same valid bytes remain in evidence; fallback would violate the view.
    with pytest.raises(FileNotFoundError):
        run(extended, work)
    assert not list(extended.store.iterdir())


def test_corrupt_replay(extended: Case) -> None:
    work = prepare(extended)
    artifact = run(extended, work)
    reference = CachedObject.model_validate(artifact["object"])
    document = json.loads(read_cached(extended.store, reference))
    nav = document["result"]["artifacts"]["nav"]["object"]
    path: Path = extended.store / nav["sha256"][7:]
    path.chmod(0o600)
    path.write_bytes(b"corrupt\n")
    with pytest.raises(ValueError):
        run(extended, work.model_copy(update={"manifest": reference}))
    assert path.read_bytes() == b"corrupt\n"


def test_market_replay(market_case: MarketCase) -> None:
    """Actual short/action accounting consumes broker-published v2 sources."""
    work = prepare(market_case)
    artifact = run(market_case, work)
    reference = CachedObject.model_validate(artifact["object"])
    document = json.loads(read_cached(market_case.store, reference))
    nav = CachedObject.model_validate(document["result"]["artifacts"]["nav"]["object"])
    assert b"2010-01-06,1000,50,-50,950,550,1000\n" in read_cached(market_case.store, nav)
    assert run(market_case, work.model_copy(update={"manifest": reference})) == artifact


def test_source_boundary(market_case: MarketCase) -> None:
    work = prepare(market_case)
    source = CachedObject.model_validate(market_case.capture["sources"][0])
    market_case.view.chmod(0o700)
    (market_case.view / source.sha256[7:]).unlink()
    market_case.view.chmod(0o555)
    # A private evidence copy cannot replace a source omitted by the broker.
    assert read_cached(market_case.evidence, source)
    with pytest.raises(FileNotFoundError):
        run(market_case, work)
    assert not list(market_case.store.iterdir())
