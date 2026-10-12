# Loop Engine README 品牌资源

README 使用独立设计的循环符号、终端式字标和青绿色强调色。宽屏与窄屏各有
深浅主题 SVG，由 `<picture>` 选择。图形是品牌插画，不是尚未交付客户端的截图。
首页只介绍已实现能力，具体信息层级见[README 设计说明](../development/readme-design.md)。
Banner 使用 `US EQUITY FACTOR RESEARCH` 定位，不宣称自主研究闭环已经交付。
八枚徽章分为两组：

- 产品属性：美股研究、CLI 入口、可插拔模型、CI 检查入口。
- 技术职责：Rust 控制平面、TypeScript 模型接入、Python 研究计算、PostgreSQL 状态。

徽章链接到实际功能文档或检查页面；`CI / checks` 是静态入口，不表示检查通过。

Banner 和八枚徽章的全部可见英文均使用 **Google Sans Code**，已转换为 SVG
字形路径，因此 GitHub 图片代理无需加载字体。SVG 保留可访问标题和描述，
README 提供替代文字。GitHub 控制 README 正文字体，仓库不能为正文注入自定义 CSS。

## 字体来源与许可

- 字体项目：[googlefonts/googlesans-code](https://github.com/googlefonts/googlesans-code)。
- 分发源：[google/fonts 固定提交](https://github.com/google/fonts/tree/bd8f81ddb5c74d5c8897b36ad88b440266245103/ofl/googlesanscode)。
- 原始文件：`GoogleSansCode[wght].ttf`，原样保存为 `vendor/GoogleSansCode.ttf`。
- 字体 SHA-256：`c9649573afcd966f61096b1e9c3c3115e7b54315ca2adf9839c6f8b17d5f8e1f`。
- 原始版权与 SIL OFL 1.1 全文：[vendor/OFL.txt](vendor/OFL.txt)。
- 原始商标说明：[vendor/TRADEMARKS.md](vendor/TRADEMARKS.md)。

字体使用不表示 Google 赞助或认可本项目。字体许可只适用于字体软件，不为本仓库
其他代码授予开源许可。外观参考了 [OpenCode](https://github.com/anomalyco/opencode)
的主题图与导航组织、[Crush](https://github.com/charmbracelet/crush) 的终端品牌呈现；
未复制这些项目的标志或资源。

## 维护

`generate.py` 仅用于文档资产维护，不参与应用构建。唯一额外工具为
`fonttools==4.60.1`，无需修改项目依赖或根虚拟环境：

```bash
uv run --no-project --with fonttools==4.60.1 --python 3.14.4 \
  python docs/brand/generate.py
uv run --no-project --with fonttools==4.60.1 --python 3.14.4 \
  python docs/brand/generate.py --check
```

生成器首先校验字体摘要，然后生成四张 Banner 与八枚徽章。修改文案或颜色时
编辑生成器并重新生成，不直接修改字形路径。徽章描述实际产品属性与组件职责，
不在首页重复维护工具链版本号，也不作为尚未实现功能的占位。

2026-10-12 以本地 Chromium 153.0.8010.12 完成真实浏览器验收：1440×900 与
390×1000 视口，各检查深浅主题；全部九个展示图片加载成功，选择对应 Banner，
无水平溢出。截图目视检查无文字重叠或缺字。该验收使用模拟 GitHub 内容宽度的
本地预览，并非对 GitHub 网站自身排版的保证。Ruff、`--check` 与差异空白检查通过。
预览、截图和临时工具不作为长期仓库资产保留。

本变更仅涉及 README 与品牌文件；回退该文档提交即可恢复旧外观，不影响运行数据。
