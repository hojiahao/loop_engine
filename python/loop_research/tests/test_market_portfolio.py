import json
from dataclasses import replace
from datetime import UTC, date, datetime
from decimal import Decimal, localcontext
from typing import Any

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st
from test_portfolio import HIGHER, instant, observation, table

from loop_research.execution_inputs import MarketRow, MarketSession
from loop_research.market_models import CashAction, ExecutionTerms, MarketPolicy, Split
from loop_research.market_portfolio import replay_market
from loop_research.portfolio import Session


def stamp(day: int, hour: int = 14, minute: int = 30) -> str:
    return datetime(2010, 1, day, hour, minute, tzinfo=UTC).isoformat()


def source() -> dict[str, str]:
    return {
        "source": "fixture",
        "dataset": "execution",
        "revision": "1",
        "record_id": "invented",
        "raw_sha256": "sha256:" + "1" * 64,
        "availability": "synthetic",
    }


def terms(day: int, security: str, **changes: Any) -> ExecutionTerms:
    return ExecutionTerms.model_validate_json(
        json.dumps(
            {
                "security_id": security,
                "session": f"2010-01-{day:02d}",
                "effective_at": stamp(day),
                "known_at": stamp(day, 14, 29),
                "ingested_at": stamp(day, 22, 0),
                "source": source(),
                "valid_until": stamp(day, 23, 0),
                "tradable": True,
                "short_allowed": True,
                "borrow_limit": 1_000_000,
                "borrow_rate_bps": 0,
                "recalled": False,
                "sec_fee_usd_per_million": "0",
                "taf_fee_usd_per_share": "0",
                "taf_fee_cap_usd": "0",
                **changes,
            }
        )
    )


def market_policy(**changes: Any) -> MarketPolicy:
    return MarketPolicy.model_validate(
        {
            "initial_cash_usd": "1000",
            "holdings": 1,
            "lot_size": 1,
            "commission_per_share_usd": "0",
            "minimum_commission_usd": "0",
            "half_spread_bps": 0,
            "long_weight_bps": 5000,
            "short_weight_bps": 5000,
            "initial_margin_bps": 5000,
            "maintenance_margin_bps": 2500,
            "participation_bps": 10000,
            "impact_bps": 0,
            "short_collateral_bps": 10200,
            "cash_debit_bps": 0,
            "cash_credit_bps": 0,
            "day_count": 360,
            **changes,
        }
    )


def long_only(**changes: Any) -> MarketPolicy:
    return market_policy(
        long_weight_bps=10000, short_weight_bps=0, initial_margin_bps=10000, **changes
    )


def market(days: tuple[int, ...] = (4, 5, 6), price: str = "10") -> tuple[MarketSession, ...]:
    result = []
    for day in days:
        rows = tuple(
            MarketRow(
                observation(day, security, signal, price, price),
                1_000_000,
                terms(day, security),
                terms(day, security),
            )
            for security, signal in (("US.A", 2.0), ("US.B", 1.0))
        )
        base = Session(
            date(2010, 1, day), instant(day, 21, 5), tuple(row.observation for row in rows)
        )
        result.append(MarketSession(base, instant(day, 14, 30), rows, ()))
    return tuple(result)


def change_row(
    session: MarketSession,
    security: str,
    *,
    observation_changes: dict[str, Any] | None = None,
    **changes: Any,
) -> MarketSession:
    rows = tuple(
        replace(row, observation=replace(row.observation, **(observation_changes or {})), **changes)
        if row.observation.security_id == security
        else row
        for row in session.rows
    )
    return replace(
        session,
        rows=rows,
        base=replace(session.base, observations=tuple(row.observation for row in rows)),
    )


