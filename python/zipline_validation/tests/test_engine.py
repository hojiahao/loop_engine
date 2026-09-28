import copy
import csv
import io
import json
from datetime import UTC, datetime
from fractions import Fraction as F

import pytest

from loop_zipline.accounting import quote, rounded
from loop_zipline.artifacts import encode
from loop_zipline.engine import calculate
from loop_zipline.models import Artifacts, Inputs, Policy, Reference, Row, Tape
from loop_zipline.native import Unavailable


def instant(day, hour=14, minute=30):
    return int(datetime(2010, 1, day, hour, minute, tzinfo=UTC).timestamp() * 1000)


def terms(**changes):
    return {
        "tradable": True,
        "short_allowed": True,
        "borrow_limit": 10000,
        "borrow_rate_bps": 0,
        "recalled": False,
        "sec_fee_usd_per_million": "0",
        "taf_fee_usd_per_share": "0",
        "taf_fee_cap_usd": "0",
        **changes,
    }


def inputs(market=False, **changes):
    ref = Reference(sha256="sha256:" + "a" * 64, byte_size=1)
    policy = {
        "initial_cash_usd": "1000",
        "holdings": 1,
        "lot_size": 1,
        "commission_per_share_usd": "0",
        "minimum_commission_usd": "0",
        "half_spread_bps": 0,
        **(
            {
                "long_weight_bps": 5000,
                "short_weight_bps": 5000,
                "initial_margin_bps": 5000,
                "maintenance_margin_bps": 2500,
                "day_count": 360,
            }
            if market
            else {}
        ),
        **changes,
    }
    return Inputs(
        schema="loop.zipline-input/v1",
        profile="zipline-accounting.1",
        primary_backtest=ref,
        observations=ref,
        engine="pit-actions-long-short.1" if market else "long-only-next-open.1",
        source_code_sha256=ref.sha256,
        environment_sha256=ref.sha256,
        direction="higher_is_better",
        policy=Policy(**policy),
        primary_artifacts=Artifacts(**dict.fromkeys(Artifacts.model_fields, ref)),
        production_eligible=False,
    )


def tape(market=False, days=(4, 5, 6)):
    return {
        "schema": "loop.zipline-observations/v1",
        "sessions": [
            {
                "day": f"2010-01-{day:02d}",
                "decision_ms": instant(day, 21, 0),
                "scheduled_open_ms": instant(day) if market else instant(day, 21, 0),
                "actions": [],
                "rows": [
                    {
                        "security_id": identity,
                        "eligible": True,
                        "factor": float(2 - index),
                        "open_ms": instant(day),
                        "opening": "10",
                        "close_ms": instant(day, 21, 0),
                        "closing": "10",
                        "auction_volume": 1000 if market else None,
                        "opening_terms": terms() if market else None,
                        "closing_terms": terms() if market else None,
                    }
                    for index, identity in enumerate(("US.A", "US.B"))
                ],
            }
            for day in days
        ],
    }


def replay(document, recipe=None):
    return calculate(recipe or inputs(), encode(document), lambda: None)


def rows(content):
    return list(csv.DictReader(io.StringIO(content.decode("ascii"))))


def action(kind, *, security="US.A", day=6, **changes):
    return {
        "event_id": f"event.{security}.{day}",
        "security_id": security,
        "kind": kind,
        "at_ms": instant(day, 13, 0),
        "pay_ms": instant(7, 13, 0),
        **changes,
    }


def test_long_nav():
    data = tape()
    data["sessions"][1]["rows"][0]["closing"] = "11"
    data["sessions"][2]["rows"][0].update(opening="12", closing="12")
    artifacts, _ = replay(data)
    assert [float(row["nav_usd"]) for row in rows(artifacts["nav"])] == [1000, 1100, 1200]
    assert [int(row["shares"]) for row in rows(artifacts["positions"])] == [0, 0, 100, 0, 100, 0]
    fills = rows(artifacts["fills"])
    assert len(fills) == 1 and fills[0]["shares"] == "100" and fills[0]["price_usd"] == "10"
    returns = rows(artifacts["returns"])
    assert returns[0]["simple_return"] == ""
    assert float(returns[1]["simple_return"]) == pytest.approx(0.1)
    assert float(returns[2]["simple_return"]) == pytest.approx(1 / 11)


def test_next_open():
    data = tape()
    data["sessions"][1]["rows"][0]["opening"] = "20"
    artifacts, _ = replay(data)
    assert rows(artifacts["targets"])[0]["shares"] == "100"
    fill = rows(artifacts["fills"])[0]
    assert fill["session"] == "2010-01-05" and fill["shares"] == "50"
    assert rows(artifacts["orders"])[0]["status"] == "partial_cash"


