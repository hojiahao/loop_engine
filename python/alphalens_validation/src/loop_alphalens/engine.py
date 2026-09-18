"""Independent raw-label, grouping and Alphalens/SciPy statistics calculation."""

import csv
import io
import math
import warnings
from collections.abc import Callable
from datetime import timedelta
from decimal import ROUND_HALF_EVEN, Context, Decimal, InvalidOperation, localcontext
from typing import Any

import pandas as pd  # type: ignore[import-untyped]
from alphalens import performance  # type: ignore[import-untyped]
from scipy import stats  # type: ignore[import-untyped]

from loop_alphalens.models import Inputs

RAW_COLUMNS = (
    "session",
    "security_id",
    "eligible",
    "factor",
    "label_session",
    "opening",
    "closing",
)


def columns(groups: int) -> tuple[str, ...]:
    return (
        "decision_session",
        "label_session",
        "status",
        "signals",
        "labels",
        "ic",
        "rank_ic",
        *(f"group_{index + 1}" for index in range(groups)),
        "spread",
        "monotonicity",
    )


def table(content: bytes, header: tuple[str, ...]) -> list[dict[str, str]]:
    reader = csv.DictReader(io.StringIO(content.decode("ascii")), strict=True)
    try:
        if tuple(reader.fieldnames or ()) != header:
            raise ValueError("independent CSV schema differs")
        rows: list[dict[str, str]] = []
        for row in reader:
            if len(rows) >= 100_000 or set(row) != set(header) or None in row.values():
                raise ValueError("independent CSV row bounds")
            rows.append(row)
    except csv.Error as error:
        raise ValueError("invalid independent CSV") from error
    return rows


def csv_bytes(header: tuple[str, ...], rows: list[list[object]]) -> bytes:
    stream = io.StringIO(newline="")
    writer = csv.writer(stream, lineterminator="\n")
    writer.writerow(header)
    writer.writerows(rows)
    return stream.getvalue().encode("ascii")


def number(value: object) -> str:
    converted = float(value)  # type: ignore[arg-type]
    return format(converted, ".17g") if math.isfinite(converted) else ""


def _price(value: str) -> Decimal | None:
    if not value:
        return None
    if len(value) > 128:
        raise ValueError("price precision bound")
    try:
        result = Decimal(value)
    except InvalidOperation as error:
        raise ValueError("invalid raw price") from error
    if not result.is_finite() or not Decimal("1e-18") <= result <= Decimal("1e15"):
        raise ValueError("invalid raw price")
    return result


def _factor(value: str) -> float | None:
    if not value:
        return None
    if len(value) > 128:
        raise ValueError("factor precision bound")
    result = float(value)
    if not math.isfinite(result):
        raise ValueError("invalid independent factor")
    return result


def _groups(signals: list[tuple[str, float]], count: int) -> dict[str, int]:
    ordered = sorted(signals, key=lambda item: (item[1], item[0]))
    quotient, remainder = divmod(len(ordered), count)
    result = {}
    offset = 0
    for group in range(1, count + 1):
        size = quotient + int(group <= remainder)
        for security, _ in ordered[offset : offset + size]:
            result[security] = group
        offset += size
    return result


