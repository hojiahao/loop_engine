import csv
import io
from dataclasses import replace
from datetime import UTC, date, datetime
from decimal import ROUND_UP, Decimal, localcontext

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st
from loop_protocol.canonical import FactorDirection

from loop_research.backtest_models import PortfolioPolicy
from loop_research.portfolio import Ledger, Observation, Session, replay

HIGHER = FactorDirection.HIGHER_IS_BETTER


def instant(day: int, hour: int, minute: int = 0) -> int:
    return int(datetime(2010, 1, day, hour, minute, tzinfo=UTC).timestamp()) * 1000


def policy(**changes: object) -> PortfolioPolicy:
    return PortfolioPolicy.model_validate(
        {
            "initial_cash_usd": "1000",
            "holdings": 1,
            "lot_size": 1,
            "commission_per_share_usd": "0",
            "minimum_commission_usd": "0",
            "half_spread_bps": 0,
            **changes,
        }
    )


def observation(
    day: int, security: str, factor: float | None, opening: str, closing: str
) -> Observation:
    return Observation(
        security,
        True,
        factor,
        instant(day, 14, 30),
        Decimal(opening),
        instant(day, 21),
        Decimal(closing),
    )


def golden() -> tuple[Session, ...]:
    return tuple(
        Session(date(2010, 1, day), instant(day, 21, 5), rows)
        for day, rows in (
            (4, (observation(4, "US.A", 2.0, "10", "10"), observation(4, "US.B", 1.0, "20", "20"))),
            (5, (observation(5, "US.A", 1.0, "12", "15"), observation(5, "US.B", 2.0, "20", "20"))),
            (6, (observation(6, "US.A", 1.0, "14", "14"), observation(6, "US.B", 2.0, "21", "22"))),
        )
    )


def table(result: Ledger, name: str) -> list[dict[str, str]]:
    return list(csv.DictReader(io.StringIO(result.artifacts[name].decode("ascii"))))


def test_hand_accounting_with_opening_gaps() -> None:
    result = replay(golden(), policy(), HIGHER)
    assert table(result, "nav") == [
        {"session": "2010-01-04", "cash_usd": "1000", "market_value_usd": "0", "nav_usd": "1000"},
        {"session": "2010-01-05", "cash_usd": "4", "market_value_usd": "1245", "nav_usd": "1249"},
        {"session": "2010-01-06", "cash_usd": "11", "market_value_usd": "1210", "nav_usd": "1221"},
    ]
    assert [
        (row["side"], row["shares"], row["filled_shares"], row["status"])
        for row in table(result, "orders")
    ] == [
        ("buy", "100", "83", "partial_cash"),
        ("sell", "83", "83", "filled"),
        ("buy", "62", "55", "partial_cash"),
    ]
    assert result.orders == result.fills == 3
    assert result.ending_nav == Decimal("1221")


def test_returns_are_ratios_and_first_is_missing() -> None:
    rows = table(replay(golden(), policy(), HIGHER), "returns")
    assert rows[0]["simple_return"] == rows[0]["previous_nav_usd"] == ""
    assert rows[1]["simple_return"] == "0.249"
    assert Decimal(rows[2]["simple_return"]) == (Decimal(-28) / 1249).quantize(Decimal("1e-18"))


def test_next_open_never_resizes_the_decision_order() -> None:
    baseline = golden()
    changed = (
        baseline[0],
        replace(
            baseline[1],
            observations=(
                replace(baseline[1].observations[0], opening=Decimal("2")),
                baseline[1].observations[1],
            ),
        ),
        baseline[2],
    )
    old, new = replay(baseline, policy(), HIGHER), replay(changed, policy(), HIGHER)
    assert table(old, "targets")[:2] == table(new, "targets")[:2]
    assert table(new, "orders")[0]["shares"] == "100"
    assert table(new, "orders")[0]["filled_shares"] == "100"
    assert table(new, "fills")[0]["session"] == "2010-01-05"


def test_future_signals_cannot_change_prior_ledger() -> None:
    baseline = golden()
    changed = (
        *baseline[:2],
        replace(
            baseline[2],
            observations=tuple(
                replace(row, factor=-100000.0, closing=Decimal("30"))
                for row in baseline[2].observations
            ),
        ),
    )
    old, new = replay(baseline, policy(), HIGHER), replay(changed, policy(), HIGHER)
    assert old.artifacts["targets"] == new.artifacts["targets"]
    assert old.artifacts["orders"] == new.artifacts["orders"]
    assert old.artifacts["fills"] == new.artifacts["fills"]
    assert table(old, "nav")[:2] == table(new, "nav")[:2]


def test_late_sale_cannot_fund_an_earlier_open() -> None:
    baseline = golden()
    changed = (
        *baseline[:2],
        replace(
            baseline[2],
            observations=(
                replace(baseline[2].observations[0], open_at_ms=instant(6, 14, 35)),
                baseline[2].observations[1],
            ),
        ),
    )
    result = replay(changed, policy(), HIGHER)
    assert table(result, "orders")[1]["status"] == "cash"
    assert table(result, "orders")[1]["security_id"] == "US.B"
    assert result.ending_nav == Decimal("1166")


def test_equal_scores_use_stable_security_id() -> None:
    baseline = golden()
    equal = (
        replace(
            baseline[0],
            observations=tuple(replace(row, factor=1.0) for row in baseline[0].observations),
        ),
        *baseline[1:],
    )
    assert table(replay(equal, policy(), HIGHER), "orders")[0]["security_id"] == "US.A"


