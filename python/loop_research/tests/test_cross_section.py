"""Independent numerical goldens for the frozen cross-sectional profile."""

from dataclasses import replace
from datetime import date

import numpy as np
import pytest
from hypothesis import given, settings
from hypothesis import strategies as st
from scipy import linalg, stats

from loop_research.cross_section import Exposures, transform
from loop_research.evaluator import Evaluation, Panel
from loop_research.transform_models import TransformPolicy


def inputs(values: list[list[float]]) -> tuple[Evaluation, Panel]:
    array = np.asarray(values, dtype=np.float64)
    panel = Panel(
        tuple(date(2010, 1, index + 4) for index in range(array.shape[0])),
        tuple(f"US.{index:03}" for index in range(array.shape[1])),
        {"market.close": array},
        np.ones(array.shape, dtype=np.bool_),
    )
    result = Evaluation(
        "factor",
        "expression",
        "registry",
        panel.sessions[0],
        array,
        array.size,
        int(np.isfinite(array).sum()),
        array.size,
    )
    return result, panel


def policy(**changes: object) -> TransformPolicy:
    return replace(TransformPolicy(0, False, 2, False, False, False), **changes)


def exposures(
    panel: Panel,
    *,
    industry: tuple[str | None, ...] | None = None,
    size: list[float] | None = None,
    beta: list[float] | None = None,
) -> Exposures:
    columns = len(panel.securities)
    return Exposures(
        panel.sessions,
        panel.securities,
        (industry or (None,) * columns,) * len(panel.sessions),
        np.tile(size if size is not None else [np.nan] * columns, (len(panel.sessions), 1)),
        np.tile(beta if beta is not None else [np.nan] * columns, (len(panel.sessions), 1)),
    )


def test_linear_quantile_clipping_matches_hand_golden() -> None:
    result, panel = inputs([[0, 1, 2, 3, 100]])
    actual = transform(result, panel, policy(winsor_tail_bps=2500), None)
    np.testing.assert_array_equal(actual.evaluation.values, [[1, 1, 2, 3, 3]])
    np.testing.assert_array_equal(result.values, [[0, 1, 2, 3, 100]])
    assert not actual.evaluation.values.flags.writeable
    assert actual.outcomes == ("ok",)


def test_standardization_matches_scipy() -> None:
    result, panel = inputs([[1, 3, 7, np.nan, 12]])
    actual = transform(result, panel, policy(standardize=True), None)
    expected = stats.zscore(result.values, axis=1, ddof=1, nan_policy="omit")
    np.testing.assert_allclose(actual.evaluation.values, expected, atol=1e-14)
    assert actual.evaluation.eligible_observations == 5
    assert actual.raw_valid_observations == actual.evaluation.valid_observations == 4


def test_industry_neutralization_matches_group_demeaning() -> None:
    result, panel = inputs([[1, 3, 4, 10, 14, 15]])
    risk = exposures(panel, industry=("A", "A", "A", "B", "B", "B"))
    actual = transform(result, panel, policy(industry=True), risk)
    expected = np.asarray([[1, 3, 4, 10, 14, 15]], dtype=np.float64)
    expected[:, :3] -= 8 / 3
    expected[:, 3:] -= 13
    np.testing.assert_allclose(actual.evaluation.values, expected, atol=1e-13)


def test_multi_exposure_fit_matches_hand_and_independent_solver() -> None:
    log_size = np.array([-1, 1, -1, 1, -1, 1, -1, 1], dtype=np.float64)
    beta = np.array([-1, -1, 1, 1, -1, -1, 1, 1], dtype=np.float64)
    group = np.array([0, 0, 0, 0, 1, 1, 1, 1], dtype=np.float64)
    noise = np.array([1, -1, -1, 1, 1, -1, -1, 1], dtype=np.float64)
    values = 3 + 2 * log_size + 4 * beta + 7 * group + noise
    result, panel = inputs([values.tolist()])
    risk = exposures(
        panel, industry=("A",) * 4 + ("B",) * 4, size=np.exp(log_size).tolist(), beta=beta.tolist()
    )
    actual = transform(result, panel, policy(industry=True, log_size=True, beta=True), risk)
    design = np.column_stack([np.ones(8), group, log_size, beta])
    coefficients, _, _, _ = linalg.lstsq(design, values, lapack_driver="gelsy")
    np.testing.assert_allclose(actual.evaluation.values[0], noise, atol=1e-13)
    np.testing.assert_allclose(
        actual.evaluation.values[0], values - design @ coefficients, atol=1e-13
    )
    np.testing.assert_allclose(design.T @ actual.evaluation.values[0], np.zeros(4), atol=1e-13)


def test_missing_exposure_preserves_eligible_denominator() -> None:
    result, panel = inputs([[1, 2, 3, 9]])
    risk = exposures(panel, industry=("A", "A", None, "A"))
    actual = transform(result, panel, policy(industry=True), risk)
    np.testing.assert_allclose(actual.evaluation.values, [[-3, -2, np.nan, 5]], atol=1e-13)
    assert actual.evaluation.eligible_observations == actual.raw_valid_observations == 4
    assert actual.evaluation.valid_observations == 3


def test_missing_unused_exposure_does_not_remove_observations() -> None:
    result, panel = inputs([[1, 2, 3]])
    actual = transform(result, panel, policy(industry=True), exposures(panel, industry=("A",) * 3))
    assert actual.evaluation.valid_observations == 3


