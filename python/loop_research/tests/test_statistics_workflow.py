import copy
import json
import os
import subprocess
import sys
from dataclasses import replace
from datetime import UTC, date, datetime
from decimal import ROUND_DOWN, Context, Decimal, localcontext
from pathlib import Path
from typing import Any

import numpy as np
import pytest
from loop.v1.factor_pb2 import FACTOR_DIRECTION_LOWER_IS_BETTER
from loop_protocol.job import factor_identity_hash
from test_backtest_workflow import Case
from test_backtest_workflow import build as build
from test_backtest_workflow import case as case
from test_backtest_workflow import prepared as prepared
from test_panel_io import declaration, manifest

from loop_research.backtest import _Deadline, _materialize
from loop_research.build_identity import canonical_bytes
from loop_research.calendar import xnys_session_dates
from loop_research.cross_section import Exposures
from loop_research.data.fetch_cache import publish, read_cached
from loop_research.data.fetch_records import CachedObject
from loop_research.factor_worker import _artifact, execute
from loop_research.portfolio_statistics import _exposures
from loop_research.statistics_models import (
    ExperimentEvidence,
    ExperimentPlan,
    PlannedTrial,
    StatisticsReport,
    StatisticsRequest,
    TrialFailure,
    TrialOutcome,
)
from loop_research.statistics_workflow import run_statistics, trial_binding, validate_statistics


def settings() -> dict[str, str]:
    return {
        "groups": "3",
        "hac_lags": "1",
        "minimum_coverage_bps": "9000",
        "minimum_cross_section": "3",
        "minimum_sessions": "8",
        "pbo_blocks": "4",
        "statistics_profile": "daily-statistics.1",
    }


def freeze(case: Case, extra: dict[str, str] | None = None) -> None:
    documents = case.request.model_dump(mode="json")["policies"]
    documents["evaluation_policy"]["settings"] = dict(
        sorted({**settings(), **(extra or {})}.items())
    )
    case.change_request(policies=documents)
    case.work.factor.frozen_policy.evaluation_policy.sha256.value = bytes.fromhex(
        case.request.policies["evaluation_policy"].digest()[7:]
    )
    case.work.factor.factor_spec_id.value = "sha256:" + factor_identity_hash(case.work.factor).hex()
    work = publish(case.evidence, case.work.SerializeToString())
    result = execute(case.work, view=case.view, output=case.evidence)
    case.change_request(
        evaluation_work=work.model_dump(),
        evaluation_result={
            "sha256": result.manifest.artifact_id.value,
            "byte_size": result.manifest.byte_size,
        },
        factor_values={
            "sha256": result.values.artifact_id.value,
            "byte_size": result.values.byte_size,
        },
    )


def stamp(day: date, hour: int, minute: int = 0) -> int:
    return int(datetime(day.year, day.month, day.day, hour, minute, tzinfo=UTC).timestamp()) * 1000


@pytest.fixture
def extended(case: Case) -> Case:
    # Seventeen genuine exchange sessions -> sixteen synchronous return rows.
    days = xnys_session_dates(date(2010, 1, 4), date(2010, 1, 27))
    assert len(days) == 17
    securities = [f"US.{index + 1:03}" for index in range(6)]
    moves = np.random.default_rng(41).integers(-300, 400, size=(len(days), 6))
    panel_rows = []
    observations = []
    for index, day in enumerate(days):
        for column, security in enumerate(securities):
            opening = Decimal(10 + column)
            closing = opening * (10000 + int(moves[index, column])) / 10000
            panel_rows.append([day.isoformat(), security, "1", str(stamp(day, 21)), str(closing)])
            observations.append(
                [
                    day.isoformat(),
                    security,
                    str(stamp(day, 14, 30)),
                    str(opening),
                    str(stamp(day, 21)),
                    str(closing),
                ]
            )
    document = declaration(case.view, panel_rows)
    document.update(
        sessions=[day.isoformat() for day in days],
        securities=securities,
        decision_times_ms=[stamp(day, 21, 5) for day in days],
    )
    reference = manifest(case.view, document)
    case.work.panel_manifest.CopyFrom(
        _artifact(
            case.evidence,
            (case.view / reference.sha256[7:]).read_bytes(),
            name="loop.factor_panel",
            media_type="application/json",
            columns=[],
            completed_ms=1_300_000_000_000,
        )
    )
    case.work.sample_end.day = 27
    case.observations = observations
    case.tape()
    return case


