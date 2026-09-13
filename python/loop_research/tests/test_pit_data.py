"""Historical security and public/ingestion visibility goldens."""

import json
from datetime import UTC, date, datetime, timedelta
from pathlib import Path

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st
from pydantic import ValidationError

from loop_research.data.models import (
    Fundamental,
    PitInput,
    PitQuery,
    RawBar,
    SecurityVersion,
    SourceEvidence,
    parse_instant,
)
from loop_research.data.query import query_capture

FIXTURE = Path(__file__).resolve().parents[3] / "fixtures/market/pit/history.json"
CAPTURED = datetime(2026, 9, 13, tzinfo=UTC)


def capture(**changes: object) -> PitInput:
    values = json.loads(FIXTURE.read_bytes())
    values.update(changes)
    return PitInput.model_validate_json(json.dumps(values))


def query(instant: str = "2020-03-02T22:00:00Z", **changes: object) -> PitQuery:
    values: dict[str, object] = {
        "market_at": instant,
        "known_at": instant,
        "ingested_at": CAPTURED,
    }
    values.update(changes)
    return PitQuery.model_validate(values)


def update[T: SecurityVersion | RawBar | Fundamental | SourceEvidence](
    record: T, **changes: object
) -> T:
    values = record.model_dump()
    values.update(changes)
    return type(record).model_validate(values)


def with_records(original: PitInput, **changes: object) -> PitInput:
    values = original.model_dump(by_alias=True)
    values.update(changes)
    return PitInput.model_validate(values)


@pytest.mark.parametrize(
    "instant,expected",
    [
        ("2017-12-29T23:00:00Z", "synthetic:old-a"),
        ("2018-01-02T22:00:00Z", None),
        ("2019-01-02T22:00:00Z", "synthetic:new-a"),
    ],
)
def test_ticker_reuse_resolves_historical_security(instant: str, expected: str | None) -> None:
    result = query_capture(capture(), query(instant, ticker="DEMO", venue="XNYS"))
    assert [row.state.security_id for row in result.securities] == ([expected] if expected else [])


def test_share_classes_remain_distinct() -> None:
    result = query_capture(capture(), query())
    assert [row.state.security_id for row in result.securities] == [
        "synthetic:new-a",
        "synthetic:new-b",
    ]
    assert len({row.state.issuer_id for row in result.securities}) == 1
    assert len(result.fundamentals) == 1  # No duplicated issuer fact per share class.


def test_delisted_id_is_not_universe_eligible() -> None:
    result = query_capture(capture(), query(security_id="synthetic:old-a"))
    assert len(result.securities) == 1
    assert not result.securities[0].state.listed
    assert not result.securities[0].universe_eligible


def test_expired_latest_state_does_not_resurrect_old_listing() -> None:
    original = capture()
    old, delisted, *others = original.securities
    expired = update(delisted, effective_until="2018-01-03T00:00:00Z")
    data = with_records(original, securities=(old, expired, *others))
    assert not query_capture(data, query(security_id=old.security_id)).securities


def test_effective_end_is_exclusive() -> None:
    original = capture()
    new = update(original.securities[2], effective_until="2020-03-02T22:00:00Z")
    data = with_records(
        original, securities=(*original.securities[:2], new, original.securities[3])
    )
    assert query_capture(
        data, query("2020-03-02T21:59:59Z", security_id=new.security_id)
    ).securities
    assert not query_capture(data, query(security_id=new.security_id)).securities


def test_security_restatement_cannot_replace_a_later_effective_event() -> None:
    original = capture()
    # A newly published correction to the original listing does not undo delisting.
    correction = update(original.securities[0], known_at="2020-01-02T00:00:00Z", ticker="PAST")
    data = with_records(original, securities=(*original.securities, correction))
    result = query_capture(data, query(security_id=correction.security_id))
    assert not result.securities[0].state.listed
    assert result.securities[0].state.ticker == "DEMO"


def test_revision_at_same_effective_time_uses_visible_knowledge() -> None:
    original = capture()
    correction = update(original.securities[2], known_at="2020-03-03T00:00:00Z", ticker="RENAMED")
    data = with_records(original, securities=(*original.securities, correction))
    before = query_capture(data, query(security_id=correction.security_id))
    after = query_capture(data, query("2020-03-04T00:00:00Z", security_id=correction.security_id))
    assert before.securities[0].state.ticker == "DEMO"
    assert after.securities[0].state.ticker == "RENAMED"


def test_unknown_security_remains_absent() -> None:
    result = query_capture(capture(), query(security_id="synthetic:missing"))
    assert not result.securities and not result.bars and not result.fundamentals


