import math

import numpy as np
import pytest
from hypothesis import given, settings
from hypothesis import strategies as st
from scipy import stats

from loop_research.statistics_kernels import (
    average_ranks,
    correlation,
    cscv,
    deflated_sharpe,
    fdr_by,
    mean_test,
    probabilistic_sharpe,
    sharpe,
)


def test_hac_golden() -> None:
    # Residual squares sum to 42, lag-one products to 26.25. Bartlett lag 1
    # gives (42 + 26.25) / (8*7) = 1.21875 for the corrected mean variance.
    result = mean_test(np.arange(1, 9, dtype=np.float64), minimum=8, lags=1)
    assert result["mean"].value == 4.5
    assert result["standard_error"].value == pytest.approx(math.sqrt(1.21875))
    assert result["t_value"].value == pytest.approx(4.5 / math.sqrt(1.21875))
    assert result["p_value"].value == pytest.approx(2 * stats.norm.sf(4.5 / math.sqrt(1.21875)))
    assert result["ci_lower"].value == pytest.approx(
        4.5 - stats.norm.ppf(0.975) * math.sqrt(1.21875)
    )


def test_hac_independent() -> None:
    values = np.array([0.03, -0.01, 0.04, 0.02, -0.03, 0.01, 0.02, 0.02])
    result = mean_test(values, minimum=8, lags=0)
    assert result["standard_error"].value == pytest.approx(stats.sem(values))
    assert result["t_value"].value == pytest.approx(stats.ttest_1samp(values, 0).statistic)


@pytest.mark.parametrize(
    "values,reason",
    [
        ([1.0] * 8, "degenerate_variance"),
        ([1.0, 2.0], "insufficient_sessions"),
        ([1.0] * 7 + [math.nan], "missing_sessions"),
    ],
)
def test_hac_unavailable(values: list[float], reason: str) -> None:
    result = mean_test(values, minimum=8, lags=1)
    assert result["p_value"].value is None
    assert result["p_value"].reason == reason
    assert result["t_value"].value is None


def test_rank_ties() -> None:
    values = np.array([4.0, 1.0, 4.0, 2.0, 2.0])
    assert average_ranks(values).tolist() == [4.5, 1, 4.5, 2.5, 2.5]
    assert average_ranks(values).tolist() == stats.rankdata(values).tolist()
    other = np.array([1.0, 5.0, 2.0, 3.0, 3.0])
    assert correlation(average_ranks(values), average_ranks(other)).value == pytest.approx(
        stats.spearmanr(values, other).statistic
    )
    assert correlation(values, other).value == pytest.approx(
        stats.pearsonr(values, other).statistic
    )


def test_constant_correlation() -> None:
    assert (
        correlation(np.ones(4), np.arange(4, dtype=np.float64)).reason == "constant_cross_section"
    )
    assert correlation(np.array([1.0, 2.0, math.nan]), np.arange(3, dtype=np.float64)).value is None


def test_by_golden() -> None:
    # m=4, harmonic sum=25/12. Step-up adjusted values, mapped to input order.
    assert fdr_by([0.04, 0.001, 1.0, 0.02]) == pytest.approx([1 / 9, 1 / 120, 1.0, 1 / 12])
    assert fdr_by([0.01, 0.01, 0.5]) == pytest.approx([0.0275, 0.0275, 11 / 12])


@given(st.lists(st.floats(min_value=0, max_value=1, allow_nan=False), min_size=1, max_size=32))
@settings(max_examples=40, deadline=None)
def test_by_properties(values: list[float]) -> None:
    adjusted = fdr_by(values)
    assert adjusted == pytest.approx(list(reversed(fdr_by(list(reversed(values))))))
    assert all(raw <= changed <= 1 for raw, changed in zip(values, adjusted, strict=True))
    ordered = [adjusted[index] for index in np.argsort(values)]
    assert ordered == sorted(ordered)


@pytest.mark.parametrize("values", [[], [math.nan], [-0.1], [1.1], [math.inf], [0.1] * 65])
def test_by_invalid(values: list[float]) -> None:
    with pytest.raises(ValueError):
        fdr_by(values)


