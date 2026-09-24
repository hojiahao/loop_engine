import hashlib
import json
import os
import subprocess
import sys
from collections.abc import Iterator
from pathlib import Path

import pytest
from loop.v1.common_pb2 import CivilDate, JobId, LeaseId, Sha256Digest
from loop.v1.evaluation_pb2 import FactorEvaluationResult, FactorEvaluationWork
from loop.v1.factor_pb2 import FACTOR_DIRECTION_HIGHER_IS_BETTER, FactorSpec
from loop_protocol.job import factor_identity_hash
from loop_protocol.provenance import PROVENANCE_COMPONENTS
from test_panel_io import declaration, manifest

from loop_research.build_identity import BuildIdentity, describe_build
from loop_research.factor_worker import _artifact, execute
from loop_research.operators import operator_registry


@pytest.fixture(scope="module")
def build() -> BuildIdentity:
    return describe_build(profile="evaluation")


@pytest.fixture
def prepared(
    tmp_path: Path, build: BuildIdentity
) -> Iterator[tuple[FactorEvaluationWork, Path, Path]]:
    view = tmp_path / "view"
    view.mkdir()
    output = tmp_path / "output"
    output.mkdir(mode=0o700)
    reference = manifest(view, declaration(view))
    work = FactorEvaluationWork(
        job_id=JobId(value="job.evaluation.01"),
        lease_id=LeaseId(value="lease.evaluation.01"),
        sample_start=CivilDate(year=2010, month=1, day=2),
        sample_end=CivilDate(year=2010, month=1, day=6),
        deterministic_seed=Sha256Digest(value=b"s" * 32),
    )
    factor = FactorSpec(direction=FACTOR_DIRECTION_HIGHER_IS_BETTER)
    factor.expression.canonical_json = b'{"node":"field","field":"market.close"}'
    factor.expression.canonicalization_profile = "loop.factor-ast/v1"
    factor.expression.ast.schema_version = 1
    factor.expression.ast.root.field.field = "market.close"
    from loop_protocol.canonical import FieldNode, canonicalize_expression

    factor.expression_id.value = canonicalize_expression(
        FieldNode("market.close"), operator_registry()
    ).expression_id
    factor.expression.expression_id.CopyFrom(factor.expression_id)
    factor.operator_registry_sha256.value = bytes.fromhex(operator_registry().sha256[7:])
    for field in factor.frozen_policy.DESCRIPTOR.fields:
        policy = getattr(factor.frozen_policy, field.name)
        policy.policy_id.value = "policy." + field.name.removesuffix("_policy")
        policy.revision = "1"
        policy.sha256.value = b"p" * 32
    factor.factor_spec_id.value = "sha256:" + factor_identity_hash(factor).hex()
    work.factor.CopyFrom(factor)
    for component in PROVENANCE_COMPONENTS:
        getattr(work.provenance, component + "_sha256").value = b"c" * 32
    work.provenance.source_code_sha256.value = bytes.fromhex(build.source["sha256"][7:])
    work.provenance.environment_sha256.value = bytes.fromhex(build.environment["sha256"][7:])
    work.provenance.operator_registry_sha256.CopyFrom(factor.operator_registry_sha256)
    work.panel_manifest.CopyFrom(
        _artifact(
            output,
            (view / reference.sha256[7:]).read_bytes(),
            name="loop.factor_panel",
            media_type="application/json",
            columns=[],
            completed_ms=1_300_000_000_000,
        )
    )
    yield work, view, output
    view.chmod(0o700)


