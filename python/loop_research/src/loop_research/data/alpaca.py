"""Explicit Alpaca raw daily bars; current identity is not a historic master."""

import hashlib
import re
from datetime import date, datetime, time, timedelta
from enum import Enum
from typing import Any
from uuid import UUID
from zoneinfo import ZoneInfo

from alpaca.common.enums import Sort, SupportedCurrencies
from alpaca.data.enums import Adjustment, DataFeed
from alpaca.data.requests import StockBarsRequest
from alpaca.data.timeframe import TimeFrame, TimeFrameUnit

from loop_research.data.fetch_config import AlpacaRequest
from loop_research.data.fetch_http import Download, FetchError
from loop_research.data.fetch_json import decimal_text, decode_object
from loop_research.data.fetch_records import DevelopmentBatch, ObservedAsset
from loop_research.data.models import RawBar, SourceEvidence, parse_instant

NEW_YORK = ZoneInfo("America/New_York")


def bars_parameters(config: AlpacaRequest, asof: date) -> dict[str, str]:
    """Use the installed official SDK's request fields, then encode scalar queries."""
    fields = StockBarsRequest(
        symbol_or_symbols=list(sorted(config.symbols)),
        timeframe=TimeFrame(1, TimeFrameUnit.Day),
        start=datetime.combine(config.start, time.min, NEW_YORK),
        end=datetime.combine(config.end, time.max, NEW_YORK),
        limit=min(1000, config.budget.records),
        currency=SupportedCurrencies.USD,
        sort=Sort.ASC,
        adjustment=Adjustment.RAW,
        feed=DataFeed(config.feed),
        asof=asof.isoformat(),
    ).to_request_fields()
    result = {}
    for key, value in fields.items():
        if isinstance(value, (Enum, TimeFrame)):
            value = value.value
        if not isinstance(key, str) or type(value) not in (str, int):
            raise ValueError("unsupported SDK request field")
        result[key] = str(value)
    return result


def _source(download: Download, dataset: str, record_id: str) -> SourceEvidence:
    raw_hash = "sha256:" + hashlib.sha256(download.body).hexdigest()
    return SourceEvidence(
        source="alpaca",
        dataset=dataset,
        revision=raw_hash,
        record_id=record_id,
        raw_sha256=raw_hash,
        availability="first_observed",
    )


def asset_record(symbol: str, download: Download) -> ObservedAsset:
    """Require the returned current symbol and a real UUID, without issuer inference."""
    value = decode_object(download.body)
    asset_id = value.get("id")
    if (
        not isinstance(asset_id, str)
        or not re.fullmatch(r"[0-9a-f-]{36}", asset_id)
        or str(UUID(asset_id)) != asset_id
        or UUID(asset_id).int == 0
        or value.get("symbol") != symbol
        or value.get("class") != "us_equity"
    ):
        raise FetchError("identity_unresolved")
    security_id = "alpaca:asset:" + asset_id
    exchange, status = value.get("exchange"), value.get("status")
    if not isinstance(exchange, str) or status not in ("active", "inactive"):
        raise ValueError("invalid current asset metadata")
    return ObservedAsset(
        security_id=security_id,
        symbol=symbol,
        exchange=exchange,
        status="active" if status == "active" else "inactive",
        asset_class="us_equity",
        observed_at=download.observed_at,
        source=_source(download, "current-assets", security_id),
    )


def page_rows(download: Download, symbols: tuple[str, ...]) -> tuple[dict[str, Any], str | None]:
    """Reject unknown symbols and unbounded cursor values; cursors never select URLs."""
    payload = decode_object(download.body)
    rows = payload.get("bars")
    token = payload.get("next_page_token")
    if not isinstance(rows, dict) or any(symbol not in symbols for symbol in rows):
        raise ValueError("unrequested Alpaca symbol or missing bars")
    if any(not isinstance(values, list) for values in rows.values()):
        raise ValueError("Alpaca bars require arrays")
    if token is not None and (
        not isinstance(token, str) or not re.fullmatch(r"[A-Za-z0-9_+/=-]{1,1024}", token)
    ):
        raise ValueError("invalid Alpaca page token")
    return rows, token


