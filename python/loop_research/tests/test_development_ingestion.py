"""Executable acquisition, real cache publication and installed CLI replay."""

import asyncio
import json
import os
import subprocess
import sys
from collections.abc import AsyncIterator, Callable
from pathlib import Path

import httpx
import pytest
from data_helpers import (
    ENVIRONMENT,
    OBSERVED,
    AdvancingClock,
    WireBytes,
    alpaca_config,
    fixture,
    fixture_handler,
    response,
    sec_config,
)

from loop_research.data.fetch_cache import publish, read_cached
from loop_research.data.fetch_config import FetchConfig
from loop_research.data.fetch_http import FetchError
from loop_research.data.fetch_records import FetchReceipt, FetchReport
from loop_research.data.ingestion import fetch_data, load_fetch_config, replay_data

REPOSITORY = Path(__file__).resolve().parents[3]


def acquire(
    store: Path,
    config: FetchConfig,
    handler: Callable[[httpx.Request], httpx.Response],
) -> FetchReport:
    return asyncio.run(
        fetch_data(
            config,
            store,
            transport=httpx.MockTransport(handler),
            environment=ENVIRONMENT,
            now=lambda: OBSERVED,
            monotonic=AdvancingClock(),
        )
    )


def receipt(store: Path, report: FetchReport) -> FetchReceipt:
    return FetchReceipt.model_validate_json(read_cached(store, report.receipt))


def run_cli(directory: Path, *arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-I", "-m", "loop_research.cli", *arguments],
        cwd=directory,
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
        env={
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("LOOP_ALPACA_", "APCA_"))
        },
    )


@pytest.mark.parametrize("provider", ["sec", "alpaca"])
def test_acquisition_replays_through_installed_cli(tmp_path: Path, provider: str) -> None:
    config = sec_config() if provider == "sec" else alpaca_config()
    requests: list[httpx.Request] = []
    report = acquire(tmp_path, config, fixture_handler(requests))
    before = {
        path.name: (path.read_bytes(), path.stat().st_mtime_ns) for path in tmp_path.iterdir()
    }
    assert report.result == "records"
    assert report.fundamental_count == (4 if provider == "sec" else 0)
    assert report.bar_count == (2 if provider == "alpaca" else 0)
    assert report.historical_pit == "not_verified"
    assert replay_data(tmp_path, report.receipt.sha256) == report
    completed = run_cli(
        tmp_path, "data-replay", "--store", str(tmp_path), "--receipt", report.receipt.sha256
    )
    assert completed.returncode == 0, completed.stderr
    assert not completed.stderr
    assert json.loads(completed.stdout) == report.model_dump(mode="json")
    assert before == {
        path.name: (path.read_bytes(), path.stat().st_mtime_ns) for path in tmp_path.iterdir()
    }
    metadata = receipt(tmp_path, report)
    assert metadata.attempts == len(requests)
    assert metadata.bytes_received == sum(item.content.byte_size for item in metadata.responses)
    assert all(request.method == "GET" for request in requests)
    for secret in ENVIRONMENT.values():
        assert all(secret.encode() not in value[0] for value in before.values())
        assert secret not in completed.stdout + completed.stderr


def test_requests_pin_feed_pages_and_current_identity(tmp_path: Path) -> None:
    requests: list[httpx.Request] = []
    report = acquire(tmp_path, alpaca_config(), fixture_handler(requests))
    assert requests[0].url == "https://paper-api.alpaca.markets/v2/assets/DEMO"
    bars = [request for request in requests if request.url.path.endswith("/bars")]
    assert len(bars) == 2
    assert "page_token" not in bars[0].url.params
    assert bars[1].url.params["page_token"] == "fixture-page-2="
    for request in bars:
        assert request.url.params["feed"] == "iex"
        assert request.url.params["adjustment"] == "raw"
        assert request.url.params["asof"] == "2026-09-13"
        assert request.url.params["end"] == "2026-09-01T03:59:59.999999+00:00"
        assert request.headers["APCA-API-KEY-ID"] == ENVIRONMENT["LOOP_TEST_KEY"]
    assert requests[-1].url.params["feed"] == "sip"
    assert report.recent_sip == "response_permitted"