def family(
    case: Case, *, failed: bool = False, second_settings: dict[str, str] | None = None
) -> tuple[StatisticsRequest, ExperimentEvidence]:
    second = copy.deepcopy(case)
    second.work.factor.direction = FACTOR_DIRECTION_LOWER_IS_BETTER
    trials = tuple(
        PlannedTrial(
            trial_id=name,
            binding_sha256=trial_binding(current.work, current.request.execution_tape),
        )
        for name, current in (("first", case), ("second", second))
    )
    plan = ExperimentPlan(family_id="test.family", trials=trials)
    plan_ref = publish(case.evidence, canonical_bytes(plan.model_dump(mode="json", by_alias=True)))
    freeze(case, {"experiment_plan": plan_ref.sha256[7:], "trial_id": "first"})
    first = case.run().receipt
    if failed:
        failure = TrialFailure(
            trial_id="second",
            binding_sha256=trials[1].binding_sha256,
            kind="infrastructure_failure",
            reason="worker_timeout",
        )
        failure_ref = publish(
            case.evidence, canonical_bytes(failure.model_dump(mode="json", by_alias=True))
        )
        outcome = TrialOutcome(trial_id="second", failure=failure_ref)
    else:
        freeze(
            second,
            {
                "experiment_plan": plan_ref.sha256[7:],
                "trial_id": "second",
                **(second_settings or {}),
            },
        )
        outcome = TrialOutcome(trial_id="second", backtest=second.run().receipt)
    evidence = ExperimentEvidence(
        plan=plan_ref, outcomes=(TrialOutcome(trial_id="first", backtest=first), outcome)
    )
    reference = publish(
        case.evidence, canonical_bytes(evidence.model_dump(mode="json", by_alias=True))
    )
    return StatisticsRequest(backtest=first, experiment=reference), evidence


def run(case: Case, request: StatisticsRequest) -> StatisticsReport:
    return run_statistics(case.evidence, case.view, case.store, request)


def document(case: Case, reference: CachedObject) -> dict[str, Any]:
    return json.loads(read_cached(case.store, reference))  # type: ignore[no-any-return]


def test_actual_statistics(extended: Case) -> None:
    request, _ = family(extended)
    report = run(extended, request)
    summary = document(extended, report.artifacts.summary)
    assert summary["return_observations"] == 16
    assert summary["return_mean"]["p_value"]["status"] == "available"
    assert summary["cross_section"]["ic"]["mean"]["status"] == "available"
    multiple = document(extended, report.artifacts.multiple_testing)
    assert multiple["planned_trials"] == multiple["completed_trials"] == 2
    assert multiple["failed_trials"] == 0
    assert multiple["pbo"]["status"] == "available" and len(multiple["splits"]) == 6
    assert all(item["dsr"]["status"] == "available" for item in multiple["trials"])
    assert multiple["scope"] == "complete-declared-family-not-global-search-history"
    assert not multiple["production_eligible"]
    before = {
        path.name: (path.stat().st_mtime_ns, path.read_bytes()) for path in extended.store.iterdir()
    }
    assert (
        validate_statistics(extended.evidence, extended.view, extended.store, report.receipt.sha256)
        == report
    )
    assert before == {
        path.name: (path.stat().st_mtime_ns, path.read_bytes()) for path in extended.store.iterdir()
    }


def test_failure_count(extended: Case) -> None:
    request, _ = family(extended, failed=True)
    report = run(extended, request)
    multiple = document(extended, report.artifacts.multiple_testing)
    assert multiple["planned_trials"] == 2 and multiple["failed_trials"] == 1
    assert multiple["trials"][1]["fdr_input"] == multiple["trials"][1]["fdr_adjusted"] == 1.0
    assert not multiple["trials"][1]["fdr_rejected"]
    assert multiple["trials"][1]["status"] == "infrastructure_failure"
    assert multiple["pbo"]["reason"] == "incomplete_trial_returns"
    assert multiple["trials"][0]["dsr"]["reason"] == "incomplete_trial_returns"


def test_policy_mismatch(case: Case) -> None:
    request, _ = family(case, second_settings={"minimum_coverage_bps": "8000"})
    with pytest.raises(ValueError, match="statistical policies differ"):
        run(case, request)


