# -*- coding: utf-8 -*-
"""打印因子库状态摘要(读 output/checkpoint.json)。供人查看 / hook 调用。

跑法:  uv run --directory factor_loop_engine code/lib_status.py
"""
from __future__ import annotations

import json

import numpy as np

from backtest.returns import pairwise_finite_corr
from engine.provenance import metrics_are_current
from paths import OUTPUT_DIR

CKPT = OUTPUT_DIR / "checkpoint.json"


def validation_status(factors: list) -> str:
    """Report legacy development validation without presenting it as unbiased OOS."""
    legacy = sum(1 for f in factors
                 if f.get("legacy_validation_metrics") or f.get("oos_metrics"))
    stale = sum(1 for f in factors if not metrics_are_current(f))
    return (f"验证状态: 当前指标={len(factors) - stale}/{len(factors)}, "
            f"历史2025开发验证={legacy}(方向曾重选,不得视为严格OOS)")


def oos_health(factors: list) -> str:
    """Backward-compatible alias; no automated decision may use contaminated results."""
    return validation_status(factors)


def _pairwise_corr(factors: list, key: str) -> tuple[int, list, list]:
    """对在库因子按指定序列字段算两两相关 → (对数, ≥0.7 违规对, 0.5-0.7 灰区对)。

    key="ic_series"(IC 口径,预测强度共振)/ "ls_ret"(PnL 口径,持仓盈亏共振——
    共同暴露的盲区,IC 口径看不到;用户 2026-08-24 定为观察项,不做准入)。
    """
    sf = [f for f in factors if f.get(key) is not None and len(f[key]) >= 20]
    n = len(sf)
    if n < 2:
        return 0, [], []
    s = [np.asarray(f[key], dtype=float) for f in sf]
    hi, gray = [], []
    for i in range(n):
        for j in range(i + 1, n):
            m = min(len(s[i]), len(s[j]))
            if m < 20:
                continue
            c = pairwise_finite_corr(s[i][-m:], s[j][-m:], min_observations=20)
            if not np.isfinite(c):
                continue
            pair = (abs(c), c, sf[i]["expr"], sf[j]["expr"])
            if abs(c) >= 0.7:
                hi.append(pair)
            elif abs(c) >= 0.5:
                gray.append(pair)
    return n * (n - 1) // 2, hi, gray


def corr_report_lines(factors: list, show_gray: int = 5) -> list[str]:
    """双口径相关性体检行(每轮汇报 + lib_status 共用)。

    灰区(0.5-0.7)持续偏多 = 单一价量数据源的相关性天花板逼近,
    是「是否引入新数据源」讨论的触发信号(用户 2026-08-24 拍板)。
    """
    lines = []
    for key, label in (("ic_series", "IC口径"), ("ls_ret", "PnL口径")):
        npairs, hi, gray = _pairwise_corr(factors, key)
        if npairs == 0:
            lines.append(f"相关性体检({label}): 样本不足(无 {key} 存档)")
            continue
        lines.append(f"相关性体检({label}): 两两 {npairs} 对,≥0.7 共 {len(hi)} 对,"
                     f"灰区0.5-0.7 共 {len(gray)} 对")
        for _a, c, e1, e2 in sorted(hi, reverse=True):
            lines.append(f"  {c:+.3f}  [{e1[:40]}] × [{e2[:40]}]")
        for _a, c, e1, e2 in sorted(gray, reverse=True)[:show_gray]:
            lines.append(f"  {c:+.3f}(灰)  [{e1[:40]}] × [{e2[:40]}]")
    return lines


def _corr_report(factors: list) -> None:
    """两两相关体检(2026-08-17 加,#9 是入库门槛而非持有门槛,入库后相关会漂移——
    0007 与第一名 0.78 即被坏数据掩盖的存量违规)。≥0.7 打印明细供处置。"""
    for line in corr_report_lines(factors):
        print(line)


def main() -> None:
    if not CKPT.exists():
        print("因子库为空(无 checkpoint.json)。")
        return
    data = json.loads(CKPT.read_text(encoding="utf-8"))
    iteration = data.get("iteration", 0)
    tested = len(data.get("tested_hashes", []))
    factors = data.get("stored_factors", [])
    print(f"迭代={iteration}  已测={tested}  入库={len(factors)}")
    if factors:
        current = [factor for factor in factors if metrics_are_current(factor)]
        if current:
            # Only provenance-valid metrics may be ranked or used for correlation decisions.
            ranked = sorted(current,
                            key=lambda f: f.get("metrics", {}).get("ic_mean", 0),
                            reverse=True)[:5]
            print("当前有效指标 Top 5(按 IC):")
            for f in ranked:
                m = f.get("metrics", {})
                print(f"  IC={m.get('ic_mean', 0):.4f} 多空年化={m.get('ls_annual', 0):.2%} "
                      f"夏普={m.get('ls_sharpe', 0):.2f} Calmar={m.get('calmar', 0):.2f} | {f.get('expr')}")
            _corr_report(current)
        else:
            print("当前有效指标为 0；过期指标已禁止排名与相关性决策，需先全库重测。")
        legacy_rows = [(f, f.get("legacy_validation_metrics") or f.get("oos_metrics"))
                       for f in factors
                       if f.get("legacy_validation_metrics") or f.get("oos_metrics")]
        if legacy_rows:
            print("历史审计: IS→2025开发验证(方向曾重选,非严格OOS,不得用于决策):")
            for f, om in legacy_rows:
                im = f.get("metrics", {})
                print(f"  IC {im.get('ic_mean', 0):+.3f}→{om.get('ic_mean', float('nan')):+.3f}  "
                      f"夏普 {im.get('ls_sharpe', 0):.2f}→{om.get('ls_sharpe', float('nan')):.2f}  "
                      f"单调 {im.get('monotonicity', 0):.2f}→{om.get('monotonicity', float('nan')):.2f}"
                      f" | {f.get('expr', '')[:46]}")
        print(validation_status(factors))
    # 失败模式库体检(用户 2026-08-24:全灭/占位骨架计数,无库文件 → 提示回填)
    from engine import failed_patterns as fplib
    if (OUTPUT_DIR / "failed_patterns.json").exists():
        print(fplib.summary_line())
    else:
        print("失败模式库: 未建(可跑 code/rebuild_failed_patterns.py 回填历史)")


if __name__ == "__main__":
    main()
