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
架构图保留早期方案，其 SQLite 标注已由
[`ADR 0007`](docs/adr/0007-postgresql-primary-store.md) 的 PostgreSQL 决策取代；
当前实现和验收状态以实施清单及下文为准。

## Phase 1 重构工作区

当前分支已经建立 Rust 控制平面、TypeScript Provider/Web 和 Python researchd
三套工作区骨架。新工作区使用统一门禁，详细版本、容器来源和宿主机要求见
[`开发环境说明`](docs/development/bootstrap.md)，实际验收证据见
[`Phase 1 验证记录`](docs/verification/phase-01-reproducible-toolchain.md)。Python
工具链已按 [`ADR 0005`](docs/adr/0005-python-314-uv-workspace.md) 升级为
3.14.4、一个根 uv workspace、一个根 `uv.lock` 和一个 `.venv`；修订验收记录见
[`Python 3.14 workspace 验证`](docs/verification/phase-01-python-314-amendment.md)。

```bash
./scripts/bootstrap.sh
just check
just test
just build
just doctor
```

开发容器的基础镜像全部通过 DaoCloud 拉取并固定 OCI 摘要。`just container-gate`
使用一次性 Compose project 和全新具名卷完成容器门禁，并在退出时清理。宿主机在仓库根使用
Git 忽略的 `.venv`；容器把独立具名卷挂载到同一 `/workspace/.venv` 路径，避免复用
宿主解释器。工具缓存和 pnpm content store 使用 runtime 卷，生成的 `node_modules`
仍位于 bind-mounted 工作区并被 Git 忽略。生产数据、Provider 密钥和 holdout
capability 均不进入构建上下文。

## Phase 2 核心协议（已验收）

