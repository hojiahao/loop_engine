"""Numerical integrity primitives; market alignment and execution live elsewhere."""

import math

import numpy as np
from numpy.typing import ArrayLike, NDArray

MAX_WINDOW = 4096


def _series(values: ArrayLike) -> NDArray[np.float64]:
    if isinstance(values, np.ma.MaskedArray):
        raise ValueError("Use explicit NaN observations, not masked arrays")
    raw = np.asarray(values)
    if raw.ndim != 1 or raw.dtype.kind not in "iuf":
        raise ValueError("Expected a one-dimensional real numeric series")
    with np.errstate(over="raise", invalid="raise"):
        try:
            result = np.asarray(raw, dtype=np.float64)
        except FloatingPointError as error:
            raise ValueError("Series cannot be represented as float64") from error
    if np.isinf(result).any():
        raise ValueError("Infinite observations are invalid")
    return result


def _centered_unit(values: NDArray[np.float64]) -> NDArray[np.float64]:
    # Subtract an observed anchor before averaging to preserve small deviations
    # on large offsets. Opposite extreme finite values need scaling first.
    with np.errstate(over="ignore"):
        shifted = values - values[0]
    if not np.isfinite(shifted).all():
        scaled = values / np.max(np.abs(values))
        shifted = scaled - scaled[0]
    scale = float(np.max(np.abs(shifted)))
    if scale == 0:
        return np.zeros_like(values)
    unit = shifted / scale
    return np.asarray(unit - float(np.mean(unit)), dtype=np.float64)


def _skew_valid(values: NDArray[np.float64]) -> float:
    count = int(values.size)
    if count < 3 or np.min(values) == np.max(values):
        return math.nan
    centered = _centered_unit(values)
    second = float(np.mean(centered**2))
    third = float(np.mean(centered**3))
    correction = math.sqrt(count * (count - 1)) / (count - 2)
    return correction * third / math.pow(second, 1.5)


def adjusted_skew(values: ArrayLike) -> float:
    """Adjusted Fisher-Pearson skew using the actual non-NaN sample count.

    Fewer than three observations or an exactly constant sample gives NaN, not
    an invented neutral observation. Infinities and non-real inputs raise
    ValueError. The caller must retain this missing-result status in coverage.
    Input arrays are never mutated.
    """
    samples = _series(values)
    return _skew_valid(samples[~np.isnan(samples)])


def rolling_skew(values: ArrayLike, *, window: int, min_observations: int) -> NDArray[np.float64]:
    """Causal trailing skew, including the current observation and partial warmup.

    Both window settings are explicit, with 3 <= min_observations <= window <=
    4096. Missing observations are omitted but count toward window positions;
    a missing current observation does not erase earlier valid observations.
    This bounded-window reference kernel is O(rows * window), not yet a panel
    evaluator or a claim of production throughput.
    """
    if (
        type(window) is not int
        or type(min_observations) is not int
        or not 3 <= min_observations <= window <= MAX_WINDOW
    ):
        raise ValueError("Require 3 <= min_observations <= window <= 4096")
    samples = _series(values)
    result = np.full(samples.size, np.nan, dtype=np.float64)
    for end in range(samples.size):
        trailing = samples[max(0, end + 1 - window) : end + 1]
        valid = trailing[~np.isnan(trailing)]
        if valid.size >= min_observations:
            result[end] = _skew_valid(valid)
    return result


def nav_to_returns(nav: ArrayLike) -> NDArray[np.float64]:
    """Simple returns NAV[t] / NAV[t-1] - 1 for cash-flow-adjusted portfolio NAV.

    NAV must be finite and nonnegative, and every prior value must be positive.
    Zero at the final observation is a total loss; observations after insolvency
    require a separate accounting policy and are rejected here. No forward fill,
    implicit cash-flow adjustment, first-period zero, or delta-NAV shortcut is
    applied. Overflow raises ValueError rather than creating an infinite metric.
    """
    values = _series(nav)
    if not np.isfinite(values).all() or (values < 0).any():
        raise ValueError("NAV must be finite and nonnegative")
    if (values[:-1] <= 0).any():
        raise ValueError("Prior NAV must be positive")
    with np.errstate(over="raise", divide="raise", invalid="raise"):
        try:
            return values[1:] / values[:-1] - 1.0
        except FloatingPointError as error:
            raise ValueError("NAV return cannot be represented as float64") from error


def aligned_nav_correlation(
    left_nav: ArrayLike, right_nav: ArrayLike, *, min_observations: int = 5
) -> float:
    """Pearson correlation of returns derived from two already aligned NAV paths.

    Callers must resolve the same session index before using this primitive.
    Unequal lengths are rejected, never silently truncated or aligned by tail.
    Fewer than min_observations return pairs, or a constant return path, gives
    NaN. Invalid NAV raises ValueError. Session/PIT resolution belongs to the
    data plane; this array API does not itself prove calendar alignment.
    """
    if type(min_observations) is not int or min_observations < 2:
        raise ValueError("Require at least two return pairs")
    left = nav_to_returns(left_nav)
    right = nav_to_returns(right_nav)
    if np.asarray(left_nav).size != np.asarray(right_nav).size:
        raise ValueError("NAV paths must use the same resolved session index")
    if left.size < min_observations:
        return math.nan
    x = _centered_unit(left)
    y = _centered_unit(right)
    if not np.any(x) or not np.any(y):
        return math.nan
    return float(np.corrcoef(x, y)[0, 1])
