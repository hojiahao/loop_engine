"""Causal selection and real administrative/worker artifact compatibility."""

import subprocess
import sys
from datetime import UTC, date, datetime, timedelta
from pathlib import Path

import numpy as np
import pytest
from hypothesis import given, settings
from hypothesis import strategies as st
from panel_helpers import FIXTURES, Case, change, make_case

from loop_research.data.fetch_cache import publish, read_cached
from loop_research.panel_builder import build_panel, validate_panel
from loop_research.panel_io import ContentRef, PanelManifest, load_panel
from loop_research.panel_models import PanelReceipt, PanelReport


@pytest.fixture
def case(tmp_path: Path) -> Case:
    return make_case(tmp_path)


# Scenario: generated panel matches worker contract.
def test_generated_panel(case: Case) -> None:
    case.request = change(
        case.request,
        fields=sorted(
            [
                "market.open",
                "market.high",
                "market.low",
                "market.close",
                "market.volume",
            ]
        ),
    )
    result = case.build()
    receipt = PanelReceipt.model_validate_json(read_cached(case.sources, result.receipt))
    view = case.sources.parent / "view"
    view.mkdir(mode=0o700)
    try:
        for reference in (receipt.panel, receipt.values):
            path = view / reference.sha256[7:]
            path.write_bytes(read_cached(case.output, reference))
            path.chmod(0o444)
        view.chmod(0o555)
        loaded = load_panel(
            view,
            ContentRef(**receipt.panel.model_dump()),
            sample_start=date(2010, 1, 2),
            sample_end=date(2010, 1, 6),
        )
        np.testing.assert_array_equal(
            loaded.panel.fields["market.close"], [[8, 8], [10, 10], [12, 12]]
        )
        np.testing.assert_array_equal(loaded.panel.fields["market.volume"], np.full((3, 2), 100))
        assert loaded.panel.eligible.all()
        loaded.check()
    finally:
        view.chmod(0o700)
    assert (result.rows, result.eligible_rows, result.observed_rows) == (6, 6, 6)
    assert not result.production_eligible
    assert not (case.output / case.request.capture.sha256[7:]).exists()


# Scenario: missing session stays in the grid.
def test_missing_session(case: Case) -> None:
    capture = case.capture()
    capture["bars"] = [bar for bar in capture["bars"] if bar["session"] != "2010-01-05"]
    case.replace_capture(capture)
    result = case.build()
    rows = case.rows(result)
    assert (result.rows, result.eligible_rows, result.observed_rows) == (6, 6, 4)
    assert [row["market.close"] for row in rows] == ["8", "8", "", "", "12", "12"]


# Scenario: warmup does not inflate evaluation coverage.
def test_warmup_inflate(case: Case) -> None:
    case.request = change(case.request, sample_start="2010-01-05")
    result = case.build()
    assert (result.eligible_rows, result.evaluation_eligible_rows) == (6, 4)
    assert (result.observed_rows, result.evaluation_observed_rows) == (6, 4)


@pytest.mark.parametrize(
    "day,opening,closing",
    [
        ("2010-03-12", "14:30", "21:00"),
        ("2010-03-15", "13:30", "20:00"),
        ("2010-11-26", "14:30", "18:00"),
    ],
)
# Scenario: decisions follow dst and early closes.
def test_decisions_dst(case: Case, day: str, opening: str, closing: str) -> None:
    capture = case.capture()
    bar = capture["bars"][0]
    close = datetime.fromisoformat(f"{day}T{closing}:00+00:00")
    bar.update(
        session=day,
        interval_start=f"{day}T{opening}:00Z",
        effective_at=close.isoformat(),
        known_at=(close + timedelta(minutes=4)).isoformat(),
    )
    capture["bars"] = [bar]
    case.replace_capture(capture)
    case.request = change(
        case.request,
        securities=["US.001"],
        warmup_start=day,
        sample_start=day,
        sample_end=day,
    )
    result = case.build()
    panel = PanelManifest.model_validate_json(read_cached(case.output, result.panel))
    assert panel.decision_times_ms == (int((close + timedelta(minutes=5)).timestamp() * 1000),)
    assert result.observed_rows == 1


# Scenario: capture must reach the exact last decision.
def test_capture_last(case: Case) -> None:
    capture = case.capture()
    cutoff = datetime(2010, 1, 6, 21, 5, tzinfo=UTC) - timedelta(microseconds=1)
    capture["captured_at"] = cutoff.isoformat()
    for record in [*capture["securities"], *capture["bars"]]:
        record["ingested_at"] = cutoff.isoformat()
    case.replace_capture(capture)
    with pytest.raises(ValueError, match="capture precedes"):
        case.build()
    assert not list(case.output.iterdir())