@pytest.mark.parametrize("kind", ["adr", "etf", "fund", "preferred", "spac", "other", "unknown"])
def test_non_common_stock_is_excluded(kind: str) -> None:
    original = capture()
    excluded = update(original.securities[2], kind=kind)
    data = with_records(
        original, securities=(*original.securities[:2], excluded, original.securities[3])
    )
    result = query_capture(data, query())
    assert [row.state.security_id for row in result.securities] == ["synthetic:new-b"]
    assert (
        not query_capture(data, query(security_id=excluded.security_id))
        .securities[0]
        .universe_eligible
    )


def test_otc_is_excluded_by_default() -> None:
    original = capture()
    otc = update(original.securities[2], venue="OOTC")
    data = with_records(
        original, securities=(*original.securities[:2], otc, original.securities[3])
    )
    assert all(
        row.state.security_id != otc.security_id for row in query_capture(data, query()).securities
    )


def test_ambiguous_ticker_fails_closed() -> None:
    original = capture()
    conflict = update(original.securities[3], ticker="DEMO")
    data = with_records(original, securities=(*original.securities[:3], conflict))
    with pytest.raises(ValueError, match="ambiguous listed ticker"):
        query_capture(data, query())


@pytest.mark.parametrize("section", ["securities", "bars", "fundamentals"])
def test_duplicate_versions_are_rejected(section: str) -> None:
    values = json.loads(FIXTURE.read_bytes())
    values[section].append(values[section][0])
    with pytest.raises(ValidationError, match="duplicate or conflicting"):
        PitInput.model_validate_json(json.dumps(values))


def test_filing_delay_and_restatement_use_publication_time() -> None:
    data = capture()
    assert not query_capture(data, query("2020-02-03T21:59:59Z")).fundamentals
    original = query_capture(data, query("2020-02-03T22:00:00Z")).fundamentals
    assert original[0].value == "9007199254740993.01"
    revised = query_capture(data, query("2020-05-04T22:00:00Z")).fundamentals
    assert revised[0].value == "9007199254740994.02"


def test_ingestion_cutoff_replays_the_original_capture_view() -> None:
    result = query_capture(
        capture(), query("2020-06-01T22:00:00Z", ingested_at="2026-09-01T00:00:00Z")
    )
    assert result.fundamentals[0].filing_id == "synthetic:original"
    assert not query_capture(capture(), query(ingested_at="2026-08-31T23:59:59Z")).securities


def test_units_and_duration_periods_do_not_collapse() -> None:
    original = capture()
    base = original.fundamentals[0]
    quarterly = update(base, period_start=date(2019, 10, 1), value="1.25")
    shares = update(base, unit="shares", value="123456789")
    instant = update(base, period_start=None, value="9")
    data = with_records(original, fundamentals=(*original.fundamentals, quarterly, shares, instant))
    assert len(query_capture(data, query()).fundamentals) == 4


def test_bar_arrival_and_revision_do_not_backfill() -> None:
    original = capture()
    bar = original.bars[0]
    corrected = update(bar, close="10.50", known_at="2020-01-03T12:00:00Z")
    data = with_records(original, bars=(bar, corrected))
    assert not query_capture(data, query("2020-01-02T21:04:59Z")).bars
    assert query_capture(data, query("2020-01-02T21:05:00Z")).bars[0].close == "11.00"
    assert query_capture(data, query("2020-01-03T12:00:00Z")).bars[0].close == "10.50"


def test_different_feeds_cannot_silently_replace_bar_revisions() -> None:
    data = capture()
    base = data.bars[0]
    other = update(
        base, known_at="2020-01-03T12:00:00Z", source=update(base.source, dataset="other-feed")
    )
    with pytest.raises(ValueError, match="one explicit source dataset"):
        with_records(data, bars=(base, other))


def test_bar_revisions_cannot_silently_change_currency() -> None:
    data = capture()
    changed = update(data.bars[0], known_at="2020-01-03T12:00:00Z", currency="CAD")
    with pytest.raises(ValueError, match="change currency"):
        with_records(data, bars=(*data.bars, changed))


def test_market_cutoff_does_not_follow_later_knowledge() -> None:
    result = query_capture(
        capture(), query("2020-01-02T20:00:00Z", known_at="2020-03-02T22:00:00Z")
    )
    assert not result.bars


@pytest.mark.parametrize(
    "change",
    [
        {"market_at": "2020-01-01"},
        {"known_at": "2020-01-01T00:00:00"},
        {"ingested_at": 123},
        {"market_at": True},
        {"known_at": "2020-01-01T00:00:00Z"},
        {"ingested_at": "2019-01-01T00:00:00Z"},
        {"ticker": "DEMO"},
        {"venue": "XNYS"},
        {"ticker": "DEMO", "venue": "XNYS", "security_id": "synthetic:old-a"},
        {"ticker": " demo", "venue": "XNYS"},
    ],
)
def test_invalid_query_is_rejected(change: dict[str, object]) -> None:
    with pytest.raises(ValueError):
        query(**change)


