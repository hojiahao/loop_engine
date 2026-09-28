"""Offline access diagnostics and production preflight share negative paths."""

import asyncio
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

import httpx
import pytest
from data_helpers import ENVIRONMENT, OBSERVED, alpaca_config, sec_config
from licensed_helpers import DB_KEY, ENV, KEY, cache, license_config
from test_data_sync import plan_for

from loop_research.data.access import (
    LicenseFile,
    check_requests,
    license_files,
    load_access_config,
    preflight,
)
from loop_research.data.fetch_http import FetchError
from loop_research.data.snapshot_models import SyncPlan
from loop_research.data.sync import synchronize

REPOSITORY = Path(__file__).resolve().parents[3]


@pytest.mark.parametrize("provider", ["sec", "alpaca", "sharadar", "wrds", "databento"])
# Scenario: ready diagnosis never connects or publishes.
def test_ready_diagnosis(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, provider: str) -> None:
    def forbidden(*_: Any, **__: Any) -> None:
        pytest.fail("offline preflight attempted a network connection")

    monkeypatch.setattr(httpx.AsyncClient, "__init__", forbidden)
    monkeypatch.setattr("psycopg.AsyncConnection.connect", forbidden)
    store = cache(tmp_path)
    licenses = ()
    if provider == "sec":
        config, environment = sec_config(), {}
    elif provider == "alpaca":
        config, environment = alpaca_config(), ENVIRONMENT
    else:
        config, license = license_config(tmp_path, provider)
        licenses = (license,)
        environment = {**ENV, "LOOP_TEST_KEY": DB_KEY if provider == "databento" else KEY}
    report = preflight(config, store, licenses=licenses, environment=environment, at=OBSERVED)
    assert report.local_ready and report.cache == "ready"
    assert report.live_access == "not_checked" and not report.production_eligible
    assert not report.sources[0].issues
    assert not list(store.iterdir())
    assert all(secret not in report.model_dump_json() for secret in environment.values())


# Scenario: all requests report missing access.
def test_requests_report(tmp_path: Path) -> None:
    plan, _ = plan_for(tmp_path)
    report = preflight(plan, cache(tmp_path), environment={}, at=OBSERVED)
    assert not report.local_ready
    assert not report.sources[0].issues
    assert report.sources[1].issues == ("missing_credentials", "license_denied")
    assert report.sources[1].missing_references == ("LOOP_TEST_KEY",)
    assert report.sources[1].license_datasets == ("SHARADAR/SEP",)


@pytest.mark.parametrize("value", ["", "line\nbreak", "密钥", "a" * 1025, "invalid key format"])
# Scenario: bad credentials are redacted.
def test_bad_credentials(tmp_path: Path, value: str) -> None:
    config, license = license_config(tmp_path)
    report = preflight(
        config,
        cache(tmp_path),
        licenses=(license,),
        environment={"LOOP_TEST_KEY": value},
        at=OBSERVED,
    )
    assert not report.local_ready
    assert report.sources[0].issues == ("missing_credentials",)
    if value:
        assert value not in report.model_dump_json()


@pytest.mark.parametrize(
    "changes",
    [
        {"expires_at": OBSERVED.isoformat()},
        {"valid_from": "2026-10-01T00:00:00Z"},
        {"data_end": "2026-08-30"},
        {"datasets": ["SHARADAR/SF1"]},
    ],
)
# Scenario: license scope and validity are checked.
def test_license_scope(tmp_path: Path, changes: dict[str, Any]) -> None:
    config, license = license_config(tmp_path, license_changes=changes)
    report = preflight(config, cache(tmp_path), licenses=(license,), environment=ENV, at=OBSERVED)
    assert report.sources[0].issues == ("license_denied",)
    assert not report.local_ready


# Scenario: replaced license does not match pinned identity.
def test_replaced_license(tmp_path: Path) -> None:
    config, license = license_config(tmp_path)
    original = license_files((license,))
    license.write_bytes(license.read_bytes() + b"\n")
    current = license_files((license,))
    assert set(current) != set(original)
    report = preflight(config, cache(tmp_path), licenses=(license,), environment=ENV, at=OBSERVED)
    assert report.sources[0].issues == ("license_denied",)
    # Even an incorrectly indexed caller mapping cannot bypass the digest check.
    forged = {config.license_sha256: LicenseFile(license, license.read_bytes())}
    assert check_requests((config,), forged, ENV, OBSERVED, config.model_dump_json().encode())[
        0
    ].issues == ("license_denied",)


@pytest.mark.parametrize("kind", ["shared", "symlink", "fifo", "duplicate"])
# Scenario: unsafe license files fail without writes.
def test_unsafe_license(tmp_path: Path, kind: str) -> None:
    config, license = license_config(tmp_path)
    store = cache(tmp_path)
    paths = (license,)
    if kind == "shared":
        license.chmod(0o644)
    elif kind == "symlink":
        link = tmp_path / "linked-license"
        link.symlink_to(license)
        paths = (link,)
    elif kind == "fifo":
        pipe = tmp_path / "license-pipe"
        os.mkfifo(pipe, 0o600)
        paths = (pipe,)
    else:
        paths = (license, license)
    with pytest.raises((OSError, ValueError)):
        preflight(config, store, licenses=paths, environment=ENV, at=OBSERVED)
    assert not list(store.iterdir())


