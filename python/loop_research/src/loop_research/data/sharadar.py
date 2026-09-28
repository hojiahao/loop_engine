"""Sharadar Tables API requests and exact native-column normalization."""

import hashlib
import json
import re
from datetime import date
from decimal import Decimal
from typing import Any

from loop_research.data.fetch_http import Download
from loop_research.data.fetch_json import decimal_text, decode_object
from loop_research.data.licensed_config import SharadarRequest
from loop_research.data.licensed_records import SourceTable

TABLE_COLUMNS: dict[str, tuple[str, ...]] = {
    "SEP": (
        "ticker",
        "date",
        "open",
        "high",
        "low",
        "close",
        "volume",
        "closeadj",
        "closeunadj",
        "lastupdated",
    ),
    "SF1": (
        "ticker",
        "dimension",
        "calendardate",
        "datekey",
        "reportperiod",
        "lastupdated",
        "assets",
        "liabilities",
        "equity",
        "revenue",
        "netinc",
        "sharesbas",
    ),
    "TICKERS": (
        "table",
        "permaticker",
        "ticker",
        "name",
        "exchange",
        "isdelisted",
        "category",
        "currency",
        "firstpricedate",
        "lastpricedate",
        "firstadded",
        "lastupdated",
        "relatedtickers",
        "siccode",
        "sicsector",
        "sicindustry",
        "famaindustry",
        "sector",
        "industry",
        "location",
        "scalemarketcap",
        "scalerevenue",
    ),
    "ACTIONS": ("date", "action", "ticker", "name", "value", "contraticker", "contraname"),
}
_DATES = {
    "date",
    "calendardate",
    "datekey",
    "reportperiod",
    "lastupdated",
    "firstpricedate",
    "lastpricedate",
    "firstadded",
}
_NUMBERS = {
    "open",
    "high",
    "low",
    "close",
    "volume",
    "closeadj",
    "closeunadj",
    "assets",
    "liabilities",
    "equity",
    "revenue",
    "netinc",
    "sharesbas",
    "value",
}
_KEYS = {
    "SEP": ("ticker", "date"),
    "SF1": ("ticker", "dimension", "calendardate", "datekey"),
    "TICKERS": ("table", "permaticker", "ticker"),
    "ACTIONS": ("source_row_id",),
}
_SEMANTICS = {
    "SEP": ("split_adjusted_ohlcv", "separate_raw_close", "separate_total_return_close"),
    "SF1": ("as_reported_dimension", "datekey_not_verified_known_at"),
    "TICKERS": ("current_metadata", "permaticker_namespace", "not_historical_universe"),
    "ACTIONS": ("native_action_values", "no_inferred_delisting_returns"),
}


def parameters(config: SharadarRequest, table: str, cursor: str | None = None) -> dict[str, str]:
    """Fixed projections and inclusive date ranges; never automatic bulk export."""
    if table not in config.tables:
        raise ValueError("unrequested Sharadar table")
    params = {
        "ticker": ",".join(sorted(config.symbols)),
        "qopts.columns": ",".join(TABLE_COLUMNS[table]),
    }
    if table == "TICKERS":
        params["table"] = "SEP"
    else:
        column = "datekey" if table == "SF1" else "date"
        params[column + ".gte"] = config.start.isoformat()
        params[column + ".lte"] = config.end.isoformat()
    if table == "SF1":
        params["dimension"] = config.dimension
    if cursor is not None:
        if re.fullmatch(r"[A-Za-z0-9_.=-]{1,1024}", cursor) is None:
            raise ValueError("invalid Sharadar pagination cursor")
        params["qopts.cursor_id"] = cursor
    return params


def endpoint(table: str) -> str:
    if table not in TABLE_COLUMNS:
        raise ValueError("unsupported Sharadar table")
    return f"https://data.nasdaq.com/api/v3/datatables/SHARADAR/{table}.json"


def page_rows(table: str, download: Download) -> tuple[list[dict[str, Any]], str | None]:
    """Validate a named-column page; column order may vary without shifting values."""
    body = decode_object(download.body)
    if set(body) != {"datatable", "meta"}:
        raise ValueError("invalid Sharadar envelope")
    data = body["datatable"]
    meta = body["meta"]
    if not isinstance(data, dict) or not isinstance(meta, dict) or "next_cursor_id" not in meta:
        raise ValueError("missing Sharadar pagination metadata")
    columns = data.get("columns")
    rows = data.get("data")
    if not isinstance(columns, list) or not isinstance(rows, list) or len(rows) > 10_000:
        raise ValueError("invalid Sharadar table")
    names: list[str] = []
    for column in columns:
        if not isinstance(column, dict) or set(column) != {"name", "type"}:
            raise ValueError("invalid Sharadar column declaration")
        name = column["name"]
        if name not in TABLE_COLUMNS[table] or name in names:
            raise ValueError("unexpected or duplicate Sharadar column")
        expected = (
            {"Date"}
            if name in _DATES
            else (
                {"Integer", "BigInteger", "BigDecimal", "Double"}
                if name in _NUMBERS
                else {"String"}
            )
        )
        if name in {"permaticker", "siccode"}:
            # Data Link vintages have used integral identifiers; the current
            # publisher describes these as text. Neither is a measurement.
            expected = {"String", "Integer", "BigInteger"}
        if column["type"] not in expected:
            raise ValueError("Sharadar column type changed")
        names.append(name)
    if set(names) != set(TABLE_COLUMNS[table]):
        raise ValueError("missing Sharadar columns")
    result = []
    for row in rows:
        if not isinstance(row, list) or len(row) != len(names):
            raise ValueError("Sharadar row width mismatch")
        result.append(dict(zip(names, row, strict=True)))
    cursor = meta["next_cursor_id"]
    if cursor is not None and (
        not isinstance(cursor, str) or re.fullmatch(r"[A-Za-z0-9_.=-]{1,1024}", cursor) is None
    ):
        raise ValueError("invalid Sharadar cursor")
    if cursor is not None and not result:
        raise ValueError("empty nonterminal Sharadar page")
    return result, cursor


