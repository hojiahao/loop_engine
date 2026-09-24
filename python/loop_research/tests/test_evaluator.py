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


# Scenario: installed registry matches the rust contract.
def test_installed_registry() -> None:
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


# Scenario: partial window skew matches scipy.
def test_window_skew() -> None:
    data = panel([[2], [np.nan], [4], [5], [6]])
    result = output(rolling("skew"), data)
    assert result[-1, 0] == pytest.approx(stats.skew([2, 4, 5, 6], bias=False), abs=1e-12)
    assert result[-1, 0] == pytest.approx(-0.7528371991317256)


# Scenario: constant skew is missing coverage.
def test_constant_skew() -> None:
    data = panel([[3], [3], [3], [3], [3]])
    result = evaluate(factor(rolling("skew")), data, evaluation_start=data.sessions[2])
    assert np.isnan(result.values).all()
    assert result.eligible_observations == 3
    assert result.valid_observations == 0


@pytest.mark.parametrize("name", ["ma", "std", "min", "max"])
# Scenario: missing target preserves valid rolling history.
def test_missing_target(name: str) -> None:
    data = panel([[2], [4], [6], [np.nan]])
    result = output(rolling(name, 4, 3), data)
    expected = {"ma": 4, "std": 2, "min": 2, "max": 6}
    assert result[-1, 0] == pytest.approx(expected[name])


@pytest.mark.parametrize("minimum", [1, 2])
# Scenario: skew requires three valid observations.
def test_skew_three(minimum: int) -> None:
    data = panel([[1], [2], [3]])
    with pytest.raises(ValueError, match="width/minimum"):
        output(rolling("skew", 3, minimum), data)


# Scenario: minimum cannot exceed width.
def test_minimum_exceed() -> None:
    with pytest.raises(ValueError, match="width/minimum"):
        output(rolling("ma", 3, 4), panel([[1], [2], [3]]))


# Scenario: rank ts preserves missing target.
def test_rank_missing() -> None:
    assert np.isnan(output(rolling("rank_ts", 4, 3), panel([[2], [4], [6], [np.nan]]))[-1, 0])


# Scenario: rank ts uses stable last ties.
def test_rank_stable() -> None:
    assert output(rolling("rank_ts", 4, 3), panel([[2], [4], [4], [4]]))[-1, 0] == 1


# Scenario: rank ts constant is midpoint.
def test_rank_ts() -> None:
    assert output(rolling("rank_ts", 3, 3), panel([[4], [4], [4]]))[-1, 0] == 0.5


# Scenario: rank cs uses average valid ranks.
def test_rank_cs() -> None:
    data = panel([[3, 1, 1, np.nan]])
    assert_allclose(output(CallNode("rank_cs", VERSION, (CLOSE,)), data), [[1, 0.5, 0.5, np.nan]])


# Scenario: zscore uses sample deviation and missingness.
def test_zscore_sample() -> None:
    data = panel([[1, 2, 3, np.nan], [3, 3, 3, np.nan]])
    assert_allclose(
        output(CallNode("zscore", VERSION, (CLOSE,)), data),
        [[-1, 0, 1, np.nan], [np.nan, np.nan, np.nan, np.nan]],
    )


@pytest.mark.parametrize("name", ["roc", "delta"])
# Scenario: lag has no fill or future data.
def test_lag_fill(name: str) -> None:
    data = panel([[2], [np.nan], [6], [8]])
    expected = [[np.nan], [np.nan], [2 if name == "roc" else 4], [np.nan]]
    assert_allclose(output(CallNode(name, VERSION, (CLOSE, DecimalNode("2"))), data), expected)


@pytest.mark.parametrize("name, expected", [("add", 5), ("sub", 1), ("mul", 6), ("div", 1.5)])
def test_binary_operator(name: str, expected: float) -> None:
    assert_allclose(output(CallNode(name, VERSION, (CLOSE, VOLUME)), panel([[3]])), [[expected]])


# Scenario: division by zero is missing.
def test_division_zero() -> None:
    data = panel([[0], [2]])
    assert_allclose(output(CallNode("div", VERSION, (VOLUME, CLOSE)), data), [[np.nan], [1]])


# Scenario: overflow does not poison other cells.
def test_overflow_poison() -> None:
    data = panel([[np.finfo(np.float64).max, 2]])
    assert_allclose(output(CallNode("mul", VERSION, (CLOSE, VOLUME)), data), [[np.nan, 4]])


# Scenario: eligibility masks values and coverage.
def test_eligibility_masks() -> None:
    data = panel([[1, 1000], [2, 10]], eligible=np.array([[True, False], [True, True]]))
    result = evaluate(factor(rolling("ma", 2, 2)), data, evaluation_start=data.sessions[1])
    assert_allclose(result.values, [[1.5, np.nan]])
    assert (result.eligible_observations, result.valid_observations) == (2, 1)


