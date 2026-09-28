import copy
import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest
from loop.v1.evaluation_pb2 import FactorEvaluationWork
from loop_protocol.job import factor_identity_hash
from test_backtest_workflow import Case, policies
from test_backtest_workflow import test_installed_cli as assert_installed_replay
from test_factor_worker import build as build
from test_factor_worker import prepared as prepared
from test_market_portfolio import cash_action, source, stamp, terms

from loop_research.backtest_models import BacktestRequest
from loop_research.build_identity import canonical_bytes
from loop_research.data.fetch_cache import publish, read_cached, read_receipt
from loop_research.data.fetch_records import CachedObject
from loop_research.factor_worker import execute
from loop_research.transform_models import PolicyDocument


@dataclass
class MarketCase(Case):
    capture: dict[str, Any]

    def save(self, **tape_changes: Any) -> None:
        capture = publish(self.evidence, canonical_bytes(self.capture))
        tape = publish(
            self.evidence,
            canonical_bytes(
                {
                    "schema": "loop.execution-tape/v2",
                    "quality": "synthetic",
                    "currency": "USD",
                    "price_basis": "raw",
                    "coverage": "explicit_development_declaration",
                    "capture": capture.model_dump(),
                    **tape_changes,
                }
            ),
        )
        self.change_request(execution_tape=tape.model_dump())


@pytest.fixture
def market_case(prepared: tuple[FactorEvaluationWork, Path, Path], tmp_path: Path) -> MarketCase:
    work, view, evidence = prepared
    documents = policies()
    settings = {
        "portfolio_policy": {
            "algorithm": "ranked-long-short.1",
            "holdings": "1",
            "lot_size": "1",
            "initial_cash_usd": "1000",
            "long_weight_bps": "5000",
            "short_weight_bps": "5000",
            "initial_margin_bps": "5000",
            "maintenance_margin_bps": "2500",
        },
        "execution_policy": {"algorithm": "pit-next-open.1", "participation_bps": "10000"},
        "cost_policy": {
            "algorithm": "commission-impact-finance.1",
            "commission_per_share_usd": "0",
            "minimum_commission_usd": "0",
            "half_spread_bps": "0",
            "impact_bps": "0",
            "short_collateral_bps": "10200",
            "cash_debit_bps": "0",
            "cash_credit_bps": "0",
            "day_count": "360",
        },
    }
    for role, values in settings.items():
        documents[role] = PolicyDocument.model_validate_json(
            json.dumps(
                {
                    **documents[role].model_dump(mode="json", by_alias=True),
                    "settings": dict(sorted(values.items())),
                }
            )
        )
    for role, document in documents.items():
        getattr(work.factor.frozen_policy, role).sha256.value = bytes.fromhex(document.digest()[7:])
    work.factor.factor_spec_id.value = "sha256:" + factor_identity_hash(work.factor).hex()
    work_ref = publish(evidence, work.SerializeToString())
    result = execute(work, view=view, output=evidence)
    request = BacktestRequest(
        evaluation_work=work_ref,
        evaluation_result=CachedObject(
            sha256=result.manifest.artifact_id.value, byte_size=result.manifest.byte_size
        ),
        factor_values=CachedObject(
            sha256=result.values.artifact_id.value, byte_size=result.values.byte_size
        ),
        execution_tape=CachedObject(sha256="sha256:" + "0" * 64, byte_size=1),
        policies=documents,
    )
    store = tmp_path / "ledger"
    store.mkdir(mode=0o700)
    prices = []
    availability = []
    for day in (4, 5, 6):
        for security in ("US.001", "US.002"):
            for kind in ("open", "close"):
                at = stamp(day, 14, 30) if kind == "open" else stamp(day, 21, 0)
                prices.append(
                    {
                        "security_id": security,
                        "session": f"2010-01-0{day}",
                        "kind": kind,
                        "effective_at": at,
                        "known_at": at,
                        "ingested_at": stamp(day, 22, 0),
                        "source": source(),
                        "price_usd": "9" if day == 6 and security == "US.001" else "10",
                        "auction_volume": 1000 if kind == "open" else None,
                    }
                )
            availability.append(terms(day, security).model_dump(mode="json"))
    actions = [cash_action(6, security="US.001").model_dump(mode="json")]
    raw = publish(
        evidence,
        canonical_bytes(
            {
                "fixture": "invented-auction-actions.1",
                "prices": prices,
                "terms": availability,
                "actions": actions,
            }
        ),
    )
    for record in [*prices, *availability, *actions]:
        record["source"]["raw_sha256"] = raw.sha256
    capture = {
        "schema": "loop.execution-capture/v1",
        "captured_at": stamp(6, 23, 0),
        "prices": prices,
        "terms": availability,
        "actions": actions,
        "sources": [raw.model_dump()],
    }
    case = MarketCase(work, evidence, view, store, request, [], capture)
    case.save()
    return case


# Scenario: actual factor to action ledger and byte replay.
def test_action_replay(market_case: MarketCase) -> None:
    result = market_case.run()
    assert result.ending_nav_usd == "1000" and not result.production_eligible
    assert result.orders == result.fills == 2
    _, receipt = read_receipt(market_case.store, result.receipt.sha256)
    assert json.loads(receipt)["engine"] == "pit-actions-long-short.1"
    assert b"2010-01-06,1000,50,-50,950,550,1000\n" in read_cached(
        market_case.store, result.artifacts.nav
    )
    before = {
        path.name: (path.stat().st_mtime_ns, path.read_bytes())
        for path in market_case.store.iterdir()
    }
    assert market_case.validate(result.receipt.sha256) == result
    assert {
        path.name: (path.stat().st_mtime_ns, path.read_bytes())
        for path in market_case.store.iterdir()
    } == before


