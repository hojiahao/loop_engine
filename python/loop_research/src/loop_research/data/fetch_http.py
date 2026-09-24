"""Sequential HTTPS reads with shared operation budgets and static error reasons."""

from __future__ import annotations

import asyncio
import logging
import math
import re
import time
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Literal

import httpx

from loop_research.data.fetch_config import FetchBudget


class _QueryKeyRedaction(logging.Filter):
    """Remove Data Link query credentials from HTTPX's normal request log."""

    def filter(self, record: logging.LogRecord) -> bool:
        def redact(value: object) -> object:
            if isinstance(value, httpx.URL):
                return value.copy_remove_param("api_key")
            if isinstance(value, str):
                return re.sub(r"api_key=[^&\s\"']+", "api_key=REDACTED", value)
            return value

        if isinstance(record.args, tuple):
            record.args = tuple(redact(value) for value in record.args)
        record.msg = redact(record.msg)
        return True


logging.getLogger("httpx").addFilter(_QueryKeyRedaction())

type FailureReason = Literal[
    "missing_credentials",
    "authentication",
    "forbidden",
    "not_found",
    "rate_limited",
    "upstream_unavailable",
    "invalid_response",
    "request_budget",
    "byte_budget",
    "record_budget",
    "page_budget",
    "deadline",
    "clock_regression",
    "invalid_cache",
    "invalid_configuration",
    "identity_unresolved",
    "license_denied",
]


class FetchError(ValueError):
    """An ingestion failure, never a factor rejection; payloads are not messages."""

    def __init__(self, reason: FailureReason, status_code: int | None = None) -> None:
        super().__init__(reason)
        self.reason = reason
        self.status_code = status_code


@dataclass(frozen=True, slots=True)
class Download:
    """Original successful response bytes with a local observation timestamp."""

    url: str
    body: bytes
    observed_at: datetime
    request_id: str | None


def _utc_now() -> datetime:
    return datetime.now(UTC)