def test_early_open():
    data = tape()
    data["sessions"][1]["rows"][1]["factor"] = 3.0
    data["sessions"][2]["rows"][0]["open_ms"] = instant(6, 14, 31)
    artifacts, _ = replay(data)
    orders = rows(artifacts["orders"])
    assert orders[1]["security_id"] == "US.B" and orders[1]["status"] == "cash"
    assert orders[2]["security_id"] == "US.A" and orders[2]["filled_shares"] == "100"


def test_dividend_claim():
    data = tape(True, (4, 5, 6, 7))
    data["sessions"][2]["actions"] = [action("dividend", amount_per_share_usd="1")]
    for session in data["sessions"][2:]:
        session["rows"][0].update(opening="9", closing="9")
    artifacts, bridge = replay(data, inputs(True))
    nav = rows(artifacts["nav"])
    assert float(nav[2]["receivable_usd"]) == 50
    assert float(nav[2]["nav_usd"]) == 1000
    assert float(nav[3]["receivable_usd"]) == 0
    records = json.loads(bridge)["sessions"]
    assert float(records[2]["zipline_nav_usd"]) == 950
    assert float(records[2]["economic_nav_usd"]) == 1000
    payments = [row for row in rows(artifacts["costs"]) if row["kind"] == "payment"]
    assert payments[0]["session"] == "2010-01-07" and payments[0]["cash_delta_usd"] == "50"


def test_fractional_split():
    data = tape(True, (4, 5, 6, 7))
    data["sessions"][2]["actions"] = [
        action("split", numerator=1, denominator=3, fraction_price_usd="30")
    ]
    for session in data["sessions"][2:]:
        session["rows"][0].update(opening="30", closing="30")
    artifacts, _ = replay(data, inputs(True, long_weight_bps=10000, short_weight_bps=0))
    nav = rows(artifacts["nav"])
    assert [float(row["nav_usd"]) for row in nav] == [1000] * 4
    assert float(nav[2]["cash_usd"]) == 0 and nav[2]["receivable_usd"] == "10"
    assert float(nav[3]["cash_usd"]) == 10 and nav[3]["receivable_usd"] == "0"
    assert rows(artifacts["positions"])[4]["shares"] == "33"


def test_short_split():
    data = tape(True)
    data["sessions"][2]["actions"] = [
        action("split", security="US.B", numerator=2, denominator=3, fraction_price_usd="15")
    ]
    data["sessions"][2]["rows"][1].update(opening="15", closing="15")
    artifacts, _ = replay(data, inputs(True))
    assert rows(artifacts["positions"])[5]["shares"] == "-33"
    final = rows(artifacts["nav"])[-1]
    assert final["receivable_usd"] == "-5" and float(final["nav_usd"]) == 1000


def test_delisting_claim():
    data = tape(True, (4, 5, 6, 7))
    data["sessions"][2]["actions"] = [action("delisting", amount_per_share_usd="8")]
    for session in data["sessions"][2:]:
        session["rows"][0].update(
            eligible=False,
            opening=None,
            open_ms=None,
            closing=None,
            close_ms=None,
            auction_volume=0,
        )
    artifacts, _ = replay(data, inputs(True))
    nav = rows(artifacts["nav"])
    assert nav[2]["receivable_usd"] == "400" and float(nav[2]["nav_usd"]) == 900
    assert rows(artifacts["positions"])[4]["shares"] == "0"
    assert float(nav[3]["nav_usd"]) == 900


def test_capacity_impact():
    data = tape(True, (4, 5))
    data["sessions"][1]["rows"][0]["auction_volume"] = 10
    artifacts, _ = replay(
        data,
        inputs(
            True,
            long_weight_bps=10000,
            short_weight_bps=0,
            participation_bps=5000,
            impact_bps=100,
            minimum_commission_usd="1",
        ),
    )
    fill = rows(artifacts["fills"])[0]
    assert fill["shares"] == "5" and float(fill["price_usd"]) == 10.05
    assert rows(artifacts["orders"])[0]["status"] == "partial_capacity"
    assert float(rows(artifacts["nav"])[-1]["nav_usd"]) == pytest.approx(998.75)


def test_margin_limit():
    data = tape(True, (4, 5))
    data["sessions"][1]["rows"][0].update(opening="20", closing="20")
    artifacts, _ = replay(
        data,
        inputs(
            True,
            long_weight_bps=10000,
            short_weight_bps=0,
            initial_margin_bps=10000,
            minimum_commission_usd="1",
        ),
    )
    assert rows(artifacts["fills"])[0]["shares"] == "49"
    assert rows(artifacts["orders"])[0]["status"] == "partial_margin"
    assert float(rows(artifacts["nav"])[-1]["cash_usd"]) == 19


