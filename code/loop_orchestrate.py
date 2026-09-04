# -*- coding: utf-8 -*-
"""单轮编排:生成 → 审查 → 去重 → 回测 → 十一项过滤 → 入库 → 检查点。

防 Goodhart:生成/结构审查只处理表达式。唯一允许的指标反馈是 M4 参数扰动器接收
成功回测的 Sharpe，用于预先声明的窗口局部搜索；LLM prompt 和机制族不接收指标值。

用法(阶段 6 默认 mock 模式,无 LLM、无真实回测):
    stats = run_round(checkpoint=..., evolver=..., evaluator=MockEvaluator(),
                      field_panels=..., fsa=..., fields=[...])
真实模式:evaluator 换 AlphalabEvaluator,evolver 接 llm hook。
"""
from __future__ import annotations

import json
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from datetime import datetime, timezone

import numpy as np
import pandas as pd

from backtest.interface import Evaluator
from backtest.returns import nav_to_returns
from engine import review
from engine.checkpoint import Checkpoint
from engine.config import (BACKTEST_END, BACKTEST_START, COVERAGE_LOCAL_RATIO_MIN,
                           DEAD_RESAMPLE_TRIES, SCALE_DOMINANCE)
from engine.evolve import Evolver
from engine import failed_patterns as fplib
from engine.expression import evaluate, parse
from engine.fsa import FSA, skeleton
from engine.provenance import evaluation_record, metrics_match_evaluator
from filters import FilterResult, apply_filters
from llm.mechanisms import (add_family_note, family_of, is_metric_reason,
                            register_family, review_expression)
from paths import OUTPUT_DIR

# 过滤项 → 短标签(供每轮拒绝分类汇总)
_FILTER_LABELS = {
    1: "IC弱", 2: "某年负", 3: "年化负", 4: "夏普低", 5: "末年夏普低",
    6: "Calmar低", 7: "近9月负", 8: "近12月负", 9: "相关性高", 10: "FSA",
    11: "失败库", 12: "多头超额负", 13: "单调性低", 14: "ICIR低", 15: "同构家族",
    16: "LLM终审",
}


@dataclass
class RoundStats:
    iteration: int
    n_generated: int
    n_tested: int               # 本轮新审查的唯一候选数(去重后)
    n_pass_review: int          # 过五过滤
    n_backtested: int
    n_pass_filters: int         # 入库
    stored_total: int
    final_vetoes: list = field(default_factory=list)   # 终审拒详情(用户 2026-08-25:进每轮汇报供裁决)
    n_resampled: int = 0            # 死骨架重采样替换数(失败模式库回流)
    gen_src_total: dict = field(default_factory=dict)    # 各生成源的新测候选数 {op: n}
    gen_src_pass_review: dict = field(default_factory=dict)  # 各生成源过审查数(质量观测,用户 2026-08-24)
    gen_src_dedup: dict = field(default_factory=dict)        # 各生成源被hash去重数(用户 2026-08-27)
    elapsed_sec: float = 0.0    # 本轮耗时(秒)
    new_factor_exprs: list = field(default_factory=list)
    sample_reject_reasons: list = field(default_factory=list)
    reject_records: list = field(default_factory=list)    # 每候选 {iter,hash,expr,disp,reasons}
    reject_summary: dict = field(default_factory=dict)    # 拒绝分类计数

    def __str__(self) -> str:
        return (f"轮次 {self.iteration}: 生成 {self.n_generated} → 新测 {self.n_tested} "
                f"→ 审查过 {self.n_pass_review} → 回测 {self.n_backtested} "
                f"→ 入库 {self.n_pass_filters}(累计 {self.stored_total}) "
                f"[{self.elapsed_sec/60:.1f}min]")


def _panel_std(panel: pd.DataFrame) -> float:
    """面板截面 std 估计(每 10 行抽样,支撑 ≥3x 阈值判断足够,省 ~20x 计算)。"""
    return float(np.nanstd(panel.values[::10]))