def cash_action(
    day: int, kind: str = "dividend", security: str = "US.A", **changes: Any
) -> CashAction:
    return CashAction.model_validate_json(
        json.dumps(
            {
                "kind": kind,
                "event_id": "action.1",
                "security_id": security,
                "session": f"2010-01-{day:02d}",
                "effective_at": stamp(day, 14, 0),
                "known_at": stamp(day - 1, 21, 0),
                "ingested_at": stamp(day, 22, 0),
                "source": source(),
                "amount_per_share_usd": "1",
                "pay_at": stamp(day + 1),
                **changes,
            }
        )
    )


def split(day: int, **changes: Any) -> Split:
    return Split.model_validate_json(
        json.dumps(
            {
                "kind": "split",
                "event_id": "action.1",
                "security_id": "US.A",
                "session": f"2010-01-{day:02d}",
                "effective_at": stamp(day, 14, 0),
                "known_at": stamp(day - 1, 21, 0),
                "ingested_at": stamp(day, 22, 0),
                "source": source(),
                "numerator": 2,
                "denominator": 1,
                **changes,
            }
        )
    )


# Scenario: split preserves wealth and adjusts holdings.
def test_split_holdings() -> None:
    before = market()
    last = change_row(
        before[2], "US.A", observation_changes={"opening": Decimal(5), "closing": Decimal(5)}
    )
    result = replay_market((*before[:2], replace(last, actions=(split(6),))), long_only(), HIGHER)
    assert result.ending_nav == 1000
    assert table(result, "positions")[-2]["shares"] == "200"
    assert result.fills == 1


# Scenario: split adjusts an unfilled target.
def test_split_target() -> None:
    before = market((4, 5))
    last = change_row(
        before[1], "US.A", observation_changes={"opening": Decimal(5), "closing": Decimal(5)}
    )
    result = replay_market((before[0], replace(last, actions=(split(5),))), long_only(), HIGHER)
    assert table(result, "orders")[0]["shares"] == "200"
    assert result.ending_nav == 1000


# Scenario: reverse split keeps cash in lieu receivable.
def test_split_receivable() -> None:
    before = market()
    last = change_row(
        before[2], "US.A", observation_changes={"opening": Decimal(20), "closing": Decimal(20)}
    )
    event = split(6, numerator=1, denominator=2, fraction_price_usd="20", pay_at=stamp(7))
    result = replay_market(
        (*before[:2], replace(last, actions=(event,))), long_only(initial_cash_usd="1010"), HIGHER
    )
    assert table(result, "positions")[-2]["shares"] == "50"
    nav = table(result, "nav")[-1]
    assert (nav["cash_usd"], nav["receivable_usd"], nav["nav_usd"]) == ("0", "10", "1010")


# Scenario: fractional split without known terms fails.
def test_fractional_terms() -> None:
    before = market()
    with pytest.raises(ValueError, match="cash-in-lieu"):
        replay_market(
            (*before[:2], replace(before[2], actions=(split(6, numerator=1, denominator=2),))),
            long_only(initial_cash_usd="1010"),
            HIGHER,
        )


@pytest.mark.parametrize("paid", [False, True])
# Scenario: dividend entitlement is not early cash.
def test_dividend_payment(paid: bool) -> None:
    before = market((4, 5, 6, 7))
    ex_day = change_row(
        before[2], "US.A", observation_changes={"opening": Decimal(9), "closing": Decimal(9)}
    )
    last = change_row(
        before[3], "US.A", observation_changes={"opening": Decimal(9), "closing": Decimal(9)}
    )
    event = cash_action(6, pay_at=stamp(7 if paid else 8))
    result = replay_market(
        (*before[:2], replace(ex_day, actions=(event,)), last), long_only(), HIGHER
    )
    navs = table(result, "nav")
    assert (navs[2]["cash_usd"], navs[2]["receivable_usd"], navs[2]["nav_usd"]) == (
        "0",
        "100",
        "1000",
    )
    assert navs[3]["nav_usd"] == "1000"
    assert table(result, "positions")[-2]["shares"] == ("111" if paid else "100")
    assert navs[3]["receivable_usd"] == ("0" if paid else "100")