# Scenario: installed subprocess produces bound evidence.
def test_installed_subprocess(
    prepared: tuple[FactorEvaluationWork, Path, Path],
) -> None:
    work, view, output = prepared
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
        input=work.SerializeToString(),
        capture_output=True,
        timeout=60,
        check=True,
        env={"OPENBLAS_NUM_THREADS": "1", "OMP_NUM_THREADS": "1"},
    )
    assert completed.stderr == b""
    result = FactorEvaluationResult.FromString(completed.stdout)
    assert result.job_id == work.job_id
    assert result.lease_id == work.lease_id
    assert result.provenance == work.provenance
    assert result.deterministic_seed == work.deterministic_seed
    assert result.factor_spec_id == work.factor.factor_spec_id
    assert result.eligible_observations == result.valid_observations == 6
    content = (output / result.values.sha256.value.hex()).read_bytes()
    assert hashlib.sha256(content).digest() == result.values.sha256.value
    assert content == (
        b"session,security_id,eligible,value\n"
        b"2010-01-04,US.001,1,8\n2010-01-04,US.002,1,8\n"
        b"2010-01-05,US.001,1,10\n2010-01-05,US.002,1,10\n"
        b"2010-01-06,US.001,1,12\n2010-01-06,US.002,1,12\n"
    )
    document = json.loads((output / result.manifest.sha256.value.hex()).read_bytes())
    assert document["sample_start"] == "2010-01-02"
    assert document["sample_end"] == "2010-01-06"
    repeated = execute(work, view=view, output=output)
    assert repeated.values.sha256 == result.values.sha256
    assert repeated.valid_observations == result.valid_observations
    assert not list(output.glob(".loop-build-*"))


# Scenario: changed build does not publish results.
def test_changed_build(
    prepared: tuple[FactorEvaluationWork, Path, Path],
) -> None:
    work, view, output = prepared
    before = set(output.iterdir())
    work.provenance.source_code_sha256.value = b"x" * 32
    with pytest.raises(ValueError, match="frozen context"):
        execute(work, view=view, output=output)
    assert set(output.iterdir()) == before


@pytest.mark.parametrize("changed", [None, "source_code_sha256", "environment_sha256"])
# Scenario: replay verifies the build without recomputing.
def test_replay_verifies(
    prepared: tuple[FactorEvaluationWork, Path, Path], changed: str | None
) -> None:
    work, _, output = prepared
    before = set(output.iterdir())
    if changed is not None:
        getattr(work.provenance, changed).value = b"x" * 32
    completed = subprocess.run(
        [
            sys.executable,
            "-I",
            "-m",
            "loop_research.factor_worker",
            "--verify-build",
            "--view",
            str(output),
            "--output",
            str(output),
        ],
        input=work.SerializeToString(),
        capture_output=True,
        timeout=30,
        check=False,
        env={"OPENBLAS_NUM_THREADS": "1", "OMP_NUM_THREADS": "1"},
    )
    assert completed.returncode == (0 if changed is None else 2)
    assert completed.stdout == b""
    assert set(output.iterdir()) == before


@pytest.mark.parametrize(
    "missing",
    ["job_id", "lease_id", "provenance", "deterministic_seed", "factor", "panel_manifest"],
)
# Scenario: incomplete work is not execution authority.
def test_incomplete_work(
    prepared: tuple[FactorEvaluationWork, Path, Path],
    missing: str,
) -> None:
    work, view, output = prepared
    before = set(output.iterdir())
    work.ClearField(missing)
    with pytest.raises(ValueError):
        execute(work, view=view, output=output)
    assert set(output.iterdir()) == before


# Scenario: subprocess rejects malformed wire.
def test_subprocess_malformed(tmp_path: Path) -> None:
    completed = subprocess.run(
        [
            sys.executable,
            "-I",
            "-m",
            "loop_research.factor_worker",
            "--view",
            str(tmp_path),
            "--output",
            str(tmp_path),
        ],
        input=b"invalid protobuf",
        capture_output=True,
        timeout=10,
        env={"OPENBLAS_NUM_THREADS": "1"},
    )
    assert completed.returncode == 2
    assert completed.stdout == b""
    assert completed.stderr == b"factor evaluation failed; no completion authorized\n"


# Scenario: caller environment cannot replace the module.
def test_caller_environment(tmp_path: Path) -> None:
    assert os.path.isabs(sys.executable)
    (tmp_path / "loop_research.py").write_text("raise RuntimeError('injected module')\n")
    completed = subprocess.run(
        [
            sys.executable,
            "-I",
            "-m",
            "loop_research.factor_worker",
            "--view",
            str(tmp_path),
            "--output",
            str(tmp_path),
        ],
        input=b"",
        capture_output=True,
        timeout=10,
        cwd=tmp_path,
        env={"PYTHONPATH": str(tmp_path), "OPENBLAS_NUM_THREADS": "1"},
    )
    assert completed.returncode == 2
    assert b"injected module" not in completed.stderr
