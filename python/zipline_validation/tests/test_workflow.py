import copy
import json
import os
from pathlib import Path

import pytest
from test_engine import inputs, replay, tape

from loop_zipline import artifacts, workflow
from loop_zipline.artifacts import Deadline, Store, encode, reference
from loop_zipline.build import read_source
from loop_zipline.models import Artifacts, Receipt
from loop_zipline.workflow import compare, lineage, run


@pytest.fixture
def stored(tmp_path):
    store_path = tmp_path / "objects"
    store_path.mkdir(mode=0o700)
    store = Store(store_path)
    recipe = inputs()
    ledgers, _ = replay(tape(), recipe)
    outputs = {name: store.publish(content) for name, content in ledgers.items()}
    roles = {
        "portfolio_policy": {
            "algorithm": "long-only-top-n.1",
            "holdings": "1",
            "initial_cash_usd": "1000",
            "lot_size": "1",
        },
        "execution_policy": {"algorithm": "next-session-open.1"},
        "cost_policy": {
            "algorithm": "commission-spread.1",
            "commission_per_share_usd": "0",
            "minimum_commission_usd": "0",
            "half_spread_bps": "0",
        },
    }
    portfolio = {
        "schema": "loop.portfolio-receipt/v1",
        "engine": recipe.engine,
        "quality": "synthetic",
        "source_code_sha256": recipe.source_code_sha256,
        "environment_sha256": recipe.environment_sha256,
        "production_eligible": False,
        "artifacts": {name: ref.model_dump() for name, ref in outputs.items()},
        "request": {"policies": {role: {"settings": settings} for role, settings in roles.items()}},
    }
    recipe = recipe.model_copy(
        update={
            "primary_backtest": store.publish(encode(portfolio)),
            "observations": store.publish(encode(tape())),
            "primary_artifacts": Artifacts(**outputs),
        }
    )
    ref = store.publish(encode(recipe.model_dump(mode="json", by_alias=True)))
    return store, ref, recipe


def test_readonly_replay(stored):
    store, inputs_ref, _ = stored
    result = run(store.root, inputs_ref.sha256)
    assert result["artifacts"]["disposition"] == "accepted"
    before = {
        path.name: (path.stat().st_mtime_ns, path.read_bytes()) for path in store.root.iterdir()
    }
    assert run(store.root, result["receipt"]["sha256"], replay=True) == result
    assert before == {
        path.name: (path.stat().st_mtime_ns, path.read_bytes()) for path in store.root.iterdir()
    }


def test_mismatch_receipt(stored):
    store, _, recipe = stored
    original = store.read(recipe.primary_artifacts.nav)
    wrong = store.publish(original.replace(b"1000", b"1001"))
    references = recipe.primary_artifacts.model_copy(update={"nav": wrong})
    portfolio = json.loads(store.read(recipe.primary_backtest))
    portfolio["artifacts"] = references.model_dump()
    changed = recipe.model_copy(
        update={
            "primary_artifacts": references,
            "primary_backtest": store.publish(encode(portfolio)),
        }
    )
    ref = store.publish(encode(changed.model_dump(mode="json", by_alias=True)))
    result = run(store.root, ref.sha256)
    assert result["artifacts"]["disposition"] == "rejected"
    receipt = Receipt.model_validate(result["artifacts"])
    report = json.loads(store.read(receipt.differences))
    assert report["total"] > 0 and report["differences"][0]["artifact"] == "nav"


def test_corrupt_output(stored):
    store, ref, _ = stored
    result = run(store.root, ref.sha256)
    receipt = Receipt.model_validate(result["artifacts"])
    path = store.root / receipt.ledgers.positions.sha256[7:]
    path.write_bytes(b"corrupt")
    with pytest.raises(ValueError, match="checksum"):
        run(store.root, result["receipt"]["sha256"], replay=True)
    assert path.read_bytes() == b"corrupt"


def test_changed_build(stored, monkeypatch):
    store, ref, _ = stored
    result = run(store.root, ref.sha256)
    monkeypatch.setattr(workflow, "describe", lambda _: b"changed build")
    with pytest.raises(ValueError, match="build changed"):
        run(store.root, result["receipt"]["sha256"], replay=True)


def test_interrupted_receipt(stored, monkeypatch):
    store, ref, _ = stored
    original = Store.publish

    def interrupted(self, content):
        if b"loop.zipline-receipt/v1" in content:
            raise KeyboardInterrupt
        return original(self, content)

    monkeypatch.setattr(Store, "publish", interrupted)
    with pytest.raises(KeyboardInterrupt):
        run(store.root, ref.sha256)
    assert not any(b"loop.zipline-receipt/v1" in path.read_bytes() for path in store.root.iterdir())