def source_date(value: object) -> date:
    """Accept source dates without timestamp truncation or locale coercion."""
    if not isinstance(value, str) or re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", value) is None:
        raise ValueError("invalid source date")
    parsed = date.fromisoformat(value)
    if not 1900 <= parsed.year <= 2100:
        raise ValueError("source date exceeds supported range")
    return parsed


def _cell(name: str, value: object) -> str | None:
    if value is None:
        return None
    if name in _DATES:
        return source_date(value).isoformat()
    if name in {"permaticker", "siccode"}:
        if type(value) is int:
            value = str(value)
        pattern = r"[A-Za-z0-9_.-]{1,160}" if name == "permaticker" else r"[0-9]{1,4}"
        if not isinstance(value, str) or re.fullmatch(pattern, value) is None:
            raise ValueError("invalid native identifier")
        return value
    if name in _NUMBERS:
        return decimal_text(value)
    if not isinstance(value, str) or len(value) > 4096 or (value and not value.isprintable()):
        raise ValueError("invalid native text field")
    return value


def normalize(config: SharadarRequest, table: str, pages: list[Download]) -> SourceTable:
    """Preserve native price/fundamental semantics; do not invent PIT availability."""
    columns = TABLE_COLUMNS[table]
    normalized: list[tuple[str | None, ...]] = []
    for page in pages:
        rows, _ = page_rows(table, page)
        for native in rows:
            if native["ticker"] not in config.symbols:
                raise ValueError("unrequested Sharadar ticker")
            values = {name: _cell(name, native[name]) for name in columns}
            if table != "TICKERS":
                period = source_date(values["datekey" if table == "SF1" else "date"])
                if not config.start <= period <= config.end:
                    raise ValueError("Sharadar row outside date scope")
            if table == "SEP":
                prices = [Decimal(values[key] or "0") for key in ("open", "high", "low", "close")]
                opening, high, low, close = prices
                if not 0 < low <= min(opening, close) <= max(opening, close) <= high:
                    raise ValueError("invalid split-adjusted OHLC")
                if any(Decimal(values[key] or "0") <= 0 for key in ("closeadj", "closeunadj")):
                    raise ValueError("missing adjusted or raw close")
                if values["volume"] is None or Decimal(values["volume"] or "0") < 0:
                    raise ValueError("invalid split-adjusted volume")
            elif table == "SF1":
                if (
                    values["dimension"] != config.dimension
                    or source_date(values["reportperiod"]) > period
                ):
                    raise ValueError("invalid as-reported fundamental period")
                source_date(values["calendardate"])
            elif table == "TICKERS":
                if not values["permaticker"] or values["table"] != "SEP":
                    raise ValueError("invalid Sharadar permanent identity")
                if values["isdelisted"] not in {"Y", "N"}:
                    raise ValueError("unresolved Sharadar listing status")
                if (
                    values["firstpricedate"]
                    and values["lastpricedate"]
                    and (
                        source_date(values["firstpricedate"]) > source_date(values["lastpricedate"])
                    )
                ):
                    raise ValueError("invalid price coverage dates")
            elif not values["action"] or not values["name"]:
                raise ValueError("missing corporate-action identity")
            row = tuple(values[name] for name in columns)
            if table == "ACTIONS":
                # Native keys contain nullable contra fields. Hash their exact
                # tuple, not the value, so contradictory values for one event
                # fail instead of being presented as independent observations.
                key = tuple(values[name] for name in columns if name != "value")
                row = (
                    hashlib.sha256(json.dumps(key, ensure_ascii=True).encode()).hexdigest(),
                    *row,
                )
            normalized.append(row)
    if len(normalized) > config.budget.records:
        raise ValueError("Sharadar record budget")
    return SourceTable(
        dataset="SHARADAR/" + table,
        columns=("source_row_id", *columns) if table == "ACTIONS" else columns,
        primary_key=_KEYS[table],
        rows=tuple(sorted(normalized, key=lambda row: tuple(value or "" for value in row))),
        semantics=(*_SEMANTICS[table], "first_observed_capture", "quality_unverified"),
    )