def calculate(
    inputs: Inputs, content: bytes, check: Callable[[], None]
) -> tuple[bytes, bytes, int]:
    rows = table(content, RAW_COLUMNS)
    expected = [
        (day.isoformat(), security) for day in inputs.sessions for security in inputs.securities
    ]
    if [(row["session"], row["security_id"]) for row in rows] != expected:
        raise ValueError("independent observation grid differs")
    width = len(inputs.securities)
    outputs: list[list[object]] = []
    memberships: list[tuple[str, str, int]] = []
    valid = 0
    for index, day in enumerate(inputs.sessions):
        check()
        label_day = (
            inputs.sessions[index + 1].isoformat() if index + 1 < len(inputs.sessions) else ""
        )
        selected: list[tuple[str, float]] = []
        labels: dict[str, float] = {}
        for row in rows[index * width : (index + 1) * width]:
            if row["label_session"] != label_day or row["eligible"] not in {"0", "1"}:
                raise ValueError("independent timing or eligibility differs")
            signal = _factor(row["factor"])
            opening, closing = _price(row["opening"]), _price(row["closing"])
            if not label_day and (opening is not None or closing is not None):
                raise ValueError("terminal signal cannot have a forward price")
            if row["eligible"] == "1" and signal is not None:
                orientation = 1 if inputs.direction == "higher_is_better" else -1
                selected.append((row["security_id"], orientation * signal))
                if opening is not None and closing is not None:
                    with localcontext(Context(prec=80, rounding=ROUND_HALF_EVEN)):
                        labels[row["security_id"]] = float((closing - opening) / opening)
        status = (
            "no_forward_session"
            if not label_day
            else "missing_forward_prices"
            if len(labels) != len(selected)
            else "insufficient_cross_section"
            if len(selected) < inputs.minimum_cross_section
            else "constant_signal"
            if len({value for _, value in selected}) == 1
            else "constant_labels"
            if len(set(labels.values())) == 1
            else "available"
        )
        metrics = [""] * (inputs.groups + 4)
        if status in {"available", "constant_labels"}:
            valid += int(status == "available")
            assigned = _groups(selected, inputs.groups)
            date = pd.Timestamp(day, tz="UTC")
            frame = pd.DataFrame(
                {
                    "factor": [value for _, value in selected],
                    "1D": [labels[security] for security, _ in selected],
                    "factor_quantile": [assigned[security] for security, _ in selected],
                },
                index=pd.MultiIndex.from_tuples(
                    [(date, security) for security, _ in selected], names=["date", "asset"]
                ),
            )
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", stats.ConstantInputWarning)
                rank_ic = performance.factor_information_coefficient(frame, group_adjust=False)
                grouped = performance.mean_return_by_quantile(frame, by_date=True, demeaned=False)[
                    0
                ]
                group_values = [
                    float(grouped.loc[(group, date), "1D"]) for group in range(1, inputs.groups + 1)
                ]
                monotonic = (
                    stats.spearmanr(range(1, inputs.groups + 1), group_values).statistic
                    if inputs.groups >= 3
                    else math.nan
                )
            pearson = (
                stats.pearsonr(frame["factor"].to_numpy(), frame["1D"].to_numpy()).statistic
                if status == "available"
                else math.nan
            )
            metrics = [
                number(pearson),
                number(rank_ic.loc[date, "1D"]),
                *(number(value) for value in group_values),
                number(group_values[-1] - group_values[0]),
                number(monotonic),
            ]
            memberships.extend(
                (day.isoformat(), security, group) for security, group in assigned.items()
            )
        outputs.append([day.isoformat(), label_day, status, len(selected), len(labels), *metrics])
    check()
    return csv_bytes(columns(inputs.groups), outputs), _turnover(inputs, memberships, check), valid


def _turnover(
    inputs: Inputs, memberships: list[tuple[str, str, int]], check: Callable[[], None]
) -> bytes:
    # Alphalens' asfreq/shift must use the complete frozen session axis. Treating
    # it as daily calendar time loses Friday-to-Monday and holiday membership.
    days = set(inputs.sessions[:-1])
    span = (inputs.sessions[-2] - inputs.sessions[0]).days
    holidays = [
        inputs.sessions[0] + timedelta(days=index)
        for index in range(span + 1)
        if inputs.sessions[0] + timedelta(days=index) not in days
    ]
    frequency = pd.offsets.CustomBusinessDay(holidays=holidays)
    dates = pd.DatetimeIndex(inputs.sessions[:-1], tz="UTC", freq=frequency)
    index = pd.MultiIndex.from_product([dates, inputs.securities], names=["date", "asset"])
    values = pd.Series(math.nan, index=index, dtype="float64")
    for day, security, group in memberships:
        values.loc[(pd.Timestamp(day, tz="UTC"), security)] = group
    reports: dict[int, Any] = {}
    for group in range(1, inputs.groups + 1):
        check()
        if (values == group).any():
            reports[group] = performance.quantile_turnover(values, group, period=1).reindex(dates)
    rows: list[list[object]] = []
    for position, day in enumerate(dates):
        for group in range(1, inputs.groups + 1):
            available = position > 0 and all(
                (values.loc[dates[offset]] == group).any() for offset in (position - 1, position)
            )
            value = number(reports[group].loc[day]) if available and group in reports else ""
            rows.append(
                [day.date().isoformat(), group, "available" if value else "unavailable", value]
            )
    return csv_bytes(("session", "quantile", "status", "membership_turnover"), rows)
