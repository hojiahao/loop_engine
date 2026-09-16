import csv
import io
import json
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest
from loop.v1.evaluation_pb2 import FactorEvaluationWork
from loop_protocol.job import factor_spec_identity_sha256
from test_factor_worker import build as build
from test_factor_worker import prepared as prepared
from test_panel_io import declaration, manifest, rows
from test_portfolio import instant

from loop_research.backtest import load_request, run_backtest, validate_backtest
from loop_research.backtest_models import POLICY_ROLES, BacktestReport, BacktestRequest
from loop_research.build_identity import BuildIdentity, canonical_bytes
from loop_research.data.fetch_cache import publish, read_cached, read_receipt
from loop_research.data.fetch_records import CachedObject
from loop_research.factor_worker import _artifact, execute
from loop_research.panel_io import PanelManifest
from loop_research.transform_models import PolicyDocument


@dataclass
class Case:
    work: FactorEvaluationWork
    evidence: Path
    view: Path
    store: Path
    request: BacktestRequest
    observations: list[list[str]]

    def run(self) -> BacktestReport:
        return run_backtest(self.evidence, self.view, self.store, self.request)

    def validate(self, digest: str) -> BacktestReport:
        return validate_backtest(self.evidence, self.view, self.store, digest)

    def change_request(self, **changes: Any) -> None:
        self.request = BacktestRequest.model_validate_json(
            json.dumps(
                {
                    **self.request.model_dump(mode="json", by_alias=True),
                    **changes,
                }
            )
        )

    def tape(self, **changes: Any) -> None:
        stream = io.StringIO(newline="")
        writer = csv.writer(stream, lineterminator="\n")
        writer.writerow(
            ["session", "security_id", "open_at_ms", "open_usd", "close_known_at_ms", "close_usd"]
        )
        writer.writerows(self.observations)
        values = publish(self.evidence, stream.getvalue().encode("ascii"))
        tape = publish(
            self.evidence,
            canonical_bytes(
                {
                    "schema": "loop.execution-tape/v1",
                    "quality": "synthetic",
                    "currency": "USD",
                    "price_basis": "raw",
                    "corporate_actions": "none_in_sample_declared",
                    "observations": values.model_dump(),
                    **changes,
                }
            ),
        )
        self.change_request(execution_tape=tape.model_dump())


def policies() -> dict[str, PolicyDocument]:
    settings: dict[str, dict[str, str]] = {
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
            "half_spread_bps": "0",
            "minimum_commission_usd": "0",
        },
        "evaluation_policy": {"minimum_coverage_bps": "9000"},
    }
    return {
        role: PolicyDocument.model_validate_json(
            json.dumps(
                {
                    "schema": "loop.research-policy/v1",
                    "policy_id": "policy." + role.removesuffix("_policy"),
                    "revision": "1",
                    "settings": settings.get(role, {}),
                }
            )
        )
        for role in POLICY_ROLES
    }


@pytest.fixture
def case(prepared: tuple[FactorEvaluationWork, Path, Path], tmp_path: Path) -> Case:
    work, view, evidence = prepared
    documents = policies()
    for role, document in documents.items():
        getattr(work.factor.frozen_policy, role).sha256.value = bytes.fromhex(document.digest()[7:])
    work.factor.factor_spec_id.value = "sha256:" + factor_spec_identity_sha256(work.factor).hex()
    work_ref = publish(evidence, work.SerializeToString())
    result = execute(work, view=view, output=evidence)
    placeholder = CachedObject(sha256="sha256:" + "0" * 64, byte_size=1)
    request = BacktestRequest(
        evaluation_work=work_ref,
        evaluation_result=CachedObject(
            sha256=result.manifest.artifact_id.value, byte_size=result.manifest.byte_size
        ),
        factor_values=CachedObject(
            sha256=result.values.artifact_id.value, byte_size=result.values.byte_size
        ),
        execution_tape=placeholder,
        policies=documents,
    )
    store = tmp_path / "ledger"
    store.mkdir(mode=0o700)
    result_case = Case(
        work,
        evidence,
        view,
        store,
        request,
        [
            [
                f"2010-01-0{day}",
                security,
                str(instant(day, 14, 30)),
                str(day * 2),
                str(instant(day, 21)),
                str(day * 2),
            ]
            for day in (4, 5, 6)
            for security in ("US.001", "US.002")
        ],
    )
    result_case.tape()
    return result_case