def test_psr_golden() -> None:
    # Normal returns, SR=0.1, threshold=0.05, T=1000 -> independent paper formula.
    expected = stats.norm.cdf((0.1 - 0.05) * math.sqrt(999) / math.sqrt(1 + 0.1**2 / 2))
    assert probabilistic_sharpe(0.1, 0.05, 1000, 0, 3).value == pytest.approx(expected)
    assert probabilistic_sharpe(0.05, 0.05, 1000, -0.5, 5).value == 0.5


def test_dsr_moments() -> None:
    returns = np.array(
        [
            [0.03, 0.01, -0.02],
            [-0.01, 0.03, 0.04],
            [0.02, -0.03, 0.01],
            [0.04, 0.02, -0.04],
            [-0.02, 0.01, 0.03],
            [0.01, -0.01, 0.02],
            [0.02, 0.03, 0.01],
            [-0.01, 0.02, -0.01],
        ]
    )
    results, threshold = deflated_sharpe(returns, 8)
    ratios = returns.mean(axis=0) / returns.std(axis=0, ddof=1)
    gamma = np.euler_gamma
    benchmark = ratios.std(ddof=1) * (
        (1 - gamma) * stats.norm.ppf(2 / 3) + gamma * stats.norm.ppf(1 - 1 / (3 * math.e))
    )
    assert threshold.value == pytest.approx(benchmark)
    # scipy's biased central moments provide the independent moment convention.
    skewness = stats.skew(returns, axis=0, bias=True)
    kurtosis = stats.kurtosis(returns, axis=0, bias=True, fisher=False)
    expected = stats.norm.cdf(
        (ratios - benchmark)
        * math.sqrt(7)
        / np.sqrt(1 - skewness * ratios + (kurtosis - 1) * ratios**2 / 4)
    )
    assert [item.value for item in results] == pytest.approx(expected)


def test_dsr_incomplete() -> None:
    results, threshold = deflated_sharpe(np.zeros((8, 2)), 8)
    assert all(item.value is None for item in results)
    assert threshold.reason == "incomplete_trial_sharpes"


def test_decimal_constant() -> None:
    values = np.full(17, 0.1)
    assert sharpe(values, 8).reason == "constant_returns"
    assert mean_test(values, minimum=8, lags=1)["p_value"].reason == "degenerate_variance"


def test_cscv_golden() -> None:
    # Opposite strategies in symmetric regimes: four winners reverse, two
    # exact ties remain at the median. Every split has logit <= 0 -> PBO=1.
    first = np.array([0.01, 0.03, 0.02, 0.04, -0.01, -0.03, -0.02, -0.04])
    value, splits = cscv(np.column_stack((first, -first)), blocks=4, minimum=8)
    assert value.value == 1.0 and value.observations == 6
    assert len(splits) == 6
    assert [row["train_blocks"] for row in splits] == [
        [0, 1],
        [0, 2],
        [0, 3],
        [1, 2],
        [1, 3],
        [2, 3],
    ]
    assert [row["logit"] for row in splits] == pytest.approx(
        [-math.log(2), 0, -math.log(2), -math.log(2), 0, -math.log(2)]
    )


@pytest.mark.parametrize(
    "case,reason",
    [
        ("constant", "degenerate_split"),
        ("missing", "insufficient_complete_returns"),
        ("unequal", "unequal_block_lengths"),
        ("short", "insufficient_complete_returns"),
    ],
)
def test_cscv_unavailable(case: str, reason: str) -> None:
    values = np.arange(16, dtype=np.float64).reshape(8, 2) / 100
    if case == "constant":
        values[:, 1] = 0
    elif case == "missing":
        values[0, 0] = math.nan
    elif case == "unequal":
        values = np.concatenate([values, values[:1]])
    else:
        values = values[:4]
    result, rows = cscv(values, blocks=4, minimum=8)
    assert result.reason == reason and rows == []


def test_cscv_budget() -> None:
    def expired() -> None:
        raise TimeoutError("expired")

    with pytest.raises(TimeoutError):
        cscv(np.arange(16, dtype=np.float64).reshape(8, 2), blocks=4, minimum=8, check=expired)


def test_sharpe_scale() -> None:
    values = np.array([0.01, -0.01, 0.03, 0.02, -0.02, 0.01, -0.01, 0.02])
    result = sharpe(values, 8)
    assert result.value == pytest.approx(values.mean() / values.std(ddof=1))
    assert sharpe(values * 10, 8).value == pytest.approx(result.value)
    assert sharpe(-values, 8).value == pytest.approx(-result.value)  # type: ignore[operator]