# Scenario: late revision cannot rewrite prior observation.
def test_late_revision(case: Case) -> None:
    before = case.build()
    capture = case.capture()
    capture["bars"].append(
        {**capture["bars"][0], "known_at": "2010-01-07T12:00:00Z", "close": "99", "high": "99"}
    )
    case.replace_capture(capture)
    after = case.build()
    assert before.panel == after.panel
    assert before.dataset != after.dataset  # Input lineage still records the later capture.


@pytest.mark.parametrize(
    "known,expected",
    [
        ("2010-01-04T21:05:00Z", "9"),
        ("2010-01-04T21:05:00.000001Z", "8"),
    ],
)
# Scenario: revision uses exact decision cutoff.
def test_revision_decision(case: Case, known: str, expected: str) -> None:
    capture = case.capture()
    capture["bars"].append({**capture["bars"][0], "known_at": known, "close": "9"})
    case.replace_capture(capture)
    assert case.rows(case.build())[0]["market.close"] == expected


# Scenario: unknown at close is missing.
def test_unknown_close(case: Case) -> None:
    capture = case.capture()
    capture["bars"][0]["known_at"] = "2010-01-04T21:05:00.000001Z"
    case.replace_capture(capture)
    result = case.build()
    assert case.rows(result)[0]["market.close"] == ""
    assert result.eligible_rows == 6 and result.observed_rows == 5


# Scenario: delisting changes eligibility without filling.
def test_delisting_eligibility(case: Case) -> None:
    capture = case.capture()
    capture["securities"].append(
        {
            **capture["securities"][0],
            "listed": False,
            "effective_at": "2010-01-05T14:30:00Z",
            "known_at": "2010-01-05T14:00:00Z",
        }
    )
    case.replace_capture(capture)
    result = case.build()
    assert [row["eligible"] for row in case.rows(result)] == ["1", "1", "0", "1", "0", "1"]
    assert result.eligible_rows == result.observed_rows == 4


# Scenario: ticker reuse keeps security axes distinct.
def test_ticker_reuse(case: Case) -> None:
    capture = case.capture()
    capture["securities"][0]["effective_until"] = "2010-01-05T14:30:00Z"
    capture["securities"][1].update(
        ticker="SYNTHA", effective_at="2010-01-05T14:30:00Z", known_at="2010-01-05T14:00:00Z"
    )
    case.replace_capture(capture)
    assert [row["eligible"] for row in case.rows(case.build())] == ["1", "0", "0", "1", "0", "1"]


@pytest.mark.parametrize("kind", ["adr", "etf", "preferred", "spac", "unknown"])
# Scenario: excluded instrument is not an observation.
def test_excluded_instrument(case: Case, kind: str) -> None:
    capture = case.capture()
    capture["securities"][0]["kind"] = kind
    case.replace_capture(capture)
    result = case.build()
    assert result.eligible_rows == result.observed_rows == 3


# Scenario: ambiguous listing fails before publication.
def test_ambiguous_listing(case: Case) -> None:
    capture = case.capture()
    capture["securities"][1].update(ticker="SYNTHA", venue="XNYS")
    case.replace_capture(capture)
    with pytest.raises(ValueError, match="ambiguous"):
        case.build()
    assert not list(case.output.iterdir())


@pytest.mark.parametrize(
    "currency,start", [("CAD", "2010-01-04T14:30:00Z"), ("USD", "2010-01-04T15:00:00Z")]
)
# Scenario: wrong currency or partial bar is rejected.
def test_wrong_currency(case: Case, currency: str, start: str) -> None:
    capture = case.capture()
    capture["bars"][0].update(currency=currency, interval_start=start)
    case.replace_capture(capture)
    with pytest.raises(ValueError, match="cover the XNYS"):
        case.build()


# Scenario: volume does not silently lose integer precision.
def test_volume_lose(case: Case) -> None:
    capture = case.capture()
    capture["bars"][0]["volume"] = 2**53 + 1
    case.replace_capture(capture)
    case.request = change(case.request, fields=["market.volume"])
    with pytest.raises(ValueError, match="integer precision"):
        case.build()


@pytest.mark.parametrize(
    "changes",
    [
        {"fields": ["market.adjusted_close"]},
        {"sample_start": "2021-01-01", "sample_end": "2021-01-04"},
        {"securities": ["US.001", "US.001"]},
        {"close_delay_ms": -1},
        {"close_delay_ms": True},
    ],
)
# Scenario: unsupported selection is rejected.
def test_unsupported_selection(case: Case, changes: dict[str, object]) -> None:
    with pytest.raises(ValueError):
        change(case.request, **changes)
    assert not list(case.output.iterdir())


# Scenario: unresolved raw reference is rejected.
def test_unresolved_raw(case: Case) -> None:
    capture = case.capture()
    capture["securities"][0]["source"]["raw_sha256"] = "sha256:" + "0" * 64
    case.replace_capture(capture)
    with pytest.raises(FileNotFoundError):
        case.build()
    assert not list(case.output.iterdir())


