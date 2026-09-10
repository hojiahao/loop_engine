import csv
import hashlib
import json
import os
import subprocess
import sys
from collections.abc import Sequence
from dataclasses import asdict
from pathlib import Path

import numpy as np
import pytest
from scipy.stats import pearsonr  # type: ignore[import-untyped]

from loop_research import nav_diagnostic
from loop_research.nav_diagnostic import correlate_nav_files

FIXTURES = Path(__file__).resolve().parents[3] / "fixtures/research/nav"
SESSIONS = [
    "2020-01-02",
    "2020-01-03",
    "2020-01-06",
    "2020-01-07",
    "2020-01-08",
    "2020-01-09",
    "2020-01-10",
    "2020-01-13",
]
LEFT = [100, 110, 121, 118, 128, 124, 139, 137]
RIGHT = [200, 195, 202, 220, 208, 230, 225, 231]


def write_nav(path: Path, sessions: Sequence[str], values: Sequence[float]) -> Path:
    with path.open("w", encoding="ascii", newline="") as destination:
        writer = csv.writer(destination)
        writer.writerow(["session", "nav"])
        writer.writerows(zip(sessions, values, strict=True))
    return path


def run_cli(*arguments: str, directory: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-m", "loop_research.cli", *arguments],
        cwd=directory,
        capture_output=True,
        text=True,
        timeout=10,
        check=False,
    )


def test_cli_matches_independent_return_correlation(tmp_path: Path) -> None:
    left, right = FIXTURES / "left.csv", FIXTURES / "right.csv"
    original = (left.read_bytes(), right.read_bytes())
    completed = run_cli(
        "nav-correlation", str(left), str(right), "--cash-flow-adjusted", directory=tmp_path
    )
    assert completed.returncode == 0, completed.stderr
    report = json.loads(completed.stdout)
    x, y = np.array(LEFT), np.array(RIGHT)
    expected = float(pearsonr(x[1:] / x[:-1] - 1, y[1:] / y[:-1] - 1).statistic)
    assert report["correlation"] == pytest.approx(expected, abs=1e-14)
    assert abs(report["correlation"] - float(pearsonr(np.diff(x), np.diff(y)).statistic)) > 1e-3
    assert report["left_sha256"] == hashlib.sha256(original[0]).hexdigest()
    assert report["right_sha256"] == hashlib.sha256(original[1]).hexdigest()
    assert report["observations"] == 8
    assert report["return_pairs"] == 7
    assert report["first_observation"] == SESSIONS[0]
    assert report["last_observation"] == SESSIONS[-1]
    assert report["status"] == "ok"
    assert report["calendar_validation"] == "not_performed"
    assert report["data_quality"] == "unverified_local_input"
    assert report["cash_flow_adjustment"] == "caller_asserted"
    assert report["return_definition"] == "nav_simple_between_matching_observation_dates"
    assert not completed.stderr
    assert (left.read_bytes(), right.read_bytes()) == original
    assert not list(tmp_path.iterdir())


def test_repeated_diagnostics_are_identical() -> None:
    left, right = FIXTURES / "left.csv", FIXTURES / "right.csv"
    first = correlate_nav_files(left, right, cash_flow_adjusted=True)
    second = correlate_nav_files(left, right, cash_flow_adjusted=True)
    assert asdict(first) == asdict(second)


@pytest.mark.parametrize(
    "sessions,values",
    [
        (SESSIONS[1:], RIGHT[1:]),
        (["2020-01-01", *SESSIONS[1:]], RIGHT),
        ([*SESSIONS[:2], "2020-01-05", *SESSIONS[3:]], RIGHT),
    ],
    ids=["unequal_lengths", "shifted_first_endpoint", "shifted_inner_endpoint"],
)
def test_alignment_is_never_inferred(
    tmp_path: Path, sessions: list[str], values: list[float]
) -> None:
    right = write_nav(tmp_path / "right.csv", sessions, values)
    with pytest.raises(ValueError, match="identical observation dates"):
        correlate_nav_files(FIXTURES / "left.csv", right, cash_flow_adjusted=True)


def test_shared_gaps_do_not_become_daily_returns(tmp_path: Path) -> None:
    dates = [SESSIONS[0], SESSIONS[3], SESSIONS[7]]
    left = write_nav(tmp_path / "left.csv", dates, [100, 110, 99])
    right = write_nav(tmp_path / "right.csv", dates, [200, 180, 189])
    report = correlate_nav_files(left, right, min_observations=2, cash_flow_adjusted=True)
    assert report.return_pairs == 2
    assert report.correlation == pytest.approx(-1.0)
    assert report.calendar_validation == "not_performed"
    assert report.return_definition == "nav_simple_between_matching_observation_dates"


@pytest.mark.parametrize("count", [1, 3, 5])
def test_insufficient_pairs_stay_null(tmp_path: Path, count: int) -> None:
    path = write_nav(tmp_path / "short.csv", SESSIONS[:count], LEFT[:count])
    report = correlate_nav_files(path, path, cash_flow_adjusted=True)
    assert report.return_pairs == count - 1
    assert report.correlation is None
    assert report.status == "insufficient_observations"
    json.dumps(asdict(report), allow_nan=False)


def test_constant_returns_stay_null(tmp_path: Path) -> None:
    left = write_nav(tmp_path / "constant.csv", SESSIONS, [100] * len(SESSIONS))
    report = correlate_nav_files(left, FIXTURES / "right.csv", cash_flow_adjusted=True)
    assert report.correlation is None
    assert report.status == "constant_returns"
    json.dumps(asdict(report), allow_nan=False)


