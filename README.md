# Loop Engine

面向美股横截面因子研究的可审计研究平台。

Loop Engine 将行情与基本面数据、因子表达式、组合回测、统计评测、独立复核和模型调用
连接到可追溯的执行流程。研究结果绑定实际代码、环境、配置和数据快照；基础设施故障、
数据缺失及证据过期会阻止继续执行，不会被当作有效的因子结论。

维护者：[hojiahao](https://github.com/hojiahao) · hojiahao@outlook.com

## 能力与适用范围

- **数据与因子**：受限数据获取、不可变 Parquet 快照、PIT 查询、交易日历、规范 AST、
  因果滚动算子、横截面变换、覆盖率、试验记录和失败记忆。
- **组合与统计**：下一可交易时点执行、现金/持仓/NAV 账本、公司行动、借券与融资、
  容量和交易成本；IC、Rank IC、Newey–West、BY-FDR、DSR 与 CSCV/PBO。
- **独立复核**：Alphalens Reloaded 重算因子统计，Zipline Reloaded 独立回放组合账本，
  保存逐项差异，并接入授权结果登记与准入前置检查。
- **模型平台**：原生协议、云部署、厂商插件、自托管与网关；工具消息、结构化输出、
  流式响应、模型能力目录、调用回执、限流和费用预算。
- **执行完整性**：PostgreSQL 事务、作业租约、幂等回执、mTLS 身份、权限隔离、
  不可变审计和有界进程执行。大数据以校验和引用传递，不放入 RPC。

当前可用入口是研究 CLI、授权 gRPC 服务和 Provider 服务。完整自动 Agent 循环、
运行级调度及操作型 React Web/Ratatui TUI 尚未交付；现有客户端骨架不代表完整产品界面。
本项目不提供实盘下单，也没有可用于收益承诺的正式美股因子结果。

## 架构与目录

| 组件 | 职责 | 技术 |
| --- | --- | --- |
| `crates/loopd` | 作业、权限、事务、研究执行与结果登记 | Rust、Tokio、Axum、SQLx |
| `apps/providerd` | 模型协议、认证、能力与调用状态 | TypeScript、Node.js |
| `python/loop_research` | 数据、因子、组合账本和统计 | Python、Polars、Arrow、NumPy、SciPy |
| `python/alphalens_validation` | 独立因子统计复核 | Alphalens Reloaded |
| `python/zipline_validation` | 独立事件驱动组合复核 | Zipline Reloaded |
| `proto`、`packages/protocol-ts`、`python/loop_protocol` | 跨语言协议与契约 | Protobuf/gRPC |
| `migrations/postgres` | 版本化元数据迁移 | PostgreSQL |
| `apps/web`、`crates/loop-tui`、`crates/loopctl` | 客户端代码；`loopctl` 当前提供诊断 | React、Ratatui、Clap |

Provider 不读取研究数据库或留出数据；数值研究服务不实现模型厂商路由。
原始数据、研究视图、Provider 提示材料和受保护样本使用独立权限与存储边界。
元数据使用 PostgreSQL，数据与结果使用内容寻址的不可变文件。

## 安装与检查

支持的宿主机为 Linux x86-64。准备 Node.js 24.17.0、Corepack、uv 0.11.29、
curl、tar、xz 和 SHA-256 工具后，在仓库根目录执行：

```bash
./scripts/bootstrap.sh
just check
just test
just build
just doctor
```

Bootstrap 按仓库清单安装 Rust 1.93.1、pnpm 11.25.0 和 just 1.45.0，
按 lockfile 安装依赖。主 Python 为 3.14.4，使用根 uv workspace 和一个根 `.venv`。
独立复核使用各自锁定的临时环境；Zipline 因上游兼容性单独使用 Python 3.12.13，
不改变主研究解释器，也不创建第二个持久项目虚拟环境。

完整集成测试需要 Docker；隔离验收还需要 root 或非交互 `sudo chown` 来准备私有挂载。
开发容器通过 DaoCloud 拉取固定摘要的镜像：

```bash
just test-isolation
just container-gate
```

安装前提、镜像与 Rust 下载源、缓存位置及资源要求见
[开发环境](docs/development/bootstrap.md)。运行数据、私有配置和凭据应与代码分开保存。

## 数据接入

| 数据路径 | 用途 | 需要准备 |
| --- | --- | --- |
| SEC Company Facts | 申报数据开发验证 | 配置真实联系邮箱，无需 API key |
| Alpaca | 指定证券与 feed 的行情开发验证 | `LOOP_ALPACA_KEY_ID`、`LOOP_ALPACA_SECRET_KEY` |
| Nasdaq Data Link / Sharadar | 明确授权表的历史数据获取 | `LOOP_SHARADAR_API_KEY`、订阅与许可配置 |
| WRDS / CRSP / Compustat | 机构授权的数据获取 | WRDS 身份、数据权限与许可配置 |

依次按[凭据接入](docs/development/data-credentials.md)、
[开发数据](docs/development/development-data.md)或
[授权数据](docs/development/licensed-data.md)准备私有配置与数据目录。
Key 由环境或 secret manager 注入，不写入配置文件、Git、日志或命令行参数。

研究 CLI 可显示实际支持的命令：

```bash
./scripts/uv-research.sh run --locked --offline --no-sync loop-research --help
```

例如，在准备好当前用户所有、权限 `0700` 的绝对路径缓存目录，并核对 SEC 联系信息后：

```bash
./scripts/uv.sh run --package loop-research --locked --offline --no-sync \
  loop-research data-fetch config/data/sec-development.toml \
  --store /absolute/private/development-cache
```

`uv --offline` 仅禁止依赖下载；`data-fetch` 会执行配置中明确要求的数据请求。
缓存可通过 `data-replay` 离线核验，随后由 `data-snapshot`/`data-sync` 构造数据快照。
具体参数与血缘要求见[源快照](docs/development/source-snapshots.md)和
[PIT 数据](docs/development/point-in-time-data.md)。

SEC 与 Alpaca 的开发接入不证明完整历史证券池、退市覆盖或历史 PIT 质量。
Alpaca 权限按实际 feed 响应记录，不静默替换为其他行情源。当前申报数据的抓取时间
不能冒充过去已知的发布时间。正式历史研究需要授权数据覆盖和质量验收。

## 研究与复核工作流

1. 获取并验证源快照，按当时可见信息构造证券池与因子面板。
2. 规范化表达式并计算 SHA-256；冻结方向、研究参数、数据、成本与执行口径。
3. 运行组合回放与统计评测，登记全部尝试并保留失败结果。
4. 导出验证后的原始输入，由 Alphalens 和 Zipline 独立重算。
5. 对账、登记不可变回执，并在统一准入路径重新检查权限、覆盖、试验历史和证据新鲜度。

实际命令及输入示例分别见：

- [因子求值](docs/development/factor-evaluation.md)、[因果面板](docs/development/causal-factor-panels.md)
  与[横截面变换](docs/development/cross-sectional-transforms.md)。
- [组合回测](docs/development/portfolio-backtest.md)、[统计评测](docs/development/portfolio-statistics.md)
  与[全局试验统计](docs/development/global-statistics.md)。
- [Alphalens 复核](docs/development/independent-statistics.md)、
  [Zipline 对账](docs/development/independent-accounting.md)与
  [授权复核](docs/development/authorized-reconciliation.md)。

检查独立环境可直接运行：

```bash
./scripts/uv-alphalens.sh run --locked --offline loop-alphalens doctor
./scripts/uv-zipline.sh run --locked --offline loop-zipline doctor
```

数值一致不等于经济有效或正式准入。全局多重检验不能消除数据污染、幸存者偏差或
研究者自由度；授权数据、预先登记的经济准入规则和语义复核仍各自构成门槛。
方向只在 IS 决定，确认集不得用于返工调参；历史留出集不称为前瞻结果。
代码、数据、环境或试验总体变化会使依赖结果失效，不能继续沿用旧指标。

## 模型 Provider

OpenAI Responses/Chat 和 Anthropic Messages 保留一级原生实现；Google、Cohere、
Bedrock、Azure 与 Vertex 使用对应协议或部署适配。Mistral、DeepSeek、Qwen、xAI、
Groq、Together、Fireworks、Cerebras、Perplexity、GLM、Kimi 和 MiniMax 有独立厂商配置。
兼容层覆盖 OpenAI/Anthropic-compatible、自托管服务和 LiteLLM、Portkey、OpenRouter 网关。

模型是否可用取决于精确模型 ID、协议能力、账号、区域和权限。
契约测试通过不代表真实账号验证；未支持的能力明确拒绝，网关不冒充底层供应商。
每次调用固定模型、价格和能力快照；模型目录可以原子更新，已解析调用不会跟随别名漂移。

- [原生协议与 mTLS 配置](docs/development/native-providers.md)
- [云部署](docs/development/cloud-providers.md)、[厂商插件](docs/development/vendor-providers.md)
  与[兼容、自托管及网关](docs/development/compatible-providers.md)
- [模型目录与签名更新](docs/development/model-catalog.md)
- [隔离容器、出站许可与限流](docs/development/provider-runtime.md)

将示例配置复制到私有目录，填写真实模型、价格、认证身份及密钥引用后，使用
`node apps/providerd/dist/index.js --describe` 检查模型与策略摘要；
`PROVIDERD_DEPLOYMENT` 指向该私有配置。API key 本身不会打开调用入口。
生产隔离部署只挂载编译产物、Provider 私有配置与状态，并通过指定目标的出站网关访问厂商。

调用具有 token、费用、时间及并发限制，不自动重试可能已计费的生成请求。
滑动窗口限流属于单进程流量控制，不是跨实例或跨重启的总预算账本。
未由账单证据确认的费用只作为估算或预留，不宣称实际收费。

## 服务运行与运维

按[PostgreSQL 部署](docs/development/postgresql.md)准备数据库、TLS、受限应用账号和
私有连接文件。迁移由管理员显式执行，普通服务启动不自动执行 DDL：

```bash
./scripts/cargo.sh run -p loopd --locked --offline -- --check-database
```

授权 worker 与服务部署见[运行时身份和数据权限](docs/development/runtime-authority.md)、
[授权组合执行](docs/development/authorized-portfolios.md)。健康检查不代表数据订阅、
模型账号或研究质量已验证。未配置权限与可信引用的操作默认拒绝。

备份 PostgreSQL、不可变数据与结果、Provider 回执及模型目录历史。
回退时先停止新作业和写入，再恢复兼容版本；保留审计与模糊调用记录，不删除历史来重新执行。
不要将数据库、留出数据、宿主目录或 Docker socket 挂入 Provider。

## 开发与许可

贡献要求见 [AGENTS.md](AGENTS.md) 和[命名与 Rust 规范](docs/development/rust-style.md)。
架构决策、实施清单与验收记录保留在 `docs/` 供维护使用，不属于产品操作界面或运行数据。

旧 `code/`、`output/` 等 A 股资产保留用于历史审计与回归，不能作为美股结果使用。
23 个旧因子绩效已失效，不继承到美股因子库；私有 AlphaLab 不是新研究服务的必要依赖。

仓库尚未授予 MIT、Apache-2.0 或其他开源许可证；继承代码的授权状态仍需确认。
第三方软件与数据受各自许可约束，历史作者信息及法定署名予以保留。
本项目以公开、可测试的工程规则验收，不声称符合任何金融机构未公开的内部标准。
