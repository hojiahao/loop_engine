"""Administrative statistics publication, full-family checking and read-only replay."""

import hashlib
import time
from collections.abc import Callable
from pathlib import Path

import numpy as np
from loop.v1.evaluation_pb2 import FactorEvaluationWork

from loop_research.backtest import PortfolioReplay, _Deadline, _paths, _reference, reconstruct
from loop_research.backtest_models import BacktestRequest
from loop_research.build_identity import canonical_bytes
from loop_research.data.fetch_cache import publish, read_cached, read_config_bytes, read_receipt
from loop_research.data.fetch_json import decode_object
from loop_research.data.fetch_records import CachedObject
from loop_research.portfolio_statistics import PortfolioStatistics, summarize
from loop_research.statistics_kernels import cscv, deflated_sharpe, fdr_by
from loop_research.statistics_models import (
    ExperimentEvidence,
    ExperimentPlan,
    StatisticsPolicy,
    StatisticsReceipt,
    StatisticsReport,
    StatisticsRequest,
    TrialFailure,
    resolve_statistics,
    unavailable,
)


def trial_binding(work: FactorEvaluationWork, tape: CachedObject) -> str:
    """Commit numerical trial context before the circular evaluation-policy hash.

    Job/lease IDs and artifact publication clocks are not research parameters.
    Configuration fingerprint is excluded because it includes the eventual
    evaluation policy. Eight actual frozen policy references remain committed.
    Reconstruction validates all excluded provenance under the final policy.
    """
    copied = FactorEvaluationWork()
    copied.CopyFrom(work)
    for field in ("job_id", "lease_id"):
        copied.ClearField(field)
    copied.factor.ClearField("factor_spec_id")
    copied.factor.frozen_policy.ClearField("evaluation_policy")
    copied.provenance.ClearField("configuration_sha256")
    copied.panel_manifest.ClearField("created_at")
    content = copied.SerializeToString(deterministic=True)
    return (
        "sha256:"
        + hashlib.sha256(
            b"loop.statistics-trial/v1\x00" + content + b"\x00" + canonical_bytes(tape.model_dump())
        ).hexdigest()
    )


def bind_request(evidence: Path, request: BacktestRequest) -> str:
    """Prepare a plan identity; this hash alone verifies neither data nor authority."""
    work = FactorEvaluationWork.FromString(read_cached(evidence, request.evaluation_work))
    return trial_binding(work, request.execution_tape)


def load_statistics(path: Path) -> StatisticsRequest:
    """Read a bounded recipe with duplicate-field rejection and no path expansion."""
    content = read_config_bytes(path)
    decode_object(content)
    return StatisticsRequest.model_validate_json(content)


def _policy(replay: PortfolioReplay) -> StatisticsPolicy:
    result = resolve_statistics(replay.receipt.request.policies["evaluation_policy"])
    if result is None:
        raise ValueError("statistics require an opt-in frozen evaluation policy")
    return result


def _context(replay: PortfolioReplay) -> tuple[object, ...]:
    provenance = replay.work.provenance
    return (
        replay.receipt.quality,
        replay.receipt.calendar_version,
        replay.receipt.request.execution_tape,
        replay.work.panel_manifest.sha256.value,
        provenance.source_code_sha256.value,
        provenance.environment_sha256.value,
        provenance.data_manifest_sha256.value,
        provenance.trading_calendar_sha256.value,
        tuple(session.day for session in replay.sessions),
    )


