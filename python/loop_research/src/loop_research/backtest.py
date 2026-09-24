"""Bounded administrative portfolio execution and immutable, read-only replay."""

import csv
import hashlib
import io
import math
import os
import re
import time
from collections.abc import Callable
from dataclasses import dataclass
from datetime import timedelta
from pathlib import Path

from google.protobuf.message import DecodeError  # type: ignore[import-untyped]
from loop.v1.evaluation_pb2 import FactorEvaluationWork

from loop_research.backtest_models import (
    MAX_REPLAY_CELLS,
    POLICY_ROLES,
    BacktestReceipt,
    BacktestReport,
    BacktestRequest,
    ExecutionTape,
    LedgerArtifacts,
    PortfolioPolicy,
    money,
)
from loop_research.build_identity import canonical_bytes, require_build
from loop_research.calendar import require_session_decisions
from loop_research.data.fetch_cache import (
    private_directory,
    publish,
    read_cached,
    read_config_bytes,
    read_receipt,
)
from loop_research.data.fetch_json import decode_object
from loop_research.data.fetch_records import CachedObject
from loop_research.execution_inputs import MarketSession, prepare_market
from loop_research.factor_worker import FactorComputation, compute, encode_manifest
from loop_research.market_models import ExecutionCapture, MarketPolicy, MarketTape
from loop_research.market_portfolio import replay_market
from loop_research.portfolio import Observation, Session, decimal_text, replay
from loop_research.statistics_models import resolve_statistics


class _Deadline:
    def __init__(self, seconds: float, clock: Callable[[], float]) -> None:
        if not math.isfinite(seconds) or not 0 < seconds <= 180:
            raise ValueError("portfolio deadline budget")
        self.clock, self.seconds = clock, seconds
        self.started = self.previous = clock()
        self.check()

    def check(self) -> None:
        current = self.clock()
        if (
            not math.isfinite(current)
            or not math.isfinite(self.previous)
            or current < self.previous
        ):
            raise ValueError("portfolio clock regression")
        self.previous = current
        if current - self.started >= self.seconds:
            raise TimeoutError("portfolio deadline exceeded")


@dataclass(frozen=True, slots=True)
class PortfolioReplay:
    """Actual reconstructed inputs/outputs and live guards, without job authority."""

    receipt: BacktestReceipt
    artifacts: dict[str, bytes]
    sessions: tuple[Session, ...]
    computed: FactorComputation
    work: FactorEvaluationWork
    check: Callable[[], None]
    policy: PortfolioPolicy | MarketPolicy
    market: tuple[MarketSession, ...] | None


def load_request(path: Path) -> BacktestRequest:
    """Read a small explicit recipe, rejecting duplicate fields and unsafe leaves."""
    content = read_config_bytes(path)
    decode_object(content)
    return BacktestRequest.model_validate_json(content)


def _integer(value: str, lower: int, upper: int) -> int:
    if not re.fullmatch(r"0|[1-9][0-9]{0,15}", value, re.ASCII):
        raise ValueError("portfolio input requires a canonical integer")
    result = int(value)
    if not lower <= result <= upper:
        raise ValueError("portfolio integer bounds")
    return result


def _policy(
    request: BacktestRequest, computed: FactorComputation
) -> PortfolioPolicy | MarketPolicy:
    factor = computed.factor.spec
    for role in POLICY_ROLES:
        document = request.policies[role]
        reference = getattr(factor, role)
        if (document.policy_id, document.revision, document.digest()) != (
            reference.policy_id,
            reference.revision,
            reference.sha256,
        ):
            raise ValueError("portfolio policy differs from the frozen FactorSpec")
    # Selection/calendar evidence comes from the already built panel. This base
    # profile must not silently implement an unrecognized declarative policy.
    for role in ("universe_policy", "data_policy", "calendar_policy"):
        if request.policies[role].settings:
            raise ValueError("unsupported panel selection policy")
    processing = computed.loaded.manifest.transform
    for role in ("preprocess_policy", "neutralization_policy"):
        document = request.policies[role]
        if processing is None:
            if document.settings:
                raise ValueError("raw factor values cannot implement a transformation policy")
        elif document != getattr(processing, role.removesuffix("_policy")):
            raise ValueError("portfolio transformation policy differs from the factor panel")
    coverage = request.policies["evaluation_policy"].settings
    resolve_statistics(request.policies["evaluation_policy"])
    minimum = _integer(coverage["minimum_coverage_bps"], 0, 10000)
    result = computed.result
    if not result.eligible_observations or (
        result.valid_observations * 10000 < result.eligible_observations * minimum
    ):
        raise ValueError("factor coverage does not satisfy its frozen policy")
    settings = request.policies["portfolio_policy"].settings
    if settings.get("algorithm") == "ranked-long-short.1":
        return _market_policy(request)
    if set(settings) != {"algorithm", "holdings", "initial_cash_usd", "lot_size"} or (
        settings["algorithm"] != "long-only-top-n.1"
    ):
        raise ValueError("unsupported portfolio policy")
    if request.policies["execution_policy"].settings != {"algorithm": "next-session-open.1"}:
        raise ValueError("unsupported execution policy")
    costs = request.policies["cost_policy"].settings
    if (
        set(costs)
        != {"algorithm", "commission_per_share_usd", "half_spread_bps", "minimum_commission_usd"}
        or costs["algorithm"] != "commission-spread.1"
    ):
        raise ValueError("unsupported cost policy")
    return PortfolioPolicy(
        initial_cash_usd=settings["initial_cash_usd"],
        holdings=_integer(settings["holdings"], 1, 1000),
        lot_size=_integer(settings["lot_size"], 1, 10000),
        commission_per_share_usd=costs["commission_per_share_usd"],
        minimum_commission_usd=costs["minimum_commission_usd"],
        half_spread_bps=_integer(costs["half_spread_bps"], 0, 1000),
    )


