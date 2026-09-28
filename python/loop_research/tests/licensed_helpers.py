"""Invented licensed-source records; no subscribed vendor data or live credentials."""

import hashlib
import json
from pathlib import Path
from typing import Any

from data_helpers import OBSERVED

from loop_research.data.fetch_http import Download
from loop_research.data.licensed_config import LICENSED_CONFIG, LicensedConfig

KEY = "test-synthetic-key-123456"
DB_KEY = "db-" + "a" * 29
ENV = {
    "LOOP_TEST_KEY": KEY,
    "LOOP_TEST_USER": "test_user",
    "LOOP_TEST_PASSWORD": "synthetic-password-xyz",
}


def license_config(
    path: Path,
    provider: str = "sharadar",
    *,
    license_changes: dict[str, Any] | None = None,
    **changes: Any,
) -> tuple[LicensedConfig, Path]:
    datasets = {
        "sharadar": ["SHARADAR/SEP", "SHARADAR/SF1", "SHARADAR/TICKERS", "SHARADAR/ACTIONS"],
        "wrds": ["crsp.stkdlysecuritydata", "comp.fundq"],
        "databento": ["databento/security_master", "databento/corporate_actions"],
    }
    license = {
        "schema": "loop.data-license/v1",
        "provider": provider,
        "license_id": "synthetic-contract-only",
        "declared_by": "fixture-owner",
        "datasets": datasets[provider],
        "valid_from": "2026-01-01T00:00:00Z",
        "expires_at": "2026-12-31T00:00:00Z",
        "data_start": "2005-01-01",
        "data_end": "2026-08-31",
        "purpose": "internal_research",
        "local_storage": True,
        "billing": "prepaid_subscription",
        "current_reference_metadata": True,
        **(license_changes or {}),
    }
    content = json.dumps(license).encode()
    file = path / "license.json"
    file.write_bytes(content)
    file.chmod(0o600)
    shape: dict[str, Any] = {
        "schema": "loop.licensed-fetch/v1",
        "provider": provider,
        "license_sha256": "sha256:" + hashlib.sha256(content).hexdigest(),
        "start": "2026-08-01",
        "end": "2026-08-31",
        "budget": {"timeout_seconds": 180, "retries": 0},
    }
    if provider == "sharadar":
        shape.update(key_reference="LOOP_TEST_KEY", symbols=["DEMO"], tables=["SEP"])
    elif provider == "databento":
        shape.update(
            key_reference="LOOP_TEST_KEY", listing_ids=["L-999999"], datasets=["security_master"]
        )
    else:
        shape.update(
            username_reference="LOOP_TEST_USER",
            password_reference="LOOP_TEST_PASSWORD",
            profile="crsp_ciz_daily_v1",
            identifiers=["999999"],
        )
    return LICENSED_CONFIG.validate_json(json.dumps({**shape, **changes})), file


def cache(path: Path) -> Path:
    store = path / "cache"
    store.mkdir(mode=0o700)
    return store


SEP_COLUMNS = [
    ("ticker", "String"),
    ("date", "Date"),
    ("open", "BigDecimal"),
    ("high", "BigDecimal"),
    ("low", "BigDecimal"),
    ("close", "BigDecimal"),
    ("volume", "BigDecimal"),
    ("closeadj", "BigDecimal"),
    ("closeunadj", "BigDecimal"),
    ("lastupdated", "Date"),
]
SEP_ROW = ["DEMO", "2026-08-28", 10, 12, 9, 11, 4000, 10, 44, "2026-09-01"]


def table_bytes(
    columns: list[tuple[str, str]] = SEP_COLUMNS,
    rows: list[list[Any]] | None = None,
    cursor: str | None = None,
) -> bytes:
    return json.dumps(
        {
            "datatable": {
                "columns": [{"name": name, "type": type_} for name, type_ in columns],
                "data": [SEP_ROW] if rows is None else rows,
            },
            "meta": {"next_cursor_id": cursor},
        }
    ).encode()


def reference_row(**changes: Any) -> dict[str, Any]:
    return {
        "listing_id": "L-999999",
        "security_id": "S-999999",
        "issuer_id": "I-999999",
        "ts_record": "2026-08-29T12:00:00.123456789Z",
        "ts_created": "2026-08-30T00:00:00Z",
        "ts_effective": "2026-08-28T00:00:00Z",
        "listing_country": "US",
        "listing_status": "L",
        "nasdaq_symbol": "DEMO",
        "trading_currency": "USD",
        **changes,
    }


def jsonl(*rows: dict[str, Any]) -> bytes:
    return b"".join(json.dumps(row).encode() + b"\n" for row in rows)


def captured(content: bytes) -> Download:
    return Download("https://fixture.invalid/not-a-live-source", content, OBSERVED, None)
