"""Replayed acquisition records mapped to lossless, source-specific tables."""

from dataclasses import dataclass
from datetime import UTC, date, datetime
from pathlib import Path
from typing import Any

from loop_research.data.fetch_cache import read_cached, read_receipt
from loop_research.data.fetch_config import FETCH_CONFIG, AlpacaRequest, SecRequest
from loop_research.data.fetch_json import decode_object
from loop_research.data.fetch_records import CachedObject, DevelopmentBatch, FetchReceipt
from loop_research.data.ingestion import replay_data
from loop_research.data.licensed_config import LICENSED_CONFIG, SharadarRequest
from loop_research.data.licensed_ingestion import replay_licensed
from loop_research.data.licensed_records import LicensedBatch, LicensedReceipt, SourceTable
from loop_research.data.snapshot_models import SourceRequest


@dataclass(frozen=True)
class Acquisition:
    """Only constructed after replaying actual source bytes and normalization."""

    reference: CachedObject
    normalized: CachedObject
    config: SourceRequest
    completed_at: datetime
    source_bytes: int
    attempts: int
    tables: tuple[SourceTable, ...]


def load_acquisition(
    store: Path,
    digest: str,
    *,
    max_source_bytes: int = 512 * 1024 * 1024,
) -> Acquisition:
    """Recheck an existing source receipt without network or new access rights."""
    reference, content = read_receipt(store, digest)
    schema = decode_object(content).get("schema")
    if schema == "loop.development-receipt/v1":
        receipt = FetchReceipt.model_validate_json(content)
        if receipt.bytes_received > max_source_bytes:
            raise ValueError("aggregate source byte budget")
        replay_data(store, digest)
        config = FETCH_CONFIG.validate_json(read_cached(store, receipt.config))
        batch = DevelopmentBatch.model_validate_json(read_cached(store, receipt.normalized))
        return Acquisition(
            reference,
            receipt.normalized,
            config,
            receipt.completed_at,
            receipt.bytes_received,
            receipt.attempts,
            _development_tables(batch),
        )
    if schema == "loop.licensed-receipt/v1":
        licensed = LicensedReceipt.model_validate_json(content)
        if licensed.bytes_received > max_source_bytes:
            raise ValueError("aggregate source byte budget")
        replay_licensed(store, digest)
        source = LICENSED_CONFIG.validate_json(read_cached(store, licensed.config))
        native = LicensedBatch.model_validate_json(read_cached(store, licensed.normalized))
        return Acquisition(
            reference,
            licensed.normalized,
            source,
            licensed.completed_at,
            licensed.bytes_received,
            licensed.attempts,
            native.tables,
        )
    raise ValueError("unsupported acquisition receipt")


def _flatten(record: dict[str, Any]) -> dict[str, str | None]:
    result: dict[str, str | None] = {}
    for key, value in record.items():
        if isinstance(value, dict):
            for nested, scalar in value.items():
                if scalar is not None and not isinstance(scalar, str):
                    raise ValueError("unsupported source evidence scalar")
                result[key + "." + nested] = scalar
        elif value is None or isinstance(value, str):
            result[key] = value
        elif type(value) is bool:
            result[key] = "true" if value else "false"
        elif type(value) is int:
            result[key] = str(value)
        else:
            raise ValueError("unsupported development source scalar")
    return result


