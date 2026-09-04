# -*- coding: utf-8 -*-
"""backtest 层测试:parse_dir 用真实 alphalab 产出校验、mock 契约、write_panel 格式。"""
import pyarrow.parquet as pq
import pandas as pd
import pytest

from backtest.alphalab_adapter import AlphalabEvaluator, UnsafeValidationConfiguration
from backtest.mock import MockEvaluator

REAL_OUT = r"C:\Users\Administrator\Desktop\因子检测操作步骤\output_factor_gru_ir"


# ---------------- parse_dir:用真实产出校验(horizon=5 已知值)----------------

@pytest.mark.skipif(not pd.io.common.os.path.exists(REAL_OUT),
                    reason="无真实 alphalab 产出目录,跳过")
def test_parse_dir_real_output():
    m = AlphalabEvaluator.parse_dir(REAL_OUT, horizon=5, name="gru_factor_ir")
    # IC 类(overview_ic h5)
    assert m.ic_mean == pytest.approx(0.08146613, rel=1e-4)
    assert m.icir == pytest.approx(1.0261509, rel=1e-4)
    assert m.icir_annual == pytest.approx(7.284949, rel=1e-4)
    assert m.t_stat_nw == pytest.approx(24.75934, rel=1e-4)
    assert m.positive_ratio == pytest.approx(0.8558187, rel=1e-4)
    # 多空(overview_group h5)
    assert m.ls_annual == pytest.approx(0.5247042, rel=1e-4)
    assert m.ls_sharpe == pytest.approx(4.1494434, rel=1e-4)
    assert m.ls_max_dd == pytest.approx(-0.1018780, rel=1e-3)
    assert m.calmar == pytest.approx(0.5247042 / 0.1018780, rel=1e-3)
    # 多头超额
    assert m.long_excess_annual == pytest.approx(0.07623139, rel=1e-4)
    assert m.long_excess_sharpe == pytest.approx(1.5550050, rel=1e-4)
    assert m.monotonicity == pytest.approx(0.9636363, rel=1e-4)
    # 方向 / horizon
    assert m.direction == 1 and m.horizon == 5
    # 按年
    assert set(m.annual_ls_return) == {2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025}
    assert m.annual_ls_return[2018] == pytest.approx(0.5696207, rel=1e-4)
    assert m.annual_ic[2018] == pytest.approx(0.07441977, rel=1e-4)
    # 序列
    assert len(m.ic_series) == 1942
    assert len(m.long_excess_nav) == 1942
    assert m.long_excess_nav[0] == pytest.approx(1.0)
    assert len(m.ls_nav) == 1942 and m.ls_nav[0] == pytest.approx(1.0)


# ---------------- Mock ----------------

def test_mock_evaluator_populates_all():
    panel = pd.DataFrame({"A": [1.0, 2, 3], "B": [4.0, 5, 6]},
                         index=pd.date_range("2018-01-02", periods=3))
    m = MockEvaluator(seed=0).evaluate(panel, name="t")
    assert m.horizon == 5
    assert set(m.annual_ls_return) == set(range(2018, 2026))
    assert len(m.ic_series) == 100          # mock 对短面板有 n≥100 下限
    assert m.meta.get("mock") is True
    # summary 能打印
    assert "IC=" in m.summary()


def test_evaluate_expr_sets_expr():
    from engine.expression import parse
    fields = {"close": pd.DataFrame({"A": [1.0, 2, 3, 4]}, index=pd.date_range("2018-01-02", periods=4))}
    node = parse("zscore(ma(close, 2))")
    m = MockEvaluator(seed=1).evaluate_expr(node, fields)
    assert m.expr == "zscore(ma(close, 2))"


# ---------------- write_panel 格式 ----------------

def test_write_panel_format(tmp_path):
    ev = AlphalabEvaluator(alphalab_dir=tmp_path, in_root=tmp_path / "in", out_root=tmp_path / "out")
    panel = pd.DataFrame(
        {"000001.XSHE": [0.1, 0.2, 0.3], "600519.XSHG": [0.5, 0.4, 0.3]},
        index=pd.date_range("2018-01-02", periods=3),
    )
    path = ev.write_panel(panel, "f")
    names = pq.ParquetFile(path).schema.names
    assert "date" in names                      # date 作为一列
    assert "000001.XSHE" in names and "600519.XSHG" in names
    # 读回:index 为 date,值为 float
    df = pd.read_parquet(path)
    assert df.index.name == "date"


def test_holdout_rejects_adaptive_direction(tmp_path):
    config = tmp_path / "adaptive.yaml"
    config.write_text(
        "sample:\n  start: 2018-01-01\n  end: 2025-12-31\n"
        "direction:\n  mode: auto\n  ref: best_icir\n",
        encoding="utf-8",
    )
    evaluator = AlphalabEvaluator(
        alphalab_dir=tmp_path, config_yaml=config,
        window=("2025-01-01", "2025-12-31"),
    )
    with pytest.raises(UnsafeValidationConfiguration, match="freeze factor direction"):
        evaluator._config_for_window()


def test_holdout_direction_validation_is_structural(tmp_path, monkeypatch):
    import backtest.alphalab_adapter as adapter

    monkeypatch.setattr(adapter, "CACHE_DIR", tmp_path / "cache")
    config = tmp_path / "fixed.yaml"
    config.write_text(
        "sample:\n  start: 2018-01-01\n  end: 2025-12-31\n"
        "notes:\n  mode: auto\n  ref: best_icir\n"
        "direction:\n  mode: fixed\n  value: -1\n",
        encoding="utf-8",
    )
    evaluator = AlphalabEvaluator(
        alphalab_dir=tmp_path, config_yaml=config,
        window=("2026-01-01", "2026-12-31"),
    )

    derived = evaluator._config_for_window()
    loaded = adapter._load_config(derived.read_text(encoding="utf-8"))

    assert loaded["sample"] == {"start": "2026-01-01", "end": "2026-12-31"}
    assert loaded["direction"] == {"mode": "fixed", "value": -1}


def test_derived_config_cache_key_includes_source_digest(tmp_path, monkeypatch):
    import backtest.alphalab_adapter as adapter

    monkeypatch.setattr(adapter, "CACHE_DIR", tmp_path / "cache")
    paths = []
    for cost in (10, 20):
        config = tmp_path / f"config-{cost}.yaml"
        config.write_text(
            f"sample:\n  start: 2000-01-01\n  end: 2001-01-01\ncost: {cost}\n"
            "direction:\n  mode: fixed\n  value: 1\n",
            encoding="utf-8",
        )
        paths.append(AlphalabEvaluator(config_yaml=config)._config_for_window())
    assert paths[0] != paths[1]
