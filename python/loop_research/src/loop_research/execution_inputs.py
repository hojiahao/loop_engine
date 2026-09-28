"""Select causal execution records from a bounded, source-backed development capture."""

import math
from collections import defaultdict
from collections.abc import Callable
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from decimal import Decimal
from zoneinfo import ZoneInfo

from loop_research.backtest_models import MAX_REPLAY_CELLS
from loop_research.data.fetch_records import CachedObject
from loop_research.data.models import TemporalRecord
from loop_research.factor_worker import FactorComputation
from loop_research.market_models import Action, ExecutionCapture, ExecutionPrice, ExecutionTerms
from loop_research.portfolio import Observation, Session

_EPOCH = datetime(1970, 1, 1, tzinfo=UTC)


def milliseconds(value: datetime) -> int:
    """Require millisecond-exact clocks instead of silently truncating visibility."""
    delta = value - _EPOCH
    if delta.microseconds % 1000:
        raise ValueError("execution clocks require millisecond precision")
    return delta.days * 86_400_000 + delta.seconds * 1000 + delta.microseconds // 1000


@dataclass(frozen=True, slots=True)
class MarketRow:
    """Resolved per-security observations and distinct open/close permissions."""

    observation: Observation
    auction_volume: int
    opening_terms: ExecutionTerms | None
    closing_terms: ExecutionTerms | None


@dataclass(frozen=True, slots=True)
class MarketSession:
    """Numerical inputs already aligned to one explicit exchange session."""

    base: Session
    scheduled_open_ms: int
    rows: tuple[MarketRow, ...]
    actions: tuple[Action, ...]


def _terms(records: list[ExecutionTerms], at: int) -> ExecutionTerms | None:
    visible = [
        record
        for record in records
        if milliseconds(record.known_at) <= at and milliseconds(record.effective_at) <= at
    ]
    if not visible:
        return None
    # Later effective states supersede earlier ones, including an expired state.
    selected = max(visible, key=lambda record: (record.effective_at, record.known_at))
    return selected if at < milliseconds(selected.valid_until) else None