@pytest.mark.parametrize("payout, nav", [("4", "400"), ("0", "100")])
# Scenario: delisting settles explicit cash and retires.
def test_delisting_settlement(payout: str, nav: str) -> None:
    before = market()
    last = change_row(
        before[2],
        "US.A",
        observation_changes={
            "eligible": False,
            "opening": None,
            "open_at_ms": None,
            "closing": None,
            "close_known_at_ms": None,
        },
    )
    capital = "1100" if payout == "0" else "1000"
    policy = long_only(initial_cash_usd=capital, lot_size=100)
    event = cash_action(6, "delisting", amount_per_share_usd=payout, pay_at=stamp(6))
    result = replay_market((*before[:2], replace(last, actions=(event,))), policy, HIGHER)
    assert result.ending_nav == Decimal(nav)
    assert table(result, "positions")[-2]["shares"] == "0"
    assert table(result, "nav")[-1]["cash_usd"] == nav


# Scenario: short borrow uses prior close collateral.
def test_borrow_collateral() -> None:
    before = market()
    second = change_row(before[1], "US.B", closing_terms=terms(5, "US.B", borrow_rate_bps=3600))
    result = replay_market((before[0], second, before[2]), market_policy(), HIGHER)
    # 50 borrowed shares * ceil(10 * 102%) * 36% / 360 = 0.55 USD.
    assert result.ending_nav == Decimal("999.45")
    assert table(result, "nav")[1]["short_collateral_usd"] == "550"
    assert [
        row["cash_delta_usd"] for row in table(result, "costs") if row["kind"] == "borrow_fee"
    ] == ["-0.55"]


# Scenario: weekends accrue actual calendar days.
def test_weekend_accrual() -> None:
    before = market((7, 8, 11))
    friday = change_row(before[1], "US.B", closing_terms=terms(8, "US.B", borrow_rate_bps=3600))
    assert replay_market(
        (before[0], friday, before[2]), market_policy(), HIGHER
    ).ending_nav == Decimal("998.35")


# Scenario: short dividend is a liability.
def test_dividend_liability() -> None:
    before = market()
    last = change_row(
        before[2], "US.B", observation_changes={"opening": Decimal(9), "closing": Decimal(9)}
    )
    result = replay_market(
        (*before[:2], replace(last, actions=(cash_action(6, security="US.B"),))),
        market_policy(),
        HIGHER,
    )
    nav = table(result, "nav")[-1]
    assert (nav["cash_usd"], nav["receivable_usd"], nav["market_value_usd"], nav["nav_usd"]) == (
        "1000",
        "-50",
        "50",
        "1000",
    )


# Scenario: borrow limit caps new shorts.
def test_borrow_limit() -> None:
    before = market((4, 5))
    last = change_row(before[1], "US.B", opening_terms=terms(5, "US.B", borrow_limit=20))
    result = replay_market((before[0], last), market_policy(), HIGHER)
    short = next(row for row in table(result, "orders") if row["security_id"] == "US.B")
    assert (short["shares"], short["filled_shares"], short["status"]) == (
        "50",
        "20",
        "partial_borrow",
    )


@pytest.mark.parametrize("tradable, volume", [(True, 1_000_000), (False, 1_000_000), (True, 0)])
# Scenario: recall requires an actual cover.
def test_recall_cover(tradable: bool, volume: int) -> None:
    before = market()
    last = change_row(
        before[2],
        "US.B",
        opening_terms=terms(6, "US.B", recalled=True, tradable=tradable),
        auction_volume=volume,
    )
    if not tradable or not volume:
        with pytest.raises(ValueError, match="recall cannot"):
            replay_market((*before[:2], last), market_policy(), HIGHER)
    else:
        result = replay_market((*before[:2], last), market_policy(), HIGHER)
        assert table(result, "orders")[-1]["reason"] == "borrow_recall"
        assert table(result, "positions")[-1]["shares"] == "0"
        assert result.ending_nav == 1000