@pytest.mark.parametrize(
    "content",
    [
        b"",
        b"session,nav\n",
        b"session,nav,nav\n2020-01-02,1,1\n",
        b"nav,session\n1,2020-01-02\n",
        b"session,nav\n2020-01-02,1,extra\n",
        b"session,nav\n2020-01-02\n",
        b"session,nav\n\n",
        b"session,nav\n20200102,1\n",
        b"session,nav\n2020-02-30,1\n",
        b"session,nav\n2020-01-02,1\n2020-01-02,2\n",
        b"session,nav\n2020-01-03,1\n2020-01-02,2\n",
        b"\xef\xbb\xbfsession,nav\n2020-01-02,1\n",
        b'session,nav\n"2020-01-02,1\n',
    ],
)
def test_invalid_csv_is_rejected(tmp_path: Path, content: bytes) -> None:
    invalid = tmp_path / "invalid.csv"
    invalid.write_bytes(content)
    with pytest.raises(ValueError):
        correlate_nav_files(invalid, invalid, cash_flow_adjusted=True)


@pytest.mark.parametrize(
    "value",
    [
        "NaN",
        "Inf",
        "-1",
        "true",
        "",
        " 1",
        "1 ",
        "1_000",
        "1e309",
        "1e-999",
        "0e-999999999999999999999999",
        "9" * 65,
    ],
)
def test_invalid_nav_is_rejected(tmp_path: Path, value: str) -> None:
    invalid = tmp_path / "invalid.csv"
    invalid.write_text(f"session,nav\n2020-01-02,{value}\n", encoding="ascii")
    with pytest.raises(ValueError):
        correlate_nav_files(invalid, invalid, cash_flow_adjusted=True)


def test_observations_after_insolvency_are_rejected(tmp_path: Path) -> None:
    invalid = write_nav(tmp_path / "invalid.csv", SESSIONS[:3], [1, 0, 1])
    with pytest.raises(ValueError, match="Prior NAV"):
        correlate_nav_files(invalid, invalid, cash_flow_adjusted=True)


def test_return_overflow_is_rejected(tmp_path: Path) -> None:
    invalid = write_nav(tmp_path / "invalid.csv", SESSIONS[:2], [1e-300, 1e300])
    with pytest.raises(ValueError, match="represented"):
        correlate_nav_files(invalid, invalid, cash_flow_adjusted=True)


def test_input_bytes_are_bounded(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(nav_diagnostic, "MAX_INPUT_BYTES", 32)
    invalid = write_nav(tmp_path / "large.csv", SESSIONS, LEFT)
    with pytest.raises(ValueError, match="MiB limit"):
        correlate_nav_files(invalid, invalid, cash_flow_adjusted=True)


def test_observation_count_is_bounded(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(nav_diagnostic, "MAX_OBSERVATIONS", 4)
    invalid = write_nav(tmp_path / "long.csv", SESSIONS, LEFT)
    with pytest.raises(ValueError, match="observation limit"):
        correlate_nav_files(invalid, invalid, min_observations=2, cash_flow_adjusted=True)


@pytest.mark.parametrize("minimum", [0, 1, True, 100_000])
def test_invalid_minimum_is_rejected(minimum: int) -> None:
    with pytest.raises(ValueError, match="Minimum return pairs"):
        correlate_nav_files(
            Path("missing"), Path("missing"), min_observations=minimum, cash_flow_adjusted=True
        )


def test_cash_flow_basis_requires_confirmation() -> None:
    with pytest.raises(ValueError, match="external cash flows"):
        correlate_nav_files(Path("missing"), Path("missing"))


def test_special_files_cannot_block_input(tmp_path: Path) -> None:
    pipe = tmp_path / "pipe"
    os.mkfifo(pipe)
    completed = run_cli(
        "nav-correlation", str(pipe), str(pipe), "--cash-flow-adjusted", directory=tmp_path
    )
    assert completed.returncode == 2
    assert "regular file" in completed.stderr
    assert not completed.stdout


def test_symlink_inputs_are_rejected(tmp_path: Path) -> None:
    link = tmp_path / "link.csv"
    link.symlink_to(FIXTURES / "left.csv")
    with pytest.raises(OSError):
        correlate_nav_files(link, link, cash_flow_adjusted=True)


def test_cli_errors_never_publish_partial_reports(tmp_path: Path) -> None:
    right = write_nav(tmp_path / "right.csv", SESSIONS[1:], RIGHT[1:])
    completed = run_cli(
        "nav-correlation",
        str(FIXTURES / "left.csv"),
        str(right),
        "--cash-flow-adjusted",
        directory=tmp_path,
    )
    assert completed.returncode == 2
    assert "identical observation dates" in completed.stderr
    assert "Traceback" not in completed.stderr
    assert not completed.stdout


def test_cli_requires_cash_flow_confirmation(tmp_path: Path) -> None:
    completed = run_cli("nav-correlation", "left", "right", directory=tmp_path)
    assert completed.returncode == 2
    assert "--cash-flow-adjusted" in completed.stderr
    assert not completed.stdout


def test_doctor_cli_is_preserved(tmp_path: Path) -> None:
    completed = run_cli("doctor", directory=tmp_path)
    assert completed.returncode == 0, completed.stderr
    assert json.loads(completed.stdout)["component"] == "researchd"
