"""WRDS transport against an isolated database in the project's TLS test server.

The fixed local opt-in is set by the unified and research CI gates. No vendor or
production application connection string is accepted by this test fixture.
"""

import asyncio
import os
import uuid
from collections.abc import Iterator
from pathlib import Path
from typing import Any

import psycopg
import pytest
from data_helpers import OBSERVED
from licensed_helpers import ENV, cache, license_config
from psycopg import sql

from loop_research.data import wrds
from loop_research.data.fetch_http import FetchError
from loop_research.data.licensed_config import WrdsRequest
from loop_research.data.licensed_ingestion import fetch_licensed, replay_licensed
from loop_research.data.snapshot_models import SnapshotRequest
from loop_research.data.snapshots import build_snapshot, validate_snapshot

_FIXTURE = {
    "host": "127.0.0.1",
    "port": 15433,
    "dbname": "loop_engine_test",
    "user": "loop_engine_test",
    "password": "loop_engine_test_only",
    "sslmode": "require",
}
_COMPOSE_URL = (
    "postgresql://loop_engine_test:loop_engine_test_only@postgres:5432/"
    "loop_engine_test?sslmode=require"
)
if os.environ.get("LOOP_TEST_POSTGRES_URL") == _COMPOSE_URL:
    _FIXTURE.update(host="postgres", port=5432)


@pytest.fixture(scope="module")
def database() -> Iterator[str]:
    if os.environ.get("LOOP_WRDS_TEST_POSTGRES") != "1":
        pytest.skip(
            "explicit local PostgreSQL fixture required; enabled in full workspace/CI gates"
        )
    assert os.environ.get("LOOP_TEST_POSTGRES_URL", "") in {"", _COMPOSE_URL}, (
        "WRDS tests accept only the project's disposable host/Compose fixture"
    )
    name = "loop_wrds_" + uuid.uuid4().hex
    with psycopg.connect(**_FIXTURE, autocommit=True) as admin:
        admin.execute(sql.SQL("CREATE DATABASE {}").format(sql.Identifier(name)))
    try:
        with psycopg.connect(**{**_FIXTURE, "dbname": name}) as connection:
            connection.execute("CREATE SCHEMA crsp")
            connection.execute("CREATE SCHEMA comp")
            connection.execute("""
                CREATE TABLE crsp.stkdlysecuritydata (
                    permno integer, dlycaldt date, dlyprc numeric, dlyprcflg text,
                    dlyvol numeric, dlyret numeric, dlyretx numeric, dlydelflg text
                )
            """)
            connection.execute("""
                INSERT INTO crsp.stkdlysecuritydata VALUES
                    (999999, '2026-08-28', 0.052, 'DP', NULL, -0.6, -0.6, 'Y'),
                    (999999, '2026-08-27', 0.13, 'TR', 1000, 0.01, 0.01, 'N'),
                    (888888, '2026-08-28', 5, 'TR', 1000, 0.01, 0.01, 'N'),
                    (999999, '2026-09-01', 0.02, 'TR', 1000, 0.01, 0.01, 'N')
            """)
            connection.execute("""
                CREATE TABLE comp.fundq (
                    gvkey text, datadate date, fyearq integer, fqtr integer, rdq date,
                    indfmt text, datafmt text, popsrc text, consol text, curcdq text,
                    atq numeric, ltq numeric, ceqq numeric,
                    revtq numeric, niq numeric, cshoq numeric
                )
            """)
            connection.execute("""
                INSERT INTO comp.fundq VALUES
                    ('123456', '2026-08-01', 2026, 2, '2026-08-25', 'INDL', 'STD', 'D', 'C',
                     'USD', 1.123456789012345678, 0.1, 1, 0.3, NULL, 0.5),
                    ('123456', '2026-08-01', 2026, 2, '2026-08-25', 'INDL', 'STD', 'D', 'N',
                     'USD', 9, 9, 9, 9, 9, 9)
            """)
        yield name
    finally:
        # Without FORCE: leaked sessions must fail the test instead of hiding a
        # cancellation/cleanup bug. This database contains only invented rows.
        with psycopg.connect(**_FIXTURE, autocommit=True) as admin:
            admin.execute(sql.SQL("DROP DATABASE {}").format(sql.Identifier(name)))


def connector_for(database: str, connections: list[psycopg.AsyncConnection[Any]]) -> wrds.Connector:
    async def connect(**options: Any) -> psycopg.AsyncConnection[Any]:
        assert options["host"] == wrds.WRDS_HOST and options["port"] == 9737
        assert options["dbname"] == "wrds" and options["sslmode"] == "require"
        assert "default_transaction_read_only=on" in options["options"]
        assert "statement_timeout=" in options["options"] and options["passfile"] == "/dev/null"
        result = await psycopg.AsyncConnection.connect(
            **{**options, **_FIXTURE, "dbname": database}
        )
        connections.append(result)
        return result

    return connect


