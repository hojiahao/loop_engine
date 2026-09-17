"""Fixed installed portfolio producer; only the Rust runtime grants authority."""

import argparse
import csv
import hashlib
import io
import json
import math
import sys
import time
from decimal import Decimal
from pathlib import Path
from typing import Literal, Self

from pydantic import Field, model_validator

from loop_research.backtest import _Deadline, _materialize, _paths, _reference
from loop_research.backtest_models import BacktestRequest
from loop_research.build_identity import canonical_bytes
from loop_research.data.fetch_cache import publish, read_cached
from loop_research.data.fetch_json import decode_object
from loop_research.data.fetch_records import CachedObject
from loop_research.data.models import Identifier, ImmutableRecord
from loop_research.panel_io import ContentRef, _read
from loop_research.portfolio import decimal_text
from loop_research.portfolio_statistics import summarize
from loop_research.statistics_models import resolve_statistics, unavailable


class TrialEntry(ImmutableRecord):
    """Complete database projection, never a user-selected successful subset."""

    job_id: Identifier
    run_id: Identifier
    factor_spec_id: str = Field(pattern=r"^sha256:[0-9a-f]{64}$")
    specification_sha256: str = Field(pattern=r"^sha256:[0-9a-f]{64}$")
    attempts: int = Field(ge=1, le=65536)


class TrialLedger(ImmutableRecord):
    """Database-local global accounting; cryptographic bytes alone grant no trust."""

    schema_version: Literal["loop.global-trials/v1"] = Field(alias="schema")
    entries: tuple[TrialEntry, ...] = Field(min_length=1, max_length=4096)

    @model_validator(mode="after")
    def bounded_ledger(self) -> Self:
        ids = tuple(item.job_id for item in self.entries)
        if ids != tuple(sorted(set(ids))) or sum(item.attempts for item in self.entries) > 65536:
            raise ValueError("global trial ordering or attempt bounds")
        return self


class PortfolioWork(ImmutableRecord):
    """Small launcher-owned references; paths and executable code never enter stdin."""

    schema_version: Literal["loop.portfolio-work/v1"] = Field(alias="schema")
    job_id: Identifier
    lease_id: Identifier
    specification: CachedObject
    request: CachedObject
    trials: TrialLedger
    manifest: CachedObject | None = None


def global_statistics(ledger: TrialLedger, job_id: str, p_value: float | None) -> bytes:
    """Conservative BY upper bound over all accepted jobs and acquired attempts.

    Unobserved/failed/other trial p-values are one, not empirical losses. The
    complete synchronous matrix is absent, so global DSR/PBO are unavailable.
    """
    if not any(entry.job_id == job_id for entry in ledger.entries):
        raise ValueError("current job is absent from the global ledger")
    if p_value is not None and (not math.isfinite(p_value) or not 0 <= p_value <= 1):
        raise ValueError("invalid global trial p-value")
    count = sum(entry.attempts for entry in ledger.entries)
    adjusted = (
        min(1.0, p_value * count * math.fsum(1.0 / index for index in range(1, count + 1)))
        if p_value is not None
        else None
    )
    missing = unavailable("global_synchronous_returns_unavailable").model_dump()
    return canonical_bytes(
        {
            "schema": "loop.global-testing/v1",
            "scope": "all-database-development-jobs-and-acquired-attempts",
            "ledger_sha256": "sha256:"
            + hashlib.sha256(
                canonical_bytes(ledger.model_dump(mode="json", by_alias=True))
            ).hexdigest(),
            "registered_jobs": len(ledger.entries),
            "counted_attempts": count,
            "job_id": job_id,
            "p_value": p_value,
            "fdr_method": "benjamini-yekutieli-conservative-upper-bound",
            "other_p_values": "one-without-claiming-empirical-failure",
            "fdr_adjusted": adjusted,
            "fdr_rejected": adjusted is not None and adjusted <= 0.05,
            "dsr": missing,
            "pbo": missing,
            "production_eligible": False,
            "independent_validation": "pending-phase-8",
        }
    )