def test_timezone_offsets_resolve_same_instant() -> None:
    assert parse_instant("2020-01-02T16:00:00-05:00") == parse_instant("2020-01-02T21:00:00Z")


def test_ingestion_after_capture_is_rejected() -> None:
    with pytest.raises(ValueError, match="ingested after the capture"):
        capture(captured_at="2026-08-31T00:00:00Z")


def test_query_cannot_project_ingestion_beyond_capture() -> None:
    with pytest.raises(ValueError, match="cutoff exceeds"):
        query_capture(capture(), query(ingested_at="2026-09-14T00:00:00Z"))


def test_first_observed_source_cannot_backdate_knowledge() -> None:
    base = capture().securities[0]
    source = update(base.source, availability="first_observed")
    with pytest.raises(ValueError, match="cannot backdate"):
        update(base, source=source)
    observed = update(base, source=source, known_at=base.ingested_at)
    assert observed.known_at == observed.ingested_at


def test_publication_cannot_follow_ingestion() -> None:
    with pytest.raises(ValueError, match="cannot follow ingestion"):
        update(capture().securities[0], known_at="2026-09-02T00:00:00Z")


def test_unknown_classification_is_explicit() -> None:
    with pytest.raises(ValueError):
        update(capture().securities[0], kind="new_vendor_asset_class")


def test_synthetic_capture_cannot_claim_public_quality() -> None:
    with pytest.raises(ValueError, match="quality must agree"):
        capture(quality="public_development")


@pytest.mark.parametrize("value", ["NaN", "Infinity", "1e3", " 1", "01", 0.1, True, "9" * 65])
def test_fundamental_values_require_bounded_exact_decimals(value: object) -> None:
    with pytest.raises(ValueError):
        update(capture().fundamentals[0], value=value)


@pytest.mark.parametrize(
    "change",
    [
        {"low": "0"},
        {"high": "10.50"},
        {"open": "-1"},
        {"close": "12"},
        {"volume": -1},
        {"volume": True},
        {"price_basis": "adjusted"},
        {"known_at": "2020-01-02T20:59:59Z"},
        {"interval_start": "2020-01-02T21:00:00Z"},
        {"session": date(2020, 1, 3)},
    ],
)
def test_invalid_raw_bar_is_rejected(change: dict[str, object]) -> None:
    with pytest.raises(ValueError):
        update(capture().bars[0], **change)


@pytest.mark.parametrize(
    "change",
    [
        {"period_start": date(2020, 1, 1)},
        {"effective_at": "2020-01-01T00:00:00Z"},
        {"known_at": "2019-12-30T00:00:00Z"},
    ],
)
def test_invalid_fiscal_period_is_rejected(change: dict[str, object]) -> None:
    with pytest.raises(ValueError):
        update(capture().fundamentals[0], **change)


def test_unresolved_security_reference_is_rejected() -> None:
    data = capture()
    with pytest.raises(ValueError, match="unresolved security"):
        with_records(data, bars=(update(data.bars[0], security_id="missing"),))


def test_unresolved_issuer_reference_is_rejected() -> None:
    data = capture()
    with pytest.raises(ValueError, match="unresolved issuer"):
        with_records(data, fundamentals=(update(data.fundamentals[0], issuer_id="missing"),))


def test_unchecked_nested_copy_is_revalidated() -> None:
    data = capture()
    invalid = data.bars[0].model_copy(update={"close": "-1"})
    with pytest.raises(ValueError):
        query_capture(data.model_copy(update={"bars": (invalid,)}), query())


@given(st.integers(min_value=1, max_value=1000), st.integers(min_value=1, max_value=1_000_000))
@settings(max_examples=30, deadline=None)
def test_future_revisions_do_not_change_historical_result(days: int, amount: int) -> None:
    data = capture()
    decision = query()
    revised = update(
        data.fundamentals[0],
        known_at=decision.known_at + timedelta(days=days, microseconds=1),
        value=str(amount),
    )
    extended = with_records(data, fundamentals=(*data.fundamentals, revised))
    assert query_capture(data, decision) == query_capture(extended, decision)


@given(st.permutations((0, 1, 2, 3)))
@settings(max_examples=24, deadline=None)
def test_source_order_does_not_change_result(order: list[int]) -> None:
    data = capture()
    reordered = with_records(data, securities=tuple(data.securities[index] for index in order))
    assert query_capture(data, query()) == query_capture(reordered, query())
