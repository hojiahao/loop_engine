"""Licensed acquisition, private cache publication and offline evidence replay."""

import asyncio
import base64
import hashlib
import json
import os
import re
import time
import tomllib
from collections.abc import Callable, Mapping
from datetime import UTC, date, datetime
from pathlib import Path
from zoneinfo import ZoneInfo

import httpx

from loop_research.data.fetch_cache import (
    private_directory,
    publish,
    read_cached,
    read_config_bytes,
    read_private_config,
    read_receipt,
)
from loop_research.data.fetch_http import BoundedHttp, Download, FetchError
from loop_research.data.fetch_json import contains_secret, decode_object
from loop_research.data.fetch_records import CachedObject
from loop_research.data.licensed_config import (
    LICENSED_CONFIG,
    DatabentoRequest,
    DataLicense,
    LicensedConfig,
    SharadarRequest,
    WrdsRequest,
    check_license,
    requested_datasets,
)
from loop_research.data.licensed_records import (
    LicensedBatch,
    LicensedCapture,
    LicensedReceipt,
    LicensedReport,
    SourceTable,
)
from loop_research.data.wrds import Connector


def load_config(path: Path) -> LicensedConfig:
    """Parse strict TOML without endpoints, SQL or credential values."""

    def toml_date(value: object) -> str:
        if type(value) is date:
            return value.isoformat()
        raise ValueError("unsupported TOML value")

    try:
        values = tomllib.loads(read_config_bytes(path).decode("utf-8"))
        return LICENSED_CONFIG.validate_json(json.dumps(values, default=toml_date, allow_nan=False))
    except OSError, ValueError, TypeError:
        raise FetchError("invalid_configuration") from None


def _license(config: LicensedConfig, content: bytes, at: datetime) -> DataLicense:
    try:
        if "sha256:" + hashlib.sha256(content).hexdigest() != config.license_sha256:
            raise ValueError("license digest mismatch")
        decode_object(content)
        license = DataLicense.model_validate_json(content)
        check_license(config, license, at)
        return license
    except ValueError, TypeError:
        raise FetchError("license_denied") from None


def _credentials(config: LicensedConfig, environment: Mapping[str, str]) -> tuple[str, ...]:
    references = (
        (config.username_reference, config.password_reference)
        if isinstance(config, WrdsRequest)
        else (config.key_reference,)
    )
    values = tuple(environment.get(name, "") for name in references)
    if any(
        not value or len(value) > 1024 or not value.isascii() or not value.isprintable()
        for value in values
    ):
        raise FetchError("missing_credentials")
    if (
        isinstance(config, SharadarRequest)
        and re.fullmatch(r"[A-Za-z0-9_-]{8,128}", values[0]) is None
    ):
        raise FetchError("missing_credentials")
    if (
        isinstance(config, DatabentoRequest)
        and re.fullmatch(r"db-[A-Za-z0-9]{29}", values[0]) is None
    ):
        raise FetchError("missing_credentials")
    return values


def _publish(store: Path, content: bytes) -> CachedObject:
    try:
        return publish(store, content)
    except OSError, ValueError:
        raise FetchError("invalid_cache") from None


