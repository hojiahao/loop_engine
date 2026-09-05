# Loop Engine

维护者：[hojiahao](https://github.com/hojiahao)

> 美股 Loop Engineering 重构正在
> [`refactor/us-equities-loop-runtime`](https://github.com/hojiahao/loop_engine/tree/refactor/us-equities-loop-runtime)
> 分支按阶段实施。执行状态与强制退出条件见
> [`docs/IMPLEMENTATION_TODO.md`](docs/IMPLEMENTATION_TODO.md)。当前 `main` 仍是已冻结的
> A 股 legacy 基线，不代表美股版本已经完成。

目标客户端、控制平面和研究服务关系见已通过 Archify showcase 与浏览器检查的
[`Loop Engine 客户端与运行架构`](docs/diagrams/loop-engine-clients.architecture.html)；
目标目录所有权和迁移顺序见
[`ADR 0003`](docs/adr/0003-repository-layout-and-ownership.md)。

## Phase 1 重构工作区

当前分支已经建立 Rust 控制平面、TypeScript Provider/Web 和 Python researchd
三套工作区骨架。新工作区使用统一门禁，详细版本、容器来源和宿主机要求见
[`开发环境说明`](docs/development/bootstrap.md)，实际验收证据见
[`Phase 1 验证记录`](docs/verification/phase-01-reproducible-toolchain.md)。

```bash
./scripts/bootstrap.sh
just check
just test
just build
just doctor
```

开发容器的基础镜像全部通过 DaoCloud 拉取并固定 OCI 摘要。工具缓存、Python
虚拟环境和 pnpm content store 放在独立具名卷中，不会复用宿主机环境；生成的
`node_modules` 仍位于 bind-mounted 工作区并被 Git 忽略。生产数据、Provider 密钥和
holdout capability 均不进入构建上下文。

Loop Engine 是一个以表达式树、演化搜索和确定性准入规则为核心的自动化
量化因子发现研究引擎。当前代码仍是 A 股研究版本：使用 Python 计算价量与
PIT 基本面因子，通过可插拔 LLM 生成/终审候选，并调用外部 AlphaLab CLI
完成横截面因子评测。

> 当前状态：代码与 checkpoint 已完成一致性和安全迁移，但 23 个历史入库因子
> 的指标全部被标记为 `stale`。在使用当前算子实现完成全库重测前，系统会拒绝
> 新一轮挖掘、指标排名和正式导出。仓库当前不包含可用于性能声明的严格样本外结果。

## 研究边界

| 区间 | 用途 | 自动发现进程权限 |
|---|---|---|
| 2015-01-01 至 2017-12-31 | 因子预热 | 只用于因果滚动计算 |
| 2018-01-01 至 2023-06-30 | IS 研究与准入 | 可见、可选择方向 |
| 2023-07-01 至 2024-12-31 | 隔离带 | 不评测 |
| 2025-01-01 至 2025-12-31 | 已污染开发验证 | 仅保留历史审计，不得用于准入或最终结论 |

2025 年曾使用 `direction.mode: auto` / `best_icir` 重新选择方向，并参与过重准入
决策，因此不是严格 OOS。当前发现进程只加载到 IS 截止日；任何非 IS 评测若仍启用
自适应方向，`AlphalabEvaluator` 会直接拒绝执行。真正的最终测试集必须在表达式、
方向、超参数和数据口径冻结后一次性解锁。

## 已修复的关键问题

1. **样本隔离**：移除自动 2025 评测，收紧数据加载、覆盖率检查和导出边界；历史
   OOS 文件改名为受污染开发验证审计文件。
2. **代码/指标一致性**：每个因子保存算子与评测器指纹；旧指标默认失效；新增全库
   原子重测工具，导出默认拒绝 stale 指标。
3. **规范哈希**：先规范表达式再计算 SHA-1；`add`/`mul` 按交换结合律统一；迁移器
   补齐 tested/failed 哈希并对入库碰撞 fail closed。
4. **缺失窗口偏度**：二、三阶矩和无偏修正统一使用窗口实际有效样本数；常数窗口
   返回 0，观测不足返回 NaN。
5. **进程安全**：生产入口使用单写者进程锁；状态采用唯一同目录临时文件、`fsync`
   和原子替换；checkpoint 带 schema/revision 冲突检测。
6. **扰动与失败记忆**：成功回测会更新窗口-Sharpe 历史，扰动状态可恢复/持久化；
   过滤器 #11 已接入生产 `failed_hashes`。
7. **重准入一致性**：重准入重新经过规范化、覆盖率、IS 回测、机器过滤和 fail-closed
   LLM 终审；`--force` 只能显式豁免 #16，并完整记录 override 审计。
8. **PnL 相关性**：统一使用 `NAV[t] / NAV[t-1] - 1` 的简单收益率，并对旧 delta-NAV
   序列执行可审计迁移。

## 运行架构

```text
候选生成 -> 结构审查/规范化 -> IS 因子求值 -> AlphaLab 评测
         -> 过滤 #1-#15 -> LLM 终审 #16 -> 入库/替换 -> 原子 checkpoint
```

- `code/engine/`：表达式、算子、演化、扰动、FSA、状态与持久化。
- `code/backtest/`：统一评测接口、AlphaLab 适配器和收益率口径。
- `code/data_layer/`：现有 A 股 OSS/DuckDB 数据加载与 PIT 字段派生。
- `code/loop_orchestrate.py`：单轮生成、回测、过滤、审计与入库。
- `code/revalidate_library.py`：使用当前语义重测全库；任一评测失败则不提交状态。
- `code/migrate_checkpoint_v2.py`：幂等迁移旧哈希、指标 provenance 和收益序列。
- `output/factors/`：当前可导出结果及明确隔离的历史审计文件。

## Legacy A 股诊断

新重构代码统一使用前述 `just` 门禁。只有在单独诊断冻结的 A 股实现时，才使用
根目录 legacy Python 3.11+ 环境：

```bash
uv sync --locked
uv run --locked pytest
uv run --locked code/lib_status.py
uv run --locked code/run_round_cli.py --mock --force --checkpoint /tmp/loop_engine_mock.json --n 100
```

当前测试集收集 217 个测试（本环境 216 通过、1 个真实 Windows AlphaLab fixture
因外部依赖缺失而跳过）。mock 模式仅验证流水线，不产生投资研究结论。

## 真实重测与导出

真实模式仍需要原项目的私有 AlphaLab CLI、RQData/行情缓存和 OSS 数据权限。
PyPI 上的同名 `alpha-lab` 包不是该 CLI 的兼容替代品，不能据此伪造重测结果。
依赖和数据就绪后按以下顺序执行：

```bash
uv run --locked code/migrate_checkpoint_v2.py
uv run --locked code/revalidate_library.py --workers 3
uv run --locked code/lib_status.py
uv run --locked code/export_factors.py
```

`--allow-stale-metrics` 只允许诊断性导出，并会在 manifest 标记 `stale`；不得用于
研究结论。迁移前的清单和 2025 开发验证保存在 `output/factors/legacy_*.csv`。

## 已知限制与下一阶段

- 当前 universe、字段、成本和企业行动口径均为 A 股专用，尚不能用于美股研究。
- 当前 JSON checkpoint 已具备单机进程安全，但美股重构会升级为事务型元数据存储、
  内容寻址 Parquet 数据快照和不可见 holdout 权限边界。
- 新工作区已经使用 Loop Engine 品牌和维护者信息；美股数据供应商与回测内核将在
  对应阶段接入。第三方许可证及不可变审计历史必须依法保留，不会伪装成原创内容。
- 本项目用于研究基础设施，不构成投资建议；任何结果都必须经过独立复核、成本与容量
  压测以及真正未触碰样本的验证。