def prepare_market(
    capture: ExecutionCapture,
    computed: FactorComputation,
    read: Callable[[CachedObject], bytes],
    check: Callable[[], None],
) -> tuple[MarketSession, ...]:
    """Resolve raw bytes and PIT revisions; never infer borrow or auction data.

    Historical queries use public knowledge at each event and ingestion no later
    than the immutable capture. First-observed records cannot backdate knowledge.
    Future corrections remain in lineage, but cannot replace an earlier fill.
    """
    import exchange_calendars as xcals  # type: ignore[import-untyped]

    panel, result = computed.loaded.panel, computed.result
    start = panel.sessions.index(result.evaluation_start)
    days = panel.sessions[start:]
    if len(days) < 2 or len(days) * len(panel.securities) > MAX_REPLAY_CELLS:
        raise ValueError("market replay grid exceeds session/cell bounds")
    allowed_days, securities = set(days), set(panel.securities)
    prices: dict[tuple[object, str, str], list[ExecutionPrice]] = defaultdict(list)
    terms: dict[tuple[object, str], list[ExecutionTerms]] = defaultdict(list)
    actions: dict[tuple[object, str], list[Action]] = defaultdict(list)
    seen: set[tuple[object, ...]] = set()
    events: dict[str, tuple[object, ...]] = {}
    records: tuple[TemporalRecord, ...] = (*capture.prices, *capture.terms, *capture.actions)
    for record in records:
        check()
        if not isinstance(record, (ExecutionPrice, ExecutionTerms, Action)):
            raise ValueError("unsupported execution record")
        if record.session not in allowed_days or record.security_id not in securities:
            raise ValueError("execution record exceeds the exact development sample")
        synthetic = record.source.availability == "synthetic"
        if synthetic != (computed.loaded.manifest.quality == "synthetic"):
            raise ValueError("execution source quality differs from the factor panel")
        if record.effective_at.astimezone(ZoneInfo("America/New_York")).date() != record.session:
            raise ValueError("execution event is outside its declared session")
        for instant in (record.effective_at, record.known_at, record.ingested_at):
            milliseconds(instant)
        key: tuple[object, ...]
        if isinstance(record, ExecutionPrice):
            key = (record.kind, record.session, record.security_id, record.known_at)
            prices[record.session, record.security_id, record.kind].append(record)
        elif isinstance(record, ExecutionTerms):
            milliseconds(record.valid_until)
            key = (
                "terms",
                record.session,
                record.security_id,
                record.effective_at,
                record.known_at,
            )
            terms[record.session, record.security_id].append(record)
        else:
            key = ("action", record.event_id, record.known_at)
            identity = (record.session, record.security_id, record.effective_at)
            if events.setdefault(record.event_id, identity) != identity:
                raise ValueError("action revisions change their effective identity")
            actions[record.session, record.security_id].append(record)
        if key in seen:
            raise ValueError("ambiguous or duplicate execution revision")
        seen.add(key)
    # Reject scope/clock metadata before resolving private raw source objects.
    for reference in capture.sources:
        check()
        read(reference)
    calendar = xcals.get_calendar(
        "XNYS",
        start=(days[0] - timedelta(days=7)).isoformat(),
        end=(days[-1] + timedelta(days=7)).isoformat(),
    )
    prepared = []
    for index, day in enumerate(days):
        check()
        open_ms = int(calendar.session_open(day.isoformat()).value // 1_000_000)
        close_ms = int(calendar.session_close(day.isoformat()).value // 1_000_000)
        decision = computed.loaded.manifest.decision_times_ms[start + index]
        rows = []
        selected_actions = []
        for column, security in enumerate(panel.securities):
            opens = prices[day, security, "open"]
            closes = prices[day, security, "close"]
            if len({record.effective_at for record in opens}) > 1:
                raise ValueError("opening revisions disagree on event time")
            for record in opens:
                if not open_ms <= milliseconds(record.effective_at) < close_ms:
                    raise ValueError("opening print falls outside regular trading")
            for record in closes:
                if milliseconds(record.effective_at) != close_ms:
                    raise ValueError("closing mark must identify the scheduled close")
            opening = min(opens, key=lambda record: record.known_at) if opens else None
            # A first-observed historical download is not an old executable print.
            if opening is not None and (
                milliseconds(opening.known_at) >= close_ms
                or milliseconds(opening.known_at) - milliseconds(opening.effective_at) > 60_000
            ):
                opening = None
            visible_closes = [
                record for record in closes if milliseconds(record.known_at) <= decision
            ]
            closing = (
                max(visible_closes, key=lambda record: record.known_at) if visible_closes else None
            )
            at = milliseconds(opening.known_at) if opening is not None else open_ms
            value = float(result.values[index, column])
            observation = Observation(
                security,
                bool(panel.eligible[start + index, column]),
                value if math.isfinite(value) else None,
                at if opening is not None else None,
                Decimal(opening.price_usd) if opening is not None else None,
                milliseconds(closing.known_at) if closing is not None else None,
                Decimal(closing.price_usd) if closing is not None else None,
            )
            rows.append(
                MarketRow(
                    observation,
                    opening.auction_volume or 0 if opening is not None else 0,
                    _terms(terms[day, security], at),
                    _terms(terms[day, security], decision),
                )
            )
            candidates = actions[day, security]
            if len({record.event_id for record in candidates}) > 1:
                raise ValueError("combined same-session corporate actions require another profile")
            visible = [record for record in candidates if record.known_at <= record.effective_at]
            if candidates and not visible:
                raise ValueError("action was unavailable at its effective time")
            if visible:
                action = max(visible, key=lambda record: record.known_at)
                if milliseconds(action.effective_at) > open_ms:
                    raise ValueError("corporate actions must be effective before the opening")
                selected_actions.append(action)
        base = Session(day, decision, tuple(row.observation for row in rows))
        prepared.append(MarketSession(base, open_ms, tuple(rows), tuple(selected_actions)))
    return tuple(prepared)
