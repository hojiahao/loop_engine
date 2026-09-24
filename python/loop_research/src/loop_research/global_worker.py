"""Fixed global-statistics worker; the runtime owns identity and registration."""

import argparse
import hashlib
import os
import sys
import time
from collections.abc import Callable
from pathlib import Path

from loop_research.backtest import PortfolioReplay, _Deadline, _paths
from loop_research.build_identity import canonical_bytes
from loop_research.data.fetch_cache import private_directory, publish, read_cached
from loop_research.data.fetch_json import decode_object
from loop_research.global_models import GlobalPolicy, GlobalPortfolio, GlobalWork
from loop_research.global_statistics import StrategySeries, matrix_report, needs_returns
from loop_research.portfolio_worker import TrialLedger, replay_portfolio
from loop_research.statistics_workflow import trial_binding


def context_digest(replay: PortfolioReplay) -> str:
    """A common return basis; factor expression and direction may vary."""
    provenance = replay.work.provenance
    content = {
        "schema": "loop.global-return-context/v1",
        "quality": replay.receipt.quality,
        "calendar_version": replay.receipt.calendar_version,
        "engine": replay.receipt.engine,
        "execution_tape": replay.receipt.request.execution_tape.model_dump(),
        "panel_sha256": replay.work.panel_manifest.sha256.value.hex(),
        "source_code_sha256": provenance.source_code_sha256.value.hex(),
        "operator_registry_sha256": provenance.operator_registry_sha256.value.hex(),
        "environment_sha256": provenance.environment_sha256.value.hex(),
        "data_manifest_sha256": provenance.data_manifest_sha256.value.hex(),
        "trading_calendar_sha256": provenance.trading_calendar_sha256.value.hex(),
    }
    return "sha256:" + hashlib.sha256(canonical_bytes(content)).hexdigest()


def historical_ledger(source: GlobalPortfolio, work: GlobalWork, store: Path) -> TrialLedger:
    """Replay original bytes, while recomputing new search-adjusted statistics."""
    document = decode_object(read_cached(store, source.manifest))
    ledger = TrialLedger.model_validate_json(canonical_bytes(document["trials"]))
    current = {entry.job_id: entry for entry in work.snapshot.ledger.entries}
    for old in ledger.entries:
        new = current.get(old.job_id)
        if (
            new is None
            or new.model_dump(exclude={"attempts"}) != old.model_dump(exclude={"attempts"})
            or new.attempts < old.attempts
        ):
            raise ValueError("historical trial commitment is outside the current registry")
    return ledger


def prepare(
    work: GlobalWork, *, evidence: Path, views: tuple[Path, ...], primary_store: Path, output: Path
) -> dict[str, object]:
    """Publish complete population diagnostics, or reconstruct without writes.

    A caller cannot use this local command's receipt as durable authority. The
    supervisor authenticates every source and rechecks its database snapshot.
    """
    deadline = _Deadline(180, time.monotonic)
    work = GlobalWork.model_validate_json(work.model_dump_json(by_alias=True))
    for path in (evidence, primary_store, output):
        os.close(private_directory(path))
    if len(views) != len(work.portfolios):
        raise ValueError("global view count differs from registered sources")
    for view in views:
        _paths(evidence, view, output)
        _paths(evidence, view, primary_store)
    if output.is_relative_to(primary_store) or primary_store.is_relative_to(output):
        raise ValueError("global statistics output must be separate")
    readonly = work.manifest is not None
    policy_bytes = read_cached(evidence, work.policy)
    decode_object(policy_bytes)
    policy = GlobalPolicy.model_validate_json(policy_bytes)
    guards: list[Callable[[], None]] = []
    series = []
    complete = needs_returns(work)
    for source, view in zip(work.portfolios, views, strict=True):
        deadline.check()
        replay, statistics, guard = replay_portfolio(
            source.work(historical_ledger(source, work, primary_store)),
            evidence=evidence,
            view=view,
            output=primary_store,
        )
        if replay.work.job_id.value != source.evaluation_job_id:
            raise ValueError("global numerical predecessor differs")
        guards.append(guard)
        # Incomplete populations still verify every successful source, but
        # never calculate selection statistics on just that successful subset.
        if complete:
            series.append(
                StrategySeries(
                    source=source,
                    binding=trial_binding(replay.work, replay.receipt.request.execution_tape),
                    context=context_digest(replay),
                    dates=statistics.dates,
                    returns=tuple(float(value) for value in statistics.returns),
                )
            )
    summary, matrix = matrix_report(work, policy, tuple(series), deadline.check)
    pending = {
        "snapshot": canonical_bytes(work.snapshot.model_dump(mode="json", by_alias=True)),
        "request": canonical_bytes(
            work.model_dump(mode="json", by_alias=True, exclude={"manifest"})
        ),
        "summary": summary,
        "matrix": matrix,
    }
    if sum(map(len, pending.values())) > 64 * 1024 * 1024:
        raise ValueError("global report output byte budget")

    def check() -> None:
        deadline.check()
        for guard in guards:
            guard()
        if read_cached(evidence, work.policy) != policy_bytes:
            raise ValueError("global statistical policy changed")

    check()
    references = {}
    for key, content in pending.items():
        deadline.check()
        references[key] = publish(output, content, readonly=readonly).model_dump()
    document = canonical_bytes(
        {
            "schema": "loop.authorized-global-statistics/v1",
            "job_id": work.job_id,
            "lease_id": work.lease_id,
            "started_at_ms": work.started_at_ms,
            "policy": work.policy.model_dump(),
            **references,
            "production_eligible": False,
        }
    )
    schema = canonical_bytes(
        {
            "schema": "loop.artifact-schema/v1",
            "name": "loop.authorized_global_statistics",
            "version": 1,
            "media_type": "application/json",
            "columns": [],
        }
    )
    if work.manifest is not None and read_cached(output, work.manifest) != document:
        raise ValueError("global report differs from immutable replay")
    check()
    schema_ref = publish(output, schema, readonly=readonly)
    reference = publish(output, document, readonly=readonly)
    if work.manifest is not None and reference != work.manifest:
        raise ValueError("global report replay identity differs")
    return {
        "object": reference.model_dump(),
        "schema": {
            "name": "loop.authorized_global_statistics",
            "version": 1,
            "document": schema_ref.model_dump(),
        },
        "media_type": "application/json",
        "created_at_ms": work.started_at_ms,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("evidence", "primary-store", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--view", type=Path, action="append", default=[])
    arguments = parser.parse_args()
    try:
        content = sys.stdin.buffer.read(1_048_577)
        if not 0 < len(content) <= 1_048_576:
            raise ValueError("global statistics work byte budget")
        decode_object(content)
        work = GlobalWork.model_validate_json(content)
        result = prepare(
            work,
            evidence=arguments.evidence,
            views=tuple(arguments.view),
            primary_store=arguments.primary_store,
            output=arguments.output,
        )
        sys.stdout.buffer.write(canonical_bytes(result))
        return 0
    except KeyboardInterrupt:
        return 130
    except OSError, ValueError, TimeoutError, KeyError, TypeError:
        print("global statistics refused execution", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
