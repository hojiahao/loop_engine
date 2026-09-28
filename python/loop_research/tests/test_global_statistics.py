import json
from dataclasses import replace
from datetime import date, timedelta

import numpy as np
import pytest

from loop_research.global_models import GlobalPolicy, GlobalWork
from loop_research.global_statistics import StrategySeries, matrix_report, needs_returns
from loop_research.statistics_kernels import cscv, deflated_sharpe, fdr_by, mean_test


def parse(value: object) -> GlobalWork:
    return GlobalWork.model_validate_json(json.dumps(value))


def work(count: int = 2) -> GlobalWork:
    reference = {"sha256": "sha256:" + "11" * 32, "byte_size": 1}
    states = [
        {
            "job_id": f"job.{kind}.{index}",
            "revision": 3,
            "kind": "factor_evaluation" if kind == "a" else "backtest",
            "state": 4,
            "attempt": 1,
        }
        for kind in ("a", "b")
        for index in range(count)
    ]
    return parse(
        {
            "schema": "loop.global-statistics-work/v1",
            "job_id": "job.report",
            "lease_id": "lease.report",
            "started_at_ms": 1,
            "policy": reference,
            "snapshot": {
                "ledger": {
                    "schema": "loop.global-trials/v1",
                    "entries": [
                        {
                            "job_id": state["job_id"],
                            "run_id": "run.global",
                            "factor_spec_id": "sha256:" + "22" * 32,
                            "specification_sha256": "sha256:" + "33" * 32,
                            "attempts": 1,
                        }
                        for state in states
                    ],
                },
                "states": states,
            },
            "portfolios": [
                {
                    "job_id": f"job.b.{index}",
                    "evaluation_job_id": f"job.a.{index}",
                    "lease_id": f"lease.b.{index}",
                    "specification": reference,
                    "request": reference,
                    "manifest": reference,
                }
                for index in range(count)
            ],
        }
    )


def policy() -> GlobalPolicy:
    return GlobalPolicy(
        schema="loop.global-statistics-policy/v1",
        policy_id="policy.global",
        revision="1",
        scope="all-database-development-trials",
        minimum_sessions=8,
        hac_lags=1,
        pbo_blocks=4,
    )


def series(value: GlobalWork) -> tuple[StrategySeries, ...]:
    return tuple(
        StrategySeries(
            source=source,
            binding=f"strategy.{column}",
            context="context.common",
            dates=tuple(
                (date(2010, 1, 1) + timedelta(days=index)).isoformat() for index in range(16)
            ),
            returns=tuple(
                ((index * (column + 3) + 7 * column) % 11 - 4) / 100 for index in range(16)
            ),
        )
        for column, source in enumerate(value.portfolios)
    )


def checked() -> None:
    pass


@pytest.mark.parametrize("revision", ["v1", "0", "01", "18446744073709551616"])
def test_policy_revision(revision: str) -> None:
    document = policy().model_dump(mode="json", by_alias=True)
    document["revision"] = revision
    with pytest.raises(ValueError):
        GlobalPolicy.model_validate_json(json.dumps(document))


def test_complete_matrix() -> None:
    value = work()
    inputs = series(value)
    summary, matrix_bytes = matrix_report(value, policy(), inputs, checked)
    result = json.loads(summary)
    matrix = np.column_stack([item.returns for item in inputs])
    dsr, benchmark = deflated_sharpe(matrix, 8)
    pbo, splits = cscv(matrix, blocks=4, minimum=8, check=checked)
    assert result["complete_matrix"] is True
    assert result["production_eligible"] is False
    assert result["counted_attempts"] == result["registered_jobs"] == 4
    assert result["distinct_strategies"] == 2
    assert result["dsr_benchmark"] == benchmark.model_dump()
    assert result["pbo"] == pbo.model_dump()
    assert result["splits"] == splits
    assert len(matrix_bytes.splitlines()) == 33
    for column, item in enumerate(inputs):
        p_value = mean_test(item.returns, minimum=8, lags=1)["p_value"].value
        assert p_value is not None
        assert result["strategies"][column]["dsr"] == dsr[column].model_dump()
        assert result["strategies"][column]["global_by_upper_bound"] == pytest.approx(
            fdr_by([p_value, 1.0, 1.0, 1.0])[0]
        )
    assert matrix_report(value, policy(), inputs, checked) == (summary, matrix_bytes)


