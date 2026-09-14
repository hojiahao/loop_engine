"""Executable licensed acquisition and cache integrity, using invented wire data."""

import asyncio
import hashlib
import json
import logging
import os
import subprocess
import sys
from datetime import timedelta
from pathlib import Path
from typing import Any

import httpx
import pytest
from data_helpers import OBSERVED, AdvancingClock, response
from licensed_helpers import (
    DB_KEY,
    ENV,
    KEY,
    SEP_ROW,
    cache,
    jsonl,
    license_config,
    reference_row,
    table_bytes,
)

from loop_research.data.fetch_cache import publish, read_cached, read_receipt
from loop_research.data.fetch_http import FetchError
from loop_research.data.licensed_ingestion import fetch_licensed, load_config, replay_licensed
from loop_research.data.licensed_records import LicensedReceipt


def test_sharadar_acquire_replays_offline(tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
    config, license = license_config(tmp_path)
    store = cache(tmp_path)
    requests: list[httpx.Request] = []

    def handle(request: httpx.Request) -> httpx.Response:
        requests.append(request)
        assert request.url.host == "data.nasdaq.com"
        assert request.url.params["api_key"] == KEY
        assert request.url.params["date.gte"] == "2026-08-01"
        assert request.url.params["date.lte"] == "2026-08-31"
        if "qopts.cursor_id" not in request.url.params:
            return response(table_bytes(cursor="page2"))
        assert request.url.params["qopts.cursor_id"] == "page2"
        row = [*SEP_ROW]
        row[1] = "2026-08-31"
        return response(table_bytes(rows=[row]))

    with caplog.at_level(logging.INFO, logger="httpx"):
        report = asyncio.run(
            fetch_licensed(
                config,
                store,
                license,
                environment=ENV,
                transport=httpx.MockTransport(handle),
                now=lambda: OBSERVED,
                monotonic=AdvancingClock(),
            )
        )
    assert len(requests) == 2
    assert report.row_counts == {"SHARADAR/SEP": 2}
    assert report.quality == "licensed_source_unverified"
    assert replay_licensed(store, report.receipt.sha256) == report
    assert KEY not in caplog.text
    for file in store.iterdir():
        assert KEY.encode() not in file.read_bytes()
    receipt = LicensedReceipt.model_validate_json(read_cached(store, report.receipt))
    batch = json.loads(read_cached(store, receipt.normalized))
    assert "split_adjusted_ohlcv" in batch["tables"][0]["semantics"]
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "loop_research.cli",
            "data-verify",
            "--store",
            str(store),
            "--receipt",
            report.receipt.sha256,
        ],
        capture_output=True,
        text=True,
        timeout=20,
    )
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout) == report.model_dump(mode="json")


@pytest.mark.parametrize("provider", ["sharadar", "wrds", "databento"])
def test_missing_credentials_deny_before_io(tmp_path: Path, provider: str) -> None:
    config, license = license_config(tmp_path, provider)
    store = cache(tmp_path)
    with pytest.raises(FetchError, match="missing_credentials"):
        asyncio.run(fetch_licensed(config, store, license, environment={}, now=lambda: OBSERVED))
    assert not list(store.iterdir())


@pytest.mark.parametrize(
    "change",
    [
        {"expires_at": "2026-09-12T00:00:00Z"},
        {"valid_from": "2026-09-14T00:00:00Z"},
        {"data_end": "2026-08-30"},
        {"datasets": ["SHARADAR/SF1"]},
        {"provider": "wrds"},
        {"local_storage": False},
        {"billing": "pay_as_you_go"},
    ],
)
def test_license_denies_before_network(tmp_path: Path, change: dict[str, Any]) -> None:
    config, license = license_config(tmp_path, license_changes=change)
    store = cache(tmp_path)
    with pytest.raises(FetchError, match="license_denied"):
        asyncio.run(fetch_licensed(config, store, license, environment=ENV, now=lambda: OBSERVED))
    assert not list(store.iterdir())


@pytest.mark.parametrize("mutation", ["mode", "digest", "symlink", "duplicate_field"])
def test_license_file_is_private_and_pinned(tmp_path: Path, mutation: str) -> None:
    config, license = license_config(tmp_path)
    if mutation == "mode":
        license.chmod(0o644)
    elif mutation == "symlink":
        link = tmp_path / "link.json"
        link.symlink_to(license)
        license = link
    else:
        license.write_bytes(license.read_bytes() + (b" " if mutation == "digest" else b'"bad"'))
    with pytest.raises(FetchError, match="license_denied"):
        asyncio.run(
            fetch_licensed(config, cache(tmp_path), license, environment=ENV, now=lambda: OBSERVED)
        )


