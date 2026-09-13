from dataclasses import replace
from datetime import date, timedelta

import numpy as np
import pytest
from hypothesis import given, settings
from hypothesis import strategies as st
from loop_protocol.canonical import (
    AstNode,
    CallNode,
    CanonicalFactorSpec,
    CanonicalizationError,
    DecimalNode,
    FactorDirection,
    FactorSpec,
    FieldNode,
    PolicyReference,
    bind_factor_spec,
    canonicalize_expression,
)
from numpy.testing import assert_allclose, assert_array_equal
from numpy.typing import NDArray
from scipy import stats

from loop_research.evaluator import Panel, evaluate
from loop_research.operators import VERSION, operator_registry, semantic_contracts

CLOSE = FieldNode("market.close")
VOLUME = FieldNode("market.volume")


def test_installed_registry_matches_the_rust_contract() -> None:
    assert operator_registry().sha256 == (
        "sha256:1e61b2328c791e46a58bf61232307c14a7100973d4540a7061f87a6df7480c34"
    )


def factor(node: AstNode, *, lower: bool = False) -> CanonicalFactorSpec:
    registry = operator_registry()
    expression = canonicalize_expression(node, registry)
    policy = PolicyReference("us.synthetic", "1", "sha256:" + "a" * 64)
    spec = FactorSpec(
        expression.expression_id,
        registry.sha256,
        FactorDirection.LOWER_IS_BETTER if lower else FactorDirection.HIGHER_IS_BETTER,
        policy,
        policy,
        policy,
        policy,
        policy,
        policy,
        policy,
        policy,
        policy,
    )
    return bind_factor_spec(spec, expression.canonical_bytes, registry)


def rolling(name: str, width: int = 5, minimum: int = 3) -> CallNode:
    return CallNode(name, VERSION, (CLOSE, DecimalNode(str(width)), DecimalNode(str(minimum))))


def panel(values: list[list[float]], *, eligible: NDArray[np.bool_] | None = None) -> Panel:
    array = np.asarray(values, dtype=np.float64)
    return Panel(
        tuple(date(2010, 1, 4) + timedelta(days=index) for index in range(len(array))),
        tuple(f"security.{index:04}" for index in range(array.shape[1])),
        {"market.close": array, "market.volume": np.full_like(array, 2.0)},
        np.ones(array.shape, dtype=np.bool_) if eligible is None else eligible,
    )


def output(node: AstNode, data: Panel) -> NDArray[np.float64]:
    return evaluate(factor(node), data, evaluation_start=data.sessions[0]).values


def test_partial_window_skew_matches_scipy() -> None:
    data = panel([[2], [np.nan], [4], [5], [6]])
    result = output(rolling("skew"), data)
    assert result[-1, 0] == pytest.approx(stats.skew([2, 4, 5, 6], bias=False), abs=1e-12)
    assert result[-1, 0] == pytest.approx(-0.7528371991317256)


def test_constant_skew_is_missing_coverage() -> None:
    data = panel([[3], [3], [3], [3], [3]])
    result = evaluate(factor(rolling("skew")), data, evaluation_start=data.sessions[2])
    assert np.isnan(result.values).all()
    assert result.eligible_observations == 3
    assert result.valid_observations == 0


@pytest.mark.parametrize("name", ["ma", "std", "min", "max"])
def test_missing_target_preserves_valid_rolling_history(name: str) -> None:
    data = panel([[2], [4], [6], [np.nan]])
    result = output(rolling(name, 4, 3), data)
    expected = {"ma": 4, "std": 2, "min": 2, "max": 6}
    assert result[-1, 0] == pytest.approx(expected[name])


@pytest.mark.parametrize("minimum", [1, 2])
def test_skew_requires_three_valid_observations(minimum: int) -> None:
    data = panel([[1], [2], [3]])
    with pytest.raises(ValueError, match="width/minimum"):
        output(rolling("skew", 3, minimum), data)


def test_minimum_cannot_exceed_width() -> None:
    with pytest.raises(ValueError, match="width/minimum"):
        output(rolling("ma", 3, 4), panel([[1], [2], [3]]))


def test_rank_ts_preserves_missing_target() -> None:
    assert np.isnan(output(rolling("rank_ts", 4, 3), panel([[2], [4], [6], [np.nan]]))[-1, 0])


def test_rank_ts_uses_stable_last_ties() -> None:
    assert output(rolling("rank_ts", 4, 3), panel([[2], [4], [4], [4]]))[-1, 0] == 1


def test_rank_ts_constant_is_midpoint() -> None:
    assert output(rolling("rank_ts", 3, 3), panel([[4], [4], [4]]))[-1, 0] == 0.5


def test_rank_cs_uses_average_valid_ranks() -> None:
    data = panel([[3, 1, 1, np.nan]])
    assert_allclose(output(CallNode("rank_cs", VERSION, (CLOSE,)), data), [[1, 0.5, 0.5, np.nan]])


def test_zscore_uses_sample_deviation_and_missingness() -> None:
    data = panel([[1, 2, 3, np.nan], [3, 3, 3, np.nan]])
    assert_allclose(
        output(CallNode("zscore", VERSION, (CLOSE,)), data),
        [[-1, 0, 1, np.nan], [np.nan, np.nan, np.nan, np.nan]],
    )


@pytest.mark.parametrize("name", ["roc", "delta"])
def test_lag_has_no_fill_or_future_data(name: str) -> None:
    data = panel([[2], [np.nan], [6], [8]])
    expected = [[np.nan], [np.nan], [2 if name == "roc" else 4], [np.nan]]
    assert_allclose(output(CallNode(name, VERSION, (CLOSE, DecimalNode("2"))), data), expected)


