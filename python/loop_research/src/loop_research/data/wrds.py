"""Fixed read-only WRDS projections with bounded PostgreSQL execution."""

import asyncio
import json
import os
from collections.abc import Awaitable, Callable
from decimal import Decimal, InvalidOperation
from typing import Any

import httpx
import psycopg

from loop_research.data.fetch_http import BoundedHttp, Download, FetchError
from loop_research.data.fetch_json import decimal_text, decode_object
from loop_research.data.licensed_config import WrdsRequest, requested_datasets
from loop_research.data.licensed_records import SourceTable
from loop_research.data.sharadar import source_date

WRDS_HOST = "wrds-pgdata.wharton.upenn.edu"
WRDS_PORT = 9737
WRDS_DATABASE = "wrds"
COLUMNS = {
    "crsp_ciz_daily_v1": (
        "permno",
        "dlycaldt",
        "dlyprc",
        "dlyprcflg",
        "dlyvol",
        "dlyret",
        "dlyretx",
        "dlydelflg",
    ),
    "compustat_fundq_v1": (
        "gvkey",
        "datadate",
        "fyearq",
        "fqtr",
        "rdq",
        "indfmt",
        "datafmt",
        "popsrc",
        "consol",
        "curcdq",
        "atq",
        "ltq",
        "ceqq",
        "revtq",
        "niq",
        "cshoq",
    ),
}
type Connector = Callable[..., Awaitable[psycopg.AsyncConnection[Any]]]


def query(config: WrdsRequest) -> tuple[str, tuple[object, ...]]:
    """Only allowlisted SQL identifiers are interpolated; selection values are bound."""
    columns = COLUMNS[config.profile]
    dataset = requested_datasets(config)[0]
    projection = ", ".join(f"{name}::text AS {name}" for name in columns)
    key, period = columns[:2]
    ids: list[int] | list[str] = (
        sorted(int(value) for value in config.identifiers)
        if config.profile == "crsp_ciz_daily_v1"
        else sorted(config.identifiers)
    )
    condition = ""
    if config.profile == "compustat_fundq_v1":
        condition = " AND indfmt = 'INDL' AND datafmt = 'STD' AND popsrc = 'D' AND consol = 'C'"
    sql = (
        f"SELECT {projection} FROM {dataset} WHERE {key} = ANY(%s) "
        f"AND {period} BETWEEN %s AND %s{condition} ORDER BY {key}, {period} LIMIT %s"
    )
    return sql, (ids, config.start, config.end, config.budget.records + 1)


def request_identity(config: WrdsRequest) -> str:
    """Public identity fixes profile and bounds; SQL/credentials never come from a URI."""
    return str(
        httpx.URL(
            f"wrds://{WRDS_HOST}:{WRDS_PORT}/{requested_datasets(config)[0]}",
            params={
                "profile": config.profile,
                "identifiers": ",".join(sorted(config.identifiers)),
                "start": config.start.isoformat(),
                "end": config.end.isoformat(),
                "limit": str(config.budget.records + 1),
            },
        )
    )


async def download(
    config: WrdsRequest,
    guard: BoundedHttp,
    username: str,
    password: str,
    *,
    connector: Connector | None = None,
) -> Download:
    """Run one bounded read-only transaction, closing on cancellation or any failure.

    A connector is injectable only by Python tests, never CLI/configuration. The
    production path denies ambient libpq variables and fixes endpoint and TLS.
    The captured bytes are a labeled PostgreSQL text projection, not wire packets.
    """
    if connector is None and any(name.startswith("PG") for name in os.environ):
        raise FetchError("invalid_configuration")
    if guard.attempts >= config.budget.requests:
        raise FetchError("request_budget")
    guard.attempts += 1
    timeout_ms = max(1, int(min(guard.check(), 60) * 1000))
    connect = connector or psycopg.AsyncConnection.connect
    connection: psycopg.AsyncConnection[Any] | None = None
    try:
        async with asyncio.timeout(guard.check()):
            connection = await connect(
                host=WRDS_HOST,
                port=WRDS_PORT,
                dbname=WRDS_DATABASE,
                user=username,
                password=password,
                sslmode="require",
                gssencmode="disable",
                passfile="/dev/null",
                connect_timeout=min(10, config.budget.timeout_seconds),
                application_name="loop-engine-licensed-ingestion",
                client_encoding="UTF8",
                options=(
                    "-c default_transaction_read_only=on -c search_path=pg_catalog "
                    f"-c statement_timeout={timeout_ms} -c lock_timeout=1000 "
                    "-c idle_in_transaction_session_timeout=10000 -c timezone=UTC "
                    "-c datestyle=ISO,YMD"
                ),
            )
            if not connection.pgconn.ssl_in_use:
                raise FetchError("authentication")
            async with connection.transaction():
                await connection.execute("SET TRANSACTION READ ONLY")
                async with connection.cursor() as check:
                    await check.execute("SHOW transaction_read_only")
                    if await check.fetchone() != ("on",):
                        raise FetchError("forbidden")
                sql, parameters = query(config)
                rows: list[list[str | None]] = []
                size = 0
                async with connection.cursor(name="loop_source_read") as cursor:
                    await cursor.execute(sql, parameters)
                    async for row in cursor:
                        guard.check()
                        if len(rows) >= config.budget.records:
                            raise FetchError("record_budget")
                        if any(value is not None and not isinstance(value, str) for value in row):
                            raise FetchError("invalid_response")
                        size += len(json.dumps(row, ensure_ascii=True).encode()) + 1
                        if size > min(config.budget.response_bytes, config.budget.total_bytes):
                            raise FetchError("byte_budget")
                        rows.append(list(row))
            body = json.dumps(
                {
                    "format": "postgres_text_projection/v1",
                    "columns": COLUMNS[config.profile],
                    "rows": rows,
                },
                ensure_ascii=True,
                separators=(",", ":"),
            ).encode()
            guard.bytes_received += len(body)
            if (
                len(body) > config.budget.response_bytes
                or guard.bytes_received > config.budget.total_bytes
            ):
                raise FetchError("byte_budget")
            return Download(request_identity(config), body, guard.observed_at(), None)
    except TimeoutError, psycopg.errors.QueryCanceled:
        raise FetchError("deadline") from None
    except psycopg.errors.InvalidPassword, psycopg.errors.InvalidAuthorizationSpecification:
        raise FetchError("authentication") from None
    except psycopg.errors.InsufficientPrivilege:
        raise FetchError("forbidden") from None
    except psycopg.errors.UndefinedTable, psycopg.errors.UndefinedColumn:
        raise FetchError("invalid_response") from None
    except psycopg.Error:
        raise FetchError("upstream_unavailable") from None
    finally:
        if connection is not None:
            await connection.close()


