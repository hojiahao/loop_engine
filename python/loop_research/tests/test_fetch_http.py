"""HTTP boundaries, retry accounting and real async cancellation paths."""

import asyncio
from collections.abc import AsyncIterator
from datetime import timedelta

import httpx
import pytest
from data_helpers import OBSERVED, AdvancingClock, WireBytes, response

from loop_research.data.fetch_config import FetchBudget
from loop_research.data.fetch_http import BoundedHttp, FetchError

URL = "https://data.sec.gov/submissions/CIK0001234567.json"
HEADERS = {"User-Agent": "Loop Engine test@example.org"}


@pytest.mark.parametrize(
    ("status", "reason"),
    [
        (401, "authentication"),
        (403, "forbidden"),
        (404, "not_found"),
        (429, "rate_limited"),
        (503, "upstream_unavailable"),
        (301, "invalid_response"),
    ],
)
def test_http_errors_are_static(status: int, reason: str) -> None:
    async def run() -> None:
        http = BoundedHttp(
            FetchBudget(retries=0),
            transport=httpx.MockTransport(lambda _: response(b"secret", status)),
            monotonic=AdvancingClock(),
            now=lambda: OBSERVED,
        )
        try:
            with pytest.raises(FetchError, match=reason) as caught:
                await http.get(URL, headers=HEADERS)
            assert str(caught.value) == reason
            assert caught.value.status_code == status
            assert http.attempts == 1 and http.bytes_received == 0
        finally:
            await http.close()

    asyncio.run(run())


@pytest.mark.parametrize(
    "url",
    [
        "http://data.sec.gov/submissions/CIK0001234567.json",
        "https://localhost/private",
        "https://secret@data.sec.gov/submissions/CIK0001234567.json",
        URL + "?token=secret",
        URL + "#fragment",
        "https://data.sec.gov:8443/other",
        "https://data.sec.gov/not-an-api",
        "https://paper-api.alpaca.markets/v2/orders",
    ],
)
def test_routes_deny_before_transport(url: str) -> None:
    async def run() -> None:
        calls = []
        http = BoundedHttp(
            FetchBudget(), transport=httpx.MockTransport(lambda request: calls.append(request))
        )
        try:
            with pytest.raises(FetchError, match="invalid_configuration"):
                await http.get(url, headers=HEADERS)
            assert not calls and http.attempts == 0
        finally:
            await http.close()

    asyncio.run(run())


@pytest.mark.parametrize(
    "headers", [{}, {"Authorization": "secret"}, {"User-Agent": "bad\nheader"}]
)
def test_wrong_headers_are_denied(headers: dict[str, str]) -> None:
    async def run() -> None:
        http = BoundedHttp(FetchBudget())
        try:
            with pytest.raises(FetchError, match="invalid_configuration"):
                await http.get(URL, headers=headers)
            assert http.attempts == 0
        finally:
            await http.close()

    asyncio.run(run())


def test_retry_counts_attempts_and_preserves_raw_bytes() -> None:
    async def run() -> None:
        calls: list[httpx.Request] = []

        def handle(request: httpx.Request) -> httpx.Response:
            calls.append(request)
            assert request.headers["accept-encoding"] == "identity"
            assert "cookie" not in request.headers
            if len(calls) == 1:
                return response(b"unread", 429, **{"retry-after": "0"})
            return response(b'{ "x":1 }', **{"set-cookie": "ignored=yes"})

        http = BoundedHttp(
            FetchBudget(retries=1),
            transport=httpx.MockTransport(handle),
            monotonic=AdvancingClock(),
            now=lambda: OBSERVED,
        )
        try:
            first = await http.get(URL, headers=HEADERS)
            await http.get(URL, headers=HEADERS)
            assert first.body == b'{ "x":1 }'
            assert http.attempts == 3 and http.bytes_received == 18
        finally:
            await http.close()

    asyncio.run(run())


@pytest.mark.parametrize("retry_after", ["100000000000", "Wed, 21 Oct 2026 07:28:00 GMT", "-1"])
@pytest.mark.parametrize("status,reason", [(429, "rate_limited"), (503, "upstream_unavailable")])
def test_unknown_retry_delay_fails_closed(retry_after: str, status: int, reason: str) -> None:
    async def run() -> None:
        http = BoundedHttp(
            FetchBudget(),
            transport=httpx.MockTransport(
                lambda _: response(b"", status, **{"retry-after": retry_after})
            ),
        )
        try:
            with pytest.raises(FetchError, match=reason):
                await http.get(URL, headers=HEADERS)
            assert http.attempts == 1
        finally:
            await http.close()

    asyncio.run(run())


