"""Databento reference JSONL, preserving every supplied historical record."""

import re
from datetime import UTC, datetime, time, timedelta
from decimal import Decimal
from typing import Any

from databento.common.parsing import datetime_to_string

from loop_research.data.fetch_http import Download
from loop_research.data.fetch_json import decimal_text, decode_object
from loop_research.data.licensed_config import DatabentoRequest
from loop_research.data.licensed_records import SourceTable
from loop_research.data.sharadar import source_date


def endpoint(dataset: str) -> str:
    if dataset not in {"security_master", "corporate_actions"}:
        raise ValueError("unsupported Databento reference dataset")
    return f"https://hist.databento.com/v0/{dataset}.get_range"


def parameters(config: DatabentoRequest, dataset: str) -> dict[str, str]:
    """Use official timestamp encoding; PIT retention is a client policy, not a wire flag."""
    if dataset not in config.datasets:
        raise ValueError("unrequested Databento reference dataset")
    return {
        "start": datetime_to_string(config.start),
        "end": datetime_to_string(config.end + timedelta(days=1)),
        "index": "ts_effective" if dataset == "security_master" else "event_date",
        "symbols": ",".join(sorted(config.listing_ids)),
        "stype_in": "listing_id",
        "countries": "US",
        "allocate_isins": "false",
        "compression": "none",
    }


def jsonl_rows(content: bytes) -> list[dict[str, Any]]:
    """Original lines remain in the raw cache; no float conversion or latest-only groupby."""
    rows: list[dict[str, Any]] = []
    for line in content.splitlines():
        if not line.strip():
            raise ValueError("unexpected blank JSONL record")
        if len(rows) >= 10_000:
            raise ValueError("Databento record budget exceeded")
        rows.append(decode_object(line))
    return rows


def _instant(value: object) -> tuple[datetime, int]:
    if (
        not isinstance(value, str)
        or re.fullmatch(
            r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]{1,9})?(?:Z|[+-][0-9]{2}:[0-9]{2})",
            value,
        )
        is None
    ):
        raise ValueError("invalid Databento timestamp")
    stamp = datetime.fromisoformat(value).astimezone(UTC)
    if not 1900 <= stamp.year <= 2100:
        raise ValueError("Databento timestamp outside bounds")
    fraction = re.search(r"\.([0-9]{1,9})", value)
    nanos = int(fraction.group(1).ljust(9, "0")) if fraction else 0
    return stamp.replace(microsecond=0), nanos


def _flatten(row: dict[str, Any]) -> dict[str, str | None]:
    result: dict[str, str | None] = {}
    pending: list[tuple[str, object, int]] = [(key, value, 0) for key, value in row.items()]
    while pending:
        key, value, depth = pending.pop()
        if (
            len(result) + len(pending) > 200
            or depth > 2
            or re.fullmatch(r"[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*){0,2}", key) is None
        ):
            raise ValueError("unbounded or invalid Databento fields")
        if isinstance(value, dict):
            pending.extend(
                (key + "." + child, nested, depth + 1) for child, nested in value.items()
            )
            continue
        if key in result:
            raise ValueError("ambiguous flattened field")
        if value is None:
            result[key] = None
        elif type(value) is bool:
            result[key] = "true" if value else "false"
        elif type(value) is int or isinstance(value, Decimal):
            result[key] = decimal_text(value)
        elif isinstance(value, str) and len(value) <= 4096 and (not value or value.isprintable()):
            result[key] = value
        else:
            raise ValueError("unsupported Databento field value")
    return result


def normalize(config: DatabentoRequest, dataset: str, capture: Download) -> SourceTable:
    """Validate listing/date scope and retain cancellations, revisions and nested fields."""
    rows = jsonl_rows(capture.body)
    if len(rows) > config.budget.records:
        raise ValueError("Databento record budget exceeded")
    security = dataset == "security_master"
    keys = (
        ("listing_id", "ts_effective", "ts_record")
        if security
        else (
            "event_unique_id",
            "ts_record",
        )
    )
    required = {
        *keys,
        "listing_id",
        "security_id",
        "issuer_id",
        "ts_record",
        "ts_created",
        "listing_country",
    }
    required |= {"listing_status"} if security else {"event_date", "event", "event_action"}
    normalized = []
    for row in rows:
        if not required <= set(row) or row["listing_id"] not in config.listing_ids:
            raise ValueError("unresolved Databento record identity")
        for key, prefix in (("security_id", "S"), ("issuer_id", "I")):
            if (
                not isinstance(row[key], str)
                or re.fullmatch(prefix + r"-[1-9][0-9]{0,15}", row[key]) is None
            ):
                raise ValueError("invalid Databento identity namespace")
        if row["listing_country"] != "US":
            raise ValueError("unrequested listing country")
        for name in ("ts_record", "ts_created"):
            if _instant(row[name]) > (
                capture.observed_at.astimezone(UTC).replace(microsecond=0),
                capture.observed_at.microsecond * 1000,
            ):
                raise ValueError("Databento record was not available at capture")
        if security:
            effective = _instant(row["ts_effective"])
            start = (datetime.combine(config.start, time.min, UTC), 0)
            end = (datetime.combine(config.end + timedelta(days=1), time.min, UTC), 0)
            if (
                not start <= effective < end
                or not isinstance(row["listing_status"], str)
                or not row["listing_status"]
            ):
                raise ValueError("invalid Databento security scope")
        else:
            if not config.start <= source_date(row["event_date"]) <= config.end:
                raise ValueError("Databento corporate event outside scope")
            if any(
                not isinstance(row[key], str) or not row[key]
                for key in (
                    "event_unique_id",
                    "event",
                    "event_action",
                )
            ):
                raise ValueError("missing Databento event identity or action")
        values = _flatten(row)
        for name in ("ts_record", "ts_created", "ts_effective"):
            if name in row:
                second, nanos = _instant(row[name])
                values[name] = second.strftime("%Y-%m-%dT%H:%M:%S") + f".{nanos:09d}Z"
        normalized.append(values)
    columns = tuple(sorted(required | {key for row in normalized for key in row}))
    table_rows = [tuple(row.get(name) for name in columns) for row in normalized]
    return SourceTable(
        dataset="databento/" + dataset,
        columns=columns,
        primary_key=keys,
        rows=tuple(sorted(table_rows, key=lambda row: tuple(value or "" for value in row))),
        semantics=(
            "native_listing_identity",
            "all_supplied_vintages",
            "existing_isins_only",
            "source_record_and_creation_clocks",
            "quality_unverified",
        ),
    )