def normalize(config: WrdsRequest, capture: Download) -> SourceTable:
    """Retain CIZ delisting semantics and Compustat vintage limitations."""
    body = decode_object(capture.body)
    columns = COLUMNS[config.profile]
    if (
        set(body) != {"format", "columns", "rows"}
        or body["format"] != "postgres_text_projection/v1"
    ):
        raise ValueError("invalid WRDS capture format")
    if body["columns"] != list(columns) or not isinstance(body["rows"], list):
        raise ValueError("WRDS projection drift")
    if len(body["rows"]) > config.budget.records:
        raise ValueError("WRDS row budget exceeded")
    result: list[tuple[str | None, ...]] = []
    is_crsp = config.profile == "crsp_ciz_daily_v1"
    numeric = set(columns[2:]) - {
        "dlyprcflg",
        "dlydelflg",
        "rdq",
        "indfmt",
        "datafmt",
        "popsrc",
        "consol",
        "curcdq",
    }
    for row in body["rows"]:
        if (
            not isinstance(row, list)
            or len(row) != len(columns)
            or any(
                value is not None and (not isinstance(value, str) or len(value) > 4096)
                for value in row
            )
        ):
            raise ValueError("invalid WRDS row")
        values = dict(zip(columns, row, strict=True))
        if values[columns[0]] not in config.identifiers:
            raise ValueError("unrequested WRDS identity")
        if not config.start <= source_date(values[columns[1]]) <= config.end:
            raise ValueError("WRDS period outside scope")
        for name in numeric:
            if values[name] is not None:
                try:
                    values[name] = decimal_text(Decimal(values[name]))
                except InvalidOperation:
                    raise ValueError("invalid WRDS numeric representation") from None
        if is_crsp:
            if values["dlydelflg"] not in {"Y", "N"} or not values["dlyprcflg"]:
                raise ValueError("missing CRSP price/delisting flags")
            for name in ("dlyret", "dlyretx"):
                if values[name] is not None and Decimal(values[name]) < -1:
                    raise ValueError("invalid CRSP return")
            if values["dlyvol"] is not None and Decimal(values["dlyvol"]) < 0:
                raise ValueError("invalid CRSP volume")
        else:
            if tuple(values[name] for name in ("indfmt", "datafmt", "popsrc", "consol")) != (
                "INDL",
                "STD",
                "D",
                "C",
            ):
                raise ValueError("unrequested Compustat reporting format")
            if values["rdq"] is not None:
                source_date(values["rdq"])
            if values["fqtr"] is not None and Decimal(values["fqtr"]) not in {1, 2, 3, 4}:
                raise ValueError("invalid fiscal quarter")
        result.append(tuple(values[name] for name in columns))
    return SourceTable(
        dataset=requested_datasets(config)[0],
        columns=columns,
        primary_key=columns[:2],
        rows=tuple(sorted(result, key=lambda row: tuple(value or "" for value in row))),
        semantics=(
            *(
                ("crsp_ciz_includes_delisting", "native_price_flags")
                if is_crsp
                else (
                    "compustat_current_vintages",
                    "rdq_not_verified_known_at",
                )
            ),
            "postgres_text_projection",
            "first_observed_capture",
            "quality_unverified",
        ),
    )
