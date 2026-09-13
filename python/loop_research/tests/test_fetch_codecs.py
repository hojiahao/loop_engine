"""Numerical/source semantics from deliberately synthetic vendor responses."""

import json
from dataclasses import replace
from datetime import UTC, date, datetime, timedelta
from decimal import Decimal

import pytest
from data_helpers import OBSERVED, alpaca_config, download, fixture, sec_config

from loop_research.data.alpaca import bars_parameters, normalize_alpaca, page_rows
from loop_research.data.fetch_config import FetchBudget
from loop_research.data.fetch_http import FetchError
from loop_research.data.fetch_json import decimal_text, decode_object
from loop_research.data.sec import normalize_sec


def test_sec_preserves_units_periods_and_revised_exact_values() -> None:
    result = normalize_sec(sec_config(), download("sec-facts"), download("sec-submissions"))
    assert len(result.fundamentals) == 4
    values = {(fact.concept, fact.unit, fact.period_start): fact for fact in result.fundamentals}
    latest = values[("us-gaap:Assets", "USD", None)]
    assert latest.value == "9007199254740993.02"
    assert latest.filing_id == "0001234567-26-000002"
    assert latest.known_at == latest.ingested_at == OBSERVED
    assert latest.source.availability == "first_observed"
    assert latest.effective_at == datetime(2026, 6, 30, 23, 59, 59, 999999, UTC)
    assert values[("us-gaap:Assets", "USD/shares", None)].value == "2.25"
    assert values[("us-gaap:Revenues", "USD", date(2026, 1, 1))].value == "500"
    assert values[("us-gaap:Revenues", "USD", date(2026, 4, 1))].value == "250"
    assert not result.assets and not result.bars
    assert result.historical_pit == "not_verified"


@pytest.mark.parametrize(
    ("old", "new"),
    [
        (b'"cik": 1234567', b'"cik": 7654321'),
        (b'"val":9007199254740993.02', b'"val":NaN'),
        (b'"val":9007199254740993.02', b'"val":"123"'),
        (b'"accn":"0001234567-26-000002"', b'"accn":"fake"'),
        (b'"filed":"2026-08-25"', b'"filed":"2027-08-25"'),
        (b'"start":"2026-04-01"', b'"start":"2026-07-01"'),
        (b'"units": {', b'"units": null, "invalid": {'),
    ],
)
def test_sec_rejects_invalid_source_fields(old: bytes, new: bytes) -> None:
    with pytest.raises(ValueError):
        normalize_sec(
            sec_config(),
            download("sec-facts", fixture("sec-facts").replace(old, new)),
            download("sec-submissions"),
        )


def test_sec_rejects_mismatched_submission_identity() -> None:
    with pytest.raises(FetchError, match="identity_unresolved"):
        normalize_sec(
            sec_config(),
            download("sec-facts"),
            download(
                "sec-submissions", fixture("sec-submissions").replace(b"0001234567", b"0007654321")
            ),
        )


def test_sec_same_date_conflicting_vintages_fail() -> None:
    content = fixture("sec-facts").replace(b'"filed":"2026-08-01"', b'"filed":"2026-08-25"')
    with pytest.raises(ValueError, match="conflicting same-date"):
        normalize_sec(sec_config(), download("sec-facts", content), download("sec-submissions"))


def test_sec_missing_concept_is_an_empty_selection() -> None:
    result = normalize_sec(
        sec_config(concepts=["us-gaap:Missing"]), download("sec-facts"), download("sec-submissions")
    )
    assert not result.fundamentals and result.missing == ("us-gaap:Missing",)


def test_sec_counts_all_selected_vintages_against_budget() -> None:
    config = sec_config(budget={"records": 1})
    with pytest.raises(FetchError, match="record_budget"):
        normalize_sec(config, download("sec-facts"), download("sec-submissions"))


def test_sdk_parameters_pin_every_market_semantic() -> None:
    assert bars_parameters(alpaca_config(), date(2026, 9, 13)) == {
        "start": "2026-08-28T04:00:00+00:00",
        "end": "2026-09-01T03:59:59.999999+00:00",
        "limit": "1000",
        "currency": "USD",
        "sort": "asc",
        "timeframe": "1Day",
        "adjustment": "raw",
        "feed": "iex",
        "asof": "2026-09-13",
        "symbols": "DEMO",
    }


