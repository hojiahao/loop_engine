"""Export raw frozen observations for a separately installed Alphalens worker."""

import csv
import io
import time
from collections.abc import Callable
from pathlib import Path

from loop_protocol.canonical import FactorDirection

from loop_research.backtest import _Deadline, _paths
from loop_research.build_identity import canonical_bytes
from loop_research.data.fetch_cache import publish
from loop_research.data.fetch_records import CachedObject
from loop_research.statistics_models import resolve_statistics
from loop_research.statistics_workflow import reconstruct_statistics


def prepare_alphalens(
    evidence: Path,
    view: Path,
    store: Path,
    digest: str,
    *,
    timeout_seconds: float = 180,
    clock: Callable[[], float] = time.monotonic,
) -> CachedObject:
    """Publish validator input only after actual portfolio/statistics replay.

    The export contains raw signals, eligibility and next-session prices, never
    primary ranks or labels. It remains an administrative diagnostic, without
    database authority, holdout access or an admission vote. Cancellation before
    the final manifest leaves only unreferenced immutable objects.
    """
    deadline = _Deadline(timeout_seconds, clock)
    _paths(evidence, view, store)
    report, primary, check = reconstruct_statistics(evidence, view, store, digest, deadline)
    policy = resolve_statistics(primary.receipt.request.policies["evaluation_policy"])
    if policy is None:
        raise ValueError("independent validation requires frozen statistics")
    stream = io.StringIO(newline="")
    writer = csv.writer(stream, lineterminator="\n")
    writer.writerow(
        ("session", "security_id", "eligible", "factor", "label_session", "opening", "closing")
    )
    for index, session in enumerate(primary.sessions):
        deadline.check()
        next_session = primary.sessions[index + 1] if index + 1 < len(primary.sessions) else None
        future = (
            {row.security_id: row for row in next_session.observations}
            if next_session is not None
            else {}
        )
        for row in session.observations:
            forward = future.get(row.security_id)
            writer.writerow(
                (
                    session.day.isoformat(),
                    row.security_id,
                    int(row.eligible),
                    "" if row.factor is None else format(float(row.factor), ".17g"),
                    "" if next_session is None else next_session.day.isoformat(),
                    "" if forward is None or forward.opening is None else str(forward.opening),
                    "" if forward is None or forward.closing is None else str(forward.closing),
                )
            )
    content = stream.getvalue().encode("ascii")
    if len(content) > 32 * 1024 * 1024:
        raise ValueError("independent observation byte budget")
    raw = publish(store, content)
    document = {
        "schema": "loop.alphalens-input/v1",
        "profile": "alphalens-statistics.1",
        "primary_statistics": report.receipt.model_dump(),
        "primary_backtest": report.artifacts.request.backtest.model_dump(),
        "source_code_sha256": report.artifacts.source_code_sha256,
        "environment_sha256": report.artifacts.environment_sha256,
        "observations": raw.model_dump(),
        "primary_cross_sections": report.artifacts.cross_sections.model_dump(),
        "sessions": [session.day.isoformat() for session in primary.sessions],
        "securities": [row.security_id for row in primary.sessions[0].observations],
        "direction": (
            "higher_is_better"
            if primary.computed.factor.spec.direction == FactorDirection.HIGHER_IS_BETTER
            else "lower_is_better"
        ),
        "groups": policy.groups,
        "minimum_cross_section": policy.minimum_cross_section,
        "minimum_sessions": policy.minimum_sessions,
        "production_eligible": False,
    }
    payload = canonical_bytes(document)
    if len(payload) > 128 * 1024:
        raise ValueError("independent input manifest byte budget")
    check()
    deadline.check()
    return publish(store, payload)
