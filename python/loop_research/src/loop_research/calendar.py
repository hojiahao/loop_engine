"""Bounded, offline NYSE session-date checks for the 2005-2026 research window."""

import hashlib
from dataclasses import dataclass
from datetime import date, timedelta
from importlib.metadata import version
from itertools import pairwise

FIRST_DATE = date(2005, 1, 1)
LAST_DATE = date(2026, 12, 31)


@dataclass(frozen=True, slots=True)
class CalendarEvidence:
    """Session-date evidence only, not intraday or market-data attestation."""

    name: str
    package_version: str
    session_dates_sha256: str


def xnys_session_dates(start: date, end: date) -> tuple[date, ...]:
    """Resolve inclusive XNYS session dates from the installed, locked library.

    Explicit 2005-2026 bounds limit generation and match the current research
    scope. Future rule dates are scheduled dates, not proof a market will open.
    No wall-clock defaults, network fetches or custom holiday rules are used.
    This does not validate venue halts, valuation times or trading availability.
    """
    if type(start) is not date or type(end) is not date:
        raise ValueError("Calendar boundaries must be dates, not timestamps")
    if not FIRST_DATE <= start <= end <= LAST_DATE:
        raise ValueError("XNYS diagnostic dates must be ordered within 2005-2026")
    # Ordinary diagnostics and doctor do not need to load every venue calendar.
    import exchange_calendars as xcals  # type: ignore[import-untyped]

    # Padding permits a one-day query, including a holiday, without relying on
    # the library's moving default schedule bounds or an empty construction.
    calendar = xcals.get_calendar(
        "XNYS",
        start=(start - timedelta(days=7)).isoformat(),
        end=(end + timedelta(days=7)).isoformat(),
    )
    labels = calendar.sessions_in_range(start.isoformat(), end.isoformat())
    return tuple(date.fromisoformat(label) for label in labels.strftime("%Y-%m-%d"))


def require_xnys_sessions(sessions: tuple[date, ...]) -> CalendarEvidence:
    """Require every session between the supplied endpoints, in exact order.

    Shared missing sessions are rejected, as are weekends, holidays, duplicates
    and unsorted dates. Missing sessions never become zeros or filled NAV rows.
    The digest covers ISO session dates, each terminated by LF, not an entire
    execution calendar or the six-component production provenance fingerprint.
    """
    if not isinstance(sessions, tuple) or not sessions:
        raise ValueError("Calendar check requires a nonempty immutable date sequence")
    if len(sessions) > (LAST_DATE - FIRST_DATE).days + 1:
        raise ValueError("Calendar observation count exceeds the bounded research window")
    if any(type(session) is not date for session in sessions):
        raise ValueError("Calendar observations must be dates, not timestamps")
    if any(left >= right for left, right in pairwise(sessions)):
        raise ValueError("Calendar observations must be unique and strictly increasing")
    expected = xnys_session_dates(sessions[0], sessions[-1])
    if sessions != expected:
        raise ValueError("NAV dates do not match the complete XNYS session sequence")
    content = "".join(f"{session.isoformat()}\n" for session in expected).encode("ascii")
    return CalendarEvidence(
        "XNYS", version("exchange-calendars"), hashlib.sha256(content).hexdigest()
    )
