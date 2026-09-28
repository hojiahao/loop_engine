"""Bounded development fetch and offline, byte-verified cache replay."""

import asyncio
import json
import os
import time
import tomllib
from collections.abc import Callable, Mapping
from datetime import UTC, date, datetime
from pathlib import Path
from typing import Literal
from zoneinfo import ZoneInfo

import httpx

from loop_research.data.fetch_cache import (
    private_directory,
    publish,
    read_cached,
    read_config_bytes,
    read_receipt,
)
from loop_research.data.fetch_config import FETCH_CONFIG, AlpacaRequest, FetchConfig, SecRequest
from loop_research.data.fetch_http import BoundedHttp, Download, FetchError
from loop_research.data.fetch_json import contains_secret, decode_object
from loop_research.data.fetch_records import (
    CachedObject,
    CapturedResponse,
    DevelopmentBatch,
    FetchReceipt,
    FetchReport,
)
from loop_research.data.sec import normalize_sec


def load_fetch_config(path: Path) -> FetchConfig:
    """Load strict TOML without endpoints or secret values; invalid input is redacted."""
    try:
        values = tomllib.loads(read_config_bytes(path).decode("utf-8"))
        # TOML dates are first-class values. JSON-mode validation also checks
        # arrays/dates against the immutable, strict domain model.
        return FETCH_CONFIG.validate_json(json.dumps(values, default=_toml_date, allow_nan=False))
    except OSError, ValueError, TypeError:
        raise FetchError("invalid_configuration") from None


def _toml_date(value: object) -> str:
    if type(value) is date:
        return value.isoformat()
    raise ValueError("unsupported TOML value")


def _headers(config: FetchConfig, environment: Mapping[str, str]) -> dict[str, str]:
    if isinstance(config, SecRequest):
        return {"User-Agent": "Loop Engine/0.2 " + config.contact_email}
    credentials = [
        environment.get(config.key_id_reference),
        environment.get(config.secret_key_reference),
    ]
    if any(
        not value or len(value) > 1024 or not value.isascii() or not value.isprintable()
        for value in credentials
    ):
        raise FetchError("missing_credentials")
    return {
        "APCA-API-KEY-ID": credentials[0] or "",
        "APCA-API-SECRET-KEY": credentials[1] or "",
    }