# Scenario: warmup is excluded from result counts.
def test_warmup_excluded() -> None:
    data = panel([[1], [2], [3]])
    result = evaluate(factor(rolling("ma", 3, 3)), data, evaluation_start=data.sessions[2])
    assert_allclose(result.values, [[2]])
    assert (result.eligible_observations, result.valid_observations) == (1, 1)


# Scenario: direction is not reselected or applied to raw values.
def test_direction_reselected() -> None:
    data = panel([[1], [2]])
    upper = evaluate(factor(CLOSE), data, evaluation_start=data.sessions[0])
    lower = evaluate(factor(CLOSE, lower=True), data, evaluation_start=data.sessions[0])
    assert upper.factor_spec_id != lower.factor_spec_id
    assert_array_equal(upper.values, lower.values)


# Scenario: panel copies inputs and has immutable arrays.
def test_panel_copies() -> None:
    data = panel([[1], [2]])
    with pytest.raises(ValueError):
        data.fields["market.close"].setflags(write=True)
    with pytest.raises(ValueError):
        data.eligible.setflags(write=True)


@pytest.mark.parametrize("securities", [("same", "same"), ("z", "a"), ("a", "../b")])
# Scenario: security axes cannot be silently joined.
def test_security_axes(securities: tuple[str, ...]) -> None:
    with pytest.raises(ValueError, match="axes"):
        replace(panel([[1, 2]]), securities=securities)


# Scenario: duplicate sessions are rejected.
def test_sessions() -> None:
    data = panel([[1], [2]])
    with pytest.raises(ValueError, match="axes"):
        replace(data, sessions=(data.sessions[0], data.sessions[0]))


# Scenario: shifted field shape is rejected.
def test_shifted_field() -> None:
    with pytest.raises(ValueError, match="same axes"):
        replace(panel([[1], [2]]), fields={"market.close": np.array([[1.0]])})


# Scenario: infinite input is not silently cleaned.
def test_infinite_input() -> None:
    with pytest.raises(ValueError, match="finite-or-NaN"):
        panel([[np.inf]])


# Scenario: absent field fails closed.
def test_absent_field() -> None:
    with pytest.raises(ValueError, match="absent"):
        output(FieldNode("market.open"), panel([[1]]))


# Scenario: old semantics are not implicitly executable.
def test_old_semantics() -> None:
    with pytest.raises(CanonicalizationError, match="unknown operator"):
        factor(CallNode("skew", "1", (CLOSE, DecimalNode("5"))))


# Scenario: floating point parentheses remain distinct.
def test_floating_point() -> None:
    left = CallNode("add", VERSION, (CallNode("add", VERSION, (CLOSE, VOLUME)), CLOSE))
    right = CallNode("add", VERSION, (CLOSE, CallNode("add", VERSION, (VOLUME, CLOSE))))
    # These differ only by commuting the outer operands and are equivalent.
    assert factor(left).expression.expression_id == factor(right).expression.expression_id
    other = CallNode("add", VERSION, (CallNode("add", VERSION, (CLOSE, CLOSE)), VOLUME))
    assert factor(left).expression.expression_id != factor(other).expression.expression_id


# Scenario: operator contracts are closed and versioned.
def test_operator_contracts() -> None:
    registry = operator_registry()
    contracts = semantic_contracts()
    assert len(contracts) == 14
    for contract in contracts:
        assert registry.require_semantic_contract(contract.operator, VERSION) == contract
        assert not registry.require_operator(contract.operator, VERSION).associative


@given(st.lists(st.integers(-1000, 1000), min_size=5, max_size=20))
@settings(max_examples=30, deadline=None, derandomize=True)
# Scenario: future changes cannot change earlier values.
def test_future_earlier(values: list[int]) -> None:
    data = panel([[float(value)] for value in values])
    changed = panel([[float(value)] for value in values[:-1]] + [[1e20]])
    node = CallNode("rank_cs", VERSION, (rolling("skew"),))
    assert_allclose(output(node, data)[:-1], output(node, changed)[:-1], rtol=0, atol=0)


@given(st.lists(st.one_of(st.none(), st.integers(-10000, 10000)), min_size=3, max_size=25))
@settings(max_examples=40, deadline=None, derandomize=True)
# Scenario: missing skew matches independent reference.
def test_missing_skew(values: list[int | None]) -> None:
    data = panel([[float(value) if value is not None else np.nan] for value in values])
    valid = np.array([value for value in values if value is not None], dtype=np.float64)
    result = output(rolling("skew", len(values), 3), data)[-1, 0]
    if len(valid) < 3 or np.ptp(valid) == 0:
        assert np.isnan(result)
    else:
        assert result == pytest.approx(stats.skew(valid, bias=False), abs=1e-11)