@pytest.mark.parametrize("ambient", [False, True])
# Scenario: ambient wrds settings do not pass readiness.
def test_ambient_wrds(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, ambient: bool) -> None:
    config, license = license_config(tmp_path, "wrds")
    environment = dict(ENV)
    if ambient:
        monkeypatch.setenv("PGHOST", "untrusted.invalid")
    else:
        environment["PGHOST"] = "untrusted.invalid"
    report = preflight(
        config,
        cache(tmp_path),
        licenses=(license,),
        environment=environment,
        at=OBSERVED,
    )
    assert report.sources[0].issues == ("invalid_configuration",)
    assert "untrusted.invalid" not in report.model_dump_json()


# Scenario: current new york date is not complete data.
def test_york_date(tmp_path: Path) -> None:
    # 01:00 UTC on September 1 is still August 31 in New York.
    from datetime import datetime

    report = preflight(
        sec_config(),
        cache(tmp_path),
        environment={},
        at=datetime.fromisoformat("2026-09-01T01:00:00+00:00"),
    )
    assert report.sources[0].issues == ("invalid_configuration",)


# Scenario: invalid cache is reported without creating it.
def test_invalid_cache(tmp_path: Path) -> None:
    absent = tmp_path / "absent"
    report = preflight(sec_config(), absent, environment={}, at=OBSERVED)
    assert report.cache == "invalid_cache" and not report.local_ready
    assert not absent.exists()


# Scenario: credential in reference name never reaches report.
def test_credential_reference(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="credential material"):
        preflight(
            alpaca_config(),
            cache(tmp_path),
            environment={**ENVIRONMENT, "LOOP_TEST_SECRET": "LOOP_TEST_KEY"},
            at=OBSERVED,
        )


@pytest.mark.parametrize("failure", ["license_secret", "future_request"])
# Scenario: later request denies batch before first download.
def test_later_request(tmp_path: Path, failure: str) -> None:
    store = cache(tmp_path)
    plan, license = plan_for(tmp_path)
    instant = OBSERVED
    if failure == "license_secret":
        config, license = license_config(tmp_path, license_changes={"license_id": KEY})
        shape = plan.model_dump(mode="json", by_alias=True)
        shape["requests"][1] = config.model_dump(mode="json", by_alias=True)
    else:
        shape = plan.model_dump(mode="json", by_alias=True)
        shape["requests"][0]["end"] = "2026-08-15"
        instant = OBSERVED.replace(month=8, day=30)
    plan = SyncPlan.model_validate_json(json.dumps(shape))
    with pytest.raises((ValueError, FetchError)):
        asyncio.run(
            synchronize(
                store,
                plan,
                licenses=(license,),
                environment=ENV,
                now=lambda: instant,
                transport=httpx.MockTransport(lambda _: pytest.fail("batch attempted network IO")),
            )
        )
    assert not list(store.iterdir())


# Scenario: previous readiness is not an access grant.
def test_previous_readiness(tmp_path: Path) -> None:
    plan, license = plan_for(tmp_path)
    store = cache(tmp_path)
    assert preflight(plan, store, licenses=(license,), environment=ENV, at=OBSERVED).local_ready
    with pytest.raises(FetchError, match="missing_credentials"):
        asyncio.run(
            synchronize(
                store,
                plan,
                licenses=(license,),
                environment={},
                now=lambda: OBSERVED,
                transport=httpx.MockTransport(lambda _: pytest.fail("stale preflight allowed IO")),
            )
        )
    assert not list(store.iterdir())


@pytest.mark.parametrize("config_path", sorted((REPOSITORY / "config/data").glob("*.toml")))
# Scenario: shipped configuration reports real local requirements.
def test_shipped_configuration(tmp_path: Path, config_path: Path) -> None:
    config = load_access_config(config_path)
    report = preflight(config, cache(tmp_path), environment={}, at=OBSERVED)
    sources = config.requests if isinstance(config, SyncPlan) else (config,)
    assert report.local_ready == all(source.provider == "sec" for source in sources)
    assert len(report.sources) == len(sources)
    assert all(source.issues for source in report.sources if source.provider != "sec")


@pytest.mark.parametrize("config_name,code", [("sec-development", 0), ("alpaca-development", 3)])
# Scenario: installed cli emits readiness and exit status.
def test_installed_cli(tmp_path: Path, config_name: str, code: int) -> None:
    store = cache(tmp_path)
    completed = subprocess.run(
        [
            sys.executable,
            "-I",
            "-m",
            "loop_research.cli",
            "data-preflight",
            str(REPOSITORY / f"config/data/{config_name}.toml"),
            "--store",
            str(store),
        ],
        env={"OPENBLAS_NUM_THREADS": "1"},
        capture_output=True,
        text=True,
        timeout=20,
        check=False,
    )
    assert completed.returncode == code, completed.stderr
    report = json.loads(completed.stdout)
    assert report["local_ready"] == (code == 0)
    assert report["live_access"] == "not_checked"
    assert not completed.stderr and not list(store.iterdir())


# Scenario: cli malformed config is redacted.
def test_cli_malformed(tmp_path: Path) -> None:
    config = tmp_path / "unsafe.toml"
    config.write_text(f'api_key = "{KEY}"\n')
    completed = subprocess.run(
        [
            sys.executable,
            "-I",
            "-m",
            "loop_research.cli",
            "data-preflight",
            str(config),
            "--store",
            str(cache(tmp_path)),
        ],
        capture_output=True,
        text=True,
        timeout=20,
        check=False,
    )
    assert completed.returncode == 2 and not completed.stdout
    assert KEY not in completed.stderr and str(config) not in completed.stderr