@pytest.mark.parametrize(
    "status,reason",
    [(401, "authentication"), (403, "forbidden"), (429, "rate_limited"), (302, "invalid_response")],
)
def test_entitlement_errors_do_not_commit(tmp_path: Path, status: int, reason: str) -> None:
    config, license = license_config(tmp_path)
    store = cache(tmp_path)
    with pytest.raises(FetchError, match=reason):
        asyncio.run(
            fetch_licensed(
                config,
                store,
                license,
                environment=ENV,
                now=lambda: OBSERVED,
                transport=httpx.MockTransport(lambda _: response(KEY.encode(), status)),
            )
        )
    assert not list(store.iterdir())


@pytest.mark.parametrize(
    "variant,reason",
    [
        ("repeated", "invalid_response"),
        ("pages", "page_budget"),
        ("rows", "record_budget"),
        ("secret", "invalid_response"),
    ],
)
def test_bad_pagination_has_no_receipt(tmp_path: Path, variant: str, reason: str) -> None:
    config, license = license_config(
        tmp_path,
        budget={
            "pages": 1 if variant == "pages" else 3,
            "records": 1 if variant == "rows" else 100,
            "retries": 0,
            "timeout_seconds": 180,
        },
    )
    store = cache(tmp_path)
    payload = (
        table_bytes(rows=[SEP_ROW, SEP_ROW])
        if variant == "rows"
        else table_bytes(cursor=KEY if variant == "secret" else "same-cursor")
    )
    with pytest.raises(FetchError, match=reason):
        asyncio.run(
            fetch_licensed(
                config,
                store,
                license,
                environment=ENV,
                now=lambda: OBSERVED,
                monotonic=AdvancingClock(),
                transport=httpx.MockTransport(lambda _: response(payload)),
            )
        )
    assert not any(b'"loop.licensed-receipt/v1"' in file.read_bytes() for file in store.iterdir())


def test_expiry_during_download_does_not_commit(tmp_path: Path) -> None:
    config, license = license_config(
        tmp_path, license_changes={"expires_at": "2026-09-13T12:00:01Z"}
    )
    store = cache(tmp_path)
    clock = [OBSERVED]

    def handle(_: httpx.Request) -> httpx.Response:
        clock[0] += timedelta(seconds=2)
        return response(table_bytes())

    with pytest.raises(FetchError, match="license_denied"):
        asyncio.run(
            fetch_licensed(
                config,
                store,
                license,
                environment=ENV,
                now=lambda: clock[0],
                transport=httpx.MockTransport(handle),
            )
        )
    assert not any(b'"loop.licensed-receipt/v1"' in file.read_bytes() for file in store.iterdir())


@pytest.mark.parametrize("empty", [False, True])
def test_databento_does_not_allocate_or_drop_vintages(tmp_path: Path, empty: bool) -> None:
    config, license = license_config(tmp_path, "databento")
    store = cache(tmp_path)
    body = b"" if empty else jsonl(reference_row(), reference_row(ts_record="2026-08-30T12:00:00Z"))

    def handle(request: httpx.Request) -> httpx.Response:
        assert request.method == "POST" and request.url.path == "/v0/security_master.get_range"
        params = httpx.QueryParams(request.content.decode())
        assert params["allocate_isins"] == "false" and params["compression"] == "none"
        assert params["stype_in"] == "listing_id" and params["symbols"] == "L-999999"
        assert "pit" not in params
        return response(body)

    report = asyncio.run(
        fetch_licensed(
            config,
            store,
            license,
            environment={"LOOP_TEST_KEY": DB_KEY},
            now=lambda: OBSERVED,
            transport=httpx.MockTransport(handle),
        )
    )
    assert report.row_counts == {"databento/security_master": 0 if empty else 2}
    assert report.allocation_filter == "existing_isins_only"
    assert replay_licensed(store, report.receipt.sha256) == report


@pytest.mark.parametrize("target", ["source", "normalized", "request", "missing_page", "license"])
def test_replay_rechecks_evidence(tmp_path: Path, target: str) -> None:
    config, license = license_config(tmp_path)
    store = cache(tmp_path)
    report = asyncio.run(
        fetch_licensed(
            config,
            store,
            license,
            environment=ENV,
            now=lambda: OBSERVED,
            transport=httpx.MockTransport(lambda _: response(table_bytes())),
        )
    )
    _, content = read_receipt(store, report.receipt.sha256)
    receipt = json.loads(content)
    if target in {"source", "normalized"}:
        ref = receipt["captures"][0]["content"] if target == "source" else receipt["normalized"]
        (store / ref["sha256"][7:]).write_bytes(b"{}")
        digest = report.receipt.sha256
    else:
        if target == "request":
            receipt["captures"][0]["request"] = "https://data.nasdaq.com/foreign"
        elif target == "missing_page":
            receipt["captures"] = []
        else:
            receipt["license"] = receipt["config"]
        digest = publish(store, json.dumps(receipt).encode()).sha256
    with pytest.raises(FetchError, match="invalid_cache"):
        replay_licensed(store, digest)


