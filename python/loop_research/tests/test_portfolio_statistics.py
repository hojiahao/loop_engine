import csv
import io
import math
from dataclasses import replace
from datetime import date
from decimal import Decimal

import numpy as np
import pytest
from loop_protocol.canonical import FactorDirection
from scipy import stats

from loop_research.portfolio import Observation, Session
from loop_research.portfolio_statistics import cross_sections
from loop_research.statistics_models import StatisticsPolicy, resolve_statistics
from loop_research.transform_models import PolicyDocument


def policy() -> StatisticsPolicy:
    return StatisticsPolicy(
        groups=3, minimum_cross_section=3, minimum_sessions=8, hac_lags=1, pbo_blocks=4
    )


def sessions() -> tuple[Session, ...]:
    result = []
    for day in (4, 5):
        rows = tuple(
            Observation(
                f"US.{index}",
                True,
                float(index + 1),
                day * 100,
                Decimal(100),
                day * 100 + 1,
                Decimal(100 + index * 2) if day == 5 else Decimal(100),
            )
            for index in range(6)
        )
        result.append(Session(date(2010, 1, day), day * 100 + 2, rows))
    return tuple(result)


def rows(
    values: tuple[Session, ...], direction: FactorDirection = FactorDirection.HIGHER_IS_BETTER
) -> list[dict[str, str]]:
    content, _ = cross_sections(values, direction, policy(), lambda: None)
    return list(csv.DictReader(io.StringIO(content.decode("ascii"))))


def test_group_golden() -> None:
    table = rows(sessions())
    assert table[0]["status"] == "available"
    assert float(table[0]["ic"]) == pytest.approx(1.0)
    assert float(table[0]["rank_ic"]) == pytest.approx(1.0)
    assert [float(table[0][f"group_{i}"]) for i in (1, 2, 3)] == pytest.approx([0.01, 0.05, 0.09])
    assert float(table[0]["spread"]) == pytest.approx(0.08)
    assert float(table[0]["monotonicity"]) == pytest.approx(1.0)
    assert table[-1]["status"] == "no_forward_session" and table[-1]["ic"] == ""


def test_frozen_direction() -> None:
    table = rows(sessions(), FactorDirection.LOWER_IS_BETTER)
    assert float(table[0]["ic"]) == pytest.approx(-1.0)
    assert float(table[0]["rank_ic"]) == pytest.approx(-1.0)
    assert float(table[0]["spread"]) == pytest.approx(-0.08)


def test_tied_groups() -> None:
    initial, forward = sessions()
    signals = (1.0, 1.0, 2.0, 4.0, 3.0, 3.0)
    initial = replace(
        initial,
        observations=tuple(
            replace(row, factor=signals[index]) for index, row in enumerate(initial.observations)
        ),
    )
    result = rows((initial, forward))[0]
    assert float(result["rank_ic"]) == pytest.approx(
        stats.spearmanr(signals, np.arange(6)).statistic
    )
    assert float(result["group_1"]) == pytest.approx(0.01)


def test_missing_labels() -> None:
    initial, forward = sessions()
    changed = list(forward.observations)
    changed[-1] = replace(changed[-1], opening=None, open_at_ms=None)
    result = rows((initial, replace(forward, observations=tuple(changed))))[0]
    assert result["status"] == "missing_forward_prices"
    assert result["signals"] == "6" and result["labels"] == "5"
    assert result["ic"] == result["group_1"] == result["spread"] == ""


def test_causal_eligibility() -> None:
    values = sessions()
    future = replace(
        values[1],
        observations=tuple(
            replace(row, eligible=False, factor=-1000.0) for row in values[1].observations
        ),
    )
    assert rows(values)[0] == rows((values[0], future))[0]


def test_constant_signals() -> None:
    initial, forward = sessions()
    initial = replace(
        initial, observations=tuple(replace(row, factor=1.0) for row in initial.observations)
    )
    result = rows((initial, forward))[0]
    assert result["status"] == "constant_signal"
    assert result["ic"] == result["group_1"] == ""


def test_group_degeneracy() -> None:
    initial, forward = sessions()
    forward = replace(
        forward,
        observations=tuple(replace(row, closing=Decimal(100)) for row in forward.observations),
    )
    result = rows((initial, forward))[0]
    assert result["status"] == "constant_labels"
    assert result["ic"] == result["monotonicity"] == ""
    assert float(result["group_1"]) == 0


@pytest.mark.parametrize(
    "change",
    [
        {"groups": 1},
        {"minimum_sessions": 7},
        {"hac_lags": 8},
        {"pbo_blocks": 5},
        {"trial_id": "unbound"},
        {"minimum_cross_section": 2},
    ],
)
def test_policy_bounds(change: dict[str, object]) -> None:
    with pytest.raises(ValueError):
        StatisticsPolicy.model_validate({**policy().model_dump(), **change})


@pytest.mark.parametrize("value", ["01", "-1", "1.0", "nan", "100000"])
def test_policy_integer(value: str) -> None:
    fields = {
        name: str(value)
        for name, value in policy().model_dump(exclude={"experiment_plan", "trial_id"}).items()
    }
    fields.update(
        statistics_profile="daily-statistics.1", minimum_coverage_bps="9000", groups=value
    )
    document = PolicyDocument.model_validate(
        {
            "schema": "loop.research-policy/v1",
            "policy_id": "evaluation",
            "revision": "1",
            "settings": dict(sorted(fields.items())),
        }
    )
    with pytest.raises(ValueError):
        resolve_statistics(document)


def test_statistic_finite() -> None:
    from loop_research.statistics_models import Statistic, available

    for value in (math.nan, math.inf, -math.inf):
        with pytest.raises(ValueError):
            available(value, 8)
    with pytest.raises(ValueError):
        Statistic(status="unavailable", observations=1, value=1.0, reason="missing")