# Scenario: corrupt capture is rejected.
def test_corrupt_capture(case: Case) -> None:
    path = case.sources / case.request.capture.sha256[7:]
    path.write_bytes(b"!" * case.request.capture.byte_size)
    with pytest.raises(ValueError, match="digest"):
        case.build()


# Scenario: symlink input is rejected.
def test_symlink_input(case: Case) -> None:
    path = case.sources / case.request.capture.sha256[7:]
    path.unlink()
    path.symlink_to(FIXTURES / "capture.json")
    with pytest.raises(OSError):
        case.build()


# Scenario: source store cannot be worker output.
def test_source_store(case: Case) -> None:
    with pytest.raises(ValueError, match="separate"):
        build_panel(case.sources, case.sources, case.request)


# Scenario: replay is read only and deterministic.
def test_replay_deterministic(case: Case) -> None:
    first = case.build()
    before = {
        path: path.stat().st_mtime_ns
        for root in (case.sources, case.output)
        for path in root.iterdir()
    }
    assert case.build() == first
    assert validate_panel(case.sources, case.output, first.receipt.sha256) == first
    assert before == {
        path: path.stat().st_mtime_ns
        for root in (case.sources, case.output)
        for path in root.iterdir()
    }


# Scenario: modified output fails replay.
def test_modified_output(case: Case) -> None:
    first = case.build()
    receipt = PanelReceipt.model_validate_json(read_cached(case.sources, first.receipt))
    path = case.output / receipt.values.sha256[7:]
    path.write_bytes(b"!" * receipt.values.byte_size)
    with pytest.raises(ValueError, match="digest"):
        validate_panel(case.sources, case.output, first.receipt.sha256)


@pytest.mark.parametrize(
    "ticks,reason", [([10.0, 10.0, 9.0], "regression"), ([0.0, 0.0, 181.0], "deadline")]
)
# Scenario: clock failures leave no output.
def test_clock_failures(case: Case, ticks: list[float], reason: str) -> None:
    clock = iter(ticks)
    with pytest.raises(ValueError, match=reason):
        build_panel(case.sources, case.output, case.request, monotonic=lambda: next(clock))
    assert not list(case.output.iterdir())


@pytest.mark.parametrize(
    "limit,reason", [("MAX_CELLS", "cell budget"), ("MAX_WORK", "work budget")]
)
# Scenario: selection budgets block publication.
def test_selection_budgets(
    case: Case, monkeypatch: pytest.MonkeyPatch, limit: str, reason: str
) -> None:
    monkeypatch.setattr("loop_research.panel_builder." + limit, 1)
    with pytest.raises(ValueError, match=reason):
        case.build()
    assert not list(case.output.iterdir())


# Scenario: cancelled final receipt is recoverable.
def test_cancelled_final(case: Case, monkeypatch: pytest.MonkeyPatch) -> None:
    import loop_research.panel_builder as builder

    def interrupted(store: Path, content: bytes):
        if store == case.sources:
            raise KeyboardInterrupt
        return publish(store, content)

    with monkeypatch.context() as patch:
        patch.setattr(builder, "publish", interrupted)
        with pytest.raises(KeyboardInterrupt):
            case.build()
    assert all(
        b"loop.panel-build-receipt/v1" not in path.read_bytes() for path in case.sources.iterdir()
    )
    first = case.build()
    assert validate_panel(case.sources, case.output, first.receipt.sha256) == first


# Scenario: installed panel commands.
def test_installed_panel(case: Case) -> None:
    command = [sys.executable, "-I", "-m", "loop_research.cli"]
    result = subprocess.run(
        [
            *command,
            "panel-build",
            str(FIXTURES / "request.json"),
            "--sources",
            str(case.sources),
            "--store",
            str(case.output),
        ],
        capture_output=True,
        timeout=35,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    built = PanelReport.model_validate_json(result.stdout)
    checked = subprocess.run(
        [
            *command,
            "panel-validate",
            "--receipt",
            built.receipt.sha256,
            "--sources",
            str(case.sources),
            "--store",
            str(case.output),
        ],
        capture_output=True,
        timeout=35,
        check=False,
    )
    assert checked.returncode == 0, checked.stderr
    assert PanelReport.model_validate_json(checked.stdout) == built


@given(st.integers(min_value=20, max_value=10000))
@settings(max_examples=12, deadline=None)
# Scenario: future values do not change the grid.
def test_future_values(value: int) -> None:
    from tempfile import TemporaryDirectory

    with TemporaryDirectory(prefix="loop-engine-panel-property-") as directory:
        case = make_case(Path(directory))
        before = case.build()
        capture = case.capture()
        capture["bars"].append(
            {
                **capture["bars"][0],
                "known_at": "2010-01-07T00:00:00Z",
                "close": str(value),
                "high": str(value),
            }
        )
        case.replace_capture(capture)
        assert case.build().panel == before.panel