@pytest.mark.parametrize("standardize", [False, True])
def test_rank_deficiency_stays_missing(standardize: bool) -> None:
    result, panel = inputs([[1, 2, 4, 7]])
    risk = exposures(panel, beta=[1.0] * 4)
    actual = transform(result, panel, policy(beta=True, standardize=standardize), risk)
    assert np.isnan(actual.evaluation.values).all()
    assert actual.outcomes == ("rank_deficient",)


def test_fully_explained_factor_is_constant() -> None:
    result, panel = inputs([[3, 5, 7, 9]])
    risk = exposures(panel, beta=[1.0, 2.0, 3.0, 4.0])
    actual = transform(result, panel, policy(beta=True, standardize=True), risk)
    assert np.isnan(actual.evaluation.values).all()
    assert actual.outcomes == ("constant",)
    raw = transform(result, panel, policy(beta=True), risk)
    np.testing.assert_array_equal(raw.evaluation.values, np.zeros((1, 4)))


def test_constant_factor_is_missing_after_standardization() -> None:
    result, panel = inputs([[7, 7, 7]])
    actual = transform(result, panel, policy(standardize=True), None)
    assert actual.outcomes == ("constant",)
    assert actual.evaluation.valid_observations == 0


def test_minimum_observations_is_explicit() -> None:
    result, panel = inputs([[1, 4, np.nan]])
    actual = transform(result, panel, policy(minimum_observations=3), None)
    assert actual.outcomes == ("insufficient",)
    assert actual.evaluation.valid_observations == 0


def test_saturated_design_is_insufficient() -> None:
    result, panel = inputs([[1, 4, 9]])
    actual = transform(
        result, panel, policy(industry=True), exposures(panel, industry=("A", "B", "C"))
    )
    assert actual.outcomes == ("insufficient",)


def test_extreme_finite_values_remain_finite() -> None:
    result, panel = inputs([[-1e308, -5e307, 5e307, 1e308]])
    actual = transform(result, panel, policy(winsor_tail_bps=2500, standardize=True), None)
    expected = stats.zscore([-0.625, -0.5, 0.5, 0.625], ddof=1)
    np.testing.assert_allclose(actual.evaluation.values[0], expected, atol=1e-14)


def test_exposure_axes_must_match() -> None:
    result, panel = inputs([[1, 2, 3]])
    risk = exposures(panel, beta=[1.0, 3.0, 5.0])
    risk = replace(risk, securities=("other.0", "other.1", "other.2"))
    with pytest.raises(ValueError, match="axes differ"):
        transform(result, panel, policy(beta=True), risk)


def test_ineligible_values_cannot_enter_the_fit() -> None:
    result, panel = inputs([[1, 2, 4, 100]])
    eligible = np.array([[True, True, True, False]])
    panel = Panel(panel.sessions, panel.securities, panel.fields, eligible)
    result = replace(
        result,
        values=np.array([[1.0, 2.0, 4.0, np.nan]]),
        eligible_observations=3,
        valid_observations=3,
    )
    actual = transform(result, panel, policy(standardize=True), None)
    np.testing.assert_allclose(
        actual.evaluation.values[0, :3], stats.zscore([1, 2, 4], ddof=1), atol=1e-14
    )
    assert np.isnan(actual.evaluation.values[0, 3])


def test_solver_failure_is_an_error(monkeypatch: pytest.MonkeyPatch) -> None:
    result, panel = inputs([[1, 2, 4]])
    risk = exposures(panel, beta=[1.0, 3.0, 7.0])

    def unavailable(*args: object, **kwargs: object) -> None:
        raise np.linalg.LinAlgError("fixture")

    monkeypatch.setattr(np.linalg, "lstsq", unavailable)
    with pytest.raises(ValueError, match="solver failed"):
        transform(result, panel, policy(beta=True), risk)


def test_work_budget_is_shared_with_raw_evaluation() -> None:
    result, panel = inputs([[1, 2, 4]])
    result = replace(result, work_units=50_000_000)
    with pytest.raises(ValueError, match="work budget"):
        transform(result, panel, policy(standardize=True), None)


def test_negative_prior_work_cannot_extend_budget() -> None:
    result, panel = inputs([[1, 2, 4]])
    with pytest.raises(ValueError, match="consistent raw values"):
        transform(replace(result, work_units=-1), panel, policy(), None)


def test_design_column_budget_is_bounded() -> None:
    result, panel = inputs([[float(index) for index in range(65)]])
    risk = exposures(panel, industry=tuple(f"GROUP{index:02}" for index in range(65)))
    with pytest.raises(ValueError, match="design column budget"):
        transform(result, panel, policy(industry=True), risk)


@given(st.floats(min_value=-1e8, max_value=1e8, allow_nan=False, allow_infinity=False))
@settings(max_examples=30, deadline=None)
def test_future_cross_sections_cannot_change_prior_results(value: float) -> None:
    first, panel = inputs([[1, 3, 9], [value, value / 2, -value]])
    actual = transform(first, panel, policy(standardize=True), None)
    np.testing.assert_allclose(
        actual.evaluation.values[0], stats.zscore([1, 3, 9], ddof=1), atol=1e-14
    )