def test_real_evaluation_to_ledger_and_read_only_replay(case: Case) -> None:
    result = case.run()
    assert result.quality == "synthetic" and not result.production_eligible
    assert result.ending_nav_usd == "1200"
    assert result.orders == 1 and result.fills == 1
    assert read_cached(case.store, result.artifacts.nav) == (
        b"session,cash_usd,market_value_usd,nav_usd\n"
        b"2010-01-04,1000,0,1000\n2010-01-05,0,1000,1000\n2010-01-06,0,1200,1200\n"
    )
    before = {
        path.name: (path.stat().st_mtime_ns, path.read_bytes()) for path in case.store.iterdir()
    }
    assert case.validate(result.receipt.sha256) == result
    assert before == {
        path.name: (path.stat().st_mtime_ns, path.read_bytes()) for path in case.store.iterdir()
    }
    assert case.run() == result


def test_installed_cli_runs_and_replays(case: Case, tmp_path: Path) -> None:
    request = tmp_path / "request.json"
    request.write_text(case.request.model_dump_json(by_alias=True), encoding="ascii")
    arguments = [
        "--evidence",
        str(case.evidence),
        "--view",
        str(case.view),
        "--store",
        str(case.store),
    ]
    environment = {**os.environ, "OPENBLAS_NUM_THREADS": "1", "OMP_NUM_THREADS": "1"}
    completed = subprocess.run(
        [sys.executable, "-I", "-m", "loop_research.cli", "backtest-run", str(request), *arguments],
        capture_output=True,
        timeout=180,
        check=True,
        env=environment,
    )
    report = BacktestReport.model_validate_json(completed.stdout)
    assert completed.stderr == b""
    replayed = subprocess.run(
        [
            sys.executable,
            "-I",
            "-m",
            "loop_research.cli",
            "backtest-validate",
            "--receipt",
            report.receipt.sha256,
            *arguments,
        ],
        capture_output=True,
        timeout=180,
        check=True,
        env=environment,
    )
    assert replayed.stdout == completed.stdout
    assert replayed.stderr == b""


@pytest.mark.parametrize("field", ["values", "coverage", "seed", "factor", "sample", "unknown"])
def test_forged_factor_evidence_cannot_enter_the_ledger(case: Case, field: str) -> None:
    document = json.loads(read_cached(case.evidence, case.request.evaluation_result))
    if field == "values":
        raw = read_cached(case.evidence, case.request.factor_values).replace(b",1,8\n", b",1,9\n")
        value_ref = publish(case.evidence, raw)
        case.change_request(factor_values=value_ref.model_dump())
        document["values_sha256"] = value_ref.sha256
    elif field == "coverage":
        document["valid_observations"] -= 1
    elif field == "seed":
        document["deterministic_seed"] = "sha256:" + "1" * 64
    elif field == "factor":
        document["factor_spec_id"] = "sha256:" + "1" * 64
    elif field == "sample":
        document["sample_start"] = "2010-01-04"
    else:
        document["ignored"] = "untrusted"
    reference = publish(case.evidence, canonical_bytes(document))
    case.change_request(evaluation_result=reference.model_dump())
    with pytest.raises(ValueError, match="numerical replay"):
        case.run()
    assert not list(case.store.iterdir())


def test_changed_frozen_policy_fails(case: Case) -> None:
    documents = case.request.model_dump(mode="json")["policies"]
    documents["portfolio_policy"]["settings"]["holdings"] = "2"
    case.change_request(policies=documents)
    with pytest.raises(ValueError, match="frozen FactorSpec"):
        case.run()
    assert not list(case.store.iterdir())


def reevaluate(case: Case) -> None:
    result = execute(case.work, view=case.view, output=case.evidence)
    case.change_request(
        evaluation_work=publish(case.evidence, case.work.SerializeToString()).model_dump(),
        evaluation_result={
            "sha256": result.manifest.artifact_id.value,
            "byte_size": result.manifest.byte_size,
        },
        factor_values={
            "sha256": result.values.artifact_id.value,
            "byte_size": result.values.byte_size,
        },
    )