def _simplify_or_combine(node, p1: pd.DataFrame, p2: pd.DataFrame,
                         ratio: float = SCALE_DOMINANCE):
    """顶层 add/sub 的分支支配处理(用户 2026-08-18 拍定):

    某分支截面 std ≥ ratio 倍于另一分支 → 复合在数值上被支配支独占,
    直接取支配支(树少一层、信号不变,失衡复合不再拒绝);否则按算子合成面板
    (与 evaluate 等价,免二次求值)。返回 (节点, 面板)。
    """
    s1, s2 = _panel_std(p1), _panel_std(p2)
    if min(s1, s2) > 0 and max(s1, s2) >= min(s1, s2) * ratio:
        return (node.children[0], p1) if s1 > s2 else (node.children[1], p2)
    return node, (p1 + p2 if node.op == "add" else p1 - p2)


def build_field_panels(df: pd.DataFrame, fields: list[str]) -> dict[str, pd.DataFrame]:
    """长表(order_book_id,date,字段)→ {字段: 宽表(date×stock)}。"""
    return {f: df.pivot(index="date", columns="order_book_id", values=f) for f in fields}


def _coverage_reason(panel: pd.DataFrame, months_ctx: int = 12,
                     ratio_min: float = COVERAGE_LOCAL_RATIO_MIN,
                     start: str = BACKTEST_START,
                     end: str = BACKTEST_END) -> str | None:
    """逐月覆盖率本地塌陷检测(回测前,确定性防线):任一月覆盖率 < 前后各 months_ctx 个月
    中位数的 ratio_min → 拒。针对嵌套时序算子 min_periods=n 的 NaN 乘性放大
    (2019-04 roc 除零 inf 事故曾把 2 个坏日放大成 40 个交易日 0% 覆盖)。
    只看回测窗口(start 起)——warmup 期(面板起始的滚动窗预热)天然低覆盖,不算塌陷
    (全库体检曾暴露:不切窗口会把每个候选都误杀)。
    返回 ValueError 前缀原因(主循环按确定性缺陷永久去重);正常返回 None。"""
    cov = panel.loc[start:end].notna().mean(axis=1).resample("ME").mean().dropna()
    if len(cov) < 6:
        return None                      # 数据太少不判(保守放行)
    med = cov.rolling(2 * months_ctx + 1, center=True, min_periods=6).median()
    # 本底下限(2026-08-27):本地中位 <30% 时该区域数据本底太低(如 2018 年财报表
    # 覆盖仅 14%),月间正常波动即触发塌陷误伤——跳过判定,与 warmup 同哲学
    med = med.where(med >= 0.30)
    bad = cov[cov < med * ratio_min]
    if len(bad):
        dt, v = bad.idxmin(), float(bad.min())
        return (f"ValueError: 覆盖率塌陷({dt.strftime('%Y-%m')} 月均{v:.0%}"
                f"<本地中位{float(med[dt]):.0%}×{ratio_min:.0%})")
    return None


def restore_fsa(checkpoint: Checkpoint) -> FSA:
    fsa = FSA()
    fsa.load_state(checkpoint.fsa_state)
    return fsa


def _metrics_summary(m) -> dict:
    return {"ic_mean": m.ic_mean, "icir": m.icir, "ls_annual": m.ls_annual,
            "ls_sharpe": m.ls_sharpe, "calmar": m.calmar, "direction": m.direction,
            "long_excess_annual": m.long_excess_annual, "long_excess_sharpe": m.long_excess_sharpe,
            "monotonicity": m.monotonicity}