@pytest.mark.parametrize(
    ("body", "headers", "reason"),
    [
        (b"{}", {"content-type": "text/html"}, "invalid_response"),
        (b"{}", {"content-encoding": "gzip"}, "invalid_response"),
        (b"{}", {"content-length": "3"}, "invalid_response"),
        (b"{}", {"content-length": "100000"}, "byte_budget"),
        (b"x" * 1025, {}, "byte_budget"),
        (b"", {}, "invalid_response"),
        (b"{}", {"x-request-id": "x" * 161}, "invalid_response"),
    ],
)
def test_response_shape_and_bytes(body: bytes, headers: dict[str, str], reason: str) -> None:
    async def run() -> None:
        wire = response(body, **headers)
        http = BoundedHttp(
            FetchBudget(response_bytes=1024), transport=httpx.MockTransport(lambda _: wire)
        )
        try:
            with pytest.raises(FetchError, match=reason):
                await http.get(URL, headers=HEADERS)
            assert wire.is_closed
        finally:
            await http.close()

    asyncio.run(run())


@pytest.mark.parametrize(
    ("budget", "reason"),
    [
        (FetchBudget(requests=1), "request_budget"),
        (FetchBudget(response_bytes=1024, total_bytes=1024), "byte_budget"),
    ],
)
def test_budgets_apply_across_requests(budget: FetchBudget, reason: str) -> None:
    async def run() -> None:
        http = BoundedHttp(
            budget,
            transport=httpx.MockTransport(lambda _: response(b"x" * 600)),
            monotonic=AdvancingClock(),
            now=lambda: OBSERVED,
        )
        try:
            await http.get(URL, headers=HEADERS)
            with pytest.raises(FetchError, match=reason):
                await http.get(URL, headers=HEADERS)
        finally:
            await http.close()

    asyncio.run(run())


@pytest.mark.parametrize("kind", ["monotonic", "wall", "nan"])
def test_clock_regression_is_denied(kind: str) -> None:
    async def run() -> None:
        ticks = iter([2.0, float("nan") if kind == "nan" else 1.0 if kind == "monotonic" else 3.0])
        walls = iter([OBSERVED, OBSERVED - timedelta(seconds=1) if kind == "wall" else OBSERVED])
        http = BoundedHttp(FetchBudget(), monotonic=lambda: next(ticks), now=lambda: next(walls))
        try:
            with pytest.raises(FetchError, match="clock_regression"):
                await http.get(URL, headers=HEADERS)
            assert http.attempts == 0
        finally:
            await http.close()

    asyncio.run(run())


def test_server_delay_larger_than_deadline_is_not_shortened() -> None:
    async def run() -> None:
        http = BoundedHttp(
            FetchBudget(timeout_seconds=1),
            transport=httpx.MockTransport(lambda _: response(b"", 429, **{"retry-after": "30"})),
        )
        try:
            with pytest.raises(FetchError, match="deadline"):
                await http.get(URL, headers=HEADERS)
            assert http.attempts == 1
        finally:
            await http.close()

    asyncio.run(run())


def test_stalled_stream_times_out_and_closes() -> None:
    async def run() -> None:
        class Stalled(WireBytes):
            async def __aiter__(self) -> AsyncIterator[bytes]:
                await asyncio.Event().wait()
                yield b"{}"

        stream = Stalled(b"")
        http = BoundedHttp(
            FetchBudget(timeout_seconds=1),
            transport=httpx.MockTransport(
                lambda _: httpx.Response(
                    200, headers={"content-type": "application/json"}, stream=stream
                )
            ),
        )
        try:
            with pytest.raises(FetchError, match="deadline"):
                await http.get(URL, headers=HEADERS)
            assert stream.closed
        finally:
            await http.close()

    asyncio.run(run())


def test_concurrent_callers_share_one_sequential_budget() -> None:
    async def run() -> None:
        active, maximum = 0, 0

        async def handle(_: httpx.Request) -> httpx.Response:
            nonlocal active, maximum
            active += 1
            maximum = max(maximum, active)
            await asyncio.sleep(0.01)
            active -= 1
            return response(b"{}")

        http = BoundedHttp(
            FetchBudget(),
            transport=httpx.MockTransport(handle),
            monotonic=AdvancingClock(),
            now=lambda: OBSERVED,
        )
        try:
            await asyncio.gather(*(http.get(URL, headers=HEADERS) for _ in range(3)))
            assert maximum == 1 and http.attempts == 3
        finally:
            await http.close()

    asyncio.run(run())
