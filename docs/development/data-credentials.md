# 美股数据凭据与接入验收

本页说明已实现的 Phase 5 数据命令如何使用。账号/API key 负责识别调用者，
数据订阅决定能读取哪些表；两者都不能替代历史数据质量检查。
预检不联网、不下载、不写缓存，也不会购买订阅。

## 先选择数据路径

| 用途 | 数据路径 | 需要准备的凭据 | 当前实现范围 |
| --- | --- | --- | --- |
| 免费基本面开发 | SEC | 无 API key；真实联系邮箱 | 选定 CIK 的 Company Facts 和申报元数据 |
| 行情接入开发 | Alpaca | API Key ID 与 Secret Key | 当前资产元数据、IEX/SIP 原始日线、独立最新 SIP 权限探测 |
| 授权历史研究候选路径 | Nasdaq Data Link 的 Sharadar | Data Link API key 与对应表的订阅权限 | SEP、SF1、TICKERS、ACTIONS |
| 已有学校/单位数据权限 | WRDS | WRDS 用户名、密码及 CRSP/Compustat 权限 | CRSP CIZ 日表、Compustat 季度表的固定只读查询 |
| 补充公司行动和证券参考资料 | Databento | Databento API key 与 Reference 数据权限 | Security Master、Corporate Actions；不分配新 ISIN |

不需要同时开通全部供应商。个人部署可以先评估 Data Link 的 Sharadar 权限，
Alpaca 用于开发接入；已有机构 WRDS 权限时再使用 WRDS 路径。购买前向供应商
确认所需历史区间、退市证券、数据修订/PIT、内部研究和本地存储权限。
当前分区和样本规划包含 2026-08-31，但下载区间配置不证明供应商具有完整覆盖。

## 获取凭据

### Nasdaq Data Link / Sharadar

