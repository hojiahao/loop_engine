import math

import numpy as np
import pytest
from hypothesis import given, settings
from hypothesis import strategies as st
from scipy.stats import pearsonr, skew

from loop_research.numerics import (
    adjusted_skew,
    aligned_nav_correlation,
    nav_to_returns,
    rolling_skew,
)


# Scenario: partial window uses valid count.
def test_window_valid() -> None:
    values = [2.0, np.nan, 4.0, 5.0, 6.0]
    expected = float(skew(values, bias=False, nan_policy="omit"))
    assert adjusted_skew(values) == pytest.approx(expected, abs=1e-14)
    assert expected == pytest.approx(-0.7528371991317256)
    result = rolling_skew(values, window=5, min_observations=4)
    assert np.isnan(result[:-1]).all()
    assert result[-1] == pytest.approx(expected, abs=1e-14)


@pytest.mark.parametrize("values", [[], [np.nan] * 5, [1, 2], [1, 1, 1], [3, np.nan, 3, 3]])
# Scenario: undefined skew stays missing.
def test_undefined_skew(values: list[float]) -> None:
    assert math.isnan(adjusted_skew(values))


@pytest.mark.parametrize("scale", [1.0, 1e-200, 1e200, -1e200])
# Scenario: skew avoids raw moment overflow.
def test_skew_avoids(scale: float) -> None:
    values = np.array([2.0, 4.0, 5.0, 6.0])
    expected = float(skew(values, bias=False)) * math.copysign(1, scale)
    assert adjusted_skew(values * scale) == pytest.approx(expected, abs=1e-14)


# Scenario: skew preserves large offset deviations.
def test_skew_large() -> None:
    values = np.array([2.0, 4.0, 5.0, 6.0])
    assert adjusted_skew(values + 2**45) == pytest.approx(adjusted_skew(values), abs=1e-14)


# Scenario: skew handles opposite extreme values.
def test_skew_handles() -> None:
    maximum = np.finfo(np.float64).max
    assert adjusted_skew([-maximum, 0.0, maximum]) == pytest.approx(0.0, abs=1e-14)


# Scenario: missing endpoint uses trailing observations.
def test_missing_endpoint() -> None:
    result = rolling_skew([2.0, 4.0, 5.0, np.nan], window=4, min_observations=3)
    assert result[-1] == pytest.approx(float(skew([2.0, 4.0, 5.0], bias=False)))


@pytest.mark.parametrize("window,minimum", [(2, 2), (5, 2), (5, 6), (4097, 3), (True, 3)])
# Scenario: window policy is explicit.
def test_window_policy(window: int, minimum: int) -> None:
    with pytest.raises(ValueError, match="Require"):
        rolling_skew([1, 2, 3], window=window, min_observations=minimum)


# Scenario: future values cannot change prefix.
def test_future_values() -> None:
    prefix = np.array([2.0, np.nan, 4.0, 5.0, 6.0, 7.0])
    result = rolling_skew(prefix, window=5, min_observations=3)
    extended = rolling_skew(np.append(prefix, [1e100, -1e100]), window=5, min_observations=3)
    np.testing.assert_array_equal(result, extended[: prefix.size])


@settings(max_examples=100, derandomize=True, database=None, deadline=None)
@given(st.lists(st.one_of(st.integers(-10_000, 10_000), st.none()), min_size=3, max_size=40))
# Scenario: skew matches scipy with missing values.
def test_skew_scipy(values: list[int | None]) -> None:
    samples = np.array([np.nan if value is None else value for value in values], dtype=np.float64)
    valid = samples[~np.isnan(samples)]
    if valid.size < 3 or np.min(valid) == np.max(valid):
        assert math.isnan(adjusted_skew(samples))
    else:
        assert adjusted_skew(samples) == pytest.approx(float(skew(valid, bias=False)), abs=1e-12)


# Scenario: nav returns use previous nav.
def test_nav_previous() -> None:
    np.testing.assert_allclose(nav_to_returns([100, 110, 99, 118.8]), [0.1, -0.1, 0.2])


# Scenario: terminal zero nav is total loss.
def test_terminal_zero() -> None:
    np.testing.assert_array_equal(nav_to_returns([10.0, 0.0]), [-1.0])