def normalize_alpaca(
    config: AlpacaRequest,
    asof: date,
    assets: dict[str, Download],
    pages: list[Download],
) -> DevelopmentBatch:
    """Validate every page and interval; complete pagination does not imply coverage."""
    if set(assets) != set(config.symbols) or not pages or len(pages) > config.budget.pages:
        raise ValueError("incomplete Alpaca acquisition")
    current = {symbol: asset_record(symbol, value) for symbol, value in assets.items()}
    if any(asset.observed_at.astimezone(NEW_YORK).date() != asof for asset in current.values()):
        raise FetchError("identity_unresolved")
    result: list[RawBar] = []
    seen: set[tuple[str, date]] = set()
    previous: tuple[str, datetime] | None = None
    for index, page in enumerate(pages):
        rows, token = page_rows(page, config.symbols)
        if (token is not None) != (index < len(pages) - 1):
            raise ValueError("incomplete Alpaca pagination")
        if page.observed_at.astimezone(NEW_YORK).date() != asof:
            raise FetchError("identity_unresolved")
        for symbol, values in sorted(rows.items()):
            for value in values:
                if len(result) >= config.budget.records:
                    raise FetchError("record_budget")
                bar = _bar(config, current[symbol], value, page)
                identity = symbol, bar.session
                order = symbol, bar.interval_start
                if identity in seen or (previous is not None and order <= previous):
                    raise ValueError("duplicate or out-of-order Alpaca bar")
                seen.add(identity)
                previous = order
                result.append(bar)
    found = {symbol for symbol, _ in seen}
    return DevelopmentBatch(
        provider="alpaca",
        start=config.start,
        end=config.end,
        feed=config.feed,
        symbol_asof=asof,
        assets=tuple(current[symbol] for symbol in sorted(current)),
        bars=tuple(result),
        missing=tuple(sorted(set(config.symbols) - found)),
    )


def _bar(config: AlpacaRequest, asset: ObservedAsset, value: object, page: Download) -> RawBar:
    if not isinstance(value, dict):
        raise ValueError("invalid Alpaca bar")
    start = parse_instant(value.get("t"))
    local_start = start.astimezone(NEW_YORK)
    if local_start.time() != time.min or not config.start <= local_start.date() <= config.end:
        raise ValueError("Alpaca daily label must match the requested New York date")
    effective = datetime.combine(local_start.date() + timedelta(days=1), time.min, NEW_YORK)
    volume = value.get("v")
    if type(volume) is not int:
        raise ValueError("Alpaca share volume requires an integer")
    return RawBar(
        security_id=asset.security_id,
        session=local_start.date(),
        interval_start=start,
        effective_at=effective,
        known_at=page.observed_at,
        ingested_at=page.observed_at,
        currency="USD",
        price_basis="raw",
        open=decimal_text(value.get("o")),
        high=decimal_text(value.get("h")),
        low=decimal_text(value.get("l")),
        close=decimal_text(value.get("c")),
        volume=volume,
        source=_source(
            page, "daily-bars-" + config.feed, asset.security_id + ":" + str(local_start.date())
        ),
    )


def validate_sip_probe(download: Download, symbol: str) -> None:
    """Validate the latest endpoint's response shape; do not infer trade freshness."""
    trades = decode_object(download.body).get("trades")
    if not isinstance(trades, dict) or set(trades) - {symbol}:
        raise ValueError("invalid SIP probe response")
    if symbol in trades and trades[symbol] is not None:
        trade = trades[symbol]
        if not isinstance(trade, dict):
            raise ValueError("invalid SIP trade")
        # Latest trade timestamps may contain nanoseconds. This probe records
        # response permission only; it does not round them into PIT evidence.
        if not isinstance(trade.get("t"), str) or not re.fullmatch(
            r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]{1,9})?Z", trade["t"]
        ):
            raise ValueError("invalid SIP trade timestamp")
        decimal_text(trade.get("p"))
