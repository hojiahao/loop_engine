"""Actual derived exposure artifacts, frozen policies and installed numerical execution."""

import csv
import io
import json
import subprocess
import sys
from pathlib import Path

import numpy as np
import pytest
from loop.v1.evaluation_pb2 import FactorEvaluationResult
from loop_protocol.job import factor_identity_hash
from scipy import stats
from transform_helpers import make_case, recipe, replace_recipe, replace_risk, risk_capture, work

from loop_research.build_identity import BuildIdentity, describe_build
from loop_research.data.fetch_cache import read_cached
from loop_research.factor_worker import execute
from loop_research.panel_builder import validate_panel
from loop_research.panel_io import PanelManifest


@pytest.fixture(scope="module")
def build() -> BuildIdentity:
    return describe_build(profile="evaluation")


# Scenario: installed worker neutralizes verified exposures.
def test_installed_worker(tmp_path: Path, build: BuildIdentity) -> None:
    case = make_case(tmp_path)
    report = case.build()
    assert validate_panel(case.sources, case.output, report.receipt.sha256) == report
    view, output = tmp_path / "view", tmp_path / "worker-output"
    request = work(case, report, build, view, output)
    try:
        completed = subprocess.run(
            [
                sys.executable,
                "-I",
                "-m",
                "loop_research.factor_worker",
                "--view",
                str(view),
                "--output",
                str(output),
            ],
            input=request.SerializeToString(),
            capture_output=True,
            check=True,
            timeout=60,
            env={"OPENBLAS_NUM_THREADS": "1", "OMP_NUM_THREADS": "1"},
        )
        assert not completed.stderr
        result = FactorEvaluationResult.FromString(completed.stdout)
        rows = list(
            csv.DictReader(io.StringIO((output / result.values.sha256.value.hex()).read_text()))
        )
        values = np.array([float(row["value"]) for row in rows]).reshape(3, 8)
        np.testing.assert_allclose(
            values, np.tile([1, -1, -1, 1, 1, -1, -1, 1], (3, 1)), atol=1e-12
        )
        assert result.eligible_observations == result.valid_observations == 24
        assert result.values.schema.version == result.manifest.schema.version == 2
        document = json.loads((output / result.manifest.sha256.value.hex()).read_bytes())
        assert document["schema"] == "loop.factor-evaluation-result/v2"
        assert document["transform"]["raw_valid_observations"] == 24
        assert document["transform"]["outcomes"] == ["ok"] * 3
        assert execute(request, view=view, output=output).values.sha256 == result.values.sha256
    finally:
        view.chmod(0o700)


# Scenario: late exposure revision cannot rewrite input.
def test_late_exposure(tmp_path: Path) -> None:
    case = make_case(tmp_path)
    first = case.build()
    before = PanelManifest.model_validate_json(read_cached(case.output, first.panel))
    capture = risk_capture(case)
    revision = {**capture["records"][0], "known_at": "2010-01-04T21:05:00.000001Z", "beta": "9999"}
    capture["records"].append(revision)
    replace_risk(case, capture)
    second = case.build()
    after = PanelManifest.model_validate_json(read_cached(case.output, second.panel))
    assert before.transform == after.transform
    assert first.dataset != second.dataset


# Scenario: preprocessing without exposures reaches worker.
def test_preprocessing_exposures(tmp_path: Path, build: BuildIdentity) -> None:
    case = make_case(tmp_path)
    current = recipe(case)
    current["preprocess"]["settings"]["standardize"] = "zscore"
    current["preprocess"]["settings"]["winsor_tail_bps"] = "2500"
    current["neutralization"]["settings"] = {"algorithm": "none"}
    current["exposure_capture"] = None
    replace_recipe(case, current)
    report = case.build()
    dataset = json.loads(read_cached(case.output, report.dataset))
    assert len(dataset["snapshots"][0]["artifacts"]) == 2
    view, output = tmp_path / "view", tmp_path / "worker-output"
    request = work(case, report, build, view, output)
    try:
        result = execute(request, view=view, output=output)
        rows = list(
            csv.DictReader(io.StringIO((output / result.values.sha256.value.hex()).read_text()))
        )
        values = np.array([float(row["value"]) for row in rows]).reshape(3, 8)
        # Hand-clipped at the linear 25th/75th percentiles, then independently
        # standardized. Each day's common price shift leaves the result equal.
        expected = stats.zscore([100, 100, 101, 107, 102, 104, 107.25, 107.25], ddof=1)
        np.testing.assert_allclose(values, np.tile(expected, (3, 1)), atol=1e-13)
        evidence = json.loads((output / result.manifest.sha256.value.hex()).read_bytes())
        assert evidence["transform"]["exposures_sha256"] is None
        assert evidence["transform"]["outcomes"] == ["ok"] * 3
        assert result.eligible_observations == result.valid_observations == 24
    finally:
        view.chmod(0o700)


