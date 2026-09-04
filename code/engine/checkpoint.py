# -*- coding: utf-8 -*-
"""M9 检查点:跨会话连续性 / 断点续跑。耐久原子写入 + 修订冲突检测。

状态内容(对应 docs/项目执行指南.md M9):
  - iteration        迭代计数
  - tested_hashes    已测试因子哈希集合(去重)
  - failed_hashes    确定性失败因子哈希集合(过滤 #11)
  - stored_factors   入库因子(完整信息:表达式、IC 序列快照、多维评分、骨架……,JSON 友好 dict)
  - perturb_state    参数扰动器状态(M4 动量/二阶矩/历史)
  - fsa_state        FSA 骨架计数(M8)
  - extra            其他任意 JSON 友好字段

stored_factors / ic_series 等必须为 JSON 可序列化(调用方负责把 numpy 标量转 float)。
"""
from __future__ import annotations

import json
from pathlib import Path

from engine.io_utils import atomic_write_json
from paths import OUTPUT_DIR

DEFAULT_PATH = OUTPUT_DIR / "checkpoint.json"
CHECKPOINT_SCHEMA_VERSION = 2


class CheckpointConflictError(RuntimeError):
    """The checkpoint changed after it was loaded."""


class Checkpoint:
    """检查点:内存状态 + 原子落盘/恢复。"""

    def __init__(self, path: str | Path = DEFAULT_PATH):
        self.path = Path(path)
        self.iteration: int = 0
        self.tested_hashes: set[str] = set()
        self.failed_hashes: set[str] = set()
        self.stored_factors: list[dict] = []
        self.perturb_state: dict = {}
        self.fsa_state: dict = {}
        self.extra: dict = {}
        self.history: list[dict] = []   # 近 N 轮 RoundStats(供动态预算/自适应判断)
        self.state_revision: int = 0
        self._loaded_revision: int = 0

    # ---- 已测去重 ----
    def add_tested(self, h: str) -> None:
        self.tested_hashes.add(h)

    def is_tested(self, h: str) -> bool:
        return h in self.tested_hashes

    def add_failed(self, h: str) -> None:
        self.failed_hashes.add(h)

    def clear_failed(self, h: str) -> None:
        self.failed_hashes.discard(h)

    # ---- 入库因子 ----
    def add_factor(self, factor: dict) -> None:
        self.stored_factors.append(factor)

    # ---- 子模块状态 ----
    def capture(self, *, perturber=None, fsa=None) -> None:
        """从 Perturber / FSA 对象抓取状态(它们提供 .state())。"""
        if perturber is not None:
            self.perturb_state = perturber.state()
        if fsa is not None:
            self.fsa_state = fsa.state()

    # ---- 落盘(原子) ----
    def save(self) -> None:
        disk_revision = 0
        if self.path.exists():
            with open(self.path, encoding="utf-8") as handle:
                disk_revision = int(json.load(handle).get("state_revision", 0))
        if disk_revision != self._loaded_revision:
            raise CheckpointConflictError(
                f"checkpoint changed on disk: loaded revision {self._loaded_revision}, "
                f"current revision {disk_revision}"
            )
        next_revision = disk_revision + 1
        data = {
            "schema_version": CHECKPOINT_SCHEMA_VERSION,
            "state_revision": next_revision,
            "iteration": self.iteration,
            "tested_hashes": sorted(self.tested_hashes),
            "failed_hashes": sorted(self.failed_hashes),
            "stored_factors": self.stored_factors,
            "perturb_state": self.perturb_state,
            "fsa_state": self.fsa_state,
            "extra": self.extra,
            "history": self.history[-20:],   # 只留近 20 轮,避免膨胀
        }
        atomic_write_json(self.path, data, indent=2)
        self.state_revision = next_revision
        self._loaded_revision = next_revision

    # ---- 恢复 ----
    @classmethod
    def load(cls, path: str | Path = DEFAULT_PATH) -> "Checkpoint":
        cp = cls(path)
        if not cp.path.exists():
            return cp  # 无检查点 → 空状态(全新开始)
        with open(cp.path, encoding="utf-8") as f:
            data = json.load(f)
        cp.iteration = data.get("iteration", 0)
        cp.tested_hashes = set(data.get("tested_hashes", []))
        cp.failed_hashes = set(data.get("failed_hashes", []))
        cp.stored_factors = data.get("stored_factors", [])
        cp.perturb_state = data.get("perturb_state", {})
        cp.fsa_state = data.get("fsa_state", {})
        cp.extra = data.get("extra", {})
        cp.history = data.get("history", [])
        cp.state_revision = int(data.get("state_revision", 0))
        cp._loaded_revision = cp.state_revision
        return cp