def run_round(*, checkpoint: Checkpoint, evolver: Evolver, evaluator: Evaluator,
              field_panels: dict[str, pd.DataFrame], fsa: FSA,
              fields: list[str], n_candidates: int = 100,
              parents: list | None = None, capture_ic_series: bool = True,
              n_workers: int = 4, llm_reviewer=None) -> RoundStats:
    """跑一轮。parents 默认从已入库因子解析(种子优先入库因子,见 M3)。
    n_workers>1 时回测(alphalab 子进程)并行;过滤+入库仍串行(#9 IC去重/#10 FSA 顺序敏感)。"""
    t0 = time.perf_counter()
    mismatched = [f.get("hash", "<missing>") for f in checkpoint.stored_factors
                  if not metrics_match_evaluator(f, evaluator)]
    if mismatched:
        raise RuntimeError(
            f"factor library contains {len(mismatched)} stale or incompatible metric artifacts; "
            "run code/revalidate_library.py before mining"
        )
    run_evaluation = evaluation_record(evaluator)
    failed_hashes = set(checkpoint.failed_hashes)
    new_iter = checkpoint.iteration + 1
    reject_records: list[dict] = []
    final_vetoes: list[dict] = []      # 终审拒详情(表达式+拒因+IS指标,进每轮汇报)
    # ---- 1) 生成(逻辑隔离:此处无任何回测指标)----
    if parents is None:
        parents = [parse(f["expr"]) for f in checkpoint.stored_factors if "expr" in f]
    # 引导字段选择偏向未充分挖掘的字段(避免过度集中在 log_mv/log_amount/log_volume)
    field_usage: Counter = Counter()
    for p in parents:
        for fld in p.fields():
            field_usage[fld] += 1
    evolver.set_field_usage(dict(field_usage))
    candidates = evolver.generate(parents, n_candidates)
    cand_meta = list(getattr(evolver, "last_gen_meta", []))
    cand_meta += [{}] * (len(candidates) - len(cand_meta))
    # 死骨架重采样(用户 2026-08-24,失败模式库回流):命中全灭骨架的候选丢弃补采,
    # 每槽最多 DEAD_RESAMPLE_TRIES 次,仍死则保留原候选——规则放宽后仍有突破机会,绝不死循环。
    dead_sk = fplib.dead_skeletons()
    n_resampled = 0
    if dead_sk:
        for i, cand in enumerate(candidates):
            if skeleton(cand) not in dead_sk:
                continue
            for _ in range(DEAD_RESAMPLE_TRIES):
                rep = evolver.generate(parents, 1)
                if not rep:
                    break
                candidates[i] = rep[0]
                cand_meta[i] = (list(getattr(evolver, "last_gen_meta", [])) or [{}])[0]
                n_resampled += 1
                if skeleton(rep[0]) not in dead_sk:
                    break
    # 族归属(2026-08-18 补链):LLM 候选生成时已登记;演化候选(mutation/crossover/perturb)
    # 继承父本的族(库存记录的 family 字段)——终审拒因回流由此对全部候选生效。
    parent_fam = {f.get("hash"): f.get("family") for f in checkpoint.stored_factors}
    for cand, meta in zip(candidates, cand_meta):
        if family_of(cand.expr_hash()) is not None:
            continue                              # LLM 生成已登记
        ph = meta.get("parent") or (meta.get("parents") or [None])[0]
        fam = parent_fam.get(ph) or family_of(ph or "")
        if fam:
            register_family(cand.expr_hash(), fam)

    # ---- 2) 审查四过滤 + 哈希去重(轮内去重用内存集;审查未过→标已测去重,回测异常→不标)----
    reviewed: list = []          # [(canonical_node, canonical_hash, raw_hash)]
    n_unique = 0
    seen_raw: set[str] = set()
    seen_canonical: set[str] = set()
    src_total: Counter = Counter()
    src_pass: Counter = Counter()
    src_dedup: Counter = Counter()      # 被去重拦下的候选(按源,用户 2026-08-27:验证 LLM 撞hash 率)
    for i, node in enumerate(candidates):
        raw_hash = node.expr_hash()
        if checkpoint.is_tested(raw_hash) or raw_hash in seen_raw:
            src_dedup[cand_meta[i].get("op", "?") if i < len(cand_meta) else "?"] += 1
            continue
        seen_raw.add(raw_hash)
        op = cand_meta[i].get("op", "?") if i < len(cand_meta) else "?"
        canonical = review.simplify(node)
        h = canonical.expr_hash()
        if checkpoint.is_tested(h) or h in seen_canonical:
            src_dedup[op] += 1
            continue
        seen_canonical.add(h)
        n_unique += 1
        src_total[op] += 1
        family = family_of(raw_hash)
        if family and family_of(h) is None:
            register_family(h, family)
        simplified, _reason = review.apply(canonical)
        if simplified is not None:
            reviewed.append((simplified, h, raw_hash))
            src_pass[op] += 1
        else:
            checkpoint.add_tested(h)   # 审查未过=确定性拒绝,标已测去重
            checkpoint.add_failed(h)
            reject_records.append({"iter": new_iter, "hash": h, "raw_hash": raw_hash,
                                   "expr": canonical.to_str(), "raw_expr": node.to_str(),
                                   "disp": "review_reject", "reasons": [_reason]})
            fplib.record_reject(canonical, "review_reject", [_reason], new_iter)

    # ---- 3) 回测 + 十一项过滤(此处才接触指标;不回流给生成端)----
    new_exprs: list[str] = []
    rejects: list[str] = []

    # 3a) 并行回测:瓶颈是 alphalab 子进程(~40s/个),ThreadPool 在 subprocess 等待时释放 GIL,
    #     且线程共享 field_panels(免 pickle)。每个候选 name=hash 唯一,无文件冲突。
    def _eval(nh):
        node, h, raw_hash = nh
        try:
            if not node.is_leaf() and node.op in ("add", "sub") and len(node.children) == 2:
                p1 = evaluate(node.children[0], field_panels)
                p2 = evaluate(node.children[1], field_panels)
                node2, panel = _simplify_or_combine(node, p1, p2)
                if node2 is not node:            # 分支支配 → 取支配支(可能变浅/与已测重复)
                    h2 = node2.expr_hash()
                    h = h2
                    if node2.depth() < 3:
                        return (node2, h, raw_hash, "ValueError: 分支支配简化后深度不足"
                                f"(原式 {node.to_str()[:60]})")
                    if checkpoint.is_tested(h2):
                        return (node2, h, raw_hash, "ValueError: 分支支配简化后与已测重复"
                                f"(原式 {node.to_str()[:60]})")
                    node, h = node2, h2
                    family = family_of(raw_hash)
                    if family and family_of(h) is None:
                        register_family(h, family)
            else:
                panel = evaluate(node, field_panels)
            cov = _coverage_reason(panel)
            if cov:   # 覆盖率塌陷:确定性结构缺陷,回测前拦下(ValueError 前缀 → 永久去重)
                return (node, h, raw_hash, cov)
            m = evaluator.evaluate(panel, name=h)
            m.expr = node.to_str()
            return (node, h, raw_hash, m)
        except Exception as e:  # noqa: BLE001
            msg = str(e).replace("\n", " ").strip()
            return (node, h, raw_hash, f"{type(e).__name__}: {msg[:300]}")

    if n_workers and n_workers > 1 and len(reviewed) > 1:
        with ThreadPoolExecutor(max_workers=n_workers) as ex:
            results = list(ex.map(_eval, reviewed))
    else:
        results = [_eval(nh) for nh in reviewed]

    # 3b) 顺序过滤 + 入库(#9 IC去重、#10 FSA 顺序敏感,必须串行)
    processed_hashes: set[str] = set()
    for node, h, raw_hash, m in results:
        if h in processed_hashes or checkpoint.is_tested(h):
            reject_records.append({"iter": new_iter, "hash": h, "raw_hash": raw_hash,
                                   "expr": node.to_str(), "disp": "canonical_duplicate",
                                   "reasons": ["normalized expression already processed"]})
            continue
        processed_hashes.add(h)
        if isinstance(m, str):   # 回测异常:瞬时错误(alphalab 抖动)不标已测→下轮可重试;
            # 确定性 ValueError(如全 NaN 拦截、字段缺失)→ 标已测,防止同表达式反复白评估
            if len(rejects) < 5:
                rejects.append(f"{node.to_str()}: 回测异常 {m}")
            reject_records.append({"iter": new_iter, "hash": h, "raw_hash": raw_hash,
                                   "expr": node.to_str(),
                                   "disp": "backtest_error", "reasons": [m]})
            if m.startswith("ValueError"):
                fplib.record_reject(node, "backtest_error", [m], new_iter)
                checkpoint.add_tested(h)   # 确定性坏表达式 → 永久去重
                checkpoint.add_failed(h)
            continue
        checkpoint.add_tested(h)   # 回测成功(指标可解析)才落盘去重
        if hasattr(evolver, "observe"):
            evolver.observe(node, m.ls_sharpe)
        fr = apply_filters(m, fsa=fsa, node=node, stored_factors=checkpoint.stored_factors,
                           failed_hashes=failed_hashes, expr_hash=h)
        admission = {
            "path": "standard",
            "raw_hash": raw_hash,
            "canonical_hash": h,
            "llm_review_required": llm_reviewer is not None,
            "review_status": "not_run",
            "override_applied": False,
        }
        # LLM 终审(用户 2026-08-17 接线,2026-08-18 升级):仅对过全部过滤、即将入库的候选把关;
        # 携带 IS 指标供诊断(选择端合法);拒因分两类——指标类只进台账,**结构/经济类回流该机制族
        # 生成 prompt 的「已知缺陷」栏**(防 Goodhart:指标数值永不回流生成端)
        if fr.passed and llm_reviewer is not None:
            accept, why = review_expression(llm_reviewer, node, metrics=m)
            admission.update({
                "review_status": "accepted" if accept else "rejected",
                "review_verdict": why[:300],
                "reviewed_at": datetime.now(timezone.utc).isoformat(),
            })
            # 终审审计(2026-08-24):裁决原文落盘——此前 47 次调用 0 拒且原文全丢,
            # 无法排除 GLM 偶发中文输出被默认放行;现在每次裁决可追溯
            try:
                with open(OUTPUT_DIR / "final_review_log.jsonl", "a", encoding="utf-8") as _f:
                    _f.write(json.dumps({"iter": new_iter, "expr": node.to_str(),
                                         "accept": accept, "raw": why[:300]},
                                        ensure_ascii=False) + "\n")
            except Exception:  # noqa: BLE001  审计写失败不崩轮
                pass
            if not accept:
                fam = family_of(h)
                if fam and not is_metric_reason(why):
                    add_family_note(fam, why[:120])
                # 终审拒详情收集(用户 2026-08-25:表达式+拒因+IS各项表现进汇报,供人工裁决)
                annual = getattr(m, "annual_ls_return", None) or {}
                final_vetoes.append({
                    "expr": node.to_str(), "reason": why[:200],
                    "ic": m.ic_mean, "icir": m.icir, "sharpe": m.ls_sharpe,
                    "calmar": m.calmar, "long_excess": m.long_excess_annual,
                    "monotonicity": m.monotonicity,
                    "annual": {str(y): v for y, v in sorted(annual.items())},
                })
                fr = FilterResult(passed=False, reasons=[f"16.LLM终审拒:{why[:80]}"])
        if fr.passed:
            factor_evaluation = dict(run_evaluation)
            result_meta = getattr(m, "meta", {}) or {}
            factor_evaluation["result_engine"] = {
                key: result_meta[key]
                for key in ("alphalab_version", "sample", "mock")
                if key in result_meta
            }
            new_factor = {
                "expr": node.to_str(), "hash": h, "skeleton": skeleton(node),
                "ic_series": (m.ic_series if capture_ic_series else None),
                "ls_ret": (nav_to_returns(m.ls_nav).tolist()
                           if getattr(m, "ls_nav", None) is not None else None),
                "ls_ret_kind": "simple_return",
                "metrics": _metrics_summary(m),
                "evaluation": factor_evaluation,
                "admission": admission,
                "family": family_of(h),           # 生成机制族(供演化子代继承/拒因回流)
            }
            if fr.replace_hashes:
                # 高相关但综合质量全面更优 → 移除所有相关旧因子,再入库新因子
                keep = []
                for f in checkpoint.stored_factors:
                    if f.get("hash") in fr.replace_hashes:
                        old_skel = f.get("skeleton")
                        if old_skel:
                            fsa.counts[old_skel] -= 1
                            if fsa.counts[old_skel] <= 0:
                                del fsa.counts[old_skel]
                    else:
                        keep.append(f)
                checkpoint.stored_factors = keep
                checkpoint.add_factor(new_factor)
                fsa.observe_tree(node)
                disp, disp_reasons = "replaced", [f"替换 {len(fr.replace_hashes)} 个: {', '.join(fr.replace_hashes)}"]
            else:
                checkpoint.add_factor(new_factor)
                fsa.observe_tree(node)
                disp, disp_reasons = "stored", []
            new_exprs.append(node.to_str())
            checkpoint.clear_failed(h)
            reject_records.append({"iter": new_iter, "hash": h, "raw_hash": raw_hash,
                                   "expr": node.to_str(),
                                   "disp": disp, "reasons": disp_reasons})
            fplib.record_stored(node, new_iter)   # 成功史永久(被替换出库不扣减)
            from engine import mined_patterns as mplib
            mplib.record(node, new_iter)          # 累计退休记账(整树+子树,代际分账)
            mplib.save()
        else:
            if len(rejects) < 5:
                rejects.append(f"{node.to_str()}: {fr.reasons}")
            checkpoint.add_failed(h)
            reject_records.append({"iter": new_iter, "hash": h, "raw_hash": raw_hash,
                                   "expr": node.to_str(),
                                   "disp": "filter_reject", "reasons": fr.reasons})
            fplib.record_reject(node, "filter_reject", fr.reasons, new_iter)

    # ---- 3c) 拒绝分类汇总(轻量计数,供每轮窗口/追溯)----
    reject_summary: Counter = Counter()
    for r in reject_records:
        if r["disp"] == "review_reject":
            reason = (r["reasons"] or [""])[0]
            if "min_complexity" in reason:
                reject_summary["审查:复杂度低"] += 1
            elif "degenerate" in reason:
                reject_summary["审查:同质退化"] += 1
            elif "cross_dimension" in reason:
                reject_summary["审查:跨量纲"] += 1
            elif "oversmoothed" in reason:
                reject_summary["审查:过度平滑"] += 1
            elif "extreme_nesting" in reason:
                reject_summary["审查:极值嵌套"] += 1
            elif "roc_on_cs" in reason:
                reject_summary["审查:roc语义"] += 1
            else:
                reject_summary["审查:其他"] += 1
        elif r["disp"] == "backtest_error":
            reason0 = (r["reasons"] or [""])[0]
            if "覆盖率" in reason0:
                reject_summary["审查:覆盖率"] += 1
            elif "简化后" in reason0:
                reject_summary["审查:结构简化"] += 1
            else:
                reject_summary["回测异常"] += 1
        elif r["disp"] == "filter_reject":
            for reason in r["reasons"]:
                num = reason.split(".")[0]
                try:
                    reject_summary[_FILTER_LABELS.get(int(num), f"#{num}")] += 1
                except ValueError:
                    reject_summary["其他"] += 1

    # ---- 4) 更新检查点并落盘 ----
    checkpoint.iteration += 1
    stats = RoundStats(
        iteration=checkpoint.iteration, n_generated=len(candidates), n_tested=n_unique,
        n_pass_review=len(reviewed), n_backtested=len(reviewed),
        n_pass_filters=len(new_exprs), stored_total=len(checkpoint.stored_factors),
        final_vetoes=final_vetoes,
        n_resampled=n_resampled,
        gen_src_total=dict(src_total), gen_src_pass_review=dict(src_pass),
        elapsed_sec=time.perf_counter() - t0,
        new_factor_exprs=new_exprs, sample_reject_reasons=rejects,
        reject_records=reject_records, reject_summary=dict(reject_summary),
        gen_src_dedup=dict(src_dedup),
    )
    checkpoint.history.append({
        "iteration": stats.iteration, "n_generated": stats.n_generated,
        "n_tested": stats.n_tested, "n_pass_review": stats.n_pass_review,
        "n_pass_filters": stats.n_pass_filters, "stored_total": stats.stored_total,
    })
    checkpoint.capture(fsa=fsa, perturber=getattr(evolver, "perturber", None))
    checkpoint.save()
    fplib.save(stats.iteration)     # 失败模式库轮末落盘(原子写)
    return stats
