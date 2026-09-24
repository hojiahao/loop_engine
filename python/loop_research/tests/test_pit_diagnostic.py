"""Actual-file and installed-command acceptance for the local PIT diagnostic."""

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

from loop_research.data import diagnostic, models
from loop_research.data.diagnostic import query_file
from loop_research.data.models import PitQuery, parse_instant

FIXTURE = Path(__file__).resolve().parents[3] / "fixtures/market/pit/history.json"
DECISION = "2020-03-02T22:00:00Z"
CAPTURED = "2026-09-13T00:00:00Z"


def query() -> PitQuery:
    return PitQuery(
        market_at=parse_instant(DECISION),
        known_at=parse_instant(DECISION),
        ingested_at=parse_instant(CAPTURED),
        ticker="DEMO",
        venue="XNYS",
    )


def run_cli(path: Path, *arguments: str, directory: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            sys.executable,
            "-I",
            "-m",
            "loop_research.cli",
            "data-query",
            str(path),
            "--market-at",
            DECISION,
            "--known-at",
            DECISION,
            "--ingested-at",
            CAPTURED,
            *arguments,
        ],
        cwd=directory,
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )


# Scenario: installed cli resolves history without writing.
def test_installed_cli(tmp_path: Path) -> None:
    before = FIXTURE.read_bytes()
    completed = run_cli(FIXTURE, "--ticker", "DEMO", "--venue", "XNYS", directory=tmp_path)
    assert completed.returncode == 0, completed.stderr
    assert not completed.stderr
    report = json.loads(completed.stdout)
    assert report["input_sha256"] == "sha256:" + hashlib.sha256(before).hexdigest()
    assert report["declared_quality"] == "synthetic"
    assert report["quality_verification"] == "not_attested"
    assert report["calendar_validation"] == "not_performed"
    result = report["result"]
    assert result["securities"][0]["state"]["security_id"] == "synthetic:new-a"
    assert result["fundamentals"][0]["value"] == "9007199254740993.01"
    assert result["bars"][0]["price_basis"] == "raw"
    encoded = json.dumps(result, separators=(",", ":")).encode()
    assert report["result_sha256"] == "sha256:" + hashlib.sha256(encoded).hexdigest()
    assert FIXTURE.read_bytes() == before
    assert not list(tmp_path.iterdir())


# Scenario: repeated reports have identical bytes.
def test_repeated_reports() -> None:
    assert (
        query_file(FIXTURE, query()).model_dump_json()
        == query_file(FIXTURE, query()).model_dump_json()
    )


# Scenario: byte provenance changes without changing selected values.
def test_byte_provenance(tmp_path: Path) -> None:
    original = query_file(FIXTURE, query())
    reformatted = tmp_path / "reformatted.json"
    reformatted.write_text(json.dumps(json.loads(FIXTURE.read_bytes())), encoding="utf8")
    changed = query_file(reformatted, query())
    assert changed.input_sha256 != original.input_sha256
    assert changed.result_sha256 == original.result_sha256


@pytest.mark.parametrize(
    "content",
    [
        b"",
        b"{",
        b"[]",
        b"{}",
        b"null",
        b"\xff",
        b"[] trailing",
        b'{"schema":"loop.pit-input/v1","schema":"loop.pit-input/v1"}',
        b"[" * 1100 + b"]" * 1100,
    ],
)
# Scenario: malformed input has no report.
def test_malformed_input(tmp_path: Path, content: bytes) -> None:
    path = tmp_path / "invalid.json"
    path.write_bytes(content)
    with pytest.raises(ValueError):
        query_file(path, query())


# Scenario: nested duplicate fields are rejected.
def test_nested_fields(tmp_path: Path) -> None:
    path = tmp_path / "ambiguous.json"
    path.write_bytes(
        FIXTURE.read_bytes().replace(b'"volume": 12345', b'"volume": 1, "volume": 12345')
    )
    with pytest.raises(ValueError, match="duplicate JSON field"):
        query_file(path, query())


# Scenario: unknown fields are rejected.
def test_unknown_fields(tmp_path: Path) -> None:
    values = json.loads(FIXTURE.read_bytes())
    values["securities"][0]["trust_me"] = True
    path = tmp_path / "unknown.json"
    path.write_text(json.dumps(values), encoding="utf8")
    with pytest.raises(ValueError):
        query_file(path, query())


# Scenario: input byte budget.
def test_input_byte(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(diagnostic, "MAX_INPUT_BYTES", 32)
    path = tmp_path / "oversized.json"
    path.write_bytes(b" " * 33)
    with pytest.raises(ValueError, match="byte budget"):
        query_file(path, query())


# Scenario: total record budget.
def test_total_budget(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(models, "MAX_RECORDS", 6)
    with pytest.raises(ValueError, match="record budget"):
        query_file(FIXTURE, query())


@pytest.mark.parametrize("kind", ["symlink", "directory", "fifo"])
# Scenario: special files are rejected without blocking.
def test_special_files(tmp_path: Path, kind: str) -> None:
    path = tmp_path / "input"
    if kind == "symlink":
        path.symlink_to(FIXTURE)
    elif kind == "directory":
        path.mkdir()
    else:
        os.mkfifo(path)
    with pytest.raises((ValueError, OSError)):
        query_file(path, query())


# Scenario: changed file is rejected.
def test_changed_file(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    path = tmp_path / "changing.json"
    path.write_bytes(FIXTURE.read_bytes())
    real_stat = os.fstat
    calls = 0

    def mutate_after_read(descriptor: int) -> os.stat_result:
        nonlocal calls
        calls += 1
        if calls == 2:
            with path.open("ab") as stream:
                stream.write(b" ")
        return real_stat(descriptor)

    monkeypatch.setattr(diagnostic.os, "fstat", mutate_after_read)
    with pytest.raises(ValueError, match="changed during read"):
        query_file(path, query())


# Scenario: cli errors do not echo source payloads.
def test_cli_errors(tmp_path: Path) -> None:
    path = tmp_path / "private-input.json"
    path.write_text('{"private_field":"fixture-secret-must-not-appear"}', encoding="utf8")
    completed = run_cli(path, directory=tmp_path)
    assert completed.returncode == 2
    assert not completed.stdout
    assert "PIT query failed" in completed.stderr
    assert "fixture-secret-must-not-appear" not in completed.stderr
    assert str(path) not in completed.stderr
    assert "Traceback" not in completed.stderr


# Scenario: cli requires a venue for ticker lookup.
def test_cli_venue(tmp_path: Path) -> None:
    completed = run_cli(FIXTURE, "--ticker", "DEMO", directory=tmp_path)
    assert completed.returncode == 2
    assert not completed.stdout
