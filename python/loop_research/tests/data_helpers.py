"""Synthetic wire fixtures shared only by development-ingestion tests."""

import json
from collections.abc import AsyncIterator, Callable
from datetime import UTC, datetime
from pathlib import Path

import httpx

from loop_research.data.fetch_config import FETCH_CONFIG, AlpacaRequest, SecRequest
from loop_research.data.fetch_http import Download

FIXTURES = Path(__file__).resolve().parents[3] / "fixtures/market/development"
OBSERVED = datetime(2026, 9, 13, 12, tzinfo=UTC)
ENVIRONMENT = {"LOOP_TEST_KEY": "fixture-key-123456", "LOOP_TEST_SECRET": "fixture-secret-abcdef"}


class AdvancingClock:
    """Advance between checks to avoid real throttle sleeps in codec tests."""

    def __init__(self) -> None:
        self.tick = 0.0

    def __call__(self) -> float:
        self.tick += 1.0
        return self.tick


class WireBytes(httpx.AsyncByteStream):
    def __init__(self, content: bytes) -> None:
        self.content = content
        self.closed = False

    async def __aiter__(self) -> AsyncIterator[bytes]:
        yield self.content

    async def aclose(self) -> None:
        self.closed = True


def response(content: bytes, status: int = 200, **headers: str) -> httpx.Response:
    return httpx.Response(
        status, headers={"Content-Type": "application/json", **headers}, stream=WireBytes(content)
    )


def fixture(name: str) -> bytes:
    return (FIXTURES / (name + ".json")).read_bytes()


def sec_config(**changes: object) -> SecRequest:
    values = {
        "schema": "loop.development-fetch/v1",
        "provider": "sec",
        "cik": "0001234567",
        "concepts": ["us-gaap:Assets", "us-gaap:Revenues"],
        "contact_email": "test@example.org",
        "start": "2026-01-01",
        "end": "2026-08-31",
        "budget": {"retries": 0, "timeout_seconds": 180},
        **changes,
    }
    config = FETCH_CONFIG.validate_json(json.dumps(values))
    assert isinstance(config, SecRequest)
    return config


def alpaca_config(**changes: object) -> AlpacaRequest:
    values = {
        "schema": "loop.development-fetch/v1",
        "provider": "alpaca",
        "symbols": ["DEMO"],
        "start": "2026-08-28",
        "end": "2026-08-31",
        "feed": "iex",
        "identity_basis": "current_asset",
        "key_id_reference": "LOOP_TEST_KEY",
        "secret_key_reference": "LOOP_TEST_SECRET",
        "probe_recent_sip": True,
        "budget": {"retries": 0, "timeout_seconds": 180},
        **changes,
    }
    config = FETCH_CONFIG.validate_json(json.dumps(values))
    assert isinstance(config, AlpacaRequest)
    return config


def download(name: str, content: bytes | None = None) -> Download:
    return Download(
        "https://fixture.invalid/unused",
        fixture(name) if content is None else content,
        OBSERVED,
        None,
    )


def fixture_handler(requests: list[httpx.Request]) -> Callable[[httpx.Request], httpx.Response]:
    def handle(request: httpx.Request) -> httpx.Response:
        requests.append(request)
        if "/companyfacts/" in request.url.path:
            return response(fixture("sec-facts"))
        if "/submissions/" in request.url.path:
            return response(fixture("sec-submissions"))
        if "/assets/" in request.url.path:
            return response(fixture("alpaca-asset"))
        if request.url.path.endswith("/bars"):
            page = "alpaca-page-2" if "page_token" in request.url.params else "alpaca-page-1"
            return response(fixture(page))
        if request.url.path.endswith("/trades/latest"):
            return response(fixture("alpaca-latest"))
        raise AssertionError("unexpected synthetic request")

    return handle