def test_lower_direction_is_frozen() -> None:
    result = replay(golden(), policy(), FactorDirection.LOWER_IS_BETTER)
    assert table(result, "orders")[0]["security_id"] == "US.B"


@pytest.mark.parametrize(
    ("per_share", "minimum", "spread", "quantity", "cash", "nav"),
    [
        ("0.01", "1", 10, "99", "8.01", "998.01"),
        ("1", "0", 0, "90", "10", "910"),
    ],
)
def test_commission_and_spread_goldens(
    per_share: str, minimum: str, spread: int, quantity: str, cash: str, nav: str
) -> None:
    sessions = tuple(
        Session(
            date(2010, 1, day), instant(day, 21, 5), (observation(day, "US.A", 1.0, "10", "10"),)
        )
        for day in (4, 5)
    )
    result = replay(
        sessions,
        policy(
            commission_per_share_usd=per_share,
            minimum_commission_usd=minimum,
            half_spread_bps=spread,
        ),
        HIGHER,
    )
    assert table(result, "fills")[0]["shares"] == quantity
    assert table(result, "nav")[-1]["cash_usd"] == cash
    assert result.ending_nav == Decimal(nav)
    recorded = table(result, "costs")[0]
    assert Decimal("1000") - result.ending_nav == (
        Decimal(recorded["commission_usd"]) + Decimal(recorded["spread_cost_usd"])
    )


def test_unfilled_open_expires_and_cannot_use_the_close() -> None:
    baseline = golden()
    changed = (
        baseline[0],
        replace(
            baseline[1],
            observations=(
                replace(baseline[1].observations[0], opening=None, open_at_ms=None),
                baseline[1].observations[1],
            ),
        ),
        baseline[2],
    )
    result = replay(changed, policy(), HIGHER)
    assert table(result, "orders")[0]["status"] == "missing_open"
    assert table(result, "nav")[1]["nav_usd"] == "1000"
    assert table(result, "fills")[0]["security_id"] == "US.B"


def test_missing_held_mark_fails() -> None:
    baseline = golden()
    changed = (
        baseline[0],
        replace(
            baseline[1],
            observations=(
                replace(baseline[1].observations[0], closing=None, close_known_at_ms=None),
                baseline[1].observations[1],
            ),
        ),
        baseline[2],
    )
    with pytest.raises(ValueError, match="held security"):
        replay(changed, policy(), HIGHER)


def test_missing_sizing_price_fails() -> None:
    baseline = golden()
    changed = (
        replace(
            baseline[0],
            observations=(
                replace(baseline[0].observations[0], closing=None, close_known_at_ms=None),
                baseline[0].observations[1],
            ),
        ),
        *baseline[1:],
    )
    with pytest.raises(ValueError, match="order sizing"):
        replay(changed, policy(), HIGHER)


def test_final_positions_are_marked_without_hidden_liquidation() -> None:
    result = replay(golden(), policy(), HIGHER)
    final = table(result, "positions")[-1]
    assert final["security_id"] == "US.B" and final["shares"] == "55"
    assert table(result, "targets")[-1]["decision_session"] == "2010-01-05"


def test_no_valid_signals_moves_to_cash() -> None:
    baseline = golden()
    changed = (
        baseline[0],
        replace(
            baseline[1],
            observations=tuple(replace(row, factor=None) for row in baseline[1].observations),
        ),
        baseline[2],
    )
    result = replay(changed, policy(), HIGHER)
    assert result.ending_nav == Decimal("1166")
    assert table(result, "nav")[-1]["market_value_usd"] == "0"


def test_lot_rounding_and_cash_constraints() -> None:
    result = replay(golden(), policy(lot_size=10), HIGHER)
    assert table(result, "orders")[0]["filled_shares"] == "80"
    assert all(int(row["shares"]) % 10 == 0 for row in table(result, "fills"))
    assert all(Decimal(row["cash_usd"]) >= 0 for row in table(result, "nav"))


def test_global_decimal_context_does_not_change_results() -> None:
    expected = replay(golden(), policy(), HIGHER)
    with localcontext() as context:
        context.prec, context.rounding = 3, ROUND_UP
        actual = replay(golden(), policy(), HIGHER)
    assert actual == expected


@settings(max_examples=40, deadline=None)
@given(price=st.integers(1, 1000), capital=st.integers(1, 10000), lots=st.integers(1, 100))
def test_flat_frictionless_market_conserves_wealth(price: int, capital: int, lots: int) -> None:
    sessions = tuple(
        Session(
            date(2010, 1, day),
            instant(day, 21, 5),
            (observation(day, "US.A", 1.0, str(price), str(price)),),
        )
        for day in (4, 5, 6)
    )
    result = replay(sessions, policy(initial_cash_usd=str(capital), lot_size=lots), HIGHER)
    assert result.ending_nav == capital
    assert all(Decimal(row["nav_usd"]) == capital for row in table(result, "nav"))


def test_deadline_cancellation_does_not_return_a_partial_ledger() -> None:
    def cancelled() -> None:
        raise TimeoutError("cancelled")

    with pytest.raises(TimeoutError):
        replay(golden(), policy(), HIGHER, check_budget=cancelled)