def _market_policy(request: BacktestRequest) -> MarketPolicy:
    settings = request.policies["portfolio_policy"].settings
    costs = request.policies["cost_policy"].settings
    execution = request.policies["execution_policy"].settings
    portfolio_integers = {
        "holdings",
        "lot_size",
        "long_weight_bps",
        "short_weight_bps",
        "initial_margin_bps",
        "maintenance_margin_bps",
    }
    cost_integers = {
        "half_spread_bps",
        "impact_bps",
        "short_collateral_bps",
        "cash_debit_bps",
        "cash_credit_bps",
        "day_count",
    }
    if set(settings) != {"algorithm", "initial_cash_usd", *portfolio_integers}:
        raise ValueError("unsupported market portfolio settings")
    if (
        set(execution) != {"algorithm", "participation_bps"}
        or execution["algorithm"] != "pit-next-open.1"
    ):
        raise ValueError("unsupported market execution policy")
    if (
        set(costs)
        != {"algorithm", "commission_per_share_usd", "minimum_commission_usd", *cost_integers}
        or costs["algorithm"] != "commission-impact-finance.1"
    ):
        raise ValueError("unsupported market cost policy")
    return MarketPolicy.model_validate(
        {
            **{key: _integer(settings[key], 0, 100000) for key in portfolio_integers},
            **{key: _integer(costs[key], 0, 100000) for key in cost_integers},
            "initial_cash_usd": settings["initial_cash_usd"],
            "commission_per_share_usd": costs["commission_per_share_usd"],
            "minimum_commission_usd": costs["minimum_commission_usd"],
            "participation_bps": _integer(execution["participation_bps"], 1, 10000),
        }
    )


