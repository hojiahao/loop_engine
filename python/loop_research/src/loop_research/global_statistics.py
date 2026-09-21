"""Complete-registry statistics over registered, synchronous strategy returns."""

import csv
import hashlib
import io
import math
from collections.abc import Callable
from dataclasses import dataclass
from datetime import date

import numpy as np

from loop_research.build_identity import canonical_bytes
from loop_research.global_models import GlobalPolicy, GlobalPortfolio, GlobalWork
from loop_research.statistics_kernels import cscv, deflated_sharpe, mean_test
from loop_research.statistics_models import unavailable


@dataclass(frozen=True)
class StrategySeries:
    """Numerically reconstructed strategy; never read directly from an RPC."""

    source: GlobalPortfolio
    binding: str
    context: str
    dates: tuple[str, ...]
    returns: tuple[float, ...]


def snapshot_digest(work: GlobalWork) -> str:
    """Bind exact membership, attempts, revisions and outcomes for this report."""
    return (
        "sha256:"
        + hashlib.sha256(
            canonical_bytes(work.snapshot.model_dump(mode="json", by_alias=True))
        ).hexdigest()
    )


def _issues(work: GlobalWork) -> list[dict[str, str]]:
    continued = {item.evaluation_job_id for item in work.portfolios}
    issues = []
    for state in work.snapshot.states:
        if state.state != 4:
            issues.append({"job_id": state.job_id, "reason": "unfinished_or_unsuccessful_trial"})
        if state.attempt != 1:
            issues.append(
                {"job_id": state.job_id, "reason": "complete_attempt_returns_unavailable"}
            )
        if state.kind == "factor_evaluation" and state.job_id not in continued:
            issues.append({"job_id": state.job_id, "reason": "missing_portfolio_continuation"})
    return issues


def matrix_report(
    work: GlobalWork,
    policy: GlobalPolicy,
    series: tuple[StrategySeries, ...],
    check: Callable[[], None],
) -> tuple[bytes, bytes]:
    """Account for every trial; no sample intersection or successful-subset analysis.

    Identical frozen strategies must have identical reconstructed returns before
    their operational duplicates can share a column. Counts remain uncollapsed.
    """
    issues = _issues(work)
    expected = () if issues else work.portfolios
    if tuple(item.source for item in series) != expected:
        raise ValueError("global reconstructed source set differs")
    total = sum(entry.attempts for entry in work.snapshot.ledger.entries)
    missing = unavailable("incomplete_global_returns").model_dump()
    summary: dict[str, object] = {
        "schema": "loop.global-statistics/v1",
        "scope": policy.scope,
        "snapshot_sha256": snapshot_digest(work),
        "registered_jobs": len(work.snapshot.states),
        "counted_attempts": total,
        "portfolio_jobs": len(work.portfolios),
        "distinct_strategies": 0,
        "complete_matrix": False,
        "issues": issues,
        "strategies": [],
        "dsr_count_assumption": (
            "all-distinct-frozen-strategies-independent; serial-independence-not-verified"
        ),
        "dsr_benchmark": missing,
        "pbo": missing,
        "pbo_scope": "internal-CSCV-selection-diagnostic-no-retraining-no-holdouts",
        "splits": [],
        "production_eligible": False,
    }
    output = io.StringIO(newline="")
    writer = csv.writer(output, lineterminator="\n")
    writer.writerow(("session_date", "strategy_binding", "simple_return"))
    if issues:
        return canonical_bytes(summary), output.getvalue().encode("ascii")
    groups: dict[str, list[StrategySeries]] = {}
    reference = series[0] if series else None
    cells = 0
    for item in series:
        check()
        if len(item.dates) != len(item.returns) or len(item.dates) > 8192:
            raise ValueError("global return axis bounds")
        if tuple(sorted(set(item.dates))) != item.dates or any(
            date.fromisoformat(day).isoformat() != day for day in item.dates
        ):
            raise ValueError("global dates must be canonical and strictly increasing")
        if any(not math.isfinite(value) or abs(value) > 1e12 for value in item.returns):
            raise ValueError("global returns must be finite and bounded")
        cells += len(item.returns)
        if cells > 100_000:
            raise ValueError("global matrix cell budget")
        if reference is not None and (
            item.context != reference.context or item.dates != reference.dates
        ):
            issues.append({"job_id": item.source.job_id, "reason": "incompatible_return_context"})
        peers = groups.setdefault(item.binding, [])
        if peers and (item.context, item.dates, item.returns) != (
            peers[0].context,
            peers[0].dates,
            peers[0].returns,
        ):
            raise ValueError("deterministic duplicate strategies have different returns")
        peers.append(item)
    summary["distinct_strategies"] = len(groups)
    if issues or len(groups) < 2:
        if not issues:
            issues.append({"job_id": work.job_id, "reason": "insufficient_distinct_strategies"})
        return canonical_bytes(summary), output.getvalue().encode("ascii")
    # Registry order is stable; deduplication preserves first occurrence and
    # therefore the pre-existing CSCV tie-breaking convention.
    columns = [items[0] for items in groups.values()]
    matrix = np.column_stack([item.returns for item in columns])
    dsr, benchmark = deflated_sharpe(matrix, policy.minimum_sessions)
    pbo, splits = cscv(
        matrix, blocks=policy.pbo_blocks, minimum=policy.minimum_sessions, check=check
    )
    harmonic = math.fsum(1.0 / index for index in range(1, total + 1))
    reports = []
    for item, metric in zip(columns, dsr, strict=True):
        check()
        p_value = mean_test(item.returns, minimum=policy.minimum_sessions, lags=policy.hac_lags)[
            "p_value"
        ].value
        adjusted = min(1.0, p_value * total * harmonic) if p_value is not None else None
        reports.append(
            {
                "binding_sha256": item.binding,
                "job_ids": [peer.source.job_id for peer in groups[item.binding]],
                "p_value": p_value,
                "global_by_upper_bound": adjusted,
                "global_by_rejected": adjusted is not None and adjusted <= 0.05,
                "dsr": metric.model_dump(),
            }
        )
        for day, value in zip(item.dates, item.returns, strict=True):
            writer.writerow((day, item.binding, repr(value)))
    summary.update(
        complete_matrix=True,
        strategies=reports,
        dsr_benchmark=benchmark.model_dump(),
        pbo=pbo.model_dump(),
        splits=splits,
    )
    return canonical_bytes(summary), output.getvalue().encode("ascii")


def needs_returns(work: GlobalWork) -> bool:
    """Incomplete attempts remain visible without evaluating a selected subset."""
    return not _issues(work)
