"""Real source-plan progress, cancellation, replay and installed CLI boundaries."""

import asyncio
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

import httpx
import pytest
from data_helpers import OBSERVED, fixture_handler, response, sec_config
from licensed_helpers import ENV, cache, license_config, table_bytes

from loop_research.data.fetch_cache import publish, read_cached, read_receipt
from loop_research.data.fetch_http import FetchError
from loop_research.data.fetch_records import CachedObject
from loop_research.data.snapshot_models import SyncPlan, SyncProgress
from loop_research.data.snapshots import validate_snapshot
from loop_research.data.sync import load_sync_plan, synchronize


def plan_for(tmp_path: Path, **changes: Any) -> tuple[SyncPlan, Path]:
    config, license = license_config(tmp_path)
    source = {
        "schema": "loop.data-sync-plan/v1",
        "start": "2026-01-01",
        "through": "2026-08-31",
        "requests": [
            sec_config().model_dump(mode="json", by_alias=True),
            config.model_dump(mode="json", by_alias=True),
        ],
        "max_requests": 48,
        "max_source_bytes": 128 * 1024 * 1024,
        "max_records": 20_000,
        "timeout_seconds": 400,
        **changes,
    }
    return SyncPlan.model_validate_json(json.dumps(source)), license


def handler(requests: list[httpx.Request]):
    development = fixture_handler(requests)

    def handle(request: httpx.Request) -> httpx.Response:
        if request.url.host == "data.nasdaq.com":
            requests.append(request)
            return response(table_bytes())
        return development(request)

    return handle


# Scenario: sync finishes and complete resume needs no credentials.
def test_sync_finishes(tmp_path: Path) -> None:
    store = cache(tmp_path)
    plan, license = plan_for(tmp_path)
    calls: list[httpx.Request] = []
    checkpoints: list[tuple[CachedObject, int]] = []
    first = asyncio.run(
        synchronize(
            store,
            plan,
            licenses=(license,),
            environment=ENV,
            transport=httpx.MockTransport(handler(calls)),
            now=lambda: OBSERVED,
            progress=lambda ref, count: checkpoints.append((ref, count)),
        )
    )
    assert len(calls) == 3 and [count for _, count in checkpoints] == [1, 2]
    assert first.row_count == 5 and validate_snapshot(store, first.snapshot.sha256) == first
    before = {path.name: path.stat().st_mtime_ns for path in store.iterdir()}
    resumed = asyncio.run(
        synchronize(
            store,
            plan,
            resume=checkpoints[-1][0].sha256,
            environment={},
            now=lambda: OBSERVED,
            transport=httpx.MockTransport(
                lambda _: pytest.fail("completed request was downloaded again")
            ),
        )
    )
    assert resumed == first
    assert before == {path.name: path.stat().st_mtime_ns for path in store.iterdir()}


# Scenario: interrupted sync resumes only unfinished requests.
def test_interrupted_sync(tmp_path: Path) -> None:
    store = cache(tmp_path)
    plan, license = plan_for(tmp_path)
    calls: list[httpx.Request] = []
    checkpoints: list[CachedObject] = []

    def stop_after_receipt(ref: CachedObject, count: int) -> None:
        checkpoints.append(ref)
        assert count == 1
        raise RuntimeError("synthetic delivery loss")

    with pytest.raises(RuntimeError, match="delivery loss"):
        asyncio.run(
            synchronize(
                store,
                plan,
                licenses=(license,),
                environment=ENV,
                now=lambda: OBSERVED,
                transport=httpx.MockTransport(handler(calls)),
                progress=stop_after_receipt,
            )
        )
    assert len(calls) == 2
    calls.clear()
    resumed = asyncio.run(
        synchronize(
            store,
            plan,
            licenses=(license,),
            resume=checkpoints[0].sha256,
            environment=ENV,
            now=lambda: OBSERVED,
            transport=httpx.MockTransport(handler(calls)),
        )
    )
    assert len(calls) == 1 and calls[0].url.host == "data.nasdaq.com"
    assert resumed.row_count == 5


@pytest.mark.parametrize("failure", ["credentials", "license", "expired"])
# Scenario: all remaining requirements preflight before io.
def test_remaining_requirements(tmp_path: Path, failure: str) -> None:
    store = cache(tmp_path)
    plan, license = plan_for(tmp_path)
    if failure == "expired":
        document = json.loads(license.read_bytes())
        document["expires_at"] = "2026-02-01T00:00:00Z"
        license.write_text(json.dumps(document))
    with pytest.raises((ValueError, FetchError)):
        asyncio.run(
            synchronize(
                store,
                plan,
                licenses=() if failure == "license" else (license,),
                environment={} if failure == "credentials" else ENV,
                now=lambda: OBSERVED,
                transport=httpx.MockTransport(
                    lambda _: pytest.fail("preflight attempted network IO")
                ),
            )
        )
    assert not list(store.iterdir())


