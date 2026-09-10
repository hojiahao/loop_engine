import csv
import hashlib
import json
import subprocess
import sys
from datetime import date, datetime
from pathlib import Path

import pytest

from loop_research.calendar import FIRST_DATE, LAST_DATE, require_xnys_sessions, xnys_session_dates
from loop_research.nav_diagnostic import correlate_nav_files

FIXTURES = Path(__file__).resolve().parents[3] / "fixtures/research/nav"


@pytest.mark.parametrize(
    "holiday",
    [
        "2026-01-01",
        "2026-01-19",
        "2026-02-16",
        "2026-04-03",
        "2026-05-25",
        "2026-06-19",
        "2026-07-03",
        "2026-09-07",
        "2026-11-26",
        "2026-12-25",
    ],
)
def test_nyse_published_holidays_are_closed(holiday: str) -> None:
    session = date.fromisoformat(holiday)
    assert xnys_session_dates(session, session) == ()
    with pytest.raises(ValueError, match="complete XNYS"):
        require_xnys_sessions((session,))


@pytest.mark.parametrize("session", [date(2026, 11, 27), date(2026, 12, 24)])
def test_early_close_is_still_a_session(session: date) -> None:
    assert xnys_session_dates(session, session) == (session,)
    assert require_xnys_sessions((session,)).name == "XNYS"


def test_preholiday_sequence_is_complete() -> None:
    expected = (date(2026, 7, 1), date(2026, 7, 2), date(2026, 7, 6))
    assert xnys_session_dates(expected[0], expected[-1]) == expected
    evidence = require_xnys_sessions(expected)
    assert evidence.name == "XNYS"
    assert evidence.package_version == "4.13.2"
    assert (
        evidence.session_dates_sha256
        == hashlib.sha256(b"2026-07-01\n2026-07-02\n2026-07-06\n").hexdigest()
    )


def test_shared_missing_session_is_not_complete() -> None:
    with pytest.raises(ValueError, match="complete XNYS"):
        require_xnys_sessions((date(2026, 7, 1), date(2026, 7, 6)))


def test_weekends_are_not_sessions() -> None:
    with pytest.raises(ValueError, match="complete XNYS"):
        require_xnys_sessions((date(2026, 7, 4),))


def test_research_window_reaches_august_2026() -> None:
    sessions = xnys_session_dates(FIRST_DATE, date(2026, 8, 31))
    assert sessions[0] == date(2005, 1, 3)
    assert sessions[-1] == date(2026, 8, 31)
    assert 5000 < len(sessions) < 6000
    assert require_xnys_sessions(sessions) == require_xnys_sessions(sessions)


@pytest.mark.parametrize(
    "start,end",
    [
        (date(2004, 12, 31), FIRST_DATE),
        (LAST_DATE, date(2027, 1, 1)),
        (date(2026, 1, 2), date(2026, 1, 1)),
        (datetime(2026, 1, 1), date(2026, 1, 2)),
    ],
)
def test_unsupported_boundaries_are_rejected(start: date, end: date) -> None:
    with pytest.raises(ValueError):
        xnys_session_dates(start, end)


@pytest.mark.parametrize(
    "sessions",
    [(), (date(2020, 1, 2),) * 2, (date(2020, 1, 3), date(2020, 1, 2)), (datetime(2020, 1, 2),)],
)
def test_invalid_sequences_are_rejected(sessions: tuple[date, ...]) -> None:
    with pytest.raises(ValueError):
        require_xnys_sessions(sessions)


def test_sequence_scan_is_bounded() -> None:
    sessions = (FIRST_DATE,) * ((LAST_DATE - FIRST_DATE).days + 2)
    with pytest.raises(ValueError, match="bounded"):
        require_xnys_sessions(sessions)


def test_diagnostic_calendar_is_explicit() -> None:
    left, right = FIXTURES / "left.csv", FIXTURES / "right.csv"
    unchecked = correlate_nav_files(left, right, cash_flow_adjusted=True)
    checked = correlate_nav_files(left, right, cash_flow_adjusted=True, calendar="XNYS")
    assert unchecked.calendar is None
    assert unchecked.calendar_validation == "not_performed"
    assert checked.calendar is not None
    assert checked.calendar.name == "XNYS"
    assert checked.calendar_validation == "complete_observation_dates"
    assert checked.data_quality == "unverified_local_input"
    assert checked.correlation == unchecked.correlation


def test_calendar_cli_validates_actual_inputs(tmp_path: Path) -> None:
    completed = subprocess.run(
        [
            sys.executable,
            "-m",
            "loop_research.cli",
            "nav-correlation",
            str(FIXTURES / "left.csv"),
            str(FIXTURES / "right.csv"),
            "--cash-flow-adjusted",
            "--calendar",
            "XNYS",
        ],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert completed.returncode == 0, completed.stderr
    report = json.loads(completed.stdout)
    assert report["calendar"]["package_version"] == "4.13.2"
    assert report["calendar_validation"] == "complete_observation_dates"
    assert not list(tmp_path.iterdir())


def test_calendar_cli_rejects_shared_gaps(tmp_path: Path) -> None:
    inputs = []
    for filename in ("left.csv", "right.csv"):
        with (FIXTURES / filename).open(newline="") as source:
            rows = list(csv.reader(source))
        del rows[3]
        path = tmp_path / filename
        with path.open("w", newline="") as destination:
            csv.writer(destination).writerows(rows)
        inputs.append(str(path))
    completed = subprocess.run(
        [
            sys.executable,
            "-m",
            "loop_research.cli",
            "nav-correlation",
            *inputs,
            "--cash-flow-adjusted",
            "--calendar",
            "XNYS",
        ],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert completed.returncode == 2
    assert "complete XNYS" in completed.stderr
    assert not completed.stdout