def test_genuine_low_coverage_is_not_backtested(case: Case) -> None:
    grid = rows()
    grid[0][-1] = ""
    reference = manifest(case.view, declaration(case.view, grid))
    case.work.panel_manifest.CopyFrom(
        _artifact(
            case.evidence,
            (case.view / reference.sha256[7:]).read_bytes(),
            name="loop.factor_panel",
            media_type="application/json",
            columns=[],
            completed_ms=1_300_000_000_000,
        )
    )
    reevaluate(case)
    with pytest.raises(ValueError, match="coverage"):
        case.run()
    assert not list(case.store.iterdir())


def test_frozen_but_unsupported_algorithm_fails(case: Case) -> None:
    documents = case.request.model_dump(mode="json")["policies"]
    documents["portfolio_policy"]["settings"]["algorithm"] = "long-short.1"
    case.change_request(policies=documents)
    case.work.factor.frozen_policy.portfolio_policy.sha256.value = bytes.fromhex(
        case.request.policies["portfolio_policy"].digest()[7:]
    )
    case.work.factor.factor_spec_id.value = (
        "sha256:" + factor_spec_identity_sha256(case.work.factor).hex()
    )
    reevaluate(case)
    with pytest.raises(ValueError, match="unsupported portfolio"):
        case.run()
    assert not list(case.store.iterdir())


@pytest.mark.parametrize(
    ("name", "value"),
    [
        ("corporate_actions", "split"),
        ("currency", "EUR"),
        ("price_basis", "adjusted"),
        ("quality", "production"),
    ],
)
def test_unsupported_execution_assumptions_fail(case: Case, name: str, value: str) -> None:
    case.tape(**{name: value})
    with pytest.raises(ValueError):
        case.run()
    assert not list(case.store.iterdir())


@pytest.mark.parametrize("change", ["missing", "duplicate", "reordered", "extra", "late", "early"])
def test_bad_execution_grid_and_clocks_fail(case: Case, change: str) -> None:
    if change == "missing":
        case.observations.pop()
    elif change == "duplicate":
        case.observations[1] = list(case.observations[0])
    elif change == "reordered":
        case.observations.reverse()
    elif change == "extra":
        case.observations.append(list(case.observations[-1]))
    elif change == "late":
        case.observations[0][4] = str(instant(4, 21, 6))
    else:
        case.observations[0][2] = str(instant(4, 14, 29))
    case.tape()
    with pytest.raises(ValueError):
        case.run()
    assert not list(case.store.iterdir())


def test_protected_window_fails_without_publication(case: Case) -> None:
    case.work.sample_start.year = case.work.sample_end.year = 2025
    reference = publish(case.evidence, case.work.SerializeToString())
    case.change_request(evaluation_work=reference.model_dump())
    with pytest.raises(ValueError, match="development sample"):
        case.run()
    assert not list(case.store.iterdir())


def test_source_drift_cannot_relabel_old_results(case: Case) -> None:
    case.work.provenance.source_code_sha256.value = b"x" * 32
    reference = publish(case.evidence, case.work.SerializeToString())
    case.change_request(evaluation_work=reference.model_dump())
    with pytest.raises(ValueError, match="frozen context"):
        case.run()
    assert not list(case.store.iterdir())


def test_corrupt_output_fails_replay_and_is_not_repaired(case: Case) -> None:
    result = case.run()
    path = case.store / result.artifacts.nav.sha256[7:]
    path.write_bytes(b"corrupt")
    with pytest.raises(ValueError):
        case.validate(result.receipt.sha256)
    with pytest.raises(ValueError):
        case.run()
    assert path.read_bytes() == b"corrupt"


def test_missing_input_invalidates_replay(case: Case) -> None:
    result = case.run()
    (case.evidence / case.request.factor_values.sha256[7:]).unlink()
    with pytest.raises(FileNotFoundError):
        case.validate(result.receipt.sha256)