def _sessions(
    content: bytes, computed: FactorComputation, deadline: _Deadline
) -> tuple[Session, ...]:
    import exchange_calendars as xcals  # type: ignore[import-untyped]

    panel, result = computed.loaded.panel, computed.result
    start = panel.sessions.index(result.evaluation_start)
    sessions = panel.sessions[start:]
    if len(sessions) < 2 or len(sessions) * len(panel.securities) > MAX_REPLAY_CELLS:
        raise ValueError("portfolio sample has no execution session or exceeds cell budget")
    calendar = xcals.get_calendar(
        "XNYS",
        start=(sessions[0] - timedelta(days=7)).isoformat(),
        end=(sessions[-1] + timedelta(days=7)).isoformat(),
    )
    reader = csv.reader(io.StringIO(content.decode("ascii"), newline=""), strict=True)
    if next(reader, None) != [
        "session",
        "security_id",
        "open_at_ms",
        "open_usd",
        "close_known_at_ms",
        "close_usd",
    ]:
        raise ValueError("execution tape CSV schema differs")
    prepared = []
    for index, day in enumerate(sessions):
        deadline.check()
        open_ms = int(calendar.session_open(day.isoformat()).value // 1_000_000)
        close_ms = int(calendar.session_close(day.isoformat()).value // 1_000_000)
        decision = computed.loaded.manifest.decision_times_ms[start + index]
        observations = []
        for column, security in enumerate(panel.securities):
            row = next(reader, None)
            if row is None or len(row) != 6 or row[:2] != [day.isoformat(), security]:
                raise ValueError("execution tape must cover the exact evaluation grid")
            if (row[2] == "") != (row[3] == "") or (row[4] == "") != (row[5] == ""):
                raise ValueError("execution observations require paired price and clock")
            for price in (row[3], row[5]):
                if price and not re.fullmatch(r"(0|[1-9][0-9]{0,9})(\.[0-9]{1,8})?", price):
                    raise ValueError("execution observation must be a bounded raw USD decimal")
            value = float(result.values[index, column])
            observations.append(
                Observation(
                    security,
                    bool(panel.eligible[start + index, column]),
                    value if math.isfinite(value) else None,
                    _integer(row[2], open_ms, close_ms - 1) if row[2] else None,
                    money(row[3], positive=True, maximum="1000000000") if row[3] else None,
                    _integer(row[4], close_ms, decision) if row[4] else None,
                    money(row[5], positive=True, maximum="1000000000") if row[5] else None,
                )
            )
        prepared.append(Session(day, decision, tuple(observations)))
    if next(reader, None) is not None:
        raise ValueError("execution tape contains rows outside the evaluation grid")
    return tuple(prepared)


def _reference(content: bytes) -> CachedObject:
    return CachedObject(
        sha256="sha256:" + hashlib.sha256(content).hexdigest(), byte_size=len(content)
    )


def _materialize(
    evidence: Path,
    view: Path,
    request: BacktestRequest,
    deadline: _Deadline,
    *,
    market_read: Callable[[CachedObject], bytes] | None = None,
) -> PortfolioReplay:
    request = BacktestRequest.model_validate_json(request.model_dump_json(by_alias=True))
    inputs: list[tuple[CachedObject, bytes]] = []

    def read(reference: CachedObject) -> bytes:
        deadline.check()
        content = read_cached(evidence, reference)
        inputs.append((reference, content))
        return content

    try:
        work = FactorEvaluationWork.FromString(read(request.evaluation_work))
    except DecodeError as error:
        raise ValueError("invalid frozen evaluation work") from error
    result_bytes = read(request.evaluation_result)
    result_document = decode_object(result_bytes)
    completed_ms = result_document.get("completed_at_ms")
    if type(completed_ms) is not int or not 0 < completed_ms < 2**53:
        raise ValueError("invalid evaluation completion clock")
    # Authorized execution supplies only the broker's immutable data view for
    # market objects. Administrative workflows keep their existing private CAS.
    read_market = market_read or read
    tape_bytes = read_market(request.execution_tape)
    tape_document = decode_object(tape_bytes)
    tape = (
        MarketTape.model_validate_json(tape_bytes)
        if tape_document.get("schema") == "loop.execution-tape/v2"
        else ExecutionTape.model_validate_json(tape_bytes)
    )
    factor_values = read(request.factor_values)
    computed = compute(work, view=view)
    deadline.check()
    if factor_values != computed.values_csv or result_bytes != (
        encode_manifest(
            work, computed, values_sha256=request.factor_values.sha256, completed_ms=completed_ms
        )
    ):
        raise ValueError("factor evidence differs from actual numerical replay")
    if tape.quality != computed.loaded.manifest.quality:
        raise ValueError("execution and factor data quality differ")
    policy = _policy(request, computed)
    market = None
    if isinstance(tape, MarketTape):
        if not isinstance(policy, MarketPolicy):
            raise ValueError("version-2 execution evidence requires frozen market policies")
        content = read_market(tape.capture)
        decode_object(content)
        capture = ExecutionCapture.model_validate_json(content)
        market = prepare_market(capture, computed, read_market, deadline.check)
        sessions = tuple(session.base for session in market)
        ledger = replay_market(
            market, policy, computed.factor.spec.direction, check_budget=deadline.check
        )
    else:
        if isinstance(policy, MarketPolicy):
            raise ValueError("market accounting requires version-2 execution evidence")
        try:
            sessions = _sessions(read_market(tape.observations), computed, deadline)
        except csv.Error as error:
            raise ValueError("invalid execution CSV encoding") from error
        ledger = replay(
            sessions, policy, computed.factor.spec.direction, check_budget=deadline.check
        )
    calendar = require_session_decisions(
        tuple(session.day for session in sessions),
        tuple(session.decision_ms for session in sessions),
    )
    source = "sha256:" + work.provenance.source_code_sha256.value.hex()
    environment = "sha256:" + work.provenance.environment_sha256.value.hex()

    def check() -> None:
        deadline.check()
        computed.loaded.check()
        require_build(source, environment, profile="evaluation")
        for reference, content in inputs:
            if read_cached(evidence, reference) != content:
                raise ValueError("portfolio input changed during execution")
        deadline.check()

    receipt = BacktestReceipt(
        engine="pit-actions-long-short.1"
        if isinstance(tape, MarketTape)
        else "long-only-next-open.1",
        request=request,
        factor_spec_id=computed.factor.factor_spec_id,
        source_code_sha256=source,
        environment_sha256=environment,
        calendar_version=calendar.package_version,
        quality=tape.quality,
        artifacts=LedgerArtifacts.model_validate(
            {name: _reference(content) for name, content in ledger.artifacts.items()}
        ),
        sessions=len(sessions),
        orders=ledger.orders,
        fills=ledger.fills,
        ending_nav_usd=decimal_text(ledger.ending_nav),
    )
    return PortfolioReplay(
        receipt, ledger.artifacts, sessions, computed, work, check, policy, market
    )


def _paths(evidence: Path, view: Path, store: Path) -> None:
    for directory in (evidence, store):
        os.close(private_directory(directory))
        if directory == view or directory.is_relative_to(view) or view.is_relative_to(directory):
            raise ValueError("portfolio cache must be separate from the read-only factor view")


def _report(reference: CachedObject, receipt: BacktestReceipt) -> BacktestReport:
    return BacktestReport(
        receipt=reference,
        artifacts=receipt.artifacts,
        quality=receipt.quality,
        sessions=receipt.sessions,
        orders=receipt.orders,
        fills=receipt.fills,
        ending_nav_usd=receipt.ending_nav_usd,
    )


def run_backtest(
    evidence: Path,
    view: Path,
    store: Path,
    request: BacktestRequest,
    *,
    timeout_seconds: float = 180,
    clock: Callable[[], float] = time.monotonic,
) -> BacktestReport:
    """Recompute a frozen development evaluation and publish its modeled ledger.

    Paths are deployment-selected administrative inputs, not capabilities. No
    protected samples, database mutations, admission, or paid requests occur.
    Interruptions may leave verified CAS objects; only the last receipt denotes
    a complete run. Existing artifacts are never overwritten or silently fixed.
    """
    deadline = _Deadline(timeout_seconds, clock)
    _paths(evidence, view, store)
    result = _materialize(evidence, view, request, deadline)
    for content in result.artifacts.values():
        deadline.check()
        publish(store, content)
    result.check()
    return _report(
        publish(store, canonical_bytes(result.receipt.model_dump(mode="json", by_alias=True))),
        result.receipt,
    )


def validate_backtest(
    evidence: Path,
    view: Path,
    store: Path,
    digest: str,
    *,
    timeout_seconds: float = 180,
    clock: Callable[[], float] = time.monotonic,
) -> BacktestReport:
    """Reconstruct every ledger byte under the current build without writing files.

    Source/environment drift, altered inputs or outputs, corruption and budget
    failures invalidate current replay while preserving historical evidence.
    """
    deadline = _Deadline(timeout_seconds, clock)
    _paths(evidence, view, store)
    reference, _ = read_receipt(store, digest)
    result = reconstruct(evidence, view, store, reference, deadline)
    return _report(reference, result.receipt)


def reconstruct(
    evidence: Path, view: Path, store: Path, reference: CachedObject, deadline: _Deadline
) -> PortfolioReplay:
    """Verify every ledger byte and retain input/output guards for downstream stats.

    This is administrative reconstruction, not capability resolution. The caller
    must check the returned guards again before publishing dependent receipts.
    """
    if reference.byte_size > 128 * 1024:
        raise ValueError("portfolio receipt byte budget")
    content = read_cached(store, reference)
    decode_object(content)
    original = BacktestReceipt.model_validate_json(content)
    result = _materialize(evidence, view, original.request, deadline)
    if canonical_bytes(result.receipt.model_dump(mode="json", by_alias=True)) != content:
        raise ValueError("portfolio receipt differs from actual replay")

    def check() -> None:
        result.check()
        for name, expected in result.artifacts.items():
            deadline.check()
            if read_cached(store, getattr(result.receipt.artifacts, name)) != expected:
                raise ValueError("portfolio output differs from actual replay")
        if read_cached(store, reference) != content:
            raise ValueError("portfolio receipt changed during replay")

    check()
    return PortfolioReplay(
        result.receipt,
        result.artifacts,
        result.sessions,
        result.computed,
        result.work,
        check,
        result.policy,
        result.market,
    )