async def fetch_data(
    config: FetchConfig,
    store: Path,
    *,
    transport: httpx.AsyncBaseTransport | None = None,
    environment: Mapping[str, str] | None = None,
    now: Callable[[], datetime] = lambda: datetime.now(UTC),
    monotonic: Callable[[], float] = time.monotonic,
) -> FetchReport:
    """Acquire selected development data and publish a success receipt last.

    Credentials are checked before network/file writes. Every successful source
    response is preserved without replacement. A receipt is published only after
    validation; cancellation after that commit may prevent acknowledgment. No registry, DB, broker
    order, holdout grant or factor admission is mutated. Test transports are a
    Python dependency injection point, never a CLI/configurable network bypass.
    """
    config = FETCH_CONFIG.validate_python(config)
    headers = _headers(config, os.environ if environment is None else environment)
    try:
        os.close(private_directory(store))
    except OSError, ValueError:
        raise FetchError("invalid_cache") from None
    http = BoundedHttp(config.budget, transport=transport, now=now, monotonic=monotonic)
    captured: list[CapturedResponse] = []
    downloads: list[Download] = []

    async def capture(
        kind: Literal[
            "sec_facts", "sec_submissions", "alpaca_asset", "alpaca_bars", "alpaca_sip_probe"
        ],
        key: str,
        url: str,
        params: Mapping[str, str] | None = None,
    ) -> Download:
        value = await http.get(url, params=params, headers=headers)
        if isinstance(config, AlpacaRequest) and (
            contains_secret(value.body, tuple(headers.values()))
            or any(
                secret in value.url or secret in (value.request_id or "")
                for secret in headers.values()
            )
        ):
            raise FetchError("invalid_response")
        http.check()
        reference = _publish(store, value.body)
        captured.append(
            CapturedResponse(
                kind=kind,
                key=key,
                url=value.url,
                observed_at=value.observed_at,
                request_id=value.request_id,
                content=reference,
            )
        )
        downloads.append(value)
        return value

    try:
        async with asyncio.timeout(http.check()):
            started = http.observed_at()
            asof = started.astimezone(ZoneInfo("America/New_York")).date()
            if config.end >= asof:
                raise FetchError("invalid_configuration")
            recent_sip: Literal["not_requested", "response_permitted", "forbidden"] = (
                "not_requested"
            )
            if isinstance(config, SecRequest):
                await capture(
                    "sec_facts",
                    config.cik,
                    f"https://data.sec.gov/api/xbrl/companyfacts/CIK{config.cik}.json",
                )
                await capture(
                    "sec_submissions",
                    config.cik,
                    f"https://data.sec.gov/submissions/CIK{config.cik}.json",
                )
            else:
                from loop_research.data.alpaca import (
                    asset_record,
                    bars_parameters,
                    page_rows,
                    validate_sip_probe,
                )

                host = "paper-api.alpaca.markets" if config.paper else "api.alpaca.markets"
                for symbol in sorted(config.symbols):
                    asset = await capture(
                        "alpaca_asset", symbol, f"https://{host}/v2/assets/{symbol}"
                    )
                    asset_record(symbol, asset)
                parameters = bars_parameters(config, asof)
                cursors: set[str] = set()
                records = 0
                for page_index in range(config.budget.pages):
                    page = await capture(
                        "alpaca_bars",
                        str(page_index),
                        "https://data.alpaca.markets/v2/stocks/bars",
                        parameters,
                    )
                    rows, token = page_rows(page, config.symbols)
                    records += sum(len(values) for values in rows.values())
                    if records > config.budget.records:
                        raise FetchError("record_budget")
                    if token is None:
                        break
                    if token in cursors:
                        raise FetchError("invalid_response")
                    if any(secret in token for secret in headers.values()):
                        raise FetchError("invalid_response")
                    cursors.add(token)
                    parameters = {**parameters, "page_token": token}
                else:
                    raise FetchError("page_budget")
                if config.probe_recent_sip:
                    try:
                        probe = await capture(
                            "alpaca_sip_probe",
                            config.symbols[0],
                            "https://data.alpaca.markets/v2/stocks/trades/latest",
                            {"symbols": config.symbols[0], "feed": "sip", "currency": "USD"},
                        )
                        validate_sip_probe(probe, config.symbols[0])
                        recent_sip = "response_permitted"
                    except FetchError as error:
                        if error.reason != "forbidden":
                            raise
                        recent_sip = "forbidden"
            http.check()
            batch = _normalize(config, captured, downloads, asof, recent_sip)
            normalized = _publish(store, batch.model_dump_json(by_alias=True).encode())
            config_object = _publish(store, config.model_dump_json(by_alias=True).encode())
            receipt = FetchReceipt(
                config=config_object,
                normalized=normalized,
                responses=tuple(captured),
                started_at=started,
                completed_at=http.observed_at(),
                attempts=http.attempts,
                bytes_received=http.bytes_received,
                recent_sip=recent_sip,
            )
            http.check()
            reference = _publish(store, receipt.model_dump_json(by_alias=True).encode())
            return _report(reference, receipt, batch)
    except TimeoutError:
        raise FetchError("deadline") from None
    except FetchError:
        raise
    except ValueError, TypeError, KeyError, OverflowError:
        raise FetchError("invalid_response") from None
    finally:
        await http.close()


def _publish(store: Path, content: bytes) -> CachedObject:
    try:
        return publish(store, content)
    except OSError, ValueError:
        raise FetchError("invalid_cache") from None