class BoundedHttp:
    """Own one operation's HTTPS pool, attempts and bytes; do not share secrets.

    Only adapters supply routes/headers. The global host gate is checked before
    constructing a request. No redirects, cookies, ambient credentials, proxy
    configuration or compression negotiation are accepted. A caller wraps the
    whole async operation in asyncio.timeout; each network phase also has a
    timeout and every request/read checks the remaining shared budget.
    """

    def __init__(
        self,
        budget: FetchBudget,
        *,
        transport: httpx.AsyncBaseTransport | None = None,
        monotonic: Callable[[], float] = time.monotonic,
        now: Callable[[], datetime] = _utc_now,
    ) -> None:
        self.budget = budget
        self._monotonic = monotonic
        self._now = now
        self._started = monotonic()
        self._last_tick = self._started
        self._last_wall = now()
        if not math.isfinite(self._started) or self._last_wall.utcoffset() is None:
            raise FetchError("clock_regression")
        self._last_wall = self._last_wall.astimezone(UTC)
        self._last_request: float | None = None
        self._lock = asyncio.Lock()
        self.attempts = 0
        self.bytes_received = 0
        self.client = httpx.AsyncClient(
            transport=transport,
            trust_env=False,
            follow_redirects=False,
            limits=httpx.Limits(max_connections=1, max_keepalive_connections=0),
            headers={"Accept": "application/json", "Accept-Encoding": "identity"},
        )

    def check(self) -> float:
        """Check monotonic and public clocks before exposing another result."""
        tick = self._monotonic()
        wall = self._now()
        if (
            not math.isfinite(tick)
            or tick < self._last_tick
            or wall.utcoffset() is None
            or wall.astimezone(UTC) < self._last_wall
        ):
            raise FetchError("clock_regression")
        self._last_tick, self._last_wall = tick, wall.astimezone(UTC)
        remaining = self.budget.timeout_seconds - (tick - self._started)
        if remaining <= 0:
            raise FetchError("deadline")
        return remaining

    def observed_at(self) -> datetime:
        self.check()
        return self._last_wall.astimezone(UTC)

    async def close(self) -> None:
        """Close all response/pool state on success, failure or cancellation."""
        await self.client.aclose()

    async def _pace(self, extra_delay: float) -> None:
        remaining = self.check()
        delay = extra_delay
        if self._last_request is not None:
            delay = max(
                delay, self.budget.interval_seconds - (self._last_tick - self._last_request)
            )
        if delay >= remaining:
            raise FetchError("deadline")
        if delay > 0:
            await asyncio.sleep(delay)
        self.check()

    async def get(
        self,
        url: str,
        *,
        params: Mapping[str, str] | None = None,
        headers: Mapping[str, str] | None = None,
    ) -> Download:
        """Read one pinned GET with bounded retries; never return an error body.

        Attempts and received bytes count even if the operation ultimately
        fails. Authentication, forbidden/missing resources, redirects, content
        schema and budgets do not retry. Cancellation propagates to the caller.
        """
        if self.client.is_closed:
            raise FetchError("invalid_configuration")
        try:
            async with asyncio.timeout(self.check()):
                async with self._lock:
                    return await self._request(url, params=params, headers=headers)
        except TimeoutError:
            raise FetchError("deadline") from None

    async def _request(
        self,
        url: str,
        *,
        params: Mapping[str, str] | None,
        headers: Mapping[str, str] | None,
        method: Literal["GET", "POST"] = "GET",
    ) -> Download:
        parsed = httpx.URL(url)
        if (
            parsed.scheme != "https"
            or parsed.username
            or parsed.password
            or parsed.port not in (None, 443)
            or parsed.fragment
            or parsed.query
            or parsed.host
            not in {
                "data.sec.gov",
                "data.alpaca.markets",
                "api.alpaca.markets",
                "paper-api.alpaca.markets",
                "data.nasdaq.com",
                "hist.databento.com",
            }
        ):
            raise FetchError("invalid_configuration")
        reference = parsed.host == "hist.databento.com"
        if (method == "POST") != reference:
            raise FetchError("invalid_configuration")
        if parsed.host == "data.nasdaq.com":
            valid_path = (
                re.fullmatch(
                    r"/api/v3/datatables/SHARADAR/(?:SEP|SF1|TICKERS|ACTIONS)\.json", parsed.path
                )
                is not None
            )
            allowed_headers = set()
            key = (params or {}).get("api_key", "")
            if not re.fullmatch(r"[A-Za-z0-9_-]{8,128}", key):
                raise FetchError("missing_credentials")
            if not set(params or {}) <= {
                "api_key",
                "ticker",
                "qopts.columns",
                "qopts.cursor_id",
                "table",
                "dimension",
                "date.gte",
                "date.lte",
                "datekey.gte",
                "datekey.lte",
            }:
                raise FetchError("invalid_configuration")
        elif reference:
            valid_path = parsed.path in {
                "/v0/security_master.get_range",
                "/v0/corporate_actions.get_range",
            }
            allowed_headers = {"authorization"}
            if (params or {}).get("allocate_isins") != "false" or (params or {}).get(
                "compression"
            ) != "none":
                raise FetchError("invalid_configuration")
        elif parsed.host == "data.sec.gov":
            valid_path = (
                re.fullmatch(
                    r"/(?:api/xbrl/companyfacts|submissions)/CIK[0-9]{10}\.json", parsed.path
                )
                is not None
            )
            allowed_headers = {"user-agent"}
        elif parsed.host == "data.alpaca.markets":
            valid_path = parsed.path in {"/v2/stocks/bars", "/v2/stocks/trades/latest"}
            allowed_headers = {"apca-api-key-id", "apca-api-secret-key"}
        else:
            valid_path = (
                re.fullmatch(r"/v2/assets/[A-Z0-9][A-Z0-9.-]{0,31}", parsed.path) is not None
            )
            allowed_headers = {"apca-api-key-id", "apca-api-secret-key"}
        if not valid_path or {name.lower() for name in headers or {}} != allowed_headers:
            raise FetchError("invalid_configuration")
        if any(
            not value or len(value) > 1024 or not value.isascii() or not value.isprintable()
            for value in (headers or {}).values()
        ):
            raise FetchError("invalid_configuration")
        delay = 0.0
        for attempt in range(self.budget.retries + 1):
            await self._pace(delay)
            if self.attempts >= self.budget.requests:
                raise FetchError("request_budget")
            self.attempts += 1
            self._last_request = self._last_tick
            self.client.cookies.clear()
            try:
                async with self.client.stream(
                    method,
                    url,
                    params=params if method == "GET" else None,
                    data=params if method == "POST" else None,
                    headers=headers,
                    timeout=httpx.Timeout(min(10.0, self.check())),
                ) as response:
                    self.check()
                    status = response.status_code
                    if reference and response.headers.get("x-warning") not in (None, "", "[]"):
                        # An unhandled provider warning may describe partial data.
                        # Never log its potentially sensitive free-form content.
                        raise FetchError("invalid_response")
                    if status in (429, 500, 502, 503, 504):
                        if attempt == self.budget.retries:
                            raise FetchError(
                                "rate_limited" if status == 429 else "upstream_unavailable", status
                            )
                        retry_after = response.headers.get("retry-after", "")
                        # A long/unknown server delay is never shortened into a
                        # hammering retry. HTTP-date delays fail closed here.
                        if retry_after and (
                            len(retry_after) > 10
                            or not retry_after.isascii()
                            or not retry_after.isdigit()
                        ):
                            raise FetchError(
                                "rate_limited" if status == 429 else "upstream_unavailable", status
                            )
                        delay = float(min(int(retry_after), 3600)) if retry_after else 1.0
                        continue
                    if status != 200:
                        reasons: dict[int, FailureReason] = {
                            401: "authentication",
                            403: "forbidden",
                            404: "not_found",
                        }
                        raise FetchError(reasons.get(status, "invalid_response"), status)
                    content_type = response.headers.get("content-type", "").split(";", 1)[0]
                    allowed_types = {"application/json"}
                    if reference:
                        allowed_types |= {
                            "application/jsonl",
                            "application/x-ndjson",
                            "application/octet-stream",
                            "text/plain",
                        }
                    if (
                        content_type.strip().lower() not in allowed_types
                        or response.headers.get("content-encoding", "identity").lower()
                        != "identity"
                    ):
                        raise FetchError("invalid_response")
                    declared = response.headers.get("content-length")
                    if declared is not None and (
                        not declared.isascii()
                        or not declared.isdigit()
                        or len(declared) > 10
                        or int(declared) > self.budget.response_bytes
                    ):
                        raise FetchError("byte_budget")
                    content = bytearray()
                    async for chunk in response.aiter_raw(chunk_size=8192):
                        self.check()
                        self.bytes_received += len(chunk)
                        if (
                            len(content) + len(chunk) > self.budget.response_bytes
                            or self.bytes_received > self.budget.total_bytes
                        ):
                            raise FetchError("byte_budget")
                        content.extend(chunk)
                    if (not content and not reference) or (
                        declared is not None and len(content) != int(declared)
                    ):
                        raise FetchError("invalid_response")
                    request_id = response.headers.get("x-request-id")
                    if request_id is not None and (
                        len(request_id) > 160
                        or not request_id.isascii()
                        or not request_id.isprintable()
                    ):
                        raise FetchError("invalid_response")
                    # Nasdaq authenticates in the query. Persist only the public
                    # request identity; exceptions never include request URLs.
                    public = response.request.url.copy_remove_param("api_key")
                    if reference:
                        public = public.copy_merge_params(params or {})
                    return Download(str(public), bytes(content), self.observed_at(), request_id)
            except httpx.TimeoutException:
                raise FetchError("deadline") from None
            except httpx.HTTPError:
                if attempt == self.budget.retries:
                    raise FetchError("upstream_unavailable") from None
                delay = 1.0
        raise FetchError("upstream_unavailable")

    async def post_reference(
        self, url: str, *, params: Mapping[str, str], headers: Mapping[str, str]
    ) -> Download:
        """Read only fixed Databento reference routes without ISIN allocation.

        POST is the provider's read protocol. Form parameters are retained in a
        credential-free request identity; response bytes remain original JSONL.
        Empty bodies are valid only for this reference protocol.
        """
        if self.client.is_closed:
            raise FetchError("invalid_configuration")
        try:
            async with asyncio.timeout(self.check()):
                async with self._lock:
                    return await self._request(url, params=params, headers=headers, method="POST")
        except TimeoutError:
            raise FetchError("deadline") from None
