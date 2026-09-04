# -*- coding: utf-8 -*-
import copy

import numpy as np
import pandas as pd
import pytest

from backtest.interface import Evaluator, FactorMetrics
from engine.checkpoint import Checkpoint
from engine.expression import parse
from engine.provenance import metrics_are_current
from revalidate_library import RevalidationError, revalidate_library


class PassingEvaluator(Evaluator):
    def evaluate(self, panel, name="factor"):
        returns = np.tile([0.001, 0.002], 350)
        nav = np.cumprod(1.0 + returns)
        rng = np.random.default_rng(7)
        return FactorMetrics(
            direction=1, ic_mean=0.06, icir=0.8, icir_annual=5.0,
            t_stat_nw=10.0, positive_ratio=0.8,
            ls_annual=0.4, ls_sharpe=2.0, ls_max_dd=-0.1, calmar=4.0,
            long_excess_annual=0.1, long_excess_sharpe=1.2, monotonicity=0.95,
            annual_ls_return={year: 0.2 for year in range(2018, 2024)},
            annual_ic={year: 0.05 for year in range(2018, 2024)},
            ic_series=rng.normal(0.06, 0.08, 700).tolist(),
            long_excess_nav=nav.tolist(), ls_nav=nav.tolist(),
        )


class FailingEvaluator(Evaluator):
    def evaluate(self, panel, name="factor"):
        raise RuntimeError("backend unavailable")


def _panels():
    rng = np.random.default_rng(0)
    index = pd.date_range("2017-01-02", periods=900, freq="B")
    columns = ["A", "B", "C"]
    return {
        field: pd.DataFrame(rng.normal(size=(900, 3)), index=index, columns=columns)
        for field in ("ret", "overnight")
    }


def _checkpoint(tmp_path):
    expr = "zscore(add(ma(ret, 20), ma(overnight, 40)))"
    cp = Checkpoint(tmp_path / "cp.json")
    cp.stored_factors = [{"expr": expr, "hash": parse(expr).expr_hash(),
                          "metrics": {"ic_mean": 999}, "evaluation": {"status": "stale"}}]
    return cp


def test_revalidation_replaces_metrics_and_rebuilds_state_atomically(tmp_path):
    cp = _checkpoint(tmp_path)
    report = revalidate_library(
        checkpoint=cp, evaluator=PassingEvaluator(), field_panels=_panels(), workers=1
    )
    assert report["evaluated"] == 1
    assert report["accepted"] == 1
    factor = cp.stored_factors[0]
    assert factor["metrics"]["ic_mean"] == 0.06
    assert factor["metric_history"][0]["metrics"]["ic_mean"] == 999
    assert factor["ls_ret_kind"] == "simple_return"
    assert metrics_are_current(factor)
    assert cp.fsa_state["counts"]
    assert cp.perturb_state["history"]


def test_revalidation_failure_does_not_mutate_checkpoint(tmp_path):
    cp = _checkpoint(tmp_path)
    before = copy.deepcopy(cp.stored_factors)
    with pytest.raises(RevalidationError, match="failed evaluation"):
        revalidate_library(
            checkpoint=cp, evaluator=FailingEvaluator(), field_panels=_panels(), workers=1
        )
    assert cp.stored_factors == before