def test_dated_fees():
    data = tape(True, (4, 5))
    data["sessions"][1]["rows"][1]["opening_terms"] = terms(
        sec_fee_usd_per_million="24", taf_fee_usd_per_share="0.000166", taf_fee_cap_usd="0.01"
    )
    artifacts, _ = replay(data, inputs(True, long_weight_bps=0, short_weight_bps=10000))
    cost = rows(artifacts["costs"])[0]
    assert cost["sec_fee_usd"] == "0.03" and cost["taf_fee_usd"] == "0.01"
    assert float(rows(artifacts["nav"])[-1]["nav_usd"]) == pytest.approx(999.96)


def test_borrow_recall():
    data = tape(True)
    for name in ("opening_terms", "closing_terms"):
        data["sessions"][2]["rows"][1][name] = terms(recalled=True, borrow_limit=0)
    artifacts, _ = replay(data, inputs(True))
    order = rows(artifacts["orders"])[-1]
    assert order["reason"] == "borrow_recall" and order["filled_shares"] == "50"
    assert rows(artifacts["positions"])[-1]["shares"] == "0"


def test_weekend_finance():
    data = tape(True, (7, 8, 11))
    for row in data["sessions"][1]["rows"]:
        row["closing_terms"]["borrow_rate_bps"] = 3600
    artifacts, _ = replay(data, inputs(True, short_collateral_bps=10200, cash_credit_bps=3600))
    borrow = [row for row in rows(artifacts["costs"]) if row["kind"] == "borrow_fee"]
    assert borrow[0]["cash_delta_usd"] == "-1.65"  # 50 shares * ceil(10.20) * 3 * 36% / 360.


@pytest.mark.parametrize(
    "problem", ["future", "missing_mark", "recall_liquidity", "unknown_action", "holdout"]
)
def test_invalid_tape(problem):
    data = tape(True)
    if problem == "future":
        data["sessions"][1]["rows"][0]["close_ms"] = instant(6, 21, 0)
    elif problem == "missing_mark":
        data["sessions"][2]["rows"][0].update(closing=None, close_ms=None)
    elif problem == "recall_liquidity":
        row = data["sessions"][2]["rows"][1]
        row["opening_terms"]["recalled"] = True
        row["auction_volume"] = 0
    elif problem == "unknown_action":
        data["sessions"][2]["actions"] = [action("merger")]
    else:
        data["sessions"][0]["day"] = "2025-01-02"
    with pytest.raises(ValueError):
        replay(data, inputs(True))


def test_prefix_causality():
    original = tape(True, (4, 5, 6, 7))
    changed = copy.deepcopy(original)
    changed["sessions"][-1]["rows"][0].update(opening="12", closing="12", factor=50.0)
    left, _ = replay(original, inputs(True))
    right, _ = replay(changed, inputs(True))
    assert rows(left["nav"])[:-1] == rows(right["nav"])[:-1]
    assert rows(left["targets"]) == rows(right["targets"])


def test_precision_denial():
    with pytest.raises(Unavailable, match="precision"):
        replay(tape(), inputs(initial_cash_usd="10000000000"))


def test_rounding_goldens():
    assert rounded(F(1, 6), 100, "up") == F(17, 100)
    assert rounded(F(-1, 6), 100, "down") == F(-17, 100)
    assert rounded(F(25, 1000), 100) == F(2, 100)
    assert rounded(F(35, 1000), 100) == F(4, 100)
    row = Row.model_validate(tape(True)["sessions"][0]["rows"][0])
    result = quote(row, 3, inputs(True, half_spread_bps=1).policy, True)
    assert result.price == F("10.001") and result.spread == F("0.003")


def test_missing_actions():
    data = tape()
    data["sessions"][2]["actions"] = [action("dividend", amount_per_share_usd="1")]
    with pytest.raises(ValueError, match="version-1"):
        replay(data)


def test_duplicate_grid():
    data = tape()
    data["sessions"][1]["rows"].pop()
    with pytest.raises(ValueError, match="grid"):
        Tape.model_validate(data)


@pytest.mark.parametrize("problem", ["dividend", "fraction", "scheduled", "gap"])
def test_action_bounds(problem):
    data = tape(True)
    if problem == "dividend":
        data["sessions"][2]["actions"] = [action("dividend", amount_per_share_usd="0")]
    elif problem == "fraction":
        data["sessions"][2]["actions"] = [
            action("split", numerator=1, denominator=3, fraction_price_usd="0")
        ]
    elif problem == "scheduled":
        data["sessions"][2]["scheduled_open_ms"] = instant(7)
    else:
        data["sessions"].pop(1)
    with pytest.raises(ValueError):
        replay(data, inputs(True))