@pytest.mark.parametrize(
    "field,value",
    [
        ("max_requests", 47),
        ("max_source_bytes", 1024),
        ("max_records", 19_999),
        ("timeout_seconds", 359),
        ("through", "2026-09-01"),
        ("start", "2026-08-01"),
    ],
)
# Scenario: aggregate reservations and dates are enforced.
def test_aggregate_reservations(tmp_path: Path, field: str, value: object) -> None:
    with pytest.raises(ValueError):
        plan_for(tmp_path, **{field: value})


@pytest.mark.parametrize("failure", ["plan", "receipt", "prefix", "source"])
# Scenario: resume tampering fails before io.
def test_resume_tampering(tmp_path: Path, failure: str) -> None:
    store = cache(tmp_path)
    plan, license = plan_for(tmp_path)
    checkpoints: list[CachedObject] = []

    def stop(ref: CachedObject, _: int) -> None:
        checkpoints.append(ref)
        raise RuntimeError("stopped")

    with pytest.raises(RuntimeError):
        asyncio.run(
            synchronize(
                store,
                plan,
                licenses=(license,),
                environment=ENV,
                now=lambda: OBSERVED,
                transport=httpx.MockTransport(handler([])),
                progress=stop,
            )
        )
    digest = checkpoints[0].sha256
    _, content = read_receipt(store, digest)
    checkpoint = SyncProgress.model_validate_json(content)
    if failure == "receipt":
        (store / checkpoint.receipts[0].sha256[7:]).write_bytes(b"bad")
    elif failure == "source":
        receipt = json.loads(read_cached(store, checkpoint.receipts[0]))
        (store / receipt["normalized"]["sha256"][7:]).write_bytes(b"bad")
    elif failure == "prefix":
        forged = SyncProgress(plan=checkpoint.plan, receipts=checkpoint.receipts * 3)
        digest = publish(store, forged.model_dump_json(by_alias=True).encode()).sha256
    else:
        plan = SyncPlan.model_validate_json(
            plan.model_dump_json().replace('"max_requests":48', '"max_requests":49')
        )
    with pytest.raises((ValueError, FetchError)):
        asyncio.run(
            synchronize(
                store,
                plan,
                licenses=(license,),
                resume=digest,
                environment=ENV,
                now=lambda: OBSERVED,
                transport=httpx.MockTransport(lambda _: pytest.fail("invalid resume attempted IO")),
            )
        )


# Scenario: cancelled acquisition has no success manifest.
def test_cancelled_acquisition(tmp_path: Path) -> None:
    store = cache(tmp_path)
    plan, license = plan_for(tmp_path)
    entered = asyncio.Event()

    async def stalled(_: httpx.Request) -> httpx.Response:
        entered.set()
        await asyncio.Future()
        raise AssertionError("unreachable")

    async def exercise() -> None:
        task = asyncio.create_task(
            synchronize(
                store,
                plan,
                licenses=(license,),
                environment=ENV,
                now=lambda: OBSERVED,
                transport=httpx.MockTransport(stalled),
            )
        )
        await asyncio.wait_for(entered.wait(), 3)
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task

    asyncio.run(exercise())
    assert all(
        not path.read_bytes().startswith(b'{"schema":"loop.source-snapshot/v1"')
        for path in store.iterdir()
    )
    assert all(
        not path.read_bytes().startswith(b'{"schema":"loop.data-sync-progress/v1"')
        for path in store.iterdir()
    )


# Scenario: actual toml and cli reject missing credentials.
def test_toml_cli(tmp_path: Path) -> None:
    store = cache(tmp_path)
    config = tmp_path / "sync.toml"
    config.write_text("""schema = "loop.data-sync-plan/v1"
start = 2026-08-01
through = 2026-08-31
max_requests = 24
max_source_bytes = 67108864
max_records = 10000
timeout_seconds = 180
[[requests]]
schema = "loop.licensed-fetch/v1"
provider = "sharadar"
start = 2026-08-01
end = 2026-08-31
license_sha256 = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
key_reference = "LOOP_UNSET_TEST_SNAPSHOT_KEY"
symbols = ["DEMO"]
tables = ["SEP"]
""")
    assert len(load_sync_plan(config).requests) == 1
    completed = subprocess.run(
        [
            sys.executable,
            "-I",
            "-m",
            "loop_research.cli",
            "data-sync",
            str(config),
            "--store",
            str(store),
        ],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert completed.returncode == 2 and "missing_credentials" in completed.stderr
    assert not list(store.iterdir())


# Scenario: plan cannot persist credential echo.
def test_plan_persist(tmp_path: Path) -> None:
    from licensed_helpers import KEY

    store = cache(tmp_path)
    plan, license = plan_for(tmp_path)
    document = plan.model_dump(mode="json", by_alias=True)
    document["requests"][0]["contact_email"] = KEY + "@example.org"
    reflected = SyncPlan.model_validate_json(json.dumps(document))
    with pytest.raises(ValueError, match="credential material"):
        asyncio.run(
            synchronize(
                store,
                reflected,
                licenses=(license,),
                environment=ENV,
                now=lambda: OBSERVED,
                transport=httpx.MockTransport(
                    lambda _: pytest.fail("secret plan attempted network IO")
                ),
            )
        )
    assert not list(store.iterdir())
