from datetime import date, timedelta
from pathlib import Path

import pytest
from test_engine import inputs, observations

from loop_alphalens import workflow
from loop_alphalens.artifacts import Store, decode, encode
from loop_alphalens.engine import RAW_COLUMNS, columns, csv_bytes
from loop_alphalens.models import Reference


def prepare(root: Path, *, wrong: bool = False, short: bool = False) -> str:
    root.chmod(0o700)
    store = Store(root)
    spec = inputs()
    if not short:
        dates = tuple(date(2010, 1, 4) + timedelta(days=index) for index in range(24))
        dates = tuple(day for day in dates if day.weekday() < 5 and day != date(2010, 1, 18))
        spec = spec.model_copy(update={"sessions": dates})
    primary = []
    for index, day in enumerate(spec.sessions):
        if index + 1 < len(spec.sessions):
            primary.append(
                [
                    day,
                    spec.sessions[index + 1],
                    "available",
                    6,
                    6,
                    1,
                    0 if wrong else 1,
                    0.005,
                    0.025,
                    0.045,
                    0.04,
                    1,
                ]
            )
        else:
            primary.append([day, "", "no_forward_session", 6, 0, *([""] * 7)])
    cross = store.publish(csv_bytes(columns(3), primary))
    common = {
        "source_code_sha256": spec.source_code_sha256,
        "environment_sha256": spec.environment_sha256,
        "production_eligible": False,
    }
    backtest = store.publish(
        encode({"schema": "loop.portfolio-receipt/v1", "quality": "synthetic", **common})
    )
    statistics = store.publish(
        encode(
            {
                "schema": "loop.statistics-receipt/v1",
                "request": {"backtest": backtest.model_dump()},
                "cross_sections": cross.model_dump(),
                **common,
            }
        )
    )
    document = spec.model_copy(
        update={
            "observations": store.publish(csv_bytes(RAW_COLUMNS, observations(spec))),
            "primary_cross_sections": cross,
            "primary_backtest": backtest,
            "primary_statistics": statistics,
        }
    )
    return store.publish(encode(document.model_dump(mode="json", by_alias=True))).sha256


def test_workflow_replay(tmp_path: Path) -> None:
    digest = prepare(tmp_path)
    result = workflow.run(tmp_path, digest)
    assert result["artifacts"]["disposition"] == "accepted"
    before = {
        path.name: (path.stat().st_mtime_ns, path.read_bytes()) for path in tmp_path.iterdir()
    }
    assert workflow.run(tmp_path, result["receipt"]["sha256"], replay=True) == result
    assert before == {
        path.name: (path.stat().st_mtime_ns, path.read_bytes()) for path in tmp_path.iterdir()
    }


def test_rejected_evidence(tmp_path: Path) -> None:
    result = workflow.run(tmp_path, prepare(tmp_path, wrong=True))
    assert result["artifacts"]["disposition"] == "rejected"
    report = decode(Store(tmp_path).read(Reference(**result["artifacts"]["differences"])))
    assert len(report["differences"]) == 16
    assert all(row["field"] == "rank_ic" for row in report["differences"])


def test_short_unavailable(tmp_path: Path) -> None:
    result = workflow.run(tmp_path, prepare(tmp_path, short=True))
    assert result["artifacts"]["disposition"] == "unavailable"


def test_corrupt_replay(tmp_path: Path) -> None:
    result = workflow.run(tmp_path, prepare(tmp_path))
    path = tmp_path / result["artifacts"]["cross_sections"]["sha256"][7:]
    path.write_bytes(b"corrupt")
    with pytest.raises(ValueError):
        workflow.run(tmp_path, result["receipt"]["sha256"], replay=True)
    assert path.read_bytes() == b"corrupt"


def test_build_drift(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    result = workflow.run(tmp_path, prepare(tmp_path))
    monkeypatch.setattr(workflow, "describe", lambda deadline: b"changed validator")
    with pytest.raises(ValueError, match="build changed"):
        workflow.run(tmp_path, result["receipt"]["sha256"], replay=True)


def test_input_corruption(tmp_path: Path) -> None:
    digest = prepare(tmp_path)
    (tmp_path / digest[7:]).write_bytes(b"bad")
    before = set(tmp_path.iterdir())
    with pytest.raises(ValueError):
        workflow.run(tmp_path, digest)
    assert set(tmp_path.iterdir()) == before


def test_cancel_publication(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    digest = prepare(tmp_path)
    before = {path.name: path.read_bytes() for path in tmp_path.iterdir()}
    original = Store.publish
    count = 0

    def interrupted(store: Store, content: bytes) -> Reference:
        nonlocal count
        count += 1
        if count == 3:
            raise KeyboardInterrupt
        return original(store, content)

    monkeypatch.setattr(Store, "publish", interrupted)
    with pytest.raises(KeyboardInterrupt):
        workflow.run(tmp_path, digest)
    assert count == 3
    assert all((tmp_path / name).read_bytes() == content for name, content in before.items())
    assert not any(b"loop.alphalens-receipt/v1" in path.read_bytes() for path in tmp_path.iterdir())