async def fetch_licensed(
    config: LicensedConfig,
    store: Path,
    license_path: Path,
    *,
    environment: Mapping[str, str] | None = None,
    transport: httpx.AsyncBaseTransport | None = None,
    connector: Connector | None = None,
    now: Callable[[], datetime] = lambda: datetime.now(UTC),
    monotonic: Callable[[], float] = time.monotonic,
) -> LicensedReport:
    """Download only explicitly licensed scope and commit a receipt last.

    Transport/connector injection is test-only Python API, never a CLI bypass.
    Cancellation before receipt publication leaves no success marker; cancellation
    afterwards may lose acknowledgment. Inspect/replay existing receipts on retry.
    This data-owner command does not grant research/holdout access or publish data.
    """
    config = LICENSED_CONFIG.validate_python(config)
    try:
        license_bytes = read_private_config(license_path)
    except OSError, ValueError:
        raise FetchError("license_denied") from None
    _license(config, license_bytes, now())
    credentials = _credentials(config, os.environ if environment is None else environment)
    try:
        os.close(private_directory(store))
    except OSError, ValueError:
        raise FetchError("invalid_cache") from None
    http = BoundedHttp(config.budget, transport=transport, now=now, monotonic=monotonic)
    captures: list[LicensedCapture] = []
    downloads: list[Download] = []
    secrets = credentials[1:] if isinstance(config, WrdsRequest) else credentials
    if contains_secret(license_bytes, secrets) or contains_secret(
        config.model_dump_json(by_alias=True).encode(), secrets
    ):
        await http.close()
        raise FetchError("invalid_configuration")
    auth: dict[str, str] = {}
    if isinstance(config, DatabentoRequest):
        encoded = base64.b64encode((credentials[0] + ":").encode()).decode()
        auth["Authorization"] = "Basic " + encoded
        secrets = (*credentials, encoded)

    def capture(dataset: str, page: int, value: Download) -> None:
        parts = value.body.splitlines() if isinstance(config, DatabentoRequest) else [value.body]
        if any(contains_secret(part, secrets) for part in parts) or any(
            secret in value.url or secret in (value.request_id or "") for secret in secrets
        ):
            raise FetchError("invalid_response")
        http.check()
        captures.append(
            LicensedCapture(
                dataset=dataset,
                page=page,
                request=value.url,
                observed_at=value.observed_at,
                content=_publish(store, value.body) if value.body else None,
                request_id=value.request_id,
            )
        )
        downloads.append(value)

    try:
        async with asyncio.timeout(http.check()):
            started = http.observed_at()
            _license(config, license_bytes, started)
            if config.end >= started.astimezone(ZoneInfo("America/New_York")).date():
                raise FetchError("invalid_configuration")
            if isinstance(config, SharadarRequest):
                from loop_research.data import sharadar

                count = 0
                for table in sorted(config.tables):
                    cursor = None
                    seen: set[str] = set()
                    for page in range(config.budget.pages):
                        params = {
                            **sharadar.parameters(config, table, cursor),
                            "api_key": credentials[0],
                        }
                        value = await http.get(sharadar.endpoint(table), params=params)
                        capture("SHARADAR/" + table, page, value)
                        rows, cursor = sharadar.page_rows(table, value)
                        count += len(rows)
                        if count > config.budget.records:
                            raise FetchError("record_budget")
                        if cursor is None:
                            break
                        if cursor in seen or credentials[0] in cursor:
                            raise FetchError("invalid_response")
                        seen.add(cursor)
                    else:
                        raise FetchError("page_budget")
            elif isinstance(config, DatabentoRequest):
                from loop_research.data import databento

                for dataset in sorted(config.datasets):
                    value = await http.post_reference(
                        databento.endpoint(dataset),
                        params=databento.parameters(config, dataset),
                        headers=auth,
                    )
                    capture("databento/" + dataset, 0, value)
            else:
                from loop_research.data import wrds

                value = await wrds.download(config, http, *credentials, connector=connector)
                capture(requested_datasets(config)[0], 0, value)
            batch = normalize(config, captures, downloads)
            normalized = _publish(store, batch.model_dump_json(by_alias=True).encode())
            config_ref = _publish(store, config.model_dump_json(by_alias=True).encode())
            license_ref = _publish(store, license_bytes)
            completed = http.observed_at()
            _license(config, license_bytes, completed)
            receipt = LicensedReceipt(
                config=config_ref,
                license=license_ref,
                normalized=normalized,
                captures=tuple(captures),
                started_at=started,
                completed_at=completed,
                attempts=http.attempts,
                bytes_received=http.bytes_received,
            )
            http.check()
            return _report(_publish(store, receipt.model_dump_json(by_alias=True).encode()), batch)
    except TimeoutError:
        raise FetchError("deadline") from None
    except FetchError:
        raise
    except ValueError, TypeError, KeyError, OverflowError:
        raise FetchError("invalid_response") from None
    finally:
        await http.close()


