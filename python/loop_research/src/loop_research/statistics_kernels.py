"""Bounded descriptive and inferential kernels with explicit assumptions (ADR 0031)."""

import math
from collections.abc import Callable, Sequence
from itertools import combinations
from statistics import NormalDist

import numpy as np
from numpy.typing import NDArray

from loop_research.statistics_models import Statistic, available, unavailable

NORMAL = NormalDist()


def _vector(values: Sequence[float] | NDArray[np.float64]) -> NDArray[np.float64]:
    result = np.asarray(values, dtype=np.float64)
    if result.ndim != 1 or len(result) > 8192 or np.isinf(result).any():
        raise ValueError("statistical vector bounds")
    if np.any(np.abs(result[np.isfinite(result)]) > 1e12):
        raise ValueError("statistical magnitude bounds")
    return result


def average_ranks(values: NDArray[np.float64]) -> NDArray[np.float64]:
    """Ascending average ranks; equal observations never get arbitrary IC ranks."""
    values = _vector(values)
    if not np.isfinite(values).all():
        raise ValueError("ranks require complete finite observations")
    order = np.argsort(values, kind="stable")
    result = np.empty(len(values), dtype=np.float64)
    start = 0
    while start < len(values):
        stop = start + 1
        while stop < len(values) and values[order[stop]] == values[order[start]]:
            stop += 1
        result[order[start:stop]] = (start + 1 + stop) / 2
        start = stop
    return result


def correlation(left: NDArray[np.float64], right: NDArray[np.float64]) -> Statistic:
    """Pearson correlation with scaled central products and no pairwise deletion."""
    left, right = _vector(left), _vector(right)
    if len(left) != len(right):
        raise ValueError("correlation axes differ")
    count = len(left)
    if count < 3 or not np.isfinite(left).all() or not np.isfinite(right).all():
        return unavailable("incomplete_cross_section", count)
    centered = []
    for values in (left, right):
        scale = float(np.max(np.abs(values)))
        if scale == 0:
            return unavailable("constant_cross_section", count)
        unit = values / scale
        unit = unit - float(np.mean(unit))
        norm = float(np.linalg.norm(unit))
        if norm <= np.finfo(np.float64).eps * math.sqrt(count):
            return unavailable("constant_cross_section", count)
        centered.append(unit / norm)
    return available(float(np.clip(np.dot(*centered), -1, 1)), count)


def mean_test(
    values: Sequence[float] | NDArray[np.float64], *, minimum: int, lags: int
) -> dict[str, Statistic]:
    """Intercept-only Bartlett HAC, finite-sample correction, normal inference.

    Adjacent entries must be adjacent sessions. Missing entries invalidate the
    inference instead of compressing calendar gaps into false adjacency.
    """
    values = _vector(values)
    count = len(values)
    keys = ("mean", "standard_error", "t_value", "p_value", "ci_lower", "ci_upper")
    if type(minimum) is not int or minimum < 2 or type(lags) is not int or not 0 <= lags <= 60:
        raise ValueError("HAC settings bounds")
    if not np.isfinite(values).all() or count < minimum or count <= lags:
        reason = "missing_sessions" if not np.isfinite(values).all() else "insufficient_sessions"
        return {key: unavailable(reason, count) for key in keys}
    mean = math.fsum(values) / count
    residual = values - mean
    terms = [float(residual @ residual)]
    for lag in range(1, lags + 1):
        terms.append(2 * (1 - lag / (lags + 1)) * float(residual[lag:] @ residual[:-lag]))
    variance = math.fsum(terms) / (count * (count - 1))
    result = {key: unavailable("degenerate_variance", count) for key in keys}
    result["mean"] = available(mean, count)
    if np.all(values == values[0]) or variance <= 0 or not math.isfinite(variance):
        return result
    error = math.sqrt(variance)
    statistic = mean / error
    if not math.isfinite(statistic):
        return result
    width = NORMAL.inv_cdf(0.975) * error
    result.update(
        standard_error=available(error, count),
        t_value=available(statistic, count),
        p_value=available(math.erfc(abs(statistic) / math.sqrt(2)), count),
        ci_lower=available(mean - width, count),
        ci_upper=available(mean + width, count),
    )
    return result


def sharpe(values: Sequence[float] | NDArray[np.float64], minimum: int) -> Statistic:
    """Daily sample Sharpe against zero, without annualization or iid claims."""
    values = _vector(values)
    if len(values) < max(2, minimum) or not np.isfinite(values).all():
        return unavailable("insufficient_complete_returns", len(values))
    deviation = float(np.std(values, ddof=1))
    if np.all(values == values[0]) or deviation == 0:
        return unavailable("constant_returns", len(values))
    return available((math.fsum(values) / len(values)) / deviation, len(values))


