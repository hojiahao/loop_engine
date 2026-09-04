# -*- coding: utf-8 -*-
"""Atomically re-evaluate the complete factor library with current semantics."""
from __future__ import annotations

import argparse
import copy
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone

from backtest.alphalab_adapter import AlphalabEvaluator
from backtest.returns import nav_to_returns
from engine import review
from engine.checkpoint import Checkpoint
from engine.evolve import Evolver
from engine.expression import evaluate, parse
from engine.fsa import FSA, skeleton
from engine.io_utils import ProcessLock, atomic_write_json
from engine.perturb import Perturber
from engine.provenance import evaluation_record
from filters import apply_filters
from loop_orchestrate import _coverage_reason, _metrics_summary
from paths import OUTPUT_DIR
from run_round_cli import DEFAULT_ALPHALAB_CONFIG, FIELDS, _real_panels


class RevalidationError(RuntimeError):
    """At least one factor could not be evaluated; no checkpoint was committed."""


def revalidate_library(*, checkpoint: Checkpoint, evaluator, field_panels: dict,
                       workers: int = 3) -> dict:
    source = list(checkpoint.stored_factors)
    if not source:
        return {"evaluated": 0, "accepted": 0, "quarantined": 0, "details": []}

    def evaluate_one(factor: dict):
        try:
            raw = parse(factor["expr"])
            canonical = review.simplify(raw)
            node, reason = review.apply(canonical)
            # Cumulative retirement prevents further mining of a pattern; it is
            # not evidence that an already-admitted incumbent became invalid.
            if node is None and "mined_out" in reason:
                node = canonical
            if node is None:
                raise ValueError(reason)
            canonical_hash = node.expr_hash()
            panel = evaluate(node, field_panels)
            coverage_reason = _coverage_reason(panel)
            if coverage_reason:
                raise ValueError(coverage_reason)
            metrics = evaluator.evaluate(panel, name="rv_" + canonical_hash[:12])
            metrics.expr = node.to_str()
            return factor, node, metrics, None
        except Exception as exc:  # noqa: BLE001
            return factor, None, None, f"{type(exc).__name__}: {str(exc)[:500]}"

    if workers > 1 and len(source) > 1:
        with ThreadPoolExecutor(max_workers=workers) as pool:
            results = list(pool.map(evaluate_one, source))
    else:
        results = [evaluate_one(factor) for factor in source]

    errors = [{"hash": f.get("hash"), "expr": f.get("expr"), "error": error}
              for f, _node, _metrics, error in results if error]
    if errors:
        raise RevalidationError(
            f"{len(errors)}/{len(source)} factors failed evaluation; first={errors[0]}"
        )

    record = evaluation_record(evaluator)
    accepted: list[dict] = []
    quarantined: list[dict] = []
    details: list[dict] = []
    fsa = FSA()
    perturber = Perturber()
    perturber.load_state(checkpoint.perturb_state)
    observer = Evolver(FIELDS, perturber=perturber)
    failed_hashes = set(checkpoint.failed_hashes)
    next_extra = copy.deepcopy(checkpoint.extra)

    for old_factor, node, metrics, _error in results:
        candidate = copy.deepcopy(old_factor)
        candidate["expr"] = node.to_str()
        candidate["hash"] = node.expr_hash()
        candidate["skeleton"] = skeleton(node)
        candidate["metrics"] = _metrics_summary(metrics)
        candidate["ic_series"] = metrics.ic_series
        candidate["ls_ret"] = nav_to_returns(metrics.ls_nav).tolist()
        candidate["ls_ret_kind"] = "simple_return"
        candidate["evaluation"] = dict(record)
        result_meta = getattr(metrics, "meta", {}) or {}
        candidate["evaluation"]["result_engine"] = {
            key: result_meta[key]
            for key in ("alphalab_version", "sample", "mock")
            if key in result_meta
        }
        history = candidate.setdefault("metric_history", [])
        history.append({
            "status": "superseded",
            "metrics": old_factor.get("metrics"),
            "evaluation": old_factor.get("evaluation"),
        })
        del history[:-5]

        # Every successful evaluation informs the declared local window model,
        # regardless of the subsequent admission outcome.
        observer.observe(node, metrics.ls_sharpe)

        result = apply_filters(
            metrics, fsa=fsa, node=node, stored_factors=accepted,
            expr_hash=candidate["hash"], failed_hashes=None,
        )
        if result.passed:
            if result.replace_hashes:
                keep = []
                for prior in accepted:
                    if prior.get("hash") in result.replace_hashes:
                        old_skeleton = prior.get("skeleton")
                        if old_skeleton:
                            fsa.counts[old_skeleton] -= 1
                            if fsa.counts[old_skeleton] <= 0:
                                del fsa.counts[old_skeleton]
                        prior["quarantine"] = {
                            "reason": "replaced by superior correlated factor during revalidation",
                            "replacement_hash": candidate["hash"],
                        }
                        quarantined.append(prior)
                    else:
                        keep.append(prior)
                accepted = keep
            accepted.append(candidate)
            fsa.observe_tree(node)
            failed_hashes.discard(candidate["hash"])
            details.append({"hash": candidate["hash"], "status": "accepted"})
        else:
            candidate["quarantine"] = {"reason": "machine filters failed",
                                       "filter_reasons": result.reasons}
            quarantined.append(candidate)
            failed_hashes.add(candidate["hash"])
            details.append({"hash": candidate["hash"], "status": "quarantined",
                            "reasons": result.reasons})

    prior_quarantine = next_extra.get("quarantined_factors", [])
    next_extra["quarantined_factors"] = prior_quarantine + quarantined
    report = {
        "evaluated_at": datetime.now(timezone.utc).isoformat(),
        "evaluation": record,
        "evaluated": len(source),
        "accepted": len(accepted),
        "quarantined": len(quarantined),
        "details": details,
    }
    next_extra["last_library_revalidation"] = report

    # Publish the fully rebuilt state only after every evaluation and filter
    # completed. Callers can then persist it with one checkpoint transaction.
    checkpoint.stored_factors = accepted
    checkpoint.failed_hashes = failed_hashes
    checkpoint.extra = next_extra
    checkpoint.fsa_state = fsa.state()
    checkpoint.perturb_state = perturber.state()
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkpoint", default=str(OUTPUT_DIR / "checkpoint.json"))
    parser.add_argument("--alphalab-config", default=str(DEFAULT_ALPHALAB_CONFIG))
    parser.add_argument("--workers", type=int, default=3)
    args = parser.parse_args()

    with ProcessLock(OUTPUT_DIR / ".engine.lock"):
        checkpoint = Checkpoint.load(args.checkpoint)
        panels = _real_panels()
        evaluator = AlphalabEvaluator(horizon=5, config_yaml=args.alphalab_config)
        try:
            report = revalidate_library(
                checkpoint=checkpoint, evaluator=evaluator,
                field_panels=panels, workers=args.workers,
            )
        except RevalidationError as exc:
            atomic_write_json(OUTPUT_DIR / "revalidation_failure.json", {
                "failed_at": datetime.now(timezone.utc).isoformat(),
                "error": str(exc),
            }, indent=2)
            raise
        checkpoint.save()
        atomic_write_json(OUTPUT_DIR / "revalidation_report.json", report, indent=2)
    print(f"revalidated={report['evaluated']} accepted={report['accepted']} "
          f"quarantined={report['quarantined']}")


if __name__ == "__main__":
    main()
