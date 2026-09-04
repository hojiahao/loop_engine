# -*- coding: utf-8 -*-
"""Readmit explicitly selected candidates through the production admission path."""
from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone

sys.path.insert(0, "code")

from backtest.alphalab_adapter import AlphalabEvaluator
from backtest.returns import nav_to_returns
from engine import failed_patterns as fplib
from engine import mined_patterns as mplib
from engine import review
from engine.checkpoint import Checkpoint
from engine.evolve import Evolver
from engine.expression import evaluate, parse
from engine.fsa import skeleton
from engine.io_utils import ProcessLock
from engine.perturb import Perturber
from engine.provenance import evaluation_record, metrics_match_evaluator
from filters import apply_filters
from llm.mechanisms import family_of, register_family, review_expression
from llm.settings import generation_provider  # noqa: F401  (加载 .env)
from llm.settings import review_provider
from loop_orchestrate import _coverage_reason, _metrics_summary, restore_fsa
from paths import OUTPUT_DIR, PROJECT_ROOT
from run_round_cli import FIELDS, _real_panels

CKPT = OUTPUT_DIR / "checkpoint.json"
ALPHACFG = str(PROJECT_ROOT / "config" / "alphalab.yaml")


def _append_jsonl(path, records: list[dict]) -> None:
    if not records:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "a", encoding="utf-8") as handle:
        for record in records:
            handle.write(json.dumps(record, ensure_ascii=False) + "\n")
        handle.flush()