def test_decimal_context(extended: Case) -> None:
    freeze(extended)
    request = StatisticsRequest(backtest=extended.run().receipt)
    report = run(extended, request)
    with localcontext(Context(prec=9, rounding=ROUND_DOWN)):
        assert run(extended, request) == report


def test_interrupted_publication(case: Case, monkeypatch: pytest.MonkeyPatch) -> None:
    import loop_research.statistics_workflow as workflow

    freeze(case)
    request = StatisticsRequest(backtest=case.run().receipt)
    before = set(case.store.iterdir())
    calls = 0

    def interrupted(store: Path, content: bytes) -> CachedObject:
        nonlocal calls
        calls += 1
        if calls == 3:
            raise KeyboardInterrupt
        return publish(store, content)

    monkeypatch.setattr(workflow, "publish", interrupted)
    with pytest.raises(KeyboardInterrupt):
        run(case, request)
    partial = set(case.store.iterdir()) - before
    assert len(partial) == 2
    assert all(b"loop.statistics-receipt/v1" not in path.read_bytes() for path in partial)
    monkeypatch.setattr(workflow, "publish", publish)
    report = run(case, request)
    assert (
        validate_statistics(case.evidence, case.view, case.store, report.receipt.sha256) == report
    )


def test_small_sample(case: Case) -> None:
    freeze(case)
    report = run(case, StatisticsRequest(backtest=case.run().receipt))
    summary = document(case, report.artifacts.summary)
    assert summary["total_return"]["value"] == 0.2
    assert summary["one_way_turnover"]["value"] == 0.5
    assert summary["maximum_drawdown"]["value"] == 0
    assert summary["return_mean"]["p_value"]["reason"] == "insufficient_sessions"
    assert summary["beta_exposure"]["mean"]["reason"] == "missing_sessions"
    assert (
        document(case, report.artifacts.multiple_testing)["fdr_by"]["reason"]
        == "missing_experiment_evidence"
    )


def test_drawdown_ledger(case: Case) -> None:
    for row in case.observations[-2:]:
        row[-1] = "8"
    case.tape()
    freeze(case)
    report = run(case, StatisticsRequest(backtest=case.run().receipt))
    summary = document(case, report.artifacts.summary)
    assert summary["total_return"]["value"] == -0.2
    assert summary["maximum_drawdown"]["value"] == 0.2


def test_legacy_policy(case: Case) -> None:
    request = StatisticsRequest(backtest=case.run().receipt)
    before = set(case.store.iterdir())
    with pytest.raises(ValueError, match="opt-in frozen"):
        run(case, request)
    assert set(case.store.iterdir()) == before


@pytest.mark.parametrize(
    "change", ["missing", "reorder", "duplicate", "wrong_failure", "wrong_receipt", "wrong_plan"]
)
def test_family_integrity(case: Case, change: str) -> None:
    request, evidence = family(case, failed=True)
    values = evidence.model_dump(mode="json", by_alias=True)
    if change == "missing":
        values["outcomes"].pop()
    elif change == "reorder":
        values["outcomes"].reverse()
    elif change == "duplicate":
        values["outcomes"][1] = values["outcomes"][0]
    elif change == "wrong_failure":
        failure = TrialFailure(
            trial_id="second",
            binding_sha256="sha256:" + "0" * 64,
            kind="rejected",
            reason="coverage",
        )
        values["outcomes"][1]["failure"] = publish(
            case.evidence, canonical_bytes(failure.model_dump(mode="json", by_alias=True))
        ).model_dump()
    elif change == "wrong_receipt":
        values["outcomes"][0]["backtest"]["byte_size"] += 1
    else:
        values["plan"]["sha256"] = "sha256:" + "0" * 64
    reference = publish(case.evidence, canonical_bytes(values))
    changed = StatisticsRequest(backtest=request.backtest, experiment=reference)
    before = set(case.store.iterdir())
    with pytest.raises(ValueError):
        run(case, changed)
    assert set(case.store.iterdir()) == before


def test_source_corruption(case: Case) -> None:
    freeze(case)
    request = StatisticsRequest(backtest=case.run().receipt)
    raw = case.evidence / case.request.factor_values.sha256[7:]
    raw.chmod(0o600)
    raw.write_bytes(b"corrupted")
    with pytest.raises(ValueError):
        run(case, request)


