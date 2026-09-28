"""Sequential acquisition with immutable progress and explicit source snapshots."""

import asyncio
import hashlib
import json
import os
import tomllib
from collections.abc import Callable, Mapping
from datetime import UTC, date, datetime
from pathlib import Path
from typing import Any

import httpx

from loop_research.data.access import check_requests, license_files
from loop_research.data.fetch_cache import (
    private_directory,
    publish,
    read_cached,
    read_config_bytes,
    read_receipt,
)
from loop_research.data.fetch_config import AlpacaRequest, SecRequest
from loop_research.data.fetch_http import FetchError
from loop_research.data.fetch_records import CachedObject
from loop_research.data.ingestion import fetch_data
from loop_research.data.licensed_ingestion import fetch_licensed
from loop_research.data.snapshot_models import (
    SnapshotReport,
    SnapshotRequest,
    SyncPlan,
    SyncProgress,
)
from loop_research.data.snapshot_sources import load_acquisition
from loop_research.data.snapshots import _Deadline, build_snapshot
from loop_research.data.wrds import Connector


def load_sync_plan(path: Path) -> SyncPlan:
    """Strict bounded TOML; requests contain named credentials, never endpoints or SQL."""
    values = tomllib.loads(read_config_bytes(path).decode())

    def scalar(value: object) -> str:
        if type(value) is date:
            return value.isoformat()
        raise ValueError("unsupported sync TOML value")

    return SyncPlan.model_validate_json(json.dumps(values, default=scalar, allow_nan=False))


async def synchronize(
    store: Path,
    plan: SyncPlan,
    *,
    licenses: tuple[Path, ...] = (),
    resume: str | None = None,
    environment: Mapping[str, str] | None = None,
    transport: httpx.AsyncBaseTransport | None = None,
    connector: Connector | None = None,
    now: Callable[[], datetime] = lambda: datetime.now(UTC),
    progress: Callable[[CachedObject, int], None] | None = None,
) -> SnapshotReport:
    """Fetch the remaining bounded prefix and publish a source snapshot on success.

    Validate all configuration, remaining credential/rights references and resume
    evidence before network IO. Each completed request publishes immutable progress;
    callers retain its digest for restart. A read completed before a hard kill but
    after the last progress publication may be repeated. No mutable latest pointer,
    factor registration, application DB write or protected capability is created.
    Cancellation propagates and successful source objects remain replayable.
    """
    plan = SyncPlan.model_validate(plan)
    deadline = _Deadline(plan.timeout_seconds)
    os.close(private_directory(store))
    plan_bytes = plan.model_dump_json(by_alias=True).encode()
    plan_reference = CachedObject(
        sha256="sha256:" + hashlib.sha256(plan_bytes).hexdigest(),
        byte_size=len(plan_bytes),
    )
    receipts: list[CachedObject] = []
    if resume is not None:
        _, content = read_receipt(store, resume)
        checkpoint = SyncProgress.model_validate_json(content)
        if checkpoint.model_dump_json(by_alias=True).encode() != content:
            raise ValueError("sync progress must be canonical JSON")
        if checkpoint.plan != plan_reference or read_cached(store, checkpoint.plan) != plan_bytes:
            raise ValueError("resume belongs to another source plan")
        if len(checkpoint.receipts) > len(plan.requests):
            raise ValueError("resume prefix exceeds its source plan")
        for index, reference in enumerate(checkpoint.receipts):
            deadline.remaining()
            source = load_acquisition(store, reference.sha256)
            if source.reference != reference or source.config != plan.requests[index]:
                raise ValueError("completed source request differs from resume plan")
            receipts.append(reference)
    declarations = license_files(licenses)
    env = os.environ if environment is None else environment
    for access in check_requests(
        plan.requests[len(receipts) :], declarations, env, now(), plan_bytes
    ):
        if access.issues:
            raise FetchError(access.issues[0])
    remaining = deadline.remaining()
    publish(store, plan_bytes)
    async with asyncio.timeout(remaining):
        for request in plan.requests[len(receipts) :]:
            deadline.remaining()
            if isinstance(request, SecRequest | AlpacaRequest):
                report = await fetch_data(
                    request, store, transport=transport, environment=env, now=now
                )
                reference = report.receipt
            else:
                kwargs: dict[str, Any] = {"transport": transport, "environment": env, "now": now}
                if connector is not None:
                    kwargs["connector"] = connector
                licensed = await fetch_licensed(
                    request, store, declarations[request.license_sha256].path, **kwargs
                )
                reference = licensed.receipt
            receipts.append(reference)
            deadline.remaining()
            checkpoint = SyncProgress(plan=plan_reference, receipts=tuple(receipts))
            checkpoint_reference = publish(
                store, checkpoint.model_dump_json(by_alias=True).encode()
            )
            if progress is not None:
                progress(checkpoint_reference, len(receipts))
            await asyncio.sleep(0)
        snapshot_request = SnapshotRequest(
            receipts=tuple(sorted(reference.sha256 for reference in receipts)),
            start=plan.start,
            through=plan.through,
        )
        return build_snapshot(store, snapshot_request, timeout_seconds=deadline.remaining())