def test_input_mutation(stored, monkeypatch):
    store, ref, recipe = stored
    original = workflow.calculate

    def changing(*args):
        result = original(*args)
        (store.root / recipe.observations.sha256[7:]).write_bytes(b"modified during execution")
        return result

    monkeypatch.setattr(workflow, "calculate", changing)
    with pytest.raises(ValueError, match="checksum"):
        run(store.root, ref.sha256)
    assert not any(b"loop.zipline-receipt/v1" in path.read_bytes() for path in store.root.iterdir())


@pytest.mark.parametrize("field", ["shares", "price_usd", "simple_return", "cash_delta_usd"])
def test_detect_changes(field):
    ledgers, _ = replay(tape())
    changed = copy.copy(ledgers)
    artifact = {
        "shares": "fills",
        "price_usd": "fills",
        "simple_return": "returns",
        "cash_delta_usd": "costs",
    }[field]
    lines = ledgers[artifact].decode().splitlines()
    column = lines[0].split(",").index(field)
    row_index = 2 if field == "simple_return" else 1
    row = lines[row_index].split(",")
    row[column] = str(float(row[column]) + (1e-8 if field == "price_usd" else 0.01))
    lines[row_index] = ",".join(row)
    changed[artifact] = ("\n".join(lines) + "\n").encode()
    count, details = compare(ledgers, changed, False)
    assert count == 1 and details[0]["field"] == field


def test_policy_binding(stored):
    store, _, recipe = stored
    changed = recipe.model_copy(
        update={"policy": recipe.policy.model_copy(update={"half_spread_bps": 20})}
    )
    with pytest.raises(ValueError, match="frozen portfolio"):
        lineage(changed, store.read(recipe.primary_backtest))


@pytest.mark.parametrize("end", [-1.0, 180.0, float("nan")])
def test_clock_denial(monkeypatch, end):
    ticks = iter((0.0, end))
    monkeypatch.setattr(artifacts.time, "monotonic", lambda: next(ticks))
    budget = Deadline()
    with pytest.raises(TimeoutError):
        budget.check()


def test_symlink_denial(tmp_path):
    root = tmp_path / "objects"
    root.mkdir(mode=0o700)
    original = tmp_path / "original"
    original.write_bytes(b"evidence")
    ref = reference(b"evidence")
    (root / ref.sha256[7:]).symlink_to(original)
    with pytest.raises(OSError):
        Store(root).read(ref)


def test_duplicate_json():
    with pytest.raises(ValueError, match="duplicate"):
        artifacts.decode(b'{"key":1,"key":2}')


def test_dependency_boundary():
    import importlib.util

    assert importlib.util.find_spec("loop_research") is None
    assert not (Path(__file__).parents[1] / ".venv").exists()


def test_unavailable_receipt(stored):
    store, _, recipe = stored
    document = json.loads(store.read(recipe.primary_backtest))
    document["request"]["policies"]["portfolio_policy"]["settings"]["initial_cash_usd"] = (
        "10000000000"
    )
    changed = recipe.model_copy(
        update={
            "policy": recipe.policy.model_copy(update={"initial_cash_usd": "10000000000"}),
            "primary_backtest": store.publish(encode(document)),
        }
    )
    ref = store.publish(encode(changed.model_dump(mode="json", by_alias=True)))
    report = run(store.root, ref.sha256)
    assert report["artifacts"]["disposition"] == "unavailable"
    receipt = Receipt.model_validate(report["artifacts"])
    summary = json.loads(store.read(receipt.summary))
    assert summary["reason"] == "zipline_dollar_precision" and summary["differences"] == 0


@pytest.mark.parametrize("source", [True, False])
def test_hardlink_race(tmp_path, monkeypatch, source):
    path = tmp_path / "dependency"
    path.write_bytes(b"unchanged dependency bytes")
    original = os.fstat
    changed = False

    def link_once(descriptor):
        nonlocal changed
        value = original(descriptor)
        if not changed:
            changed = True
            (tmp_path / "uv-link").hardlink_to(path)
        return value

    monkeypatch.setattr(artifacts.os, "fstat", link_once)
    if source:
        assert read_source(path, Deadline()) == b"unchanged dependency bytes"
    else:
        with pytest.raises(ValueError, match="changed during read"):
            artifacts.read_file(path, 100)