def _family(
    request: StatisticsRequest,
    primary: PortfolioReplay,
    statistics: PortfolioStatistics,
    policy: StatisticsPolicy,
    evidence: Path,
    view: Path,
    store: Path,
    deadline: _Deadline,
    read: Callable[[CachedObject], bytes],
) -> tuple[bytes, list[Callable[[], None]]]:
    if request.experiment is None:
        missing_family = unavailable("missing_experiment_evidence").model_dump()
        return canonical_bytes(
            {
                "schema": "loop.multiple-testing/v1",
                "scope": "no-family-evidence",
                "fdr_by": missing_family,
                "dsr": missing_family,
                "pbo": missing_family,
                "production_eligible": False,
            }
        ), []
    if policy.experiment_plan is None:
        raise ValueError("experiment was not frozen in the evaluation policy")
    family = ExperimentEvidence.model_validate_json(read(request.experiment))
    if family.plan.sha256 != policy.experiment_plan:
        raise ValueError("experiment plan differs from the frozen evaluation policy")
    plan = ExperimentPlan.model_validate_json(read(family.plan))
    if tuple(item.trial_id for item in plan.trials) != tuple(
        item.trial_id for item in family.outcomes
    ):
        raise ValueError("experiment outcomes must cover the exact ordered plan")
    if not any(
        item.trial_id == policy.trial_id and item.backtest == request.backtest
        for item in family.outcomes
    ):
        raise ValueError("requested portfolio is not its planned trial outcome")
    guards = []
    reports: list[dict[str, object]] = []
    p_values: list[float] = []
    returns = []
    cells = 0
    has_failures = False
    for planned, outcome in zip(plan.trials, family.outcomes, strict=True):
        deadline.check()
        if outcome.failure is not None:
            failure = TrialFailure.model_validate_json(read(outcome.failure))
            if (failure.trial_id, failure.binding_sha256) != (
                planned.trial_id,
                planned.binding_sha256,
            ):
                raise ValueError("failure does not identify the planned trial")
            p_values.append(1.0)
            reports.append(
                {
                    "trial_id": planned.trial_id,
                    "status": failure.kind,
                    "reason": failure.reason,
                    "p_value": None,
                    "fdr_input": 1.0,
                }
            )
            has_failures = True
            continue
        if outcome.backtest is None:
            raise ValueError("trial outcome is unresolved")
        current = (
            primary
            if outcome.backtest == request.backtest
            else reconstruct(evidence, view, store, outcome.backtest, deadline)
        )
        current_policy = _policy(current)
        current_document = current.receipt.request.policies["evaluation_policy"]
        primary_document = primary.receipt.request.policies["evaluation_policy"]
        if (
            current_policy.trial_id != planned.trial_id
            or current_policy.model_copy(update={"trial_id": policy.trial_id}) != policy
            or current_document.policy_id != primary_document.policy_id
            or current_document.revision != primary_document.revision
            or {key: value for key, value in current_document.settings.items() if key != "trial_id"}
            != {key: value for key, value in primary_document.settings.items() if key != "trial_id"}
        ):
            raise ValueError("family statistical policies differ")
        if (
            trial_binding(current.work, current.receipt.request.execution_tape)
            != planned.binding_sha256
        ):
            raise ValueError("trial differs from its predeclared binding")
        if _context(current) != _context(primary):
            raise ValueError("family data, sample or build context differs")
        cells += len(current.sessions) * len(current.sessions[0].observations)
        if cells > 100_000:
            raise ValueError("experiment aggregate replay cell budget")
        result = (
            statistics if current is primary else summarize(current, current_policy, deadline.check)
        )
        if result.dates != statistics.dates:
            raise ValueError("trial return dates are not exactly synchronous")
        if current is not primary:
            guards.append(current.check)
        p_values.append(result.mean_p if result.mean_p is not None else 1.0)
        returns.append(result.returns)
        reports.append(
            {
                "trial_id": planned.trial_id,
                "status": "completed",
                "p_value": result.mean_p,
                "fdr_input": p_values[-1],
            }
        )
    corrected = fdr_by(p_values)
    for report, value in zip(reports, corrected, strict=True):
        report.update(
            fdr_adjusted=value, fdr_rejected=value <= 0.05 and report["p_value"] is not None
        )
    split_rows: list[dict[str, object]] = []
    if has_failures:
        missing = unavailable("incomplete_trial_returns", len(statistics.returns))
        dsr = [missing] * len(plan.trials)
        threshold = pbo = missing
    else:
        matrix = np.column_stack(returns)
        dsr, threshold = deflated_sharpe(matrix, policy.minimum_sessions)
        pbo, split_rows = cscv(
            matrix, blocks=policy.pbo_blocks, minimum=policy.minimum_sessions, check=deadline.check
        )
    for report, metric in zip(reports, dsr, strict=True):
        report["dsr"] = metric.model_dump()
    return canonical_bytes(
        {
            "schema": "loop.multiple-testing/v1",
            "scope": "complete-declared-family-not-global-search-history",
            "family_id": plan.family_id,
            "plan": family.plan.model_dump(),
            "planned_trials": len(plan.trials),
            "completed_trials": len(returns),
            "failed_trials": len(plan.trials) - len(returns),
            "fdr_method": "benjamini-yekutieli-two-sided-hac-mean",
            "fdr_alpha": 0.05,
            "dsr_count_assumption": (
                "all-declared-trials-independent; serial-independence-not-verified"
            ),
            "dsr_benchmark": threshold.model_dump(),
            "trials": reports,
            "pbo": pbo.model_dump(),
            "pbo_scope": "internal-CSCV-selection-diagnostic-no-retraining-no-holdouts",
            "splits": split_rows,
            "production_eligible": False,
        }
    ), guards