# Scenario: capacity and impact charge once.
def test_capacity_costs() -> None:
    before = market((4, 5))
    last = before[1]
    for security in ("US.A", "US.B"):
        last = change_row(last, security, auction_volume=100)
    result = replay_market(
        (before[0], last),
        market_policy(participation_bps=1000, impact_bps=100, minimum_commission_usd="1"),
        HIGHER,
    )
    assert [row["filled_shares"] for row in table(result, "orders")] == ["10", "10"]
    assert [row["price_usd"] for row in table(result, "fills")] == ["9.99", "10.01"]
    assert result.ending_nav == Decimal("997.8")
    assert [row["impact_cost_usd"] for row in table(result, "costs")] == ["0.1", "0.1"]


# Scenario: negative cash has explicit financing cost.
def test_cash_financing() -> None:
    result = replay_market(
        market(),
        market_policy(long_weight_bps=15000, short_weight_bps=0, cash_debit_bps=7200),
        HIGHER,
    )
    assert table(result, "nav")[1]["cash_usd"] == "-500"
    assert result.ending_nav == Decimal("999")
    assert [
        row["cash_delta_usd"] for row in table(result, "costs") if row["kind"] == "cash_interest"
    ] == ["-1"]


@pytest.mark.parametrize(
    "price, opening, error", [("40", True, "insolvency"), ("25", False, "maintenance")]
)
# Scenario: short losses fail insolvency and margin.
def test_short_losses(price: str, opening: bool, error: str) -> None:
    before = market()
    changes = {"closing": Decimal(price)}
    if opening:
        changes["opening"] = Decimal(price)
    last = change_row(before[2], "US.B", observation_changes=changes)
    with pytest.raises(ValueError, match=error):
        replay_market((*before[:2], last), market_policy(), HIGHER)


# Scenario: suspended holdings require marks without synthetic fills.
def test_suspension_marks() -> None:
    before = market()
    last = change_row(before[2], "US.A", observation_changes={"factor": 0.0})
    second = change_row(before[1], "US.A", observation_changes={"factor": 0.0})
    last = change_row(last, "US.A", opening_terms=terms(6, "US.A", tradable=False))
    result = replay_market((before[0], second, last), long_only(), HIGHER)
    assert (
        next(row for row in table(result, "orders") if row["status"] == "untradable")[
            "filled_shares"
        ]
        == "0"
    )
    assert table(result, "positions")[-2]["shares"] == "100"
    missing = change_row(
        last, "US.A", observation_changes={"closing": None, "close_known_at_ms": None}
    )
    with pytest.raises(ValueError, match="visible closing mark"):
        replay_market((before[0], second, missing), long_only(), HIGHER)


# Scenario: missing borrow evidence is not free borrow.
def test_borrow_evidence() -> None:
    before = market()
    last = change_row(before[2], "US.B", opening_terms=None)
    with pytest.raises(ValueError, match="opening borrow terms"):
        replay_market((*before[:2], last), market_policy(), HIGHER)


# Scenario: sale levies use dated terms and are separate from commission.
def test_sale_levies() -> None:
    before = market((4, 5))
    last = change_row(
        before[1],
        "US.B",
        opening_terms=terms(
            5,
            "US.B",
            sec_fee_usd_per_million="20",
            taf_fee_usd_per_share="0.0002",
            taf_fee_cap_usd="1",
        ),
    )
    result = replay_market((before[0], last), market_policy(), HIGHER)
    sale = next(row for row in table(result, "costs") if row["security_id"] == "US.B")
    assert (sale["commission_usd"], sale["sec_fee_usd"], sale["taf_fee_usd"]) == (
        "0",
        "0.01",
        "0.01",
    )
    assert result.ending_nav == Decimal("999.98")