@pytest.mark.parametrize("name, expected", [("add", 5), ("sub", 1), ("mul", 6), ("div", 1.5)])
def test_binary_operator(name: str, expected: float) -> None:
    assert_allclose(output(CallNode(name, VERSION, (CLOSE, VOLUME)), panel([[3]])), [[expected]])


def test_division_by_zero_is_missing() -> None:
    data = panel([[0], [2]])
    assert_allclose(output(CallNode("div", VERSION, (VOLUME, CLOSE)), data), [[np.nan], [1]])


def test_overflow_does_not_poison_other_cells() -> None:
    data = panel([[np.finfo(np.float64).max, 2]])
    assert_allclose(output(CallNode("mul", VERSION, (CLOSE, VOLUME)), data), [[np.nan, 4]])


def test_eligibility_masks_values_and_coverage() -> None:
    data = panel([[1, 1000], [2, 10]], eligible=np.array([[True, False], [True, True]]))
    result = evaluate(factor(rolling("ma", 2, 2)), data, evaluation_start=data.sessions[1])
    assert_allclose(result.values, [[1.5, np.nan]])
    assert (result.eligible_observations, result.valid_observations) == (2, 1)


def test_warmup_is_excluded_from_result_counts() -> None:
    data = panel([[1], [2], [3]])
    result = evaluate(factor(rolling("ma", 3, 3)), data, evaluation_start=data.sessions[2])
    assert_allclose(result.values, [[2]])
    assert (result.eligible_observations, result.valid_observations) == (1, 1)


def test_direction_is_not_reselected_or_applied_to_raw_values() -> None:
    data = panel([[1], [2]])
    upper = evaluate(factor(CLOSE), data, evaluation_start=data.sessions[0])
    lower = evaluate(factor(CLOSE, lower=True), data, evaluation_start=data.sessions[0])
    assert upper.factor_spec_id != lower.factor_spec_id
    assert_array_equal(upper.values, lower.values)


def test_panel_copies_inputs_and_has_immutable_arrays() -> None:
    data = panel([[1], [2]])
    with pytest.raises(ValueError):
        data.fields["market.close"].setflags(write=True)
    with pytest.raises(ValueError):
        data.eligible.setflags(write=True)


@pytest.mark.parametrize("securities", [("same", "same"), ("z", "a"), ("a", "../b")])
def test_security_axes_cannot_be_silently_joined(securities: tuple[str, ...]) -> None:
    with pytest.raises(ValueError, match="axes"):
        replace(panel([[1, 2]]), securities=securities)


def test_duplicate_sessions_are_rejected() -> None:
    data = panel([[1], [2]])
    with pytest.raises(ValueError, match="axes"):
        replace(data, sessions=(data.sessions[0], data.sessions[0]))


def test_shifted_field_shape_is_rejected() -> None:
    with pytest.raises(ValueError, match="same axes"):
        replace(panel([[1], [2]]), fields={"market.close": np.array([[1.0]])})


def test_infinite_input_is_not_silently_cleaned() -> None:
    with pytest.raises(ValueError, match="finite-or-NaN"):
        panel([[np.inf]])


def test_absent_field_fails_closed() -> None:
    with pytest.raises(ValueError, match="absent"):
        output(FieldNode("market.open"), panel([[1]]))


def test_old_semantics_are_not_implicitly_executable() -> None:
    with pytest.raises(CanonicalizationError, match="unknown operator"):
        factor(CallNode("skew", "1", (CLOSE, DecimalNode("5"))))


def test_floating_point_parentheses_remain_distinct() -> None:
    left = CallNode("add", VERSION, (CallNode("add", VERSION, (CLOSE, VOLUME)), CLOSE))
    right = CallNode("add", VERSION, (CLOSE, CallNode("add", VERSION, (VOLUME, CLOSE))))
    # These differ only by commuting the outer operands and are equivalent.
    assert factor(left).expression.expression_id == factor(right).expression.expression_id
    other = CallNode("add", VERSION, (CallNode("add", VERSION, (CLOSE, CLOSE)), VOLUME))
    assert factor(left).expression.expression_id != factor(other).expression.expression_id


def test_operator_contracts_are_closed_and_versioned() -> None:
    registry = operator_registry()
    contracts = semantic_contracts()
    assert len(contracts) == 14
    for contract in contracts:
        assert registry.require_semantic_contract(contract.operator, VERSION) == contract
        assert not registry.require_operator(contract.operator, VERSION).associative


@given(st.lists(st.integers(-1000, 1000), min_size=5, max_size=20))
@settings(max_examples=30, deadline=None, derandomize=True)
def test_future_changes_cannot_change_earlier_values(values: list[int]) -> None:
    data = panel([[float(value)] for value in values])
    changed = panel([[float(value)] for value in values[:-1]] + [[1e20]])
    node = CallNode("rank_cs", VERSION, (rolling("skew"),))
    assert_allclose(output(node, data)[:-1], output(node, changed)[:-1], rtol=0, atol=0)


@given(st.lists(st.one_of(st.none(), st.integers(-10000, 10000)), min_size=3, max_size=25))
@settings(max_examples=40, deadline=None, derandomize=True)
def test_missing_skew_matches_independent_reference(values: list[int | None]) -> None:
    data = panel([[float(value) if value is not None else np.nan] for value in values])
    valid = np.array([value for value in values if value is not None], dtype=np.float64)
    result = output(rolling("skew", len(values), 3), data)[-1, 0]
    if len(valid) < 3 or np.ptp(valid) == 0:
        assert np.isnan(result)
    else:
        assert result == pytest.approx(stats.skew(valid, bias=False), abs=1e-11)
