"""Real source-receipt/Parquet/panel integration using invented wire responses."""

import asyncio
import json
from datetime import date, timedelta
from itertools import count
from pathlib import Path

import httpx
import pytest
from data_helpers import ENVIRONMENT, OBSERVED, AdvancingClock, alpaca_config, fixture, response
from panel_helpers import Case, change, make_case

from loop_research.data.fetch_cache import publish, read_cached
from loop_research.data.fetch_records import DevelopmentBatch, FetchReceipt
from loop_research.data.ingestion import fetch_data
from loop_research.data.snapshot_models import SnapshotRequest
from loop_research.data.snapshots import build_snapshot, read_snapshot
from loop_research.panel_builder import validate_panel
from loop_research.panel_models import PanelRequest


def public_case(directory: Path, *, acquisition_end: str = "2020-12-29") -> Case:
    sources, output = directory / "sources", directory / "output"
    sources.mkdir(mode=0o700)
    output.mkdir(mode=0o700)

    def handle(request: httpx.Request) -> httpx.Response:
        if "/assets/" in request.url.path:
            return response(fixture("alpaca-asset"))
        assert request.url.path.endswith("/bars")
        return response(
            json.dumps(
                {
                    "bars": {
                        "DEMO": [
                            {
                                "t": "2020-12-28T05:00:00Z",
                                "o": 10,
                                "h": 12,
                                "l": 9,
                                "c": 11,
                                "v": 1000,
                                "n": 20,
                                "vw": 10.5,
                            }
                        ]
                    },
                    "next_page_token": None,
                }
            ).encode()
        )

    instants = count()
    fetched = asyncio.run(
        fetch_data(
            alpaca_config(start="2020-12-28", end=acquisition_end, probe_recent_sip=False),
            sources,
            transport=httpx.MockTransport(handle),
            environment=ENVIRONMENT,
            now=lambda: OBSERVED + timedelta(seconds=next(instants)),
            monotonic=AdvancingClock(),
        )
    )
    receipt = FetchReceipt.model_validate_json(read_cached(sources, fetched.receipt))
    batch = DevelopmentBatch.model_validate_json(read_cached(sources, receipt.normalized))
    asset = batch.assets[0]
    snapshot = build_snapshot(
        sources,
        SnapshotRequest(
            receipts=(fetched.receipt.sha256,),
            start=date(2020, 12, 28),
            through=date(2020, 12, 29),
        ),
    )
    capture = publish(
        sources,
        json.dumps(
            {
                "schema": "loop.pit-input/v1",
                "quality": "public_development",
                "captured_at": (OBSERVED + timedelta(days=1)).isoformat(),
                "securities": [
                    {
                        "security_id": asset.security_id,
                        "issuer_id": "issuer.unverified",
                        "ticker": asset.symbol,
                        "venue": "XNAS",
                        "kind": "unknown",
                        "listed": True,
                        "effective_at": asset.observed_at.isoformat(),
                        "known_at": asset.observed_at.isoformat(),
                        "ingested_at": asset.observed_at.isoformat(),
                        "source": asset.source.model_dump(mode="json"),
                    }
                ],
                "bars": [],
                "fundamentals": [],
            }
        ).encode(),
    )
    request = PanelRequest(
        capture=capture,
        source_snapshot=snapshot.snapshot,
        securities=(asset.security_id,),
        fields=("market.close",),
        warmup_start=date(2020, 12, 28),
        sample_start=date(2020, 12, 28),
        sample_end=date(2020, 12, 29),
    )
    return Case(sources, output, request)


# Scenario: source snapshot cannot backfill knowledge.
def test_source_backfill(tmp_path: Path) -> None:
    case = public_case(tmp_path)
    result = case.build()
    assert result.quality == "public_development"
    assert (result.rows, result.eligible_rows, result.observed_rows) == (2, 0, 0)
    assert not result.production_eligible
    assert all(row["known_at_ms"] == row["market.close"] == "" for row in case.rows(result))
    assert validate_panel(case.sources, case.output, result.receipt.sha256) == result
    assert {path.name for path in case.sources.iterdir()}.isdisjoint(
        path.name for path in case.output.iterdir()
    )


# Scenario: public capture requires a snapshot.
def test_public_capture(tmp_path: Path) -> None:
    case = public_case(tmp_path)
    case.request = change(case.request, source_snapshot=None)
    with pytest.raises(ValueError, match="verified source snapshot"):
        case.build()
    assert not list(case.output.iterdir())


# Scenario: public prices cannot be injected.
def test_public_prices(tmp_path: Path) -> None:
    case = public_case(tmp_path)
    assert case.request.source_snapshot is not None
    _, snapshot = read_snapshot(case.sources, case.request.source_snapshot.sha256)
    batch = DevelopmentBatch.model_validate_json(
        read_cached(case.sources, snapshot.parts[0].normalized_source)
    )
    capture = case.capture()
    capture["bars"] = [bar.model_dump(mode="json") for bar in batch.bars]
    case.replace_capture(capture)
    with pytest.raises(ValueError, match="public prices must come from replayed"):
        case.build()
    assert not list(case.output.iterdir())


# Scenario: source parquet corruption blocks publication.
def test_source_parquet(tmp_path: Path) -> None:
    case = public_case(tmp_path)
    assert case.request.source_snapshot is not None
    _, snapshot = read_snapshot(case.sources, case.request.source_snapshot.sha256)
    (case.sources / snapshot.parts[-1].parquet.sha256[7:]).write_bytes(b"damaged")
    with pytest.raises(ValueError):
        case.build()
    assert not list(case.output.iterdir())


# Scenario: acquisition range is checked before replay.
def test_acquisition_range(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    case = public_case(tmp_path, acquisition_end="2020-12-30")

    def forbidden(*args: object, **kwargs: object) -> None:
        pytest.fail("broader source records were replayed before range validation")

    monkeypatch.setattr("loop_research.panel_sources.validate_snapshot", forbidden)
    with pytest.raises(ValueError, match="source acquisition exceeds"):
        case.build()
    assert not list(case.output.iterdir())


# Scenario: snapshot range is checked before replay.
def test_range_replay(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    case = public_case(tmp_path)
    case.request = change(case.request, sample_end="2020-12-28")

    def forbidden(*args: object, **kwargs: object) -> None:
        pytest.fail("broader source snapshot was replayed before range validation")

    monkeypatch.setattr("loop_research.panel_sources.validate_snapshot", forbidden)
    with pytest.raises(ValueError, match="source snapshot exceeds"):
        case.build()
    assert not list(case.output.iterdir())


# Scenario: acquisition after capture cutoff is rejected.
def test_acquisition_capture(tmp_path: Path) -> None:
    case = public_case(tmp_path)
    capture = case.capture()
    capture["captured_at"] = capture["securities"][0]["ingested_at"]
    case.replace_capture(capture)
    with pytest.raises(ValueError, match="source acquisition follows"):
        case.build()
    assert not list(case.output.iterdir())


# Scenario: requested security requires history.
def test_requested_security(tmp_path: Path) -> None:
    case = public_case(tmp_path)
    case.request = change(case.request, securities=["unknown.security"])
    with pytest.raises(ValueError, match="explicit history"):
        case.build()


# Scenario: synthetic capture cannot import public data.
def test_synthetic_capture(tmp_path: Path) -> None:
    case = make_case(tmp_path)
    case.request = change(case.request, source_snapshot=case.request.capture.model_dump())
    with pytest.raises(ValueError, match="synthetic captures cannot import"):
        case.build()
    assert not list(case.output.iterdir())