@pytest.mark.parametrize("state", [1, 2, 3, 5, 6, 7, 8])
def test_unfinished_trial(state: int) -> None:
    document = work().model_dump(mode="json", by_alias=True)
    document["snapshot"]["states"][0]["state"] = state
    value = parse(document)
    assert not needs_returns(value)
    summary, matrix = matrix_report(value, policy(), (), checked)
    result = json.loads(summary)
    assert not result["complete_matrix"]
    assert result["pbo"]["status"] == "unavailable"
    assert result["counted_attempts"] == 4
    assert len(matrix.splitlines()) == 1
    with pytest.raises(ValueError, match="source set"):
        matrix_report(value, policy(), series(value), checked)


def test_retry_missing() -> None:
    document = work().model_dump(mode="json", by_alias=True)
    document["snapshot"]["states"][0]["attempt"] = 2
    document["snapshot"]["ledger"]["entries"][0]["attempts"] = 2
    value = parse(document)
    summary, _ = matrix_report(value, policy(), (), checked)
    result = json.loads(summary)
    assert result["counted_attempts"] == 5
    assert result["issues"][0]["reason"] == "complete_attempt_returns_unavailable"


def test_duplicate_strategy() -> None:
    value = work(3)
    first, second, third = series(value)
    duplicate = replace(first, source=third.source)
    summary, matrix = matrix_report(value, policy(), (first, second, duplicate), checked)
    result = json.loads(summary)
    assert result["distinct_strategies"] == 2
    assert result["counted_attempts"] == 6
    assert result["strategies"][0]["job_ids"] == [first.source.job_id, third.source.job_id]
    assert len(matrix.splitlines()) == 33
    with pytest.raises(ValueError, match="different returns"):
        matrix_report(
            value, policy(), (first, second, replace(third, binding=first.binding)), checked
        )


@pytest.mark.parametrize("change", ["dates", "context"])
def test_incompatible_returns(change: str) -> None:
    value = work()
    first, second = series(value)
    if change == "dates":
        second = replace(second, dates=(*second.dates[:-1], "2010-02-01"))
    else:
        second = replace(second, context="context.other")
    summary, matrix = matrix_report(value, policy(), (first, second), checked)
    result = json.loads(summary)
    assert not result["complete_matrix"]
    assert result["issues"][0]["reason"] == "incompatible_return_context"
    assert len(matrix.splitlines()) == 1


@pytest.mark.parametrize("change", ["missing", "reordered", "attempt", "predecessor"])
def test_population_binding(change: str) -> None:
    document = work().model_dump(mode="json", by_alias=True)
    if change == "missing":
        document["portfolios"].pop()
    elif change == "reordered":
        document["snapshot"]["states"].reverse()
    elif change == "attempt":
        document["snapshot"]["states"][0]["attempt"] = 2
    else:
        document["portfolios"][0]["evaluation_job_id"] = "job.unknown"
    with pytest.raises(ValueError):
        parse(document)


def test_missing_continuation() -> None:
    document = work().model_dump(mode="json", by_alias=True)
    document["portfolios"][1]["evaluation_job_id"] = "job.a.0"
    value = parse(document)
    summary, _ = matrix_report(value, policy(), (), checked)
    assert json.loads(summary)["issues"][0]["reason"] == "missing_portfolio_continuation"


def test_deadline_propagates() -> None:
    def expired() -> None:
        raise TimeoutError("deadline")

    value = work()
    with pytest.raises(TimeoutError):
        matrix_report(value, policy(), series(value), expired)


@pytest.mark.parametrize("number", [float("nan"), float("inf"), 1e13])
def test_invalid_returns(number: float) -> None:
    value = work()
    first, second = series(value)
    with pytest.raises(ValueError, match="finite and bounded"):
        matrix_report(
            value, policy(), (replace(first, returns=(number, *first.returns[1:])), second), checked
        )
