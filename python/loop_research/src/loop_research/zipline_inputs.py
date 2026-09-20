"""Verified raw execution export; independent workers never receive primary fills as orders."""

import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

from loop_protocol.canonical import FactorDirection

from loop_research.backtest import PortfolioReplay, _Deadline, _paths, reconstruct
from loop_research.build_identity import canonical_bytes
from loop_research.data.fetch_cache import publish, read_receipt
from loop_research.data.fetch_records import CachedObject
from loop_research.execution_inputs import milliseconds
from loop_research.market_models import Action, CashAction, ExecutionTerms, Split


def _terms(value: ExecutionTerms | None) -> dict[str, Any] | None:
    if value is None:
        return None
    return {
        name: getattr(value, name)
        for name in (
            "tradable",
            "short_allowed",
            "borrow_limit",
            "borrow_rate_bps",
            "recalled",
            "sec_fee_usd_per_million",
            "taf_fee_usd_per_share",
            "taf_fee_cap_usd",
        )
    }


def _action(value: Action) -> dict[str, Any]:
    common = {
        "event_id": value.event_id,
        "security_id": value.security_id,
        "at_ms": milliseconds(value.effective_at),
    }
    if isinstance(value, Split):
        return {
            **common,
            "kind": "split",
            "numerator": value.numerator,
            "denominator": value.denominator,
            "fraction_price_usd": value.fraction_price_usd,
            "pay_ms": milliseconds(value.pay_at) if value.pay_at is not None else None,
        }
    if isinstance(value, CashAction):
        return {
            **common,
            "kind": value.kind,
            "amount_per_share_usd": value.amount_per_share_usd,
            "pay_ms": milliseconds(value.pay_at),
        }
    raise ValueError("unsupported independent corporate action")


def observations(primary: PortfolioReplay, check: Callable[[], None]) -> bytes:
    """Serialize only causal signals, resolved raw quotes, declared terms and events.

    PIT selection remains a disclosed common dependency. Neither primary orders,
    targets, positions nor return calculations enter this numerical input.
    """
    sessions = []
    for index, session in enumerate(primary.sessions):
        check()
        market = primary.market[index] if primary.market is not None else None
        rows = []
        for column, row in enumerate(session.observations):
            resolved = market.rows[column] if market is not None else None
            rows.append(
                {
                    "security_id": row.security_id,
                    "eligible": row.eligible,
                    "factor": row.factor,
                    "open_ms": row.open_at_ms,
                    "opening": None if row.opening is None else str(row.opening),
                    "close_ms": row.close_known_at_ms,
                    "closing": None if row.closing is None else str(row.closing),
                    "auction_volume": resolved.auction_volume if resolved is not None else None,
                    "opening_terms": _terms(resolved.opening_terms)
                    if resolved is not None
                    else None,
                    "closing_terms": _terms(resolved.closing_terms)
                    if resolved is not None
                    else None,
                }
            )
        sessions.append(
            {
                "day": session.day.isoformat(),
                "decision_ms": session.decision_ms,
                "scheduled_open_ms": market.scheduled_open_ms if market else session.decision_ms,
                "rows": rows,
                "actions": [_action(action) for action in market.actions] if market else [],
            }
        )
    result = canonical_bytes({"schema": "loop.zipline-observations/v1", "sessions": sessions})
    if len(result) > 64 * 1024 * 1024:
        raise ValueError("independent execution byte budget")
    return result


def prepare_zipline(
    evidence: Path,
    view: Path,
    store: Path,
    digest: str,
    *,
    timeout_seconds: float = 180,
    clock: Callable[[], float] = time.monotonic,
    readonly: bool = False,
) -> CachedObject:
    """Reconstruct actual primary evidence before publishing a diagnostic manifest.

    No holdout capability or database authority is granted. Cancellation leaves
    at most unreferenced immutable objects; the manifest is published last.
    """
    deadline = _Deadline(timeout_seconds, clock)
    _paths(evidence, view, store)
    reference, _ = read_receipt(store, digest)
    primary = reconstruct(evidence, view, store, reference, deadline)
    return _export_zipline(store, reference, primary, deadline, readonly=readonly)


def _export_zipline(
    store: Path,
    reference: CachedObject,
    primary: PortfolioReplay,
    deadline: _Deadline,
    *,
    readonly: bool,
) -> CachedObject:
    """Export from the caller's live verified reconstruction, retaining its guards."""
    raw = publish(store, observations(primary, deadline.check), readonly=readonly)
    content = canonical_bytes(
        {
            "schema": "loop.zipline-input/v1",
            "profile": "zipline-accounting.1",
            "primary_backtest": reference.model_dump(),
            "observations": raw.model_dump(),
            "engine": primary.receipt.engine,
            "source_code_sha256": primary.receipt.source_code_sha256,
            "environment_sha256": primary.receipt.environment_sha256,
            "direction": "higher_is_better"
            if primary.computed.factor.spec.direction == FactorDirection.HIGHER_IS_BETTER
            else "lower_is_better",
            "policy": primary.policy.model_dump(),
            "primary_artifacts": primary.receipt.artifacts.model_dump(),
            "production_eligible": False,
        }
    )
    if len(content) > 128 * 1024:
        raise ValueError("independent manifest byte budget")
    primary.check()
    deadline.check()
    return publish(store, content, readonly=readonly)