def test_raw_bars_keep_decimal_precision_and_complete_day_end() -> None:
    result = normalize_alpaca(
        alpaca_config(),
        date(2026, 9, 13),
        {"DEMO": download("alpaca-asset")},
        [download("alpaca-page-1"), download("alpaca-page-2")],
    )
    assert result.bars[0].open == "10.000000000000000001"
    assert result.bars[0].volume == 12345
    assert result.bars[0].effective_at == datetime(2026, 8, 29, 4, tzinfo=UTC)
    assert result.bars[0].known_at == OBSERVED
    assert result.bars[0].source.dataset == "daily-bars-iex"
    assert result.assets[0].security_id == "alpaca:asset:00000000-0000-4000-8000-000000000001"
    assert "issuer_id" not in type(result.assets[0]).model_fields
    assert "amount" not in type(result.bars[0]).model_fields
    assert result.symbol_asof == date(2026, 9, 13)


@pytest.mark.parametrize(
    ("session", "start", "end", "hours"),
    [
        ("2026-03-08", "2026-03-08T05:00:00Z", "2026-03-09T04:00:00Z", 23),
        ("2026-11-01", "2026-11-01T04:00:00Z", "2026-11-02T05:00:00Z", 25),
    ],
)
def test_daily_intervals_follow_dst_not_fixed_24_hours(
    session: str, start: str, end: str, hours: int
) -> None:
    # Sunday fixtures exercise interval semantics only, not tradable-session validation.
    content = json.dumps(
        {
            "bars": {"DEMO": [{"t": start, "o": 1, "h": 1, "l": 1, "c": 1, "v": 1}]},
            "next_page_token": None,
        }
    ).encode()
    asof = date(2026, 12, 1)
    observed = datetime(2026, 12, 1, 12, tzinfo=UTC)
    result = normalize_alpaca(
        alpaca_config(start=session, end=session),
        asof,
        {"DEMO": replace(download("alpaca-asset"), observed_at=observed)},
        [replace(download("alpaca-page-2", content), observed_at=observed)],
    )
    assert result.bars[0].effective_at == datetime.fromisoformat(end)
    assert result.bars[0].effective_at - result.bars[0].interval_start == timedelta(hours=hours)
    assert result.calendar_validation == "not_performed"


@pytest.mark.parametrize(
    ("old", "new"),
    [
        (b'"o":11', b'"o":14'),
        (b'"v":54321', b'"v":1.5'),
        (b'"v":54321', b'"v":true'),
        (b"2026-08-31T04:00:00Z", b"2026-08-31T16:00:00Z"),
        (b"2026-08-31T04:00:00Z", b"2026-08-27T04:00:00Z"),
        (b'"DEMO"', b'"UNREQUESTED"'),
    ],
)
def test_alpaca_rejects_invalid_bar_semantics(old: bytes, new: bytes) -> None:
    with pytest.raises(ValueError):
        normalize_alpaca(
            alpaca_config(),
            date(2026, 9, 13),
            {"DEMO": download("alpaca-asset")},
            [download("alpaca-page-2", fixture("alpaca-page-2").replace(old, new))],
        )


@pytest.mark.parametrize(
    ("old", "new"),
    [
        (b'"symbol": "DEMO"', b'"symbol": "OTHER"'),
        (b'"class": "us_equity"', b'"class": "crypto"'),
        (b'"status": "active"', b'"status": "unknown"'),
        (b'"exchange": "NASDAQ"', b'"exchange": null'),
        (b"00000000-0000-4000-8000-000000000001", b"00000000-0000-0000-0000-000000000000"),
    ],
)
def test_current_asset_identity_is_not_inferred(old: bytes, new: bytes) -> None:
    with pytest.raises(ValueError):
        normalize_alpaca(
            alpaca_config(),
            date(2026, 9, 13),
            {"DEMO": download("alpaca-asset", fixture("alpaca-asset").replace(old, new))},
            [download("alpaca-page-2")],
        )


def test_duplicate_bars_are_denied_across_pages() -> None:
    content = fixture("alpaca-page-1").replace(b'"fixture-page-2="', b"null")
    with pytest.raises(ValueError, match="duplicate or out-of-order"):
        normalize_alpaca(
            alpaca_config(),
            date(2026, 9, 13),
            {"DEMO": download("alpaca-asset")},
            [download("alpaca-page-1"), download("alpaca-page-2", content)],
        )