@pytest.mark.parametrize("values", [[np.nan], [np.inf], [-1], [0, 1], [1, 0, 1], [1, -1]])
# Scenario: invalid nav is rejected.
def test_invalid_nav(values: list[float]) -> None:
    with pytest.raises(ValueError):
        nav_to_returns(values)


# Scenario: unrepresentable return is rejected.
def test_unrepresentable() -> None:
    with pytest.raises(ValueError, match="represented"):
        nav_to_returns([1e-300, 1e300])


@pytest.mark.parametrize("values", [[], [1.0]])
# Scenario: no return is invented.
def test_invented(values: list[float]) -> None:
    assert nav_to_returns(values).size == 0


# Scenario: correlation uses returns not nav deltas.
def test_correlation_nav() -> None:
    left = np.array([1.0, 1.1, 1.21, 1.18, 1.28, 1.24, 1.39, 1.37])
    right = np.array([2.0, 1.95, 2.02, 2.2, 2.08, 2.3, 2.25, 2.31])
    expected = float(pearsonr(left[1:] / left[:-1] - 1, right[1:] / right[:-1] - 1).statistic)
    observed = aligned_nav_correlation(left, right)
    assert observed == pytest.approx(expected, abs=1e-14)
    assert abs(observed - float(np.corrcoef(np.diff(left), np.diff(right))[0, 1])) > 1e-3


# Scenario: correlation rejects implicit tail alignment.
def test_correlation_implicit() -> None:
    with pytest.raises(ValueError, match="session index"):
        aligned_nav_correlation([1, 2, 3], [1, 2, 3, 4])
    with pytest.raises(ValueError, match="session index"):
        aligned_nav_correlation([], [1])


# Scenario: unavailable correlation is not zero.
def test_unavailable_correlation() -> None:
    assert math.isnan(aligned_nav_correlation([1, 2], [1, 3]))
    assert math.isnan(aligned_nav_correlation([1, 1, 1], [1, 2, 3], min_observations=2))


@pytest.mark.parametrize("minimum", [0, 1, True])
# Scenario: correlation requires observation policy.
def test_correlation_observation(minimum: int) -> None:
    with pytest.raises(ValueError, match="Require"):
        aligned_nav_correlation([1, 2, 3], [1, 2, 3], min_observations=minimum)


@pytest.mark.parametrize("values", [[True, False], ["1", "2"], [[1, 2]], np.array([1 + 2j])])
# Scenario: non real series are rejected.
def test_series(values: object) -> None:
    with pytest.raises(ValueError, match="real numeric"):
        adjusted_skew(values)


# Scenario: masks cannot silently lose missingness.
def test_masks_lose() -> None:
    with pytest.raises(ValueError, match="masked"):
        adjusted_skew(np.ma.array([1, 2, 3], mask=[False, True, False]))


# Scenario: inputs are not mutated.
def test_inputs_mutated() -> None:
    values = np.array([2.0, np.nan, 4.0, 5.0, 6.0])
    original = values.copy()
    adjusted_skew(values)
    rolling_skew(values, window=5, min_observations=3)
    np.testing.assert_array_equal(values, original)


@settings(max_examples=75, derandomize=True, database=None, deadline=None)
@given(st.lists(st.integers(1, 100_000), min_size=2, max_size=40), st.integers(-10, 10))
# Scenario: nav returns are scale invariant.
def test_nav_scale(values: list[int], exponent: int) -> None:
    samples = np.asarray(values, dtype=np.float64)
    np.testing.assert_allclose(nav_to_returns(samples * 2.0**exponent), nav_to_returns(samples))


@settings(max_examples=75, derandomize=True, database=None, deadline=None)
@given(
    st.lists(
        st.tuples(st.integers(100, 100_000), st.integers(100, 100_000)),
        min_size=6,
        max_size=40,
    )
)
# Scenario: nav correlation matches scipy.
def test_nav_correlation(values: list[tuple[int, int]]) -> None:
    paths = np.asarray(values, dtype=np.float64)
    left = paths[:, 0]
    right = paths[:, 1]
    left_returns = left[1:] / left[:-1] - 1
    right_returns = right[1:] / right[:-1] - 1
    observed = aligned_nav_correlation(left, right)
    if np.ptp(left_returns) == 0 or np.ptp(right_returns) == 0:
        assert math.isnan(observed)
    else:
        expected = float(pearsonr(left_returns, right_returns).statistic)
        assert observed == pytest.approx(expected, abs=1e-12)
