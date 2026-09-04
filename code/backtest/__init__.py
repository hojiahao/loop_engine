# -*- coding: utf-8 -*-
"""回测接口层:把表达式 / 因子面板 → 回测指标。

文件:
  - interface.py          FactorMetrics 契约 + 抽象 Evaluator
  - alphalab_adapter.py   真实适配器(调用户的 alphalab check)
  - mock.py               离线 Mock evaluator

回测引擎口径以配置的 alphalab 为准。自动发现只能使用 2018-01-01 至
2023-06-30 的 IS 窗口；2025 历史结果已污染，不得用于准入或最终性能声明。
"""
