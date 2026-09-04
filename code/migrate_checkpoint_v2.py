# -*- coding: utf-8 -*-
"""Idempotent checkpoint migration for canonical identity and metric provenance."""
from __future__ import annotations

import argparse
import json
import shutil
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

import numpy as np

from engine import review
from engine.checkpoint import Checkpoint
from engine.expression import parse
from engine.fsa import skeleton
from engine.io_utils import ProcessLock
from engine.provenance import EVALUATION_SCHEMA_VERSION, metrics_are_current
from paths import OUTPUT_DIR


class MigrationError(RuntimeError):
    """Migration cannot preserve an unambiguous factor-library identity."""


def _canonical(expr: str) -> tuple[str, str]:
    node = review.simplify(parse(expr))
    node.validate()
    return node.to_str(), node.expr_hash()


def _convert_nav_deltas(deltas) -> list[float]:
    values = np.asarray(deltas, dtype=float)
    if values.ndim != 1 or not np.isfinite(values).all():
        raise MigrationError("legacy NAV deltas are malformed")
    if values.size == 0:
        return []
    previous_nav = 1.0 + np.concatenate(([0.0], np.cumsum(values[:-1])))
    if np.any(previous_nav == 0):
        raise MigrationError("legacy NAV deltas cross a zero NAV denominator")
    return (values / previous_nav).tolist()


def _read_rejects(path: Path):
    if not path.exists():
        return
    with open(path, encoding="utf-8") as handle:
        for line_no, line in enumerate(handle, 1):
            if not line.strip():
                continue
            try:
                yield json.loads(line)
            except json.JSONDecodeError as exc:
                raise MigrationError(f"invalid rejection ledger line {line_no}") from exc


def migrate_checkpoint(cp: Checkpoint, rejection_records=()) -> dict:
    """Mutate *cp* in memory; callers commit only after the full migration succeeds."""
    stats = {
        "rehash_stored": 0,
        "canonical_tested_added": 0,
        "failed_hashes_added": 0,
        "metrics_marked_stale": 0,
        "legacy_validation_relabelled": 0,
        "pnl_series_converted": 0,
        "ledger_parse_skipped": 0,
        "fsa_skeletons_rebuilt": 0,
    }
    canonical_seen: dict[str, str] = {}
    for factor in cp.stored_factors:
        expr, canonical_hash = _canonical(factor["expr"])
        prior = canonical_seen.get(canonical_hash)
        if prior is not None:
            raise MigrationError(
                f"stored-factor canonical collision: {canonical_hash} maps both "
                f"{prior!r} and {factor['expr']!r}"
            )
        canonical_seen[canonical_hash] = expr
        old_hash = factor.get("hash")
        if old_hash != canonical_hash:
            factor.setdefault("legacy_hash", old_hash)
            stats["rehash_stored"] += 1
        factor["expr"] = expr
        factor["hash"] = canonical_hash
        factor["skeleton"] = skeleton(parse(expr))
        before = len(cp.tested_hashes)
        cp.tested_hashes.add(canonical_hash)
        stats["canonical_tested_added"] += len(cp.tested_hashes) - before

        if not metrics_are_current(factor):
            stale_record = {
                "status": "stale",
                "schema_version": EVALUATION_SCHEMA_VERSION,
                "reason": "metrics predate current operator/evaluator provenance; revalidation required",
            }
            if factor.get("evaluation") != stale_record:
                factor["evaluation"] = stale_record
                stats["metrics_marked_stale"] += 1

        if factor.get("oos_metrics") is not None:
            factor["legacy_validation_metrics"] = factor.pop("oos_metrics")
            factor["legacy_validation_provenance"] = {
                "status": "contaminated_development_validation",
                "reason": "direction was re-selected on the 2025 window",
                "eligible_for_selection": False,
            }
            stats["legacy_validation_relabelled"] += 1

        if factor.get("ls_ret") is not None and factor.get("ls_ret_kind") != "simple_return":
            factor["ls_ret"] = _convert_nav_deltas(factor["ls_ret"])
            factor["ls_ret_kind"] = "simple_return"
            factor["ls_ret_migration"] = "converted_from_nav_delta_assuming_initial_nav_1"
            stats["pnl_series_converted"] += 1

    fsa_counts = Counter(factor["skeleton"] for factor in cp.stored_factors)
    cp.fsa_state = {"counts": dict(fsa_counts)}
    stats["fsa_skeletons_rebuilt"] = len(fsa_counts)

    stored_hashes = set(canonical_seen)
    for record in rejection_records:
        expr = record.get("expr")
        if not expr:
            continue
        try:
            _expr, canonical_hash = _canonical(expr)
        except Exception:  # Historical diagnostics may contain truncated expressions.
            stats["ledger_parse_skipped"] += 1
            continue
        before = len(cp.tested_hashes)
        cp.tested_hashes.add(canonical_hash)
        stats["canonical_tested_added"] += len(cp.tested_hashes) - before

        disposition = record.get("disp")
        reasons = record.get("reasons") or []
        deterministic_error = (
            disposition == "backtest_error"
            and any(str(reason).startswith("ValueError") for reason in reasons)
        )
        if (canonical_hash not in stored_hashes
                and (disposition in {"review_reject", "filter_reject"}
                     or deterministic_error)):
            before_failed = len(cp.failed_hashes)
            cp.failed_hashes.add(canonical_hash)
            stats["failed_hashes_added"] += len(cp.failed_hashes) - before_failed

    cp.failed_hashes.difference_update(stored_hashes)
    migration = {
        "name": "checkpoint_v2_canonical_provenance",
        "at": datetime.now(timezone.utc).isoformat(),
        "stats": stats,
    }
    migrations = cp.extra.setdefault("migrations", [])
    existing = next((item for item in migrations
                     if item.get("name") == migration["name"]), None)
    if existing is None:
        migrations.append(migration)
    else:
        existing["last_verified_at"] = migration["at"]
        existing["last_verification_stats"] = stats
    return stats


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkpoint", default=str(OUTPUT_DIR / "checkpoint.json"))
    parser.add_argument("--rejects", default=str(OUTPUT_DIR / "rejects.jsonl"))
    args = parser.parse_args()

    checkpoint_path = Path(args.checkpoint)
    with ProcessLock(OUTPUT_DIR / ".engine.lock"):
        cp = Checkpoint.load(checkpoint_path)
        backup = checkpoint_path.with_name(checkpoint_path.name + ".pre-v2.bak")
        if checkpoint_path.exists() and not backup.exists():
            shutil.copy2(checkpoint_path, backup)
        stats = migrate_checkpoint(cp, _read_rejects(Path(args.rejects)))
        cp.save()
    print(json.dumps(stats, ensure_ascii=False, sort_keys=True))
    print(f"backup={backup}")


if __name__ == "__main__":
    main()