# Scenario: installed market cli.
def test_market_cli(market_case: MarketCase, tmp_path: Path) -> None:
    assert_installed_replay(market_case, tmp_path)


# Scenario: late price revisions cannot rewrite fills.
def test_price_revisions(market_case: MarketCase) -> None:
    original = market_case.run()
    records = market_case.capture["prices"]
    opening = copy.deepcopy(
        next(row for row in records if row["session"] == "2010-01-05" and row["kind"] == "open")
    )
    opening.update(known_at=stamp(5, 15, 0), price_usd="1", auction_volume=1)
    closing = copy.deepcopy(
        next(row for row in records if row["session"] == "2010-01-06" and row["kind"] == "close")
    )
    closing.update(known_at=stamp(6, 21, 6), price_usd="1")
    records.extend((opening, closing))
    market_case.save()
    revised = market_case.run()
    assert revised.artifacts == original.artifacts
    assert revised.receipt != original.receipt


# Scenario: future borrow limit cannot limit an earlier fill.
def test_borrow_visibility(market_case: MarketCase) -> None:
    record = copy.deepcopy(market_case.capture["terms"][3])
    record.update(known_at=stamp(5, 14, 31), borrow_limit=20)
    market_case.capture["terms"].append(record)
    market_case.save()
    # The 14:31 record cannot authorize/cap the 14:30 fill. An unsupported
    # intraday withdrawal subsequently invalidates the completed daily replay.
    with pytest.raises(ValueError, match="intraday borrow"):
        market_case.run()
    assert not list(market_case.store.iterdir())


@pytest.mark.parametrize(
    "problem, error",
    [
        ("duplicate", "duplicate"),
        ("future_action", "unavailable"),
        ("combined_action", "combined"),
        ("sample", "development sample"),
        ("missing_mark", "visible closing mark"),
        ("clock_precision", "millisecond"),
        ("wrong_quality", "source quality"),
        ("open_event", "regular trading"),
        ("source_set", "source set"),
    ],
)
# Scenario: bad execution evidence fails before publication.
def test_invalid_evidence(market_case: MarketCase, problem: str, error: str) -> None:
    capture = market_case.capture
    if problem == "duplicate":
        capture["prices"].append(copy.deepcopy(capture["prices"][0]))
    elif problem == "future_action":
        capture["actions"][0]["known_at"] = stamp(6, 14, 1)
    elif problem == "combined_action":
        record = copy.deepcopy(capture["actions"][0])
        record["event_id"] = "other.action"
        capture["actions"].append(record)
    elif problem == "sample":
        capture["prices"][0]["session"] = "2025-01-02"
    elif problem == "missing_mark":
        capture["prices"] = [
            row
            for row in capture["prices"]
            if not (row["session"] == "2010-01-06" and row["kind"] == "close")
        ]
    elif problem == "clock_precision":
        capture["prices"][0]["effective_at"] = "2010-01-04T14:30:00.000001+00:00"
        capture["prices"][0]["known_at"] = "2010-01-04T14:30:00.000001+00:00"
    elif problem == "wrong_quality":
        capture["prices"][0]["source"]["availability"] = "publisher_timestamp"
    elif problem == "open_event":
        capture["prices"][0]["effective_at"] = stamp(4, 14, 0)
    else:
        capture["sources"][0]["sha256"] = "sha256:" + "b" * 64
    market_case.save()
    with pytest.raises(ValueError, match=error):
        market_case.run()
    assert not list(market_case.store.iterdir())


# Scenario: missing opening terms cancel new exposure.
def test_missing_terms(market_case: MarketCase) -> None:
    # Keep closing records, but make opening availability not yet known.
    for record in market_case.capture["terms"]:
        record["known_at"] = stamp(int(record["session"][-2:]), 15, 0)
    market_case.save()
    result = market_case.run()
    assert result.fills == 0 and result.ending_nav_usd == "1000"
    assert b"missing_terms" in read_cached(market_case.store, result.artifacts.orders)


# Scenario: corrupt raw source is not repaired.
def test_corrupt_source(market_case: MarketCase) -> None:
    reference = market_case.capture["sources"][0]
    path = market_case.evidence / reference["sha256"][7:]
    path.write_bytes(b"corrupt")
    with pytest.raises(ValueError):
        market_case.run()
    assert path.read_bytes() == b"corrupt" and not list(market_case.store.iterdir())


# Scenario: out of scope metadata rejects before raw access.
def test_source_scope(market_case: MarketCase) -> None:
    market_case.capture["prices"][0]["session"] = "2025-01-02"
    reference = market_case.capture["sources"][0]
    (market_case.evidence / reference["sha256"][7:]).write_bytes(b"corrupt raw source")
    market_case.save()
    with pytest.raises(ValueError, match="development sample"):
        market_case.run()
    assert not list(market_case.store.iterdir())


# Scenario: market tape cannot use an unfrozen policy.
def test_frozen_policy(market_case: MarketCase) -> None:
    documents = market_case.request.model_dump(mode="json", by_alias=True)["policies"]
    documents["execution_policy"]["settings"]["participation_bps"] = "5000"
    market_case.change_request(policies=documents)
    with pytest.raises(ValueError, match="frozen FactorSpec"):
        market_case.run()
    assert not list(market_case.store.iterdir())
