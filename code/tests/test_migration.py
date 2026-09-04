# -*- coding: utf-8 -*-
import copy

import pytest

from engine import review
from engine.checkpoint import Checkpoint
from engine.expression import parse
from migrate_checkpoint_v2 import MigrationError, migrate_checkpoint


def test_migration_rehashes_relabels_and_converts_without_rewriting_ledger(tmp_path):
    raw_expr = "zscore(add(rank_cs(amplitude), add(rank_cs(overnight), rank_cs(ret))))"
    raw_hash = parse(raw_expr).expr_hash()
    canonical = review.simplify(parse(raw_expr))
    cp = Checkpoint(tmp_path / "cp.json")
    cp.stored_factors = [{
        "expr": raw_expr,
        "hash": raw_hash,
        "metrics": {"direction": 1, "ic_mean": 0.05},
        "oos_metrics": {"direction": -1, "ic_mean": 0.03},
        "ls_ret": [0.10, 0.11],
    }]
    rejected = "zscore(add(add(rank_cs(ret), rank_cs(overnight)), rank_cs(amplitude)))"
    ledger = [{"expr": rejected, "hash": parse(rejected).expr_hash(),
               "disp": "filter_reject", "reasons": ["1.weak"]}]
    ledger_before = copy.deepcopy(ledger)

    stats = migrate_checkpoint(cp, ledger)

    factor = cp.stored_factors[0]
    assert factor["hash"] == canonical.expr_hash()
    assert factor["hash"] == parse(factor["expr"]).expr_hash()
    assert factor["legacy_hash"] == raw_hash
    assert factor["evaluation"]["status"] == "stale"
    assert "oos_metrics" not in factor
    assert factor["legacy_validation_metrics"]["ic_mean"] == 0.03
    assert factor["legacy_validation_provenance"]["eligible_for_selection"] is False
    assert factor["ls_ret"] == pytest.approx([0.10, 0.10])
    assert factor["ls_ret_kind"] == "simple_return"
    assert canonical.expr_hash() in cp.tested_hashes
    assert cp.fsa_state["counts"] == {factor["skeleton"]: 1}
    assert ledger == ledger_before
    assert stats["rehash_stored"] == 1


def test_migration_fails_closed_on_stored_canonical_collision(tmp_path):
    first = "zscore(add(add(rank_cs(ret), rank_cs(overnight)), rank_cs(amplitude)))"
    second = "zscore(add(rank_cs(amplitude), add(rank_cs(overnight), rank_cs(ret))))"
    cp = Checkpoint(tmp_path / "cp.json")
    cp.stored_factors = [
        {"expr": first, "hash": parse(first).expr_hash()},
        {"expr": second, "hash": parse(second).expr_hash()},
    ]
    with pytest.raises(MigrationError, match="canonical collision"):
        migrate_checkpoint(cp)


def test_migration_is_semantically_idempotent(tmp_path):
    expr = "zscore(add(rank_cs(amplitude), rank_cs(ret)))"
    cp = Checkpoint(tmp_path / "cp.json")
    cp.stored_factors = [{
        "expr": expr,
        "hash": parse(expr).expr_hash(),
        "metrics": {"ic_mean": 0.05},
    }]
    ledger = [{"expr": expr, "disp": "filter_reject", "reasons": ["1.weak"]}]

    first = migrate_checkpoint(cp, ledger)
    semantic_state = {
        "stored_factors": copy.deepcopy(cp.stored_factors),
        "tested_hashes": set(cp.tested_hashes),
        "failed_hashes": set(cp.failed_hashes),
        "fsa_state": copy.deepcopy(cp.fsa_state),
    }
    second = migrate_checkpoint(cp, ledger)

    assert first["metrics_marked_stale"] == 1
    assert second["rehash_stored"] == 0
    assert second["canonical_tested_added"] == 0
    assert second["failed_hashes_added"] == 0
    assert second["metrics_marked_stale"] == 0
    assert cp.stored_factors == semantic_state["stored_factors"]
    assert cp.tested_hashes == semantic_state["tested_hashes"]
    assert cp.failed_hashes == semantic_state["failed_hashes"] == set()
    assert cp.fsa_state == semantic_state["fsa_state"]