# Scenario: taf cap is applied per modeled order.
def test_taf_cap() -> None:
    before = market((4, 5))
    last = change_row(
        before[1],
        "US.B",
        opening_terms=terms(
            5,
            "US.B",
            taf_fee_usd_per_share="0.1",
            taf_fee_cap_usd="1",
        ),
    )
    result = replay_market((before[0], last), market_policy(), HIGHER)
    assert table(result, "costs")[0]["taf_fee_usd"] == "1"
    assert result.ending_nav == 999


# Scenario: taf cap cannot be rounded above its declared limit.
def test_taf_rounding() -> None:
    with pytest.raises(ValueError, match="whole cents"):
        terms(5, "US.B", taf_fee_usd_per_share="0.0002", taf_fee_cap_usd="0.001")


# Scenario: short reverse split preserves fractional liability.
def test_fractional_liability() -> None:
    before = market()
    last = change_row(
        before[2], "US.B", observation_changes={"opening": Decimal(20), "closing": Decimal(20)}
    )
    event = split(
        6, security_id="US.B", numerator=1, denominator=2, fraction_price_usd="20", pay_at=stamp(7)
    )
    result = replay_market(
        (*before[:2], replace(last, actions=(event,))),
        market_policy(initial_cash_usd="1020"),
        HIGHER,
    )
    assert table(result, "positions")[-1]["shares"] == "-25"
    assert table(result, "nav")[-1]["receivable_usd"] == "-10"
    assert result.ending_nav == 1020


# Scenario: short cash delisting pays the lender.
def test_delisting_liability() -> None:
    before = market()
    last = change_row(
        before[2],
        "US.B",
        observation_changes={
            "eligible": False,
            "opening": None,
            "open_at_ms": None,
            "closing": None,
            "close_known_at_ms": None,
        },
    )
    event = cash_action(6, "delisting", "US.B", amount_per_share_usd="4", pay_at=stamp(6))
    result = replay_market((*before[:2], replace(last, actions=(event,))), market_policy(), HIGHER)
    assert result.ending_nav == 1300
    assert table(result, "nav")[-1]["cash_usd"] == "800"
    assert table(result, "positions")[-1]["shares"] == "0"


# Scenario: unpaid receivables do not offset cash payable reserves.
def test_payable_reserves() -> None:
    before = market((4, 5, 6, 7))
    ex_day, last = before[2], before[3]
    for security in ("US.A", "US.B"):
        ex_day = change_row(
            ex_day, security, observation_changes={"opening": Decimal(9), "closing": Decimal(9)}
        )
        last = change_row(
            last, security, observation_changes={"opening": Decimal(9), "closing": Decimal(9)}
        )
    actions = (
        cash_action(6, pay_at=stamp(8)),
        cash_action(6, security="US.B", event_id="action.2", pay_at=stamp(8)),
    )
    result = replay_market(
        (*before[:2], replace(ex_day, actions=actions), last),
        market_policy(cash_credit_bps=3600),
        HIGHER,
    )
    assert table(result, "nav")[2]["receivable_usd"] == "0"
    assert result.ending_nav == Decimal("1001.902451")


# Scenario: future values cannot change prior accounting.
def test_causal_accounting() -> None:
    before = market()
    changed = change_row(
        before[2], "US.B", observation_changes={"factor": 100.0, "closing": Decimal(9)}
    )
    left, right = [
        replay_market(sessions, market_policy(), HIGHER)
        for sessions in (before, (*before[:2], changed))
    ]
    for name in ("nav", "positions", "costs", "fills"):
        assert [row for row in table(left, name) if row["session"] < "2010-01-06"] == [
            row for row in table(right, name) if row["session"] < "2010-01-06"
        ]
    assert table(left, "targets") == table(right, "targets")


@given(st.integers(min_value=1, max_value=100))
@settings(max_examples=20, deadline=None)
# Scenario: flat long short conserves nav.
def test_nav_conservation(price: int) -> None:
    before = market(price=str(price))
    result = replay_market(before, market_policy(), HIGHER)
    assert result.ending_nav == 1000
    with localcontext() as context:
        context.prec = 9
        assert replay_market(before, market_policy(), HIGHER) == result