# Scenario: missing exposure reduces only valid coverage.
def test_missing_exposure(tmp_path: Path, build: BuildIdentity) -> None:
    case = make_case(tmp_path)
    capture = risk_capture(case)
    capture["records"][0]["market_cap"] = None
    replace_risk(case, capture)
    report = case.build()
    view, output = tmp_path / "view", tmp_path / "worker-output"
    request = work(case, report, build, view, output)
    try:
        result = execute(request, view=view, output=output)
        assert result.eligible_observations == 24
        assert result.valid_observations == 23
    finally:
        view.chmod(0o700)


# Scenario: policy identity is checked before calculation.
def test_policy_identity(
    tmp_path: Path, build: BuildIdentity, monkeypatch: pytest.MonkeyPatch
) -> None:
    case = make_case(tmp_path)
    view, output = tmp_path / "view", tmp_path / "worker-output"
    request = work(case, case.build(), build, view, output)
    request.factor.frozen_policy.preprocess_policy.sha256.value = b"x" * 32
    request.factor.factor_spec_id.value = "sha256:" + factor_identity_hash(request.factor).hex()
    before = set(output.iterdir())

    def forbidden(*args: object, **kwargs: object) -> None:
        pytest.fail("evaluation ran before policy verification")

    monkeypatch.setattr("loop_research.factor_worker.evaluate", forbidden)
    try:
        with pytest.raises(ValueError, match="frozen FactorSpec"):
            execute(request, view=view, output=output)
        assert set(output.iterdir()) == before
    finally:
        view.chmod(0o700)


@pytest.mark.parametrize(
    "field,value", [("market_cap", "0"), ("currency", "CAD"), ("industry", "bad\nname")]
)
# Scenario: invalid exposure fails before publication.
def test_invalid_exposure(tmp_path: Path, field: str, value: str) -> None:
    case = make_case(tmp_path)
    capture = risk_capture(case)
    capture["records"][0][field] = value
    replace_risk(case, capture)
    with pytest.raises(ValueError):
        case.build()
    assert not list(case.output.iterdir())


# Scenario: exposure source must resolve.
def test_exposure_source(tmp_path: Path) -> None:
    case = make_case(tmp_path)
    capture = risk_capture(case)
    capture["records"][0]["source"]["raw_sha256"] = "sha256:" + "0" * 64
    replace_risk(case, capture)
    with pytest.raises(FileNotFoundError):
        case.build()
    assert not list(case.output.iterdir())


# Scenario: exposure cutoff must match prices.
def test_exposure_cutoff(tmp_path: Path) -> None:
    case = make_case(tmp_path)
    capture = risk_capture(case)
    capture["captured_at"] = "2026-09-16T00:00:00Z"
    replace_risk(case, capture)
    with pytest.raises(ValueError, match="cutoff differs"):
        case.build()


@pytest.mark.parametrize(
    "setting,value", [("algorithm", "latest"), ("standardize", "auto"), ("winsor_tail_bps", "01")]
)
# Scenario: unknown policy semantics fail closed.
def test_unknown_policy(tmp_path: Path, setting: str, value: str) -> None:
    case = make_case(tmp_path)
    current = recipe(case)
    current["preprocess"]["settings"][setting] = value
    replace_recipe(case, current)
    with pytest.raises(ValueError):
        case.build()
    assert not list(case.output.iterdir())