def test_output_corruption(case: Case) -> None:
    freeze(case)
    report = run(case, StatisticsRequest(backtest=case.run().receipt))
    raw = case.store / report.artifacts.summary.sha256[7:]
    raw.chmod(0o600)
    raw.write_bytes(b"corrupted")
    with pytest.raises(ValueError):
        validate_statistics(case.evidence, case.view, case.store, report.receipt.sha256)
    assert raw.read_bytes() == b"corrupted"


def test_bound_deadline(case: Case) -> None:
    freeze(case)
    request = StatisticsRequest(backtest=case.run().receipt)
    ticks = iter([1.0, 2.0])
    with pytest.raises(TimeoutError):
        run_statistics(
            case.evidence,
            case.view,
            case.store,
            request,
            timeout_seconds=1,
            clock=lambda: next(ticks),
        )
    ticks = iter([2.0, 1.0])
    with pytest.raises(ValueError, match="clock regression"):
        run_statistics(case.evidence, case.view, case.store, request, clock=lambda: next(ticks))


def test_installed_cli(case: Case, tmp_path: Path) -> None:
    freeze(case)
    request = StatisticsRequest(backtest=case.run().receipt)
    config = tmp_path / "statistics.json"
    config.write_text(request.model_dump_json(by_alias=True))
    arguments = [
        "--evidence",
        str(case.evidence),
        "--view",
        str(case.view),
        "--store",
        str(case.store),
    ]
    environment = {**os.environ, "OPENBLAS_NUM_THREADS": "1", "OMP_NUM_THREADS": "1"}
    result = subprocess.run(
        [
            sys.executable,
            "-I",
            "-m",
            "loop_research.cli",
            "statistics-run",
            str(config),
            *arguments,
        ],
        check=True,
        capture_output=True,
        timeout=180,
        env=environment,
    )
    report = StatisticsReport.model_validate_json(result.stdout)
    replay = subprocess.run(
        [
            sys.executable,
            "-I",
            "-m",
            "loop_research.cli",
            "statistics-validate",
            "--receipt",
            report.receipt.sha256,
            *arguments,
        ],
        check=True,
        capture_output=True,
        timeout=180,
        env=environment,
    )
    assert result.stdout == replay.stdout and result.stderr == replay.stderr == b""
    work_request = tmp_path / "portfolio.json"
    work_request.write_text(case.request.model_dump_json(by_alias=True))
    bound = subprocess.run(
        [
            sys.executable,
            "-I",
            "-m",
            "loop_research.cli",
            "statistics-bind",
            str(work_request),
            "--evidence",
            str(case.evidence),
        ],
        check=True,
        capture_output=True,
        timeout=180,
        env=environment,
    )
    assert json.loads(bound.stdout)["binding_sha256"] == trial_binding(
        case.work, case.request.execution_tape
    )


def test_exposure_weights(case: Case) -> None:
    replay = _materialize(case.evidence, case.view, case.request, _Deadline(180, lambda: 0.0))
    panel = replay.computed.loaded.panel
    exposures = Exposures(
        panel.sessions,
        panel.securities,
        (("tech", "bank"),) * 3,
        np.array([[100.0, 400.0]] * 3),
        np.array([[1.2, 0.8]] * 3),
    )
    loaded = replace(replay.computed.loaded, exposures=exposures)
    replay = replace(replay, computed=replace(replay.computed, loaded=loaded))
    positions = [
        {"security_id": "US.001", "market_value_usd": "600"},
        {"security_id": "US.002", "market_value_usd": "-400"},
    ]
    beta, size, industries = _exposures(replay, positions, Decimal(1000), 0)
    assert beta == pytest.approx(0.4)
    assert size == pytest.approx(0.6 * np.log(100) - 0.4 * np.log(400))
    assert industries == {"tech": 0.6, "bank": -0.4}
    assert _exposures(replay, [], Decimal(1000), 0) == (0.0, 0.0, {})


def test_binding_context(case: Case) -> None:
    original = trial_binding(case.work, case.request.execution_tape)
    case.work.job_id.value = "another.job"
    case.work.lease_id.value = "another.lease"
    assert trial_binding(case.work, case.request.execution_tape) == original
    case.work.deterministic_seed.value = b"z" * 32
    assert trial_binding(case.work, case.request.execution_tape) != original
