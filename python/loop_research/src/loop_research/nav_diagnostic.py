"""Read-only NAV diagnostics, not an admission or market-data authority boundary."""

import csv
import hashlib
import io
import math
import os
import re
import stat
from dataclasses import dataclass
from datetime import date
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Literal

from loop_research.numerics import aligned_nav_correlation, nav_to_returns

MAX_INPUT_BYTES = 10 * 1024 * 1024
MAX_OBSERVATIONS = 100_000
_NUMBER = re.compile(r"[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?\Z")


@dataclass(frozen=True, slots=True)
class NavCorrelationReport:
    """Diagnostic output with byte provenance, never a verified backtest result."""

    left_sha256: str
    right_sha256: str
    first_observation: str
    last_observation: str
    observations: int
    return_pairs: int
    minimum_return_pairs: int
    correlation: float | None
    status: Literal["ok", "insufficient_observations", "constant_returns"]
    schema: str = "loop.nav-correlation-diagnostic/v1"
    data_quality: str = "unverified_local_input"
    calendar_validation: str = "not_performed"
    cash_flow_adjustment: str = "caller_asserted"
    return_definition: str = "nav_simple_between_matching_observation_dates"


@dataclass(frozen=True, slots=True)
class _NavFile:
    sessions: tuple[date, ...]
    values: tuple[float, ...]
    sha256: str


def _read_bytes(path: Path) -> bytes:
    # Nonblocking open lets FIFOs fail the regular-file check without waiting
    # for another process. This Linux CLI deliberately rejects symlink inputs.
    descriptor = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as source:
        if not stat.S_ISREG(os.fstat(source.fileno()).st_mode):
            raise ValueError("NAV input must be a regular file")
        content = source.read(MAX_INPUT_BYTES + 1)
    if len(content) > MAX_INPUT_BYTES:
        raise ValueError("NAV input exceeds the 10 MiB limit")
    return content


def _parse_nav(value: str) -> float:
    if len(value) > 64 or _NUMBER.fullmatch(value) is None:
        raise ValueError("NAV must be an ASCII number of at most 64 characters")
    parsed = float(value)
    if not math.isfinite(parsed) or parsed < 0:
        raise ValueError("NAV must be finite and nonnegative")
    if parsed == 0:
        try:
            if Decimal(value) != 0:
                raise ValueError("NAV underflows float64")
        except InvalidOperation as error:
            raise ValueError("NAV exponent is not representable") from error
    return parsed


def _load(path: Path) -> _NavFile:
    content = _read_bytes(path)
    try:
        rows = csv.reader(io.StringIO(content.decode("ascii"), newline=""), strict=True)
        if next(rows, None) != ["session", "nav"]:
            raise ValueError("NAV CSV requires exactly the header session,nav")
        sessions: list[date] = []
        values: list[float] = []
        for row in rows:
            if len(sessions) >= MAX_OBSERVATIONS:
                raise ValueError("NAV input exceeds the 100000 observation limit")
            if len(row) != 2:
                raise ValueError("Every NAV row must contain exactly two fields")
            try:
                session = date.fromisoformat(row[0])
            except ValueError as error:
                raise ValueError("NAV sessions must be ISO dates YYYY-MM-DD") from error
            if row[0] != session.isoformat():
                raise ValueError("NAV sessions must be ISO dates YYYY-MM-DD")
            if sessions and session <= sessions[-1]:
                raise ValueError("NAV sessions must be unique and strictly increasing")
            sessions.append(session)
            values.append(_parse_nav(row[1]))
    except (UnicodeError, csv.Error) as error:
        raise ValueError("NAV input must be a valid ASCII CSV") from error
    if not sessions:
        raise ValueError("NAV input must contain an observation")
    nav_to_returns(values)
    return _NavFile(tuple(sessions), tuple(values), hashlib.sha256(content).hexdigest())


def correlate_nav_files(
    left: Path,
    right: Path,
    *,
    min_observations: int = 5,
    cash_flow_adjusted: bool = False,
) -> NavCorrelationReport:
    """Compare returns on exactly matching consecutive observation intervals.

    Require the caller to assert external cash flows were already removed.
    Dates prove matching interval endpoints, not exchange-calendar completeness,
    daily frequency, PIT quality, holdout access, or trustworthy accounting.
    Inputs are bounded regular ASCII CSV files; neither file is modified.
    Invalid NAV, shifted/missing dates and implicit alignment raise ValueError;
    file access raises OSError. Undefined correlations stay null with a reason.
    """
    if cash_flow_adjusted is not True:
        raise ValueError("Confirm that both NAV inputs already adjust for external cash flows")
    if type(min_observations) is not int or not 2 <= min_observations < MAX_OBSERVATIONS:
        raise ValueError("Minimum return pairs must be an integer from 2 to 99999")
    left_data = _load(left)
    right_data = _load(right)
    if left_data.sessions != right_data.sessions:
        raise ValueError("NAV inputs must have identical observation dates; alignment is explicit")
    correlation = aligned_nav_correlation(
        left_data.values, right_data.values, min_observations=min_observations
    )
    pairs = len(left_data.sessions) - 1
    status: Literal["ok", "insufficient_observations", "constant_returns"] = "ok"
    if pairs < min_observations:
        status = "insufficient_observations"
    elif math.isnan(correlation):
        status = "constant_returns"
    return NavCorrelationReport(
        left_sha256=left_data.sha256,
        right_sha256=right_data.sha256,
        first_observation=left_data.sessions[0].isoformat(),
        last_observation=left_data.sessions[-1].isoformat(),
        observations=len(left_data.sessions),
        return_pairs=pairs,
        minimum_return_pairs=min_observations,
        correlation=correlation if status == "ok" else None,
        status=status,
    )