@pytest.mark.parametrize(
    "profile,identifiers,count",
    [
        ("crsp_ciz_daily_v1", ["999999"], 2),
        ("compustat_fundq_v1", ["123456"], 1),
    ],
)
def test_actual_sql_acquisition_replays(
    tmp_path: Path, database: str, profile: str, identifiers: list[str], count: int
) -> None:
    config, license = license_config(tmp_path, "wrds", profile=profile, identifiers=identifiers)
    store = cache(tmp_path)
    connections: list[psycopg.AsyncConnection[Any]] = []
    report = asyncio.run(
        fetch_licensed(
            config,
            store,
            license,
            environment=ENV,
            now=lambda: OBSERVED,
            connector=connector_for(database, connections),
        )
    )
    assert sum(report.row_counts.values()) == count
    assert replay_licensed(store, report.receipt.sha256) == report
    assert len(connections) == 1 and connections[0].closed
    snapshot = build_snapshot(
        store,
        SnapshotRequest(
            receipts=(report.receipt.sha256,),
            start=config.start,
            through=config.end,
        ),
    )
    assert snapshot.row_count == count
    assert validate_snapshot(store, snapshot.snapshot.sha256) == snapshot


def test_row_limit_rolls_back_and_closes(tmp_path: Path, database: str) -> None:
    config, license = license_config(tmp_path, "wrds", budget={"records": 1, "timeout_seconds": 10})
    store = cache(tmp_path)
    connections: list[psycopg.AsyncConnection[Any]] = []
    with pytest.raises(FetchError, match="record_budget"):
        asyncio.run(
            fetch_licensed(
                config,
                store,
                license,
                environment=ENV,
                now=lambda: OBSERVED,
                connector=connector_for(database, connections),
            )
        )
    assert connections[0].closed and not list(store.iterdir())


def test_read_only_session_rejects_writes(
    tmp_path: Path, database: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    config, license = license_config(tmp_path, "wrds")
    connections: list[psycopg.AsyncConnection[Any]] = []

    def forbidden(_: WrdsRequest) -> tuple[str, tuple[object, ...]]:
        return (
            "WITH changed AS (DELETE FROM crsp.stkdlysecuritydata RETURNING permno) "
            "SELECT permno::text FROM changed",
            (),
        )

    monkeypatch.setattr(wrds, "query", forbidden)
    with pytest.raises(FetchError, match="upstream_unavailable"):
        asyncio.run(
            fetch_licensed(
                config,
                cache(tmp_path),
                license,
                environment=ENV,
                now=lambda: OBSERVED,
                connector=connector_for(database, connections),
            )
        )
    assert connections[0].closed
    with psycopg.connect(**{**_FIXTURE, "dbname": database}) as check:
        assert check.execute("SELECT count(*) FROM crsp.stkdlysecuritydata").fetchone() == (4,)


@pytest.mark.parametrize("cancel", [False, True])
def test_timeout_and_cancel_close_query(
    tmp_path: Path, database: str, monkeypatch: pytest.MonkeyPatch, cancel: bool
) -> None:
    config, license = license_config(
        tmp_path, "wrds", budget={"timeout_seconds": 1 if not cancel else 10}
    )
    connections: list[psycopg.AsyncConnection[Any]] = []
    store = cache(tmp_path)
    entered = asyncio.Event()

    def slow(_: WrdsRequest) -> tuple[str, tuple[object, ...]]:
        entered.set()
        return "SELECT pg_sleep(10)::text", ()

    monkeypatch.setattr(wrds, "query", slow)

    async def run() -> None:
        task = asyncio.create_task(
            fetch_licensed(
                config,
                store,
                license,
                environment=ENV,
                now=lambda: OBSERVED,
                connector=connector_for(database, connections),
            )
        )
        if cancel:
            await asyncio.wait_for(entered.wait(), 3)
            task.cancel()
            with pytest.raises(asyncio.CancelledError):
                await task
        else:
            with pytest.raises(FetchError, match="deadline"):
                await task

    asyncio.run(run())
    assert connections[0].closed and not list(store.iterdir())


def test_missing_projection_fails_without_fallback(
    tmp_path: Path, database: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    config, license = license_config(tmp_path, "wrds")
    connections: list[psycopg.AsyncConnection[Any]] = []
    monkeypatch.setattr(
        wrds, "query", lambda _: ("SELECT unavailable FROM crsp.stkdlysecuritydata", ())
    )
    with pytest.raises(FetchError, match="invalid_response"):
        asyncio.run(
            fetch_licensed(
                config,
                cache(tmp_path),
                license,
                environment=ENV,
                now=lambda: OBSERVED,
                connector=connector_for(database, connections),
            )
        )
    assert connections[0].closed