实现已通过提交 `0615d81` 推送；本地 `just check/test/build/doctor` 全部通过，
[GitHub Actions](https://github.com/hojiahao/loop_engine/actions/runs/34101687394)
的 7 个任务全部通过，包括 DaoCloud 干净容器验收。阶段完成不表示已经合并到 `main`，
也不表示美股引擎已完整可用。阶段状态及证据以
[`实施清单`](docs/IMPLEMENTATION_TODO.md)和
[`Phase 2 验证记录`](docs/verification/phase-02-core-contracts.md)为准。

当前重构分支已加入共享 `loop.v1` DTO，以及按角色隔离的
`loop.{protocol,discovery,provider,research,jobs,audit,holdout}.v1` gRPC 服务入口。
协议规定长耗时的发现、因子评测、回测和对账只通过提交 RPC 返回窄作业句柄，不在请求
线程内执行。Phase 3 正在实现 PostgreSQL 持久化和作业生命周期，当前进展见下节；
角色 RPC 尚未对外开放。
Discovery、Research 和 Provider 三项 role RPC 的请求/响应消息图均不能到达 holdout
输入；Discovery 与 Research 还通过独立的 `development_data.proto` 叶子依赖避免加载
锁定样本窗口和完整数据快照。这只是类型可达性隔离；opaque snapshot ID 的角色必须在
Phase 4/5 由服务端 registry 与 capability 解析后才能持久化或执行，当前 wire 校验不作
该项能力声明。RPC 验证或基础设施故障使用
非 OK gRPC status 和类型化 `ServiceError`，有效因子的确定性拒绝则是独立的
`FactorRejection` 作业结果，两者不可互换。

Model content 与 stream 在本阶段只定义类型化 wire DTO。完整的内容、JSON 和能力验证
属于 Phase 9；`ModelResolutionSnapshot` 当前也只校验 wire 形状，必须在 Phase 9 由
服务端 catalog 重算并验证 capability、catalog、plugin 与 snapshot identity 后才可用于
enqueue 或 dispatch；通用 `ArtifactRef` 也必须受 prompt-safe schema、独立 namespace
和 providerd 存储 ACL 限制。序号、请求绑定、唯一完成事件和 OK EOF 等 stream 状态机验证属于
Phase 10；在这些门禁通过前，协议协商不会广告 `streams.terminal-event.v1`，也不会把
DTO 定义误报为可执行能力。

Phase 2 已建立 Rust、TypeScript 和 Python 共用的规范因子身份。表达式先依据
固定算子注册表完成类型检查与规范化，再计算 SHA-256；冻结的 `FactorSpec` 进一步绑定
表达式 ID、算子注册表摘要、固定方向和九项研究 policy。AST 可以表示类型化参数子树，
但只有严格重解析后根类型为 `series` 的表达式才能进入 FactorSpec 或研究执行。规范详情见
[`Factor canonicalization v1`](docs/specs/factor-canonicalization-v1.md)。

协议生成将当前源码描述符写入 `schema.current.binpb`，兼容性检查则针对不可由普通生成
流程覆盖的 `schema.baseline.binpb`。协议规定作业绑定协议选择、VCS/tree、数据和回测
provenance，并定义逐个认证人的 holdout 审批记录和单次不可逆 period 状态机；holdout
批量事务和 capability 强制执行分别属于 Phase 3 后续工作和 Phase 4，当前尚未实现。审计链使用
独立规范文档计算 payload/event SHA-256，不对 Protobuf 字节做哈希。相关约束见
[`协议兼容与安全规范`](docs/specs/protocol-compatibility.md)和
[`审计事件规范化规范`](docs/specs/audit-event-canonicalization-v1.md)。

Phase 2 验收时，跨语言向量、兼容性、边界和生成确定性测试均已通过；Python 协议测试
为 236 项，旧系统回归为 216 passed / 1 skipped。这是已封存阶段的历史测试基线。

## Phase 3 持久状态（实施中）

PostgreSQL 与区间注册检查点 `d85ae71` 已推送，
[CI 7 项全部通过](https://github.com/hojiahao/loop_engine/actions/runs/34200778090)，
包括 142 项 Rust 测试、独立进程竞争、强杀恢复及 DaoCloud 干净容器门禁。
这是中间检查点，不是 Phase 3 或整个美股引擎完成。

依据维护者确认的 [`ADR 0007`](docs/adr/0007-postgresql-primary-store.md)，主存储已改为
PostgreSQL，运行时不再提供 SQLite 后端。`crates/loopd/src/store` 和 `migrations/postgres`
实现 TLS 连接、迁移校验和、revision CAS、租约、取消、完成和过期恢复。作业变更、幂等回执和
规范审计事件在同一个数据库事务提交；锁定 ledger 行保证独立进程之间的审计顺序，锁等待和
语句执行均有超时。该初版刻意串行化同一 ledger 的写事务，不宣称无限水平写扩展能力。
租约过期不会自动重跑外部操作，恢复结果明确区分基础设施失败与预算耗尽。

并发验收使用 2、4、8 个独立 OS 进程；故障验收覆盖提交前、提交后以及租约期间强制
终止。迁移锁等待有超时并支持取消。未执行便被取消或耗尽预算的作业保留 `attempt = 0`，
三语言共享向量覆盖此语义，并要求协商 `jobs.prelease-terminal.v1`。

角色提交模块将 Discovery、Factor Evaluation、Backtest 和 Reconciliation 的窄请求
转换为内部作业，原子写入作业、回执与审计。作业 ID 和提交时间由服务端在事务内分配；
重试保留首次接受的 ID、时间和协议快照，不触发重复执行。测试覆盖字段映射、未知引用、
协议不可用、身份不匹配、事务回滚，以及独立进程竞争与强杀后的重放。

锁定区间注册使用独立的默认拒绝策略；规范区间、不可变回执和区间审计同事务提交。
重试返回首次注册结果，不能用新的幂等键重置区间。该注册接口不解锁数据，也不签发 capability。

人工审批存储检查点已加入：单条记录绑定独立认证的人类主体、区间、冻结清单、完整评估计划、
证据和有效期，与审计、幂等回执同事务提交；重试不延长有效期。实现边界见
[`ADR 0008`](docs/adr/0008-immutable-human-approvals.md)。生产引用解析器仍默认拒绝，
这不是可使用的留出集解锁入口；grant 和整批作业的原子消费仍待实现。
本地 19 项审批测试、进程竞争和强杀恢复测试已通过；新检查点远程 CI 尚待核验，
迁移 `0003` 尚未部署到生产库，不将此检查点表述为 Phase 3 完成。

数据库框架是 SQLx，HTTP 框架是 Axum。`migrations/postgres/*.sql` 是 SQLx 管理的版本化
数据库变更，不是另一套数据库实现。已部署迁移不可修改，新增审批表使用新版本 `0003`；
应用运行账号不执行 DDL。Rust 负责权限和事务，SQL 负责数据库约束，两者共同维护状态完整性。

工程仍遵循 [`ADR 0003`](docs/adr/0003-repository-layout-and-ownership.md) 的目录所有权：
`crates/` 为 Rust 控制平面和客户端，`apps/` 为 TypeScript Provider 与 React Web，
`python/` 为 Python 协议与研究，`proto/` 为跨服务协议，`migrations/postgres/` 为数据库迁移。
目录骨架不等于业务已实现；旧 `code/`、`output/` 等路径按计划保留到 Phase 13 校验归档，
不能提前删除回归基线。已确认的技术调整为 PostgreSQL 主存储和 Python 3.14.4。

生产数据库名为 `loop_engine`，应用账号为 `loop_engine_app`；无登录权限的
`loop_engine_owner` 持有 schema。运行时读取权限受限的连接文件，强制 `sslmode=require`
或更强模式，仅核验 schema，不自动执行 DDL。远程开发通过 SSH 隧道访问，不新增公网端口。
`require` 加密流量但不验证自签名证书身份；具备受信 CA 后应升级为 `verify-full`。

```bash
./scripts/cargo.sh run -p loopd --locked --offline -- --check-database
./scripts/cargo.sh run -p loopd --locked --offline -- \
  --database-url-file var/secrets/loopd-database-url
```

连接配置、管理员迁移、权限和本机测试库操作见
[`PostgreSQL 部署说明`](docs/development/postgresql.md)。`/readyz` 检查存储状态。
默认准入和变更策略拒绝所有作业；生产 mutating RPC 尚未注册，不能把内部存储接口当作
已完成的美股研究服务。生产身份认证与 registry 解析仍是后续门禁；holdout grant 签发与批量原子
消费以及阶段最终 CI 仍待完成。
进展及验收边界见 [`Phase 3 验证记录`](docs/verification/phase-03-durable-state.md)。

新增 `loopd` 手写代码禁止 `unsafe`，存储公开接口强制文档；Rust 格式和 Clippy、跨语言
协议测试、旧数值回归仍为强制门禁。项目执行可公开核验的工程规则，不宣称符合任何公司
未公开的内部规范。持续要求见 [`贡献规则`](AGENTS.md)。
命名和测试拆分要求见 [`Rust 开发规范`](docs/development/rust-style.md)。

## 旧研究引擎基线

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
根目录统一 Python 3.14.4 uv workspace：

```bash
./scripts/uv.sh sync --all-packages --all-groups --locked
./scripts/uv.sh run --all-packages --all-groups --locked pytest
./scripts/uv.sh run --all-packages --all-groups --locked code/lib_status.py
./scripts/uv.sh run --all-packages --all-groups --locked code/run_round_cli.py --mock --force --checkpoint /tmp/loop_engine_mock.json --n 100
```

当前测试集收集 217 个测试（本环境 216 通过、1 个真实 Windows AlphaLab fixture
因外部依赖缺失而跳过）。mock 模式仅验证流水线，不产生投资研究结论。

## 真实重测与导出

真实模式仍需要原项目的私有 AlphaLab CLI、RQData/行情缓存和 OSS 数据权限。
PyPI 上的同名 `alpha-lab` 包不是该 CLI 的兼容替代品，不能据此伪造重测结果。
依赖和数据就绪后按以下顺序执行：

```bash
./scripts/uv.sh run --all-packages --all-groups --locked code/migrate_checkpoint_v2.py
./scripts/uv.sh run --all-packages --all-groups --locked code/revalidate_library.py --workers 3
./scripts/uv.sh run --all-packages --all-groups --locked code/lib_status.py
./scripts/uv.sh run --all-packages --all-groups --locked code/export_factors.py
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