def _normalize(
    config: FetchConfig,
    captured: list[CapturedResponse],
    downloads: list[Download],
    asof: date,
    recent_sip: Literal["not_requested", "response_permitted", "forbidden"],
) -> DevelopmentBatch:
    if len(captured) != len(downloads) or config.end >= asof:
        raise ValueError("inconsistent acquisition bounds")
    if isinstance(config, SecRequest):
        if [(record.kind, record.key) for record in captured] != [
            ("sec_facts", config.cik),
            ("sec_submissions", config.cik),
        ] or recent_sip != "not_requested":
            raise ValueError("invalid SEC capture sequence")
        if [record.url for record in captured] != [
            f"https://data.sec.gov/api/xbrl/companyfacts/CIK{config.cik}.json",
            f"https://data.sec.gov/submissions/CIK{config.cik}.json",
        ]:
            raise ValueError("inconsistent SEC source URL")
        return normalize_sec(config, downloads[0], downloads[1])
    from loop_research.data.alpaca import (
        bars_parameters,
        normalize_alpaca,
        page_rows,
        validate_sip_probe,
    )

    count = len(config.symbols)
    if [(record.kind, record.key) for record in captured[:count]] != [
        ("alpaca_asset", symbol) for symbol in sorted(config.symbols)
    ]:
        raise ValueError("invalid Alpaca asset sequence")
    assets = {
        record.key: value for record, value in zip(captured[:count], downloads[:count], strict=True)
    }
    host = "paper-api.alpaca.markets" if config.paper else "api.alpaca.markets"
    if any(record.url != f"https://{host}/v2/assets/{record.key}" for record in captured[:count]):
        raise ValueError("inconsistent Alpaca asset URL")
    pages: list[Download] = []
    parameters = bars_parameters(config, asof)
    cursors: set[str] = set()
    for record, value in zip(captured[count:], downloads[count:], strict=True):
        if record.kind == "alpaca_bars" and record.key == str(len(pages)):
            expected_url = httpx.Request(
                "GET", "https://data.alpaca.markets/v2/stocks/bars", params=parameters
            ).url
            if record.url != str(expected_url):
                raise ValueError("inconsistent Alpaca page parameters")
            _, token = page_rows(value, config.symbols)
            if token is not None:
                if token in cursors:
                    raise ValueError("repeated Alpaca cursor")
                cursors.add(token)
                parameters = {**parameters, "page_token": token}
            pages.append(value)
        elif (
            record == captured[-1]
            and record.kind == "alpaca_sip_probe"
            and record.key == config.symbols[0]
            and config.probe_recent_sip
            and recent_sip == "response_permitted"
        ):
            expected_url = httpx.Request(
                "GET",
                "https://data.alpaca.markets/v2/stocks/trades/latest",
                params={"symbols": config.symbols[0], "feed": "sip", "currency": "USD"},
            ).url
            if record.url != str(expected_url):
                raise ValueError("inconsistent SIP probe parameters")
            validate_sip_probe(value, record.key)
        else:
            raise ValueError("invalid Alpaca page/probe sequence")
    has_probe = any(record.kind == "alpaca_sip_probe" for record in captured)
    if config.probe_recent_sip == (recent_sip == "not_requested") or has_probe != (
        recent_sip == "response_permitted"
    ):
        raise ValueError("invalid recent SIP evidence")
    return normalize_alpaca(config, asof, assets, pages)


def replay_data(store: Path, digest: str) -> FetchReport:
    """Verify all cached bytes and reproduce normalization without HTTP or secrets.

    This verifies local content consistency, not a supplier signature or current
    entitlement. Unsupported schemas, missing/corrupt objects and normalization
    drift fail closed; existing bytes are never rewritten to make replay pass.
    """
    try:
        reference, content = read_receipt(store, digest)
        decode_object(content)
        receipt = FetchReceipt.model_validate_json(content)
        config_bytes = read_cached(store, receipt.config)
        decode_object(config_bytes)
        config = FETCH_CONFIG.validate_json(config_bytes)
        if (
            receipt.attempts > config.budget.requests
            or receipt.bytes_received > config.budget.total_bytes
            or (receipt.completed_at - receipt.started_at).total_seconds()
            > config.budget.timeout_seconds
            or any(
                record.content.byte_size > config.budget.response_bytes
                for record in receipt.responses
            )
        ):
            raise ValueError("receipt exceeds acquisition budgets")
        downloads = [
            Download(
                record.url,
                read_cached(store, record.content),
                record.observed_at,
                record.request_id,
            )
            for record in receipt.responses
        ]
        asof = receipt.started_at.astimezone(ZoneInfo("America/New_York")).date()
        batch = _normalize(config, list(receipt.responses), downloads, asof, receipt.recent_sip)
        if read_cached(store, receipt.normalized) != batch.model_dump_json(by_alias=True).encode():
            raise ValueError("normalized records differ from original evidence")
        return _report(reference, receipt, batch)
    except OSError, ValueError, TypeError, KeyError, OverflowError:
        raise FetchError("invalid_cache") from None


def _report(reference: CachedObject, receipt: FetchReceipt, batch: DevelopmentBatch) -> FetchReport:
    return FetchReport(
        receipt=reference,
        provider=batch.provider,
        result="records" if batch.bars or batch.fundamentals else "empty",
        bar_count=len(batch.bars),
        fundamental_count=len(batch.fundamentals),
        missing=batch.missing,
        recent_sip=receipt.recent_sip,
    )