def test_sec_user_agent_and_no_credential_headers(tmp_path: Path) -> None:
    requests: list[httpx.Request] = []
    acquire(tmp_path, sec_config(), fixture_handler(requests))
    assert len(requests) == 2
    for request in requests:
        assert request.headers["user-agent"] == "Loop Engine/0.2 test@example.org"
        assert "APCA-API-KEY-ID" not in request.headers


def test_recent_sip_denial_does_not_deny_completed_historical_request(tmp_path: Path) -> None:
    requests: list[httpx.Request] = []
    original = fixture_handler(requests)

    def handle(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/trades/latest"):
            return response(b"private entitlement error", 403)
        return original(request)

    report = acquire(tmp_path, alpaca_config(feed="sip"), handle)
    assert report.bar_count == 2 and report.recent_sip == "forbidden"
    assert replay_data(tmp_path, report.receipt.sha256) == report
    assert all(b"private entitlement" not in path.read_bytes() for path in tmp_path.iterdir())


@pytest.mark.parametrize("status", [401, 403, 429, 503])
def test_historical_denial_never_falls_back_feed(tmp_path: Path, status: int) -> None:
    requests: list[httpx.Request] = []
    original = fixture_handler(requests)
    denied: list[httpx.Request] = []

    def handle(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/bars"):
            denied.append(request)
            return response(b"secret server detail", status)
        return original(request)

    with pytest.raises(FetchError):
        acquire(tmp_path, alpaca_config(feed="sip"), handle)
    assert len(denied) == 1 and denied[0].url.params["feed"] == "sip"
    assert len(list(tmp_path.iterdir())) == 1  # The already captured current asset survives.
    assert not any(b"loop.development-receipt" in path.read_bytes() for path in tmp_path.iterdir())


def test_empty_data_is_not_authentication_failure(tmp_path: Path) -> None:
    original = fixture_handler([])

    def handle(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/bars"):
            return response(b'{"bars":{},"next_page_token":null}')
        return original(request)

    report = acquire(tmp_path, alpaca_config(probe_recent_sip=False), handle)
    assert report.result == "empty" and report.missing == ("DEMO",)
    assert report.recent_sip == "not_requested"
    assert replay_data(tmp_path, report.receipt.sha256) == report


def test_missing_credentials_fail_before_io(tmp_path: Path) -> None:
    calls: list[httpx.Request] = []
    with pytest.raises(FetchError, match="missing_credentials"):
        asyncio.run(
            fetch_data(
                alpaca_config(),
                tmp_path,
                transport=httpx.MockTransport(fixture_handler(calls)),
                environment={},
                now=lambda: OBSERVED,
            )
        )
    assert not calls and not list(tmp_path.iterdir())


def test_cli_missing_credentials_is_redacted(tmp_path: Path) -> None:
    result = run_cli(
        tmp_path,
        "data-fetch",
        str(REPOSITORY / "config/data/alpaca-development.toml"),
        "--store",
        str(tmp_path),
    )
    assert result.returncode == 2 and not result.stdout
    assert "missing_credentials" in result.stderr and "Traceback" not in result.stderr
    assert not list(tmp_path.iterdir())


def test_reflected_credential_cannot_enter_cache(tmp_path: Path) -> None:
    secret = ENVIRONMENT["LOOP_TEST_SECRET"].encode()
    with pytest.raises(FetchError, match="invalid_response"):
        acquire(tmp_path, alpaca_config(), lambda _: response(b'{"detail":"' + secret + b'"}'))
    assert not list(tmp_path.iterdir())


@pytest.mark.parametrize(
    ("budget", "reason"),
    [
        ({"pages": 1}, "page_budget"),
        ({"records": 1}, "record_budget"),
    ],
)
def test_partial_pagination_cannot_complete(
    tmp_path: Path, budget: dict[str, int], reason: str
) -> None:
    with pytest.raises(FetchError, match=reason):
        acquire(tmp_path, alpaca_config(budget=budget), fixture_handler([]))
    assert not any(b"loop.development-receipt" in path.read_bytes() for path in tmp_path.iterdir())


def test_repeated_page_token_cannot_loop(tmp_path: Path) -> None:
    original = fixture_handler([])

    def handle(request: httpx.Request) -> httpx.Response:
        return (
            response(fixture("alpaca-page-1"))
            if request.url.path.endswith("/bars")
            else original(request)
        )

    with pytest.raises(FetchError, match="invalid_response"):
        acquire(tmp_path, alpaca_config(), handle)
    assert not any(b"loop.development-receipt" in path.read_bytes() for path in tmp_path.iterdir())


def test_data_fetch_cancellation_preserves_only_captured_evidence(tmp_path: Path) -> None:
    async def run() -> None:
        waiting = asyncio.Event()

        class Stalled(WireBytes):
            async def __aiter__(self) -> AsyncIterator[bytes]:
                waiting.set()
                await asyncio.Event().wait()
                yield b"{}"

        stream = Stalled(b"")
        original = fixture_handler([])

        def handle(request: httpx.Request) -> httpx.Response:
            if "/submissions/" in request.url.path:
                return httpx.Response(
                    200, headers={"content-type": "application/json"}, stream=stream
                )
            return original(request)

        task = asyncio.create_task(
            fetch_data(
                sec_config(),
                tmp_path,
                transport=httpx.MockTransport(handle),
                now=lambda: OBSERVED,
                monotonic=AdvancingClock(),
            )
        )
        await asyncio.wait_for(waiting.wait(), timeout=3)
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
        assert stream.closed
        assert [path.read_bytes() for path in tmp_path.iterdir()] == [fixture("sec-facts")]

    asyncio.run(run())


@pytest.mark.parametrize("part", ["config", "normalized", "source", "receipt"])
def test_corrupted_cache_is_not_repaired(tmp_path: Path, part: str) -> None:
    report = acquire(tmp_path, sec_config(), fixture_handler([]))
    metadata = receipt(tmp_path, report)
    reference = {
        "config": metadata.config,
        "normalized": metadata.normalized,
        "source": metadata.responses[0].content,
        "receipt": report.receipt,
    }[part]
    path = tmp_path / reference.sha256[7:]
    original = path.read_bytes()
    path.write_bytes(original + b" ")
    with pytest.raises(FetchError, match="invalid_cache"):
        replay_data(tmp_path, report.receipt.sha256)
    assert path.read_bytes() == original + b" "


def test_replay_verifies_request_semantics_not_just_hashes(tmp_path: Path) -> None:
    report = acquire(tmp_path, alpaca_config(), fixture_handler([]))
    metadata = receipt(tmp_path, report).model_dump(mode="json", by_alias=True)
    metadata["responses"][1]["url"] = metadata["responses"][1]["url"].replace(
        "feed=iex", "feed=sip"
    )
    forged = publish(tmp_path, json.dumps(metadata).encode())
    with pytest.raises(FetchError, match="invalid_cache"):
        replay_data(tmp_path, forged.sha256)
    assert replay_data(tmp_path, report.receipt.sha256) == report


def test_replay_recomputes_normalized_values(tmp_path: Path) -> None:
    report = acquire(tmp_path, sec_config(), fixture_handler([]))
    metadata = receipt(tmp_path, report).model_dump(mode="json", by_alias=True)
    normalized = read_cached(tmp_path, receipt(tmp_path, report).normalized)
    changed = publish(tmp_path, normalized.replace(b"9007199254740993.02", b"9007199254740993.03"))
    metadata["normalized"] = changed.model_dump()
    forged = publish(tmp_path, json.dumps(metadata).encode())
    with pytest.raises(FetchError, match="invalid_cache"):
        replay_data(tmp_path, forged.sha256)


def test_identical_capture_reuses_objects_without_overwrite(tmp_path: Path) -> None:
    report = acquire(tmp_path, sec_config(), fixture_handler([]))
    before = {path.name: path.stat().st_ino for path in tmp_path.iterdir()}
    assert acquire(tmp_path, sec_config(), fixture_handler([])) == report
    assert before == {path.name: path.stat().st_ino for path in tmp_path.iterdir()}


def test_cache_publication_detects_existing_corruption(tmp_path: Path) -> None:
    report = acquire(tmp_path, sec_config(), fixture_handler([]))
    path = tmp_path / receipt(tmp_path, report).responses[0].content.sha256[7:]
    path.write_bytes(b"corrupt")
    with pytest.raises(FetchError, match="invalid_cache"):
        acquire(tmp_path, sec_config(), fixture_handler([]))
    assert path.read_bytes() == b"corrupt"


@pytest.mark.parametrize("kind", ["permissions", "symlink"])
def test_cache_requires_private_canonical_directory(tmp_path: Path, kind: str) -> None:
    store = tmp_path / "cache"
    store.mkdir(mode=0o700)
    if kind == "permissions":
        store.chmod(0o755)
    else:
        linked = tmp_path / "alias"
        linked.symlink_to(store, target_is_directory=True)
        store = linked
    requests: list[httpx.Request] = []
    with pytest.raises(FetchError, match="invalid_cache"):
        acquire(store, sec_config(), fixture_handler(requests))
    assert not requests


def test_current_incomplete_day_is_not_downloaded(tmp_path: Path) -> None:
    requests: list[httpx.Request] = []
    with pytest.raises(FetchError, match="invalid_configuration"):
        acquire(tmp_path, sec_config(end="2026-09-13"), fixture_handler(requests))
    assert not requests and not list(tmp_path.iterdir())


@pytest.mark.parametrize("name", ["sec-development", "alpaca-development"])
def test_shipped_toml_config_is_executable(name: str) -> None:
    config = load_fetch_config(REPOSITORY / ("config/data/" + name + ".toml"))
    assert config.end.isoformat() == "2026-08-31"


@pytest.mark.parametrize(
    "body",
    [
        b'provider="secret-invalid"',
        b'provider="sec"\nprovider="sec"',
        b'api_key="fixture-secret-must-not-appear"',
        b"x=" + b" " * (64 * 1024),
    ],
)
def test_bad_configuration_is_redacted(tmp_path: Path, body: bytes) -> None:
    path = tmp_path / "bad.toml"
    path.write_bytes(body)
    with pytest.raises(FetchError, match="invalid_configuration") as caught:
        load_fetch_config(path)
    assert "fixture-secret" not in str(caught.value)


def test_escaped_credential_echo_is_not_persisted(tmp_path: Path) -> None:
    secret = ENVIRONMENT["LOOP_TEST_SECRET"]
    escaped = "".join("\\u" + format(ord(char), "04x") for char in secret)
    body = ('{"detail":"' + escaped + '"}').encode()
    assert secret.encode() not in body
    with pytest.raises(FetchError, match="invalid_response"):
        acquire(tmp_path, alpaca_config(), lambda _: response(body))
    assert not list(tmp_path.iterdir())


def test_failed_publish_never_emits_a_receipt(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import loop_research.data.ingestion as ingestion

    real_publish = ingestion.publish

    def deny_normalized(store: Path, content: bytes):
        if b"loop.development-batch" in content:
            raise OSError("private path must not be printed")
        return real_publish(store, content)

    monkeypatch.setattr(ingestion, "publish", deny_normalized)
    with pytest.raises(FetchError, match="invalid_cache"):
        acquire(tmp_path, sec_config(), fixture_handler([]))
    assert len(list(tmp_path.iterdir())) == 2
    assert not any(b"loop.development-receipt" in path.read_bytes() for path in tmp_path.iterdir())


@pytest.mark.parametrize("kind", ["symlink", "fifo", "directory"])
def test_configuration_rejects_special_files(tmp_path: Path, kind: str) -> None:
    path = tmp_path / "config"
    if kind == "symlink":
        path.symlink_to(REPOSITORY / "config/data/sec-development.toml")
    elif kind == "fifo":
        os.mkfifo(path)
    else:
        path.mkdir()
    with pytest.raises(FetchError, match="invalid_configuration"):
        load_fetch_config(path)