def normalize(
    config: LicensedConfig, captures: list[LicensedCapture], downloads: list[Download]
) -> LicensedBatch:
    """Verify full request/page sequence before reproducing normalized source tables."""
    if len(captures) != len(downloads):
        raise ValueError("capture/download mismatch")
    tables: list[SourceTable] = []
    offset = 0
    for dataset in requested_datasets(config):
        group: list[Download] = []
        cursor = None
        seen: set[str] = set()
        while offset < len(captures) and captures[offset].dataset == dataset:
            record, value = captures[offset], downloads[offset]
            if record.page != len(group) or record.page >= config.budget.pages:
                raise ValueError("invalid capture page sequence")
            if isinstance(config, SharadarRequest):
                from loop_research.data import sharadar

                table = dataset.split("/", 1)[1]
                expected = str(
                    httpx.URL(
                        sharadar.endpoint(table), params=sharadar.parameters(config, table, cursor)
                    )
                )
                _, cursor = sharadar.page_rows(table, value)
            elif isinstance(config, DatabentoRequest):
                from loop_research.data import databento

                name = dataset.split("/", 1)[1]
                expected = str(
                    httpx.URL(databento.endpoint(name), params=databento.parameters(config, name))
                )
            else:
                from loop_research.data import wrds

                expected = wrds.request_identity(config)
            if (
                record.request != expected
                or value.url != expected
                or value.observed_at != record.observed_at
            ):
                raise ValueError("captured request differs from configuration")
            if cursor is not None and cursor in seen:
                raise ValueError("repeated pagination cursor")
            if cursor is not None:
                seen.add(cursor)
            group.append(value)
            offset += 1
            if cursor is None:
                break
        if not group or cursor is not None:
            raise ValueError("incomplete dataset capture")
        if isinstance(config, SharadarRequest):
            tables.append(sharadar.normalize(config, dataset.split("/", 1)[1], group))
        elif isinstance(config, DatabentoRequest):
            tables.append(databento.normalize(config, dataset.split("/", 1)[1], group[0]))
        else:
            tables.append(wrds.normalize(config, group[0]))
    if offset != len(captures) or sum(len(table.rows) for table in tables) > config.budget.records:
        raise ValueError("unrequested captures or record budget exceeded")
    return LicensedBatch(
        provider=config.provider,
        tables=tuple(tables),
        allocation_filter="existing_isins_only"
        if isinstance(config, DatabentoRequest)
        else "not_applicable",
    )


def replay_licensed(store: Path, digest: str) -> LicensedReport:
    """Revalidate byte identities, original license scope, requests and source semantics.

    No network or current credentials are needed. This is a consistency check,
    not vendor attestation or renewed authorization to acquire data after expiry.
    """
    try:
        reference, content = read_receipt(store, digest)
        decode_object(content)
        receipt = LicensedReceipt.model_validate_json(content)
        config_bytes = read_cached(store, receipt.config)
        decode_object(config_bytes)
        config = LICENSED_CONFIG.validate_json(config_bytes)
        license_bytes = read_cached(store, receipt.license)
        _license(config, license_bytes, receipt.started_at)
        _license(config, license_bytes, receipt.completed_at)
        if (
            config.end >= receipt.started_at.astimezone(ZoneInfo("America/New_York")).date()
            or receipt.attempts > config.budget.requests
            or receipt.bytes_received > config.budget.total_bytes
            or (receipt.completed_at - receipt.started_at).total_seconds()
            > config.budget.timeout_seconds
            or any(
                c.content is not None and c.content.byte_size > config.budget.response_bytes
                for c in receipt.captures
            )
        ):
            raise ValueError("receipt exceeds operation bounds")
        downloads = [
            Download(
                capture.request,
                read_cached(store, capture.content) if capture.content is not None else b"",
                capture.observed_at,
                capture.request_id,
            )
            for capture in receipt.captures
        ]
        batch = normalize(config, list(receipt.captures), downloads)
        if read_cached(store, receipt.normalized) != batch.model_dump_json(by_alias=True).encode():
            raise ValueError("cached normalization differs from source evidence")
        return _report(reference, batch)
    except OSError, ValueError, TypeError, KeyError, OverflowError:
        raise FetchError("invalid_cache") from None


def _report(reference: CachedObject, batch: LicensedBatch) -> LicensedReport:
    counts = {table.dataset: len(table.rows) for table in batch.tables}
    return LicensedReport(
        receipt=reference,
        provider=batch.provider,
        result="records" if any(counts.values()) else "empty",
        row_counts=counts,
        allocation_filter=batch.allocation_filter,
    )