def _development_tables(batch: DevelopmentBatch) -> tuple[SourceTable, ...]:
    tables = []
    groups = (
        ("assets", batch.assets, ("security_id",), ("current_metadata",)),
        ("bars", batch.bars, ("security_id", "session", "known_at"), ("raw_ohlcv",)),
        ("fundamentals", batch.fundamentals, ("source.record_id",), ("filing_level_facts",)),
    )
    for name, records, keys, semantics in groups:
        if not records:
            continue
        flattened = [_flatten(record.model_dump(mode="json")) for record in records]
        columns = tuple(sorted(flattened[0]))
        if any(tuple(sorted(row)) != columns for row in flattened):
            raise ValueError("inconsistent development record schema")
        tables.append(
            SourceTable(
                dataset=f"{batch.provider}/{name}",
                columns=columns,
                primary_key=keys,
                rows=tuple(tuple(row[column] for column in columns) for row in flattened),
                semantics=semantics,
            )
        )
    # Empty observations still have a named table for an explicit quality report.
    expected = "sec/fundamentals" if batch.provider == "sec" else "alpaca/bars"
    if expected not in {table.dataset for table in tables}:
        tables.append(
            SourceTable(
                dataset=expected,
                columns=("source.record_id",),
                primary_key=("source.record_id",),
                rows=(),
                semantics=("empty_selected_response",),
            )
        )
    return tuple(tables)


def timestamp_ns(value: str | datetime) -> int:
    """Convert validated source RFC3339 timestamps without discarding nanoseconds."""
    if isinstance(value, datetime):
        instant, nanos = value.astimezone(UTC).replace(microsecond=0), value.microsecond * 1000
    else:
        # Native Databento parsing already handles timezone offsets and 1-9 digits.
        from loop_research.data.databento import _instant

        instant, nanos = _instant(value)
    delta = instant - datetime(1970, 1, 1, tzinfo=UTC)
    return (delta.days * 86400 + delta.seconds) * 1_000_000_000 + nanos


def row_clocks(
    acquisition: Acquisition,
    table: SourceTable,
    row: dict[str, str | None],
) -> tuple[date, int, int, str]:
    """Keep business dates separate from first observation and native record clocks."""
    observed = acquisition.completed_at
    ingested = row.get("ingested_at") or observed.isoformat()
    known = row.get("known_at") or row.get("ts_record") or observed.isoformat()
    basis = row.get("source.availability") or (
        "source_record_timestamp" if row.get("ts_record") else "first_observed"
    )
    dates = {
        "alpaca/bars": "session",
        "sec/fundamentals": "period_end",
        "SHARADAR/SEP": "date",
        "SHARADAR/SF1": "datekey",
        "SHARADAR/ACTIONS": "date",
        "crsp.stkdlysecuritydata": "dlycaldt",
        "comp.fundq": "datadate",
        "databento/corporate_actions": "event_date",
    }
    if table.dataset in {"SHARADAR/TICKERS", "alpaca/assets"}:
        business = date.fromisoformat((row.get("observed_at") or observed.isoformat())[:10])
    elif table.dataset == "databento/security_master":
        effective = row.get("ts_effective")
        if effective is None:
            raise ValueError("missing source effective date")
        business = date.fromisoformat(effective[:10])
    else:
        field = dates.get(table.dataset)
        value = row.get(field) if field else None
        if value is None:
            raise ValueError("unsupported or absent source observation date")
        business = date.fromisoformat(value)
    known_ns, ingested_ns = timestamp_ns(known), timestamp_ns(ingested)
    if known_ns > ingested_ns:
        raise ValueError("source knowledge follows snapshot ingestion")
    return business, known_ns, ingested_ns, basis


def daily_identifiers(
    acquisition: Acquisition, table: SourceTable
) -> tuple[str, tuple[str, ...]] | None:
    """Requested identifiers for calendar coverage; not historical universe eligibility."""
    source = acquisition.config
    if isinstance(source, AlpacaRequest) and table.dataset == "alpaca/bars":
        assets = next(item for item in acquisition.tables if item.dataset == "alpaca/assets")
        column = assets.columns.index("security_id")
        return "security_id", tuple(sorted(str(row[column]) for row in assets.rows))
    if isinstance(source, SharadarRequest) and table.dataset == "SHARADAR/SEP":
        return "ticker", tuple(sorted(source.symbols))
    if (
        not isinstance(source, SecRequest | AlpacaRequest)
        and source.provider == "wrds"
        and source.profile == "crsp_ciz_daily_v1"
    ):
        return "permno", tuple(sorted(source.identifiers))
    return None