def _materialize(
    evidence: Path, view: Path, store: Path, request: StatisticsRequest, deadline: _Deadline
) -> tuple[StatisticsReceipt, dict[str, bytes], PortfolioReplay, Callable[[], None]]:
    request = StatisticsRequest.model_validate_json(request.model_dump_json(by_alias=True))
    inputs: list[tuple[CachedObject, bytes]] = []

    def read(reference: CachedObject) -> bytes:
        deadline.check()
        if reference.byte_size > 128 * 1024:
            raise ValueError("statistical metadata byte budget")
        content = read_cached(evidence, reference)
        decode_object(content)
        inputs.append((reference, content))
        return content

    primary = reconstruct(evidence, view, store, request.backtest, deadline)
    policy = _policy(primary)
    result = summarize(primary, policy, deadline.check)
    multiple, guards = _family(
        request, primary, result, policy, evidence, view, store, deadline, read
    )
    artifacts = {**result.artifacts, "multiple_testing": multiple}
    if sum(map(len, artifacts.values())) > 64 * 1024 * 1024:
        raise ValueError("statistics output byte budget")

    def check() -> None:
        primary.check()
        for guard in guards:
            guard()
        for reference, content in inputs:
            deadline.check()
            if read_cached(evidence, reference) != content:
                raise ValueError("statistical input changed during execution")
        deadline.check()

    receipt = StatisticsReceipt(
        request=request,
        source_code_sha256=primary.receipt.source_code_sha256,
        environment_sha256=primary.receipt.environment_sha256,
        summary=_reference(artifacts["summary"]),
        cross_sections=_reference(artifacts["cross_sections"]),
        portfolio=_reference(artifacts["portfolio"]),
        exposures=_reference(artifacts["exposures"]),
        multiple_testing=_reference(artifacts["multiple_testing"]),
    )
    return receipt, artifacts, primary, check


def run_statistics(
    evidence: Path,
    view: Path,
    store: Path,
    request: StatisticsRequest,
    *,
    timeout_seconds: float = 180,
    clock: Callable[[], float] = time.monotonic,
) -> StatisticsReport:
    """Publish actual statistics, then a receipt; interruption preserves CAS history.

    Administrative paths are not data capabilities. No holdout, database write,
    research admission or network request is authorized by this operation.
    """
    deadline = _Deadline(timeout_seconds, clock)
    report, _, _ = prepare_statistics(evidence, view, store, request, deadline)
    return report


def prepare_statistics(
    evidence: Path, view: Path, store: Path, request: StatisticsRequest, deadline: _Deadline
) -> tuple[StatisticsReport, PortfolioReplay, Callable[[], None]]:
    """Publish statistics and retain this operation's verified primary and guards.

    Exporters can share this live reconstruction. It is never serialized as
    authority and its guards must still pass before the final export receipt.
    """
    _paths(evidence, view, store)
    receipt, artifacts, primary, check = _materialize(evidence, view, store, request, deadline)
    for content in artifacts.values():
        deadline.check()
        publish(store, content)
    check()
    reference = publish(store, canonical_bytes(receipt.model_dump(mode="json", by_alias=True)))
    return StatisticsReport(receipt=reference, artifacts=receipt), primary, check


def validate_statistics(
    evidence: Path,
    view: Path,
    store: Path,
    digest: str,
    *,
    timeout_seconds: float = 180,
    clock: Callable[[], float] = time.monotonic,
) -> StatisticsReport:
    """Reconstruct all numerical/family evidence without creating or repairing files."""
    deadline = _Deadline(timeout_seconds, clock)
    _paths(evidence, view, store)
    report, _, check = reconstruct_statistics(evidence, view, store, digest, deadline)
    check()
    return report


def reconstruct_statistics(
    evidence: Path, view: Path, store: Path, digest: str, deadline: _Deadline
) -> tuple[StatisticsReport, PortfolioReplay, Callable[[], None]]:
    """Verify a complete receipt and return original frozen inputs for validators.

    This read grants no authority, publishes nothing and retains input/output
    guards for callers that subsequently export independent verification inputs.
    """
    reference, content = read_receipt(store, digest)
    decode_object(content)
    original = StatisticsReceipt.model_validate_json(content)
    receipt, artifacts, primary, check = _materialize(
        evidence, view, store, original.request, deadline
    )
    if canonical_bytes(receipt.model_dump(mode="json", by_alias=True)) != content:
        raise ValueError("statistics receipt differs from actual replay")
    for name, expected in artifacts.items():
        if read_cached(store, getattr(receipt, name)) != expected:
            raise ValueError("statistics output differs from actual replay")

    def guard() -> None:
        check()
        if read_cached(store, reference) != content:
            raise ValueError("statistics receipt changed during replay")
        for name, expected in artifacts.items():
            deadline.check()
            if read_cached(store, getattr(receipt, name)) != expected:
                raise ValueError("statistics output changed during replay")

    guard()
    return StatisticsReport(receipt=reference, artifacts=receipt), primary, guard