1. 在 [Nasdaq Data Link](https://data.nasdaq.com/) 注册并登录自己的账号。
2. 根据需要申请/订阅 [Sharadar SF1](https://data.nasdaq.com/databases/SF1)
   和 [Sharadar SEP](https://data.nasdaq.com/databases/SEP)。确认账号能调用实际选定的
   `SHARADAR/SEP`、`SHARADAR/SF1`、`SHARADAR/TICKERS`、`SHARADAR/ACTIONS` 表。
   API key 本身不代表已经订阅这些表，也不要把免费样例当作完整数据权限。
3. 按 SF1 官方说明，在账号设置中获取 Data Link API key。
4. 将其注入 `LOOP_SHARADAR_API_KEY`。

本项目当前调用 `data.nasdaq.com/api/v3/datatables/...`。
[Sharadar 自有 API](https://sharadar.com/docs/auth) 使用另一个域名和接口；不能把
那里的 key 当成 Data Link key，也不能只更改 URL 就认为两种协议兼容。
具体订阅价格、地区可用性及授权范围，以申请时供应商给出的合同为准。

### Alpaca

1. 从 [Alpaca 控制台](https://app.alpaca.markets/) 注册/登录账号。
2. 按 [Paper Trading 官方文档](https://docs.alpaca.markets/us/docs/paper-trading)
   进入 Paper 环境，生成该环境的 API key，保存 Key ID 和 Secret Key。
3. 分别注入 `LOOP_ALPACA_KEY_ID` 和 `LOOP_ALPACA_SECRET_KEY`。
4. 保持示例的 `paper=true` 与 Paper 凭据匹配；使用 Live 资产端点时才改成
   `paper=false` 并换成对应凭据。本项目的数据命令不提交交易订单。

Paper Only 权限与全市场行情权限不同。示例选择 `feed="iex"`；SIP 历史请求
和最新 SIP 探测分别记录结果，不会在拒绝后静默切换 feed。
参见 [Alpaca Market Data FAQ](https://docs.alpaca.markets/us/docs/market-data-faq)。
当前资产列表不能作为完整历史上市/退市 universe。

### WRDS / CRSP / Compustat

1. 先向所在学校/单位的数据管理员确认：机构是否订阅 WRDS，以及是否包含
   本研究需要的 CRSP 和 Compustat 数据。拥有 WRDS 账号不代表拥有所有数据库。
2. 使用 [WRDS 注册页](https://wrds-www.wharton.upenn.edu/register/) 选择真实所属
   订阅机构和账户类型，完成邮件确认、机构审批和网站要求的身份验证。
   没有机构权限时，通过 [WRDS 联系入口](https://wrds-www.wharton.upenn.edu/pages/about/contact-wrds/)
   了解适用的订阅途径，不使用他人的账号。
3. 确认该账号可通过 PostgreSQL 客户端读取 `crsp.stkdlysecuritydata` 和/或
   `comp.fundq`；账号类型限制和额外身份验证要求由 WRDS 决定。
4. 注入 `LOOP_WRDS_USERNAME`、`LOOP_WRDS_PASSWORD`。

WRDS 是数据访问平台，CRSP 是证券市场历史数据，Compustat 是公司基本面数据。
连接固定为 `wrds-pgdata.wharton.upenn.edu:9737/wrds`、`sslmode=require`。
这里的用户名/密码是供应商账号，和 Loop Engine 自己的 PostgreSQL 数据库账号
完全独立。普通数据库加密连接不等于已经通过供应商实际登录验证。

### Databento（可选）

1. 注册并登录 Databento Portal，在 API Keys 页面创建专用于 Loop Engine 的 key；
   操作见 [官方 API keys 指南](https://databento.com/docs/portal/api-keys)。
2. 确认账号的 Reference API 订阅覆盖所选 Security Master / Corporate Actions。
   当前适配器只支持已预付订阅读取，并固定 `allocate_isins=false`。
3. 注入 `LOOP_DATABENTO_API_KEY`，填写真实、已获权限的 listing ID。

普通历史行情 credits 不能自动证明 Reference 数据权限。供应商 Portal 的用量限制
和本项目的请求/字节预算都应保留；本阶段没有按量购买或自动追加预算的操作。

### SEC

SEC 公共接口不需要 key。示例中的 `contact_email` 应使用自己的真实联系方式，
当前维护者为 `hojiahao@outlook.com`。客户端发送可识别的 User-Agent，并按配置限流。
参见 [SEC 开发者说明](https://www.sec.gov/about/developer-resources)。
已保留的 SEC 实际采集证据可离线重放，无需重复下载。

## 在本机配置

所有命令从仓库根目录执行，复用现有 Python 3.14.4 根 `.venv`。
`uv --offline` 只控制包解析；下文的 `data-fetch` / `data-acquire` / `data-sync`
仍会联网，`data-preflight` / `data-replay` / `data-verify` / `data-validate` 不联网。

```bash
install -d -m 700 /home/hojiahao/loop_engine/var/data/private
install -d -m 700 /home/hojiahao/loop_engine/var/data/source-cache
```

在自己的终端或 secret manager 中注入凭据。下例使用 Bash 隐藏输入，输入值不会
进入命令行参数或 shell 命令历史；不要开启 `set -x`，也不要把实际 key 发到聊天中：

```bash
read -r -s -p 'Nasdaq Data Link API key: ' LOOP_SHARADAR_API_KEY
export LOOP_SHARADAR_API_KEY
```

其他供应商使用上表给出的变量名。当前程序读取环境变量，不会自动加载 `.env`。
需要持久服务配置时，由部署环境的 secret manager 或受保护的环境文件注入。

对 Sharadar、WRDS、Databento，还需按 [许可文件格式](licensed-data.md#rights-and-configuration)
编写真实授权声明，保存到 `var/data/private/`，设置模式 `0600`。声明记录已有授权的
来源、表、历史范围、期限和本地存储用途；它不是向供应商购买权限的凭证。

复制对应的 `config/data/*.toml` 示例到 `var/data/private/`，保留 secret reference
名称。对实际许可文件运行 `sha256sum`，将 TOML 中全零的 `license_sha256` 替换为
`sha256:<实际摘要>`，再选择明确的日期与标识符。不填写真实许可时，示例应被拒绝。
原始/订阅数据、凭据和私人许可文件不得提交到 Git；`var/` 已被忽略。

## 先预检，再最小下载

无需凭据即可验证公共 SEC 的本地配置：

```bash
./scripts/uv.sh run --package loop-research --locked --offline --no-sync loop-research data-preflight \
  config/data/sec-development.toml \
  --store /home/hojiahao/loop_engine/var/data/source-cache
```

授权源预检使用已经准备好的私人文件：

```bash
./scripts/uv.sh run --package loop-research --locked --offline --no-sync loop-research data-preflight \
  /home/hojiahao/loop_engine/var/data/private/sharadar.toml \
  --license /home/hojiahao/loop_engine/var/data/private/sharadar-license.json \
  --store /home/hojiahao/loop_engine/var/data/source-cache
```

同一个命令也接受 `loop.data-sync-plan/v1` 批量配置，检查所有请求。
输出的 `missing_references` 是缺少的环境变量名称，`issues` 是固定错误代码。

| 退出码/状态 | 含义 | 操作 |
| --- | --- | --- |
| 0 / `local_ready=true` | 本地配置、凭据格式、许可和目录检查通过 | 可以在批准预算内做最小源请求；尚未证明 key 或订阅有效 |
| 3 / `missing_credentials` | 未注入凭据，或格式不符 | 检查所需变量和实际厂商 key 类型 |
| 3 / `license_denied` | 未提供匹配许可，或内容格式/摘要/表/日期/有效期不符 | 核对实际权限、文件摘要和声明范围 |
| 3 / `invalid_configuration` | 请求包含当前/未来纽约日期，或 WRDS 存在环境 `PG*` 覆盖 | 缩小到完整历史日期，隔离 WRDS 客户端环境 |
| 3 / `invalid_cache` | 目录缺失、非规范路径或所有权/0700 模式不符 | 修复指定项目缓存目录 |
| 2 | TOML 非法、许可文件无法安全读取、重复许可或发现敏感配置 | 修复输入文件；不会输出其中内容 |

预检始终返回 `live_access="not_checked"`、`production_eligible=false`。
运行过预检不会缓存授权；下载时仍再次检查凭据和许可。

确认订阅和预算后，将日期缩小到少量完整交易日、选择一两个明确标识符，执行
`data-fetch`（SEC/Alpaca）或 `data-acquire`（授权源）。用返回的 receipt 执行
`data-replay` 或 `data-verify`；然后按 [快照说明](source-snapshots.md) 生成
Parquet 并运行 `data-validate`。这条顺序先验证实际认证、响应和离线一致性，再逐步
扩展分区。单次请求最多 8 个标识符、10,000 条记录；不能直接当作全市场批量导入器。

## Phase 5 的最终证据边界

编码验收包括五条源路径、原始缓存、精确值/时间语义、离线重放、Parquet 快照、
可恢复同步及预检。供应商契约使用合成数据，WRDS 查询还使用独立的本地 TLS
PostgreSQL 验证；已有真实 SEC 公共请求和离线快照证据。

Alpaca、Sharadar、WRDS、Databento 尚未使用真实供应商凭据完成 live 验证。
即使最小下载通过，还必须检查实际证券历史、退市、调整价格、修订可见时间与
完整样本覆盖，才能评估数据是否适合生产研究。当前数据报告如实保持未认证状态。
Phase 6 因子面板和 Phase 7 回测不能由这些接入测试替代。