def test_interrupted_publication_has_no_receipt(
    case: Case, monkeypatch: pytest.MonkeyPatch
) -> None:
    import loop_research.backtest as backtest

    original = backtest.publish
    calls = 0

    def interrupted(store: Path, content: bytes) -> CachedObject:
        nonlocal calls
        calls += 1
        if calls == 4:
            raise KeyboardInterrupt
        return original(store, content)

    with monkeypatch.context() as patch:
        patch.setattr(backtest, "publish", interrupted)
        with pytest.raises(KeyboardInterrupt):
            case.run()
    assert len(list(case.store.iterdir())) == 3
    assert all(b"loop.portfolio-receipt" not in path.read_bytes() for path in case.store.iterdir())
    completed = case.run()
    assert case.validate(completed.receipt.sha256) == completed


def test_malformed_csv_fails_without_publication(case: Case) -> None:
    raw = publish(case.evidence, b'"unterminated')
    case.tape(observations=raw.model_dump())
    with pytest.raises(ValueError, match="CSV encoding"):
        case.run()
    assert not list(case.store.iterdir())


def test_receipt_corruption_is_not_a_valid_summary(case: Case) -> None:
    result = case.run()
    _, content = read_receipt(case.store, result.receipt.sha256)
    document = json.loads(content)
    document["ending_nav_usd"] = "999999"
    fake = publish(case.store, canonical_bytes(document))
    with pytest.raises(ValueError, match="receipt differs"):
        case.validate(fake.sha256)


@pytest.mark.parametrize("clock_values", [(0.0, 1.0), (1.0, 0.0), (0.0, float("nan"))])
def test_budget_and_clock_regression_publish_nothing(
    case: Case, clock_values: tuple[float, ...]
) -> None:
    clock = iter(clock_values)
    with pytest.raises((ValueError, TimeoutError)):
        run_backtest(
            case.evidence,
            case.view,
            case.store,
            case.request,
            timeout_seconds=0.5,
            clock=lambda: next(clock),
        )
    assert not list(case.store.iterdir())


def test_duplicate_request_keys_fail_before_execution(case: Case, tmp_path: Path) -> None:
    path = tmp_path / "request.json"
    content = case.request.model_dump_json(by_alias=True)
    path.write_text('{"schema":"loop.portfolio-request/v1",' + content[1:], encoding="ascii")
    with pytest.raises(ValueError, match="duplicate"):
        load_request(path)


def test_transformed_factor_reaches_the_portfolio(tmp_path: Path, build: BuildIdentity) -> None:
    from transform_helpers import make_case, work

    source_case = make_case(tmp_path)
    panel_report = source_case.build()
    view, evidence = tmp_path / "transformed-view", tmp_path / "transformed-evidence"
    evaluation = work(source_case, panel_report, build, view, evidence)
    try:
        panel = PanelManifest.model_validate_json(
            read_cached(source_case.output, panel_report.panel)
        )
        assert panel.transform is not None
        documents = policies()
        documents["preprocess_policy"] = panel.transform.preprocess
        documents["neutralization_policy"] = panel.transform.neutralization
        for role, document in documents.items():
            getattr(evaluation.factor.frozen_policy, role).sha256.value = bytes.fromhex(
                document.digest()[7:]
            )
        evaluation.factor.factor_spec_id.value = (
            "sha256:" + factor_spec_identity_sha256(evaluation.factor).hex()
        )
        result = execute(evaluation, view=view, output=evidence)
        store = tmp_path / "transformed-ledger"
        store.mkdir(mode=0o700)
        request = BacktestRequest(
            evaluation_work=publish(evidence, evaluation.SerializeToString()),
            evaluation_result=CachedObject(
                sha256=result.manifest.artifact_id.value, byte_size=result.manifest.byte_size
            ),
            factor_values=CachedObject(
                sha256=result.values.artifact_id.value, byte_size=result.values.byte_size
            ),
            execution_tape=CachedObject(sha256="sha256:" + "0" * 64, byte_size=1),
            policies=documents,
        )
        case = Case(
            evaluation,
            evidence,
            view,
            store,
            request,
            [
                [
                    f"2010-01-0{day}",
                    security,
                    str(instant(day, 14, 30)),
                    "10",
                    str(instant(day, 21)),
                    "10",
                ]
                for day in (4, 5, 6)
                for security in panel.securities
            ],
        )
        case.tape()
        report = case.run()
        assert report.ending_nav_usd == "1000" and report.fills > 0
        assert case.validate(report.receipt.sha256) == report
    finally:
        view.chmod(0o700)