def _remove_replacements(checkpoint: Checkpoint, fsa, hashes: list[str]) -> int:
    remove = set(hashes)
    keep = []
    for factor in checkpoint.stored_factors:
        if factor.get("hash") not in remove:
            keep.append(factor)
            continue
        old_skeleton = factor.get("skeleton")
        if old_skeleton:
            fsa.counts[old_skeleton] = fsa.counts.get(old_skeleton, 1) - 1
            if fsa.counts[old_skeleton] <= 0:
                del fsa.counts[old_skeleton]
    replaced = len(checkpoint.stored_factors) - len(keep)
    checkpoint.stored_factors = keep
    return replaced


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("expressions", nargs="+")
    parser.add_argument("--force", action="store_true",
                        help="仅显式豁免 LLM 终审 #16;机器过滤不可豁免")
    parser.add_argument("--checkpoint", default=str(CKPT))
    parser.add_argument("--alphalab-config", default=ALPHACFG)
    args = parser.parse_args()

    with ProcessLock(OUTPUT_DIR / ".engine.lock"):
        panels = _real_panels()
        evaluator = AlphalabEvaluator(horizon=5, config_yaml=args.alphalab_config)
        reviewer = review_provider()
        checkpoint = Checkpoint.load(args.checkpoint)
        stale = [f.get("hash") for f in checkpoint.stored_factors
                 if not metrics_match_evaluator(f, evaluator)]
        if stale:
            raise RuntimeError(
                f"cannot readmit against {len(stale)} stale library metrics; "
                "run code/revalidate_library.py first"
            )
        fsa = restore_fsa(checkpoint)
        perturber = Perturber()
        perturber.load_state(checkpoint.perturb_state)
        observer = Evolver(FIELDS, perturber=perturber)
        eval_record = evaluation_record(evaluator)
        rejection_events: list[dict] = []
        review_events: list[dict] = []

        for raw_expr in args.expressions:
            raw_node = parse(raw_expr)
            raw_node.validate()
            raw_hash = raw_node.expr_hash()
            node, structural_reason = review.apply(raw_node)
            canonical_node = review.simplify(raw_node)
            canonical_hash = canonical_node.expr_hash()
            family = family_of(raw_hash)
            if family and family_of(canonical_hash) is None:
                register_family(canonical_hash, family)

            identity = {"hash": canonical_hash, "raw_hash": raw_hash,
                        "expr": canonical_node.to_str(), "raw_expr": raw_expr}
            if node is None:
                checkpoint.add_tested(canonical_hash)
                checkpoint.add_failed(canonical_hash)
                event = {"iter": checkpoint.iteration, **identity,
                         "disp": "review_reject", "readmit": True,
                         "reasons": [structural_reason]}
                rejection_events.append(event)
                fplib.record_reject(canonical_node, "review_reject",
                                    [structural_reason], checkpoint.iteration)
                print(f"[NG] {canonical_node.to_str()}: {structural_reason}")
                continue

            if any(f.get("hash") == canonical_hash for f in checkpoint.stored_factors):
                print(f"[SKIP] already stored: {canonical_node.to_str()}")
                continue

            panel = evaluate(node, panels)
            coverage_reason = _coverage_reason(panel)
            if coverage_reason:
                checkpoint.add_tested(canonical_hash)
                checkpoint.add_failed(canonical_hash)
                rejection_events.append({"iter": checkpoint.iteration, **identity,
                                         "disp": "backtest_error", "readmit": True,
                                         "reasons": [coverage_reason]})
                fplib.record_reject(node, "backtest_error", [coverage_reason],
                                    checkpoint.iteration)
                print(f"[NG] {node.to_str()}: {coverage_reason}")
                continue

            metrics = evaluator.evaluate(panel, name="ra_" + canonical_hash[:10])
            metrics.expr = node.to_str()
            checkpoint.add_tested(canonical_hash)
            observer.observe(node, metrics.ls_sharpe)
            result = apply_filters(
                metrics, fsa=fsa, node=node,
                stored_factors=checkpoint.stored_factors,
                failed_hashes=checkpoint.failed_hashes - {canonical_hash},
                expr_hash=canonical_hash,
            )
            if not result.passed:
                checkpoint.add_failed(canonical_hash)
                rejection_events.append({"iter": checkpoint.iteration, **identity,
                                         "disp": "filter_reject", "readmit": True,
                                         "reasons": result.reasons})
                fplib.record_reject(node, "filter_reject", result.reasons,
                                    checkpoint.iteration)
                print(f"[NG] {node.to_str()}: filters {result.reasons[:3]}")
                continue

            accepted, verdict = review_expression(reviewer, node, metrics=metrics)
            override_applied = bool(args.force and not accepted)
            review_events.append({
                "iter": checkpoint.iteration, **identity, "accept": accepted,
                "raw": verdict[:300], "readmit": True,
                "force_requested": bool(args.force),
                "override_applied": override_applied,
                "reviewed_at": datetime.now(timezone.utc).isoformat(),
            })
            if not accepted and not override_applied:
                checkpoint.add_failed(canonical_hash)
                reasons = [f"16.LLM终审拒:{verdict[:160]}"]
                rejection_events.append({"iter": checkpoint.iteration, **identity,
                                         "disp": "filter_reject", "readmit": True,
                                         "reasons": reasons,
                                         "force_requested": bool(args.force),
                                         "override_applied": False})
                fplib.record_reject(node, "filter_reject", reasons, checkpoint.iteration)
                print(f"[NG] {node.to_str()}: {reasons[0]}")
                continue

            replaced = _remove_replacements(checkpoint, fsa, result.replace_hashes)
            factor_evaluation = dict(eval_record)
            result_meta = getattr(metrics, "meta", {}) or {}
            factor_evaluation["result_engine"] = {
                key: result_meta[key]
                for key in ("alphalab_version", "sample", "mock")
                if key in result_meta
            }
            checkpoint.add_factor({
                "expr": node.to_str(), "hash": canonical_hash,
                "skeleton": skeleton(node), "ic_series": metrics.ic_series,
                "ls_ret": nav_to_returns(metrics.ls_nav).tolist(),
                "ls_ret_kind": "simple_return", "metrics": _metrics_summary(metrics),
                "evaluation": factor_evaluation, "family": family_of(canonical_hash),
                "admission": {
                    "path": "readmit", "force_requested": bool(args.force),
                    "override_applied": override_applied, "review_accept": accepted,
                    "review_verdict": verdict[:300],
                },
            })
            checkpoint.clear_failed(canonical_hash)
            fsa.observe_tree(node)
            fplib.record_stored(node, checkpoint.iteration)
            mplib.record(node, checkpoint.iteration)
            reasons = [f"readmit: replaced={replaced}, override_applied={override_applied}"]
            rejection_events.append({"iter": checkpoint.iteration, **identity,
                                     "disp": "replaced" if replaced else "stored",
                                     "readmit": True, "reasons": reasons,
                                     "force_requested": bool(args.force),
                                     "override_applied": override_applied})
            print(f"[STORED] {node.to_str()} replaced={replaced} override={override_applied}")

        checkpoint.capture(fsa=fsa, perturber=perturber)
        checkpoint.extra.setdefault("readmission_audit", []).extend(review_events)
        checkpoint.save()
        mplib.save()
        fplib.save(checkpoint.iteration)
        _append_jsonl(OUTPUT_DIR / "final_review_log.jsonl", review_events)
        _append_jsonl(OUTPUT_DIR / "rejects.jsonl", rejection_events)
        print(f"stored={len(checkpoint.stored_factors)} iteration={checkpoint.iteration}")


if __name__ == "__main__":
    main()