def execute(work: PortfolioWork, *, evidence: Path, view: Path, output: Path) -> dict[str, object]:
    """Reconstruct frozen inputs; publish last, or verify every byte without writes.

    The launcher authenticates identity/data/lease and checks the registered
    predecessor. These local files alone cannot complete a durable research job.
    """
    deadline = _Deadline(180, time.monotonic)
    work = PortfolioWork.model_validate_json(work.model_dump_json(by_alias=True))
    _paths(evidence, view, output)
    request_bytes = read_cached(evidence, work.request)
    decode_object(request_bytes)
    request = BacktestRequest.model_validate_json(request_bytes)
    market_guards = []

    def read_market(reference: CachedObject) -> bytes:
        deadline.check()
        content, guard = _read(view, ContentRef(**reference.model_dump()), 64 * 1024 * 1024)
        market_guards.append(guard)
        return content

    replay = _materialize(evidence, view, request, deadline, market_read=read_market)
    policy = resolve_statistics(request.policies["evaluation_policy"])
    if policy is None:
        raise ValueError("authorized portfolio requires frozen statistical settings")
    statistics = summarize(replay, policy, deadline.check)
    multiple = global_statistics(work.trials, work.job_id, statistics.mean_p)
    original = None
    if work.manifest is not None:
        original = read_cached(output, work.manifest)
        document = decode_object(original)
        result = document.get("result")
        if not isinstance(result, dict) or type(result.get("completed_at_ms")) is not int:
            raise ValueError("invalid portfolio completion clock")
        completed_ms = result["completed_at_ms"]
    else:
        completed_ms = time.time_ns() // 1_000_000
    pending: dict[str, bytes] = {}

    def describe(content: bytes, name: str, media: str, version: int = 1) -> dict[str, object]:
        columns = (
            next(csv.reader(io.StringIO(content.decode("ascii"))), [])
            if media == "text/csv"
            else []
        )
        schema = canonical_bytes(
            {
                "schema": "loop.artifact-schema/v1",
                "name": name,
                "version": version,
                "media_type": media,
                "columns": columns,
            }
        )
        for value in (schema, content):
            pending[_reference(value).sha256] = value
        return {
            "object": _reference(content).model_dump(),
            "schema": {
                "name": name,
                "version": version,
                "document": _reference(schema).model_dump(),
            },
            "media_type": media,
            "created_at_ms": completed_ms,
        }

    factor_version = 1 if replay.computed.transformation is None else 2
    series = {
        "factor_values": describe(
            replay.computed.values_csv, "loop.factor_values", "text/csv", factor_version
        )
    }
    for name, source in (
        ("target_positions", "targets"),
        ("orders", "orders"),
        ("fills", "fills"),
        ("nav", "nav"),
        ("simple_returns", "returns"),
        ("cost_ledger", "costs"),
    ):
        series[name] = describe(replay.artifacts[source], "loop.portfolio_" + source, "text/csv")
    series["risk_exposures"] = describe(
        statistics.artifacts["portfolio"], "loop.portfolio_statistics", "text/csv"
    )
    # Struct field order is shared with Rust's strict manifest reader.
    series = {
        key: series[key]
        for key in (
            "factor_values",
            "target_positions",
            "orders",
            "fills",
            "nav",
            "simple_returns",
            "risk_exposures",
            "cost_ledger",
        )
    }
    supplementary = [
        describe(replay.artifacts["positions"], "loop.portfolio_positions", "text/csv"),
        describe(
            canonical_bytes(replay.receipt.model_dump(mode="json", by_alias=True)),
            "loop.portfolio_receipt",
            "application/json",
        ),
        describe(statistics.artifacts["summary"], "loop.statistics_summary", "application/json"),
        describe(statistics.artifacts["cross_sections"], "loop.cross_sections", "text/csv"),
        describe(statistics.artifacts["exposures"], "loop.industry_exposures", "text/csv"),
        describe(multiple, "loop.global_testing", "application/json"),
    ]
    # This JSON was emitted by our finite-float statistical kernel. Vendor
    # decimal precision bounds do not describe statistical p-values/moments.
    summary = json.loads(statistics.artifacts["summary"])
    metrics: list[dict[str, str]] = [
        {
            "name": "ending_nav",
            "value": replay.receipt.ending_nav_usd,
            "unit": "USD",
            "estimator": replay.receipt.engine,
        },
        {
            "name": "global_attempts",
            "value": str(sum(entry.attempts for entry in work.trials.entries)),
            "unit": "count",
            "estimator": "all-accepted-jobs-and-acquired-attempts",
        },
    ]
    for name in (
        "total_return",
        "maximum_drawdown",
        "one_way_turnover",
        "sharpe_daily",
        "sharpe_annualized",
    ):
        metric = summary[name]
        if isinstance(metric, dict) and isinstance(metric.get("value"), (int, float)):
            metrics.append(
                {
                    "name": name,
                    "value": decimal_text(Decimal(str(metric["value"]))),
                    "unit": "ratio",
                    "estimator": "daily-statistics.1",
                }
            )
    document = {
        "schema": "loop.authorized-portfolio/v1",
        "lease_id": work.lease_id,
        "request": work.request.model_dump(),
        "trials": work.trials.model_dump(mode="json", by_alias=True),
        "result": {
            "schema": "loop.backtest-result/v1",
            "job_id": work.job_id,
            "specification": work.specification.model_dump(),
            "engine": "primary_cross_sectional",
            "engine_version": "authorized-portfolio.1",
            "metrics": sorted(metrics, key=lambda metric: metric["name"]),
            "artifacts": series,
            "completed_at_ms": completed_ms,
        },
        "supplementary": supplementary,
    }
    content = canonical_bytes(document)
    artifact = describe(content, "loop.authorized_portfolio", "application/json")
    replay.check()
    for guard in market_guards:
        guard.check()
    if read_cached(evidence, work.request) != request_bytes:
        raise ValueError("portfolio request changed")
    if original is not None and content != original:
        raise ValueError("authorized portfolio differs from numerical replay")
    for digest, value in pending.items():
        deadline.check()
        if original is None:
            publish(output, value)
        elif read_cached(output, CachedObject(sha256=digest, byte_size=len(value))) != value:
            raise ValueError("registered portfolio output differs from numerical replay")
    replay.check()
    for guard in market_guards:
        guard.check()
    return artifact


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("evidence", "view", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    arguments = parser.parse_args()
    try:
        content = sys.stdin.buffer.read(1_048_577)
        if not 0 < len(content) <= 1_048_576:
            raise ValueError("portfolio work byte budget")
        decode_object(content)
        work = PortfolioWork.model_validate_json(content)
        artifact = execute(
            work, evidence=arguments.evidence, view=arguments.view, output=arguments.output
        )
        sys.stdout.buffer.write(canonical_bytes(artifact))
        return 0
    except KeyboardInterrupt:
        return 130
    except OSError, ValueError, TimeoutError:
        print("portfolio worker refused execution", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