def test_cancelled_operation_closes_without_receipt(tmp_path: Path) -> None:
    config, license = license_config(tmp_path)
    store = cache(tmp_path)

    async def run() -> None:
        entered = asyncio.Event()

        async def handle(_: httpx.Request) -> httpx.Response:
            entered.set()
            await asyncio.Event().wait()
            raise AssertionError("cancelled request resumed")

        task = asyncio.create_task(
            fetch_licensed(
                config,
                store,
                license,
                environment=ENV,
                now=lambda: OBSERVED,
                transport=httpx.MockTransport(handle),
            )
        )
        await asyncio.wait_for(entered.wait(), 2)
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task

    asyncio.run(run())
    assert not list(store.iterdir())


def test_config_and_actual_cli_gate(tmp_path: Path) -> None:
    config, license = license_config(tmp_path)
    file = tmp_path / "request.toml"
    file.write_text(
        'schema = "loop.licensed-fetch/v1"\nprovider = "sharadar"\n'
        f'license_sha256 = "{config.license_sha256}"\nstart = 2026-08-01\nend = 2026-08-31\n'
        'key_reference = "LOOP_MISSING_SYNTHETIC_KEY"\nsymbols = ["DEMO"]\ntables = ["SEP"]\n'
    )
    assert load_config(file).provider == "sharadar"
    environment = {
        name: value for name, value in os.environ.items() if name != "LOOP_MISSING_SYNTHETIC_KEY"
    }
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "loop_research.cli",
            "data-acquire",
            str(file),
            "--license",
            str(license),
            "--store",
            str(cache(tmp_path)),
        ],
        env=environment,
        capture_output=True,
        text=True,
        timeout=20,
    )
    assert result.returncode == 2
    assert "missing_credentials" in result.stderr and KEY not in result.stderr
    assert hashlib.sha256(license.read_bytes()).hexdigest() == config.license_sha256[7:]


def test_current_metadata_requires_declared_rights(tmp_path: Path) -> None:
    config, license = license_config(
        tmp_path, tables=["TICKERS"], license_changes={"current_reference_metadata": False}
    )
    with pytest.raises(FetchError, match="license_denied"):
        asyncio.run(
            fetch_licensed(config, cache(tmp_path), license, environment=ENV, now=lambda: OBSERVED)
        )


def test_databento_warning_does_not_silently_accept_partial_data(tmp_path: Path) -> None:
    config, license = license_config(tmp_path, "databento")
    store = cache(tmp_path)
    with pytest.raises(FetchError, match="invalid_response"):
        asyncio.run(
            fetch_licensed(
                config,
                store,
                license,
                environment={"LOOP_TEST_KEY": DB_KEY},
                now=lambda: OBSERVED,
                transport=httpx.MockTransport(
                    lambda _: response(b"", **{"X-Warning": '["BentoWarning: partial result"]'})
                ),
            )
        )
    assert not list(store.iterdir())


def test_secret_echo_inside_jsonl_is_not_cached(tmp_path: Path) -> None:
    config, license = license_config(tmp_path, "databento")
    store = cache(tmp_path)
    body = jsonl(reference_row(), reference_row(nasdaq_symbol=DB_KEY))
    with pytest.raises(FetchError, match="invalid_response"):
        asyncio.run(
            fetch_licensed(
                config,
                store,
                license,
                environment={"LOOP_TEST_KEY": DB_KEY},
                now=lambda: OBSERVED,
                transport=httpx.MockTransport(lambda _: response(body)),
            )
        )
    assert not list(store.iterdir())


def test_rehashed_normalization_is_still_checked(tmp_path: Path) -> None:
    config, license = license_config(tmp_path)
    store = cache(tmp_path)
    report = asyncio.run(
        fetch_licensed(
            config,
            store,
            license,
            environment=ENV,
            now=lambda: OBSERVED,
            transport=httpx.MockTransport(lambda _: response(table_bytes())),
        )
    )
    receipt = json.loads(read_cached(store, report.receipt))
    changed = json.loads(
        read_cached(
            store,
            LicensedReceipt.model_validate_json(read_cached(store, report.receipt)).normalized,
        )
    )
    changed["tables"][0]["rows"][0][5] = "500"
    receipt["normalized"] = publish(store, json.dumps(changed).encode()).model_dump()
    digest = publish(store, json.dumps(receipt).encode()).sha256
    with pytest.raises(FetchError, match="invalid_cache"):
        replay_licensed(store, digest)