def fdr_by(p_values: Sequence[float]) -> list[float]:
    """Benjamini-Yekutieli adjusted p-values for every predeclared hypothesis."""
    values = _vector(p_values)
    count = len(values)
    if (
        not count
        or count > 64
        or not np.isfinite(values).all()
        or np.any((values < 0) | (values > 1))
    ):
        raise ValueError("FDR requires the complete bounded p-value family")
    order = np.argsort(values, kind="stable")
    harmonic = math.fsum(1 / rank for rank in range(1, count + 1))
    adjusted = np.empty(count)
    previous = 1.0
    for index in range(count - 1, -1, -1):
        previous = min(previous, float(values[order[index]]) * count * harmonic / (index + 1))
        adjusted[order[index]] = previous
    return [float(value) for value in adjusted]


def probabilistic_sharpe(
    ratio: float, benchmark: float, count: int, skewness: float, kurtosis: float
) -> Statistic:
    """DSR paper equation 2; kurtosis is Pearson, and SRs are unannualized."""
    if count < 4 or not all(
        math.isfinite(value) for value in (ratio, benchmark, skewness, kurtosis)
    ):
        return unavailable("insufficient_moments", count)
    variance = 1 - skewness * ratio + (kurtosis - 1) * ratio * ratio / 4
    if variance <= 0:
        return unavailable("degenerate_sharpe_variance", count)
    return available(NORMAL.cdf((ratio - benchmark) * math.sqrt((count - 1) / variance)), count)


def _matrix(values: NDArray[np.float64]) -> None:
    if (
        values.ndim != 2
        or not 2 <= values.shape[1] <= 64
        or values.shape[0] > 8192
        or values.size > 100_000
    ):
        raise ValueError("complete trial matrix exceeds bounds")
    if np.isinf(values).any() or np.any(np.abs(values[np.isfinite(values)]) > 1e12):
        raise ValueError("trial return magnitude bounds")


def deflated_sharpe(values: NDArray[np.float64], minimum: int) -> tuple[list[Statistic], Statistic]:
    """Use the declared trial count as an explicit independence assumption.

    This sensitivity calculation does not estimate the effective independent
    search count or correct return autocorrelation. Missing trials invalidate it.
    """
    _matrix(values)
    count, trials = values.shape
    ratios = [sharpe(values[:, column], minimum) for column in range(trials)]
    if any(item.value is None for item in ratios):
        missing = unavailable("incomplete_trial_sharpes", count)
        return [missing] * trials, missing
    estimates = np.array([item.value for item in ratios], dtype=np.float64)
    variance = float(np.var(estimates, ddof=1))
    if np.all(estimates == estimates[0]) or variance <= 0:
        missing = unavailable("degenerate_trial_variance", count)
        return [missing] * trials, missing
    gamma = 0.5772156649015329
    benchmark = math.sqrt(variance) * (
        (1 - gamma) * NORMAL.inv_cdf(1 - 1 / trials)
        + gamma * NORMAL.inv_cdf(1 - 1 / (trials * math.e))
    )
    results = []
    for index, ratio in enumerate(estimates):
        centered = values[:, index] - float(np.mean(values[:, index]))
        centered /= float(np.sqrt(np.mean(centered**2)))
        results.append(
            probabilistic_sharpe(
                float(ratio),
                benchmark,
                count,
                float(np.mean(centered**3)),
                float(np.mean(centered**4)),
            )
        )
    return results, available(benchmark, trials)


def cscv(
    values: NDArray[np.float64],
    *,
    blocks: int,
    minimum: int,
    check: Callable[[], None] = lambda: None,
) -> tuple[Statistic, list[dict[str, object]]]:
    """Exhaustive equal-block CSCV; no sample trimming, split deletion or refitting."""
    _matrix(values)
    count, trials = values.shape
    if type(blocks) is not int or not 4 <= blocks <= 10 or blocks % 2:
        raise ValueError("CSCV requires a bounded even block count")
    if not np.isfinite(values).all() or count < minimum or count < blocks * 2:
        return unavailable("insufficient_complete_returns", count), []
    if count % blocks:
        return unavailable("unequal_block_lengths", count), []
    slices = np.arange(count).reshape(blocks, -1)
    outputs: list[dict[str, object]] = []
    failures = 0
    for selected in combinations(range(blocks), blocks // 2):
        check()
        complement = [index for index in range(blocks) if index not in selected]
        train = values[slices[list(selected)].flatten()]
        test = values[slices[complement].flatten()]
        train_scores = [sharpe(train[:, column], 2) for column in range(trials)]
        test_scores = [sharpe(test[:, column], 2) for column in range(trials)]
        if any(item.value is None for item in (*train_scores, *test_scores)):
            return unavailable("degenerate_split", count), []
        winner = int(np.argmax(np.array([item.value for item in train_scores], dtype=np.float64)))
        ranks = average_ranks(np.array([item.value for item in test_scores], dtype=np.float64))
        relative = float(ranks[winner]) / (trials + 1)
        logit = math.log(relative / (1 - relative))
        failures += logit <= 0
        outputs.append(
            {
                "train_blocks": list(selected),
                "winner_index": winner,
                "relative_rank": relative,
                "logit": logit,
            }
        )
    return available(failures / len(outputs), len(outputs)), outputs