def test_truncated_pagination_has_no_normalized_batch() -> None:
    with pytest.raises(ValueError, match="incomplete Alpaca pagination"):
        normalize_alpaca(
            alpaca_config(),
            date(2026, 9, 13),
            {"DEMO": download("alpaca-asset")},
            [download("alpaca-page-1")],
        )


def test_asset_lookup_crossing_new_york_date_is_denied() -> None:
    with pytest.raises(FetchError, match="identity_unresolved"):
        normalize_alpaca(
            alpaca_config(),
            date(2026, 9, 13),
            {"DEMO": replace(download("alpaca-asset"), observed_at=OBSERVED - timedelta(days=1))},
            [download("alpaca-page-2")],
        )


@pytest.mark.parametrize("token", ["https://elsewhere.invalid", "x" * 1025, "", 17])
def test_cursors_are_bounded_strings(token: object) -> None:
    content = json.dumps({"bars": {}, "next_page_token": token}).encode()
    with pytest.raises(ValueError):
        page_rows(download("alpaca-page-2", content), ("DEMO",))


@pytest.mark.parametrize(
    "content",
    [
        b'{"x":1,"x":2}',
        b'{"x":NaN}',
        b'{"x":Infinity}',
        b'{"x":1e200}',
        b'{"x":0.0000000000000000001}',
        b'{"x":12345678901234567890123456}',
        b"[]",
        b"{} trailing",
        b"\xff",
        b"[" * 1100 + b"]" * 1100,
    ],
)
def test_vendor_json_rejects_ambiguous_or_unbounded_values(content: bytes) -> None:
    with pytest.raises(ValueError):
        decode_object(content)


def test_exact_json_numbers_never_pass_through_float() -> None:
    value = decode_object(b'{"x":9007199254740993.01,"y":1e-18}')
    assert decimal_text(value["x"]) == "9007199254740993.01"
    assert decimal_text(value["y"]) == "0.000000000000000001"
    with pytest.raises(ValueError):
        decimal_text(0.1)
    with pytest.raises(ValueError):
        decimal_text(True)
    assert decimal_text(Decimal("0")) == "0"


@pytest.mark.parametrize(
    "budget",
    [
        {"requests": 0},
        {"pages": 33},
        {"timeout_seconds": 181},
        {"interval_seconds": 0.1},
        {"response_bytes": 4096, "total_bytes": 1024},
        {"retries": 3},
        {"records": 10001},
    ],
)
def test_download_budgets_are_always_finite(budget: dict[str, object]) -> None:
    with pytest.raises(ValueError):
        FetchBudget.model_validate(budget)


def test_sec_old_conflicts_are_rejected_in_every_row_order() -> None:
    from itertools import permutations

    rows = [
        {"end": "2026-06-30", "val": 1, "filed": "2026-08-01", "accn": "0001234567-26-000001"},
        {"end": "2026-06-30", "val": 2, "filed": "2026-08-01", "accn": "0001234567-26-000002"},
        {"end": "2026-06-30", "val": 3, "filed": "2026-08-25", "accn": "0001234567-26-000003"},
    ]
    for ordering in permutations(rows):
        content = json.dumps(
            {
                "cik": 1234567,
                "facts": {"us-gaap": {"Assets": {"units": {"USD": ordering}}}},
            }
        ).encode()
        with pytest.raises(ValueError, match="conflicting same-date"):
            normalize_sec(sec_config(), download("sec-facts", content), download("sec-submissions"))


def test_sec_equivalent_decimal_scales_are_not_conflicting() -> None:
    content = b'{"cik":1234567,"facts":{"us-gaap":{"Assets":{"units":{"USD":['
    row = b'{"end":"2026-06-30","filed":"2026-08-01","accn":"0001234567-26-000001","val":'
    rows = [row + b"1}", row + b"1.0}"]
    results = []
    for ordering in (rows, list(reversed(rows))):
        result = normalize_sec(
            sec_config(),
            download("sec-facts", content + b",".join(ordering) + b"]}}}}}"),
            download("sec-submissions"),
        )
        results.append(result.fundamentals[0].value)
    assert results == ["1.0", "1.0"]
