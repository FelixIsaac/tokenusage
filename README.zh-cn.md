<p align="center">
  <img src="assets/branding/tokenusage-logomark.svg" width="128" height="128" alt="tokenusage logo" />
</p>

<h1 align="center">tokenusage</h1>

<p align="center">
  <strong>不再毫无预警地被限速。0.08 秒掌握你的 AI 编码开销。</strong>
</p>

<p align="center">
  <a href="https://github.com/FelixIsaac/tokenusage/actions/workflows/ci.yml"><img src="https://github.com/FelixIsaac/tokenusage/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/FelixIsaac/tokenusage/actions/workflows/release.yml"><img src="https://github.com/FelixIsaac/tokenusage/actions/workflows/release.yml/badge.svg" alt="Release" /></a>
  <a href="https://crates.io/crates/tokenusage"><img src="https://img.shields.io/crates/v/tokenusage?color=orange" alt="crates.io" /></a>
  <a href="https://www.npmjs.com/package/tokenusage"><img src="https://img.shields.io/npm/v/tokenusage?color=red" alt="npm" /></a>
  <a href="https://pypi.org/project/tokenusage/"><img src="https://img.shields.io/pypi/v/tokenusage?color=blue" alt="PyPI" /></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="License" /></a>
</p>

<p align="center">
  <a href="./README.md">English</a> | 中文
</p>

---

## ⚡ 概述

**tokenusage** (`tu`) 是一款极速、100% 本地的 AI 编码助手用量与 Token 成本监控仪表盘（支持 CLI、TUI 与 GUI）。

解析本地日志仅需 **0.08 秒**（比同类工具快 214 倍），`tu` 将 **Claude Code, OpenAI Codex, Antigravity/AGY, Gemini CLI, OpenCode, Grok, DeepSeek, OpenRouter, Kimi 与 Anthropic API** 的用量数据统一收录到一个本地控制面板中。

---

## 📸 界面截图

<table align="center" width="100%">
  <tr>
    <td valign="top" width="50%">
      <code>tu</code> — 每日开销报表<br/>
      <p align="center">
        <a href="docs/images/cli-demo-padded.png"><img src="docs/images/thumbs/cli-demo-padded.png" alt="tu cli demo" height="220" loading="lazy" /></a>
      </p>
    </td>
    <td valign="top" width="50%">
      <code>tu gui</code> — 桌面仪表盘<br/>
      <p align="center">
        <a href="docs/images/gui-demo.png"><img src="docs/images/thumbs/gui-demo.png" alt="tu gui demo" height="220" loading="lazy" /></a>
      </p>
    </td>
  </tr>
  <tr>
    <td valign="top" width="50%">
      <code>tu img day</code> — 日报分享卡片<br/>
      <p align="center">
        <a href="docs/images/share-demo.png"><img src="docs/images/thumbs/share-demo.png" alt="tu img daily demo" height="260" loading="lazy" /></a>
      </p>
    </td>
    <td valign="top" width="50%">
      <code>tu img week</code> — 周报分享卡片<br/>
      <p align="center">
        <a href="docs/images/share-week-demo.png"><img src="docs/images/thumbs/share-week-demo.png" alt="tu img weekly demo" height="260" loading="lazy" /></a>
      </p>
    </td>
  </tr>
  <tr>
    <td valign="top" colspan="2">
      <code>tu live</code> — 实时 TUI 监控<br/>
      <p align="center">
        <a href="docs/images/live-demo.png"><img src="docs/images/thumbs/live-demo.png" alt="tu live demo" width="100%" loading="lazy" /></a>
      </p>
    </td>
  </tr>
</table>

---

## 🎯 为什么选择 tokenusage

| 痛点 | tokenusage (`tu`) 解决方案 |
| :--- | :--- |
| **重构到一半被限速，毫无预警** | `tu live` 实时显示用量与剩余配额百分比 |
| **不知道 AI 编码每天花多少钱** | `tu` 0.08 秒给出每日费用明细 |
| **日志分散在多个 AI 编码工具中** | 统一仪表盘，合并 10 大 AI 服务商数据源 |
| **现有日志工具在日志量大时卡顿** | 比 ccusage 快 214 倍（Rust + Rayon 并行扫描 + SQLite WAL） |
| **担心隐私日志上传到云端** | 100% 本地解析，数据绝不离开你的电脑 |
| **不仅想看 Token，还想看编码时间** | `--with-activity` 增加真实编码时长与 Tokens/Hour 上下文 |
| **想向团队或社交媒体分享战报** | `tu img` 生成高清可分享卡片 |

---

## 🤖 支持的 AI 助手与功能矩阵

| Provider | 数据源类型 | 本地日志解析 | 实时 TUI (`tu live`/`top`) | 配额/余额探测 | 快捷命令 |
| :--- | :--- | :---: | :---: | :---: | :--- |
| **Claude Code** | 日志 / JSON | ✅ | ✅ | ✅ (`tu statusline`) | `tu claude` |
| **OpenAI Codex** | 日志 / JSON | ✅ | ✅ | ✅ (`tu codex status` / `tu blocks --official-limits-only`) | `tu codex` |
| **Antigravity / AGY** | Protobuf (`*.db`) | ✅ | ✅ | ✅ (`tu antigravity status`) | `tu antigravity` / `tu agy` |
| **Gemini CLI** | 日志 / JSON | ✅ | ✅ | — | `tu gemini` |
| **OpenCode** | SQLite / JSON | ✅ | ✅ | — | `tu opencode` |
| **Grok (xAI)** | 会话 / JSONL | ✅ | ✅ | ✅ (`tu grok`) | `tu grok` |
| **DeepSeek API** | API 余额 | — | — | ✅ (`tu deepseek`) | `tu deepseek` |
| **OpenRouter API** | API 余额 | — | — | ✅ (`tu openrouter`) | `tu openrouter` |
| **Kimi (Moonshot)** | API 余额 | — | — | ✅ (`tu kimi`) | `tu kimi` |
| **Anthropic API** | API 用量 | — | — | ✅ (`tu anthropic-api`) | `tu anthropic-api` |

### 🗺️ 生态支持现状与社区治理策略

| 工具 / 助手 | 支持状态 | 优先级 / 路径 | 策略与说明 |
| :--- | :---: | :---: | :--- |
| **Claude Code** | 已支持 | 核心 | 官方核心日常高频支持。 |
| **OpenAI Codex** | 已支持 | 核心 | 完整本地日志解析 + 官方 OAuth 额度探测。 |
| **Antigravity / AGY** | 已支持 | 核心 | Protobuf 会话解析 + 实时套餐计划配额探测。 |
| **Grok Build (xAI)** | 已支持 | 核心 | 完整会话目录 (`~/.grok/sessions`) 与统一日志解析。 |
| **OpenCode** | 已支持 | 核心 | 双 SQLite 数据库与历史消息目录解析。 |
| **Cursor CLI (`cursor-agent`)** | 评估中 | 高 | 热门 CLI 模式 (`~/.cursor/chats`)，下一个核心候选。 |
| **Goose (Block / AAIF)** | 社区驱动 | 欢迎 PR | 社区提交 PR 并附带测试用例即可合入。 |
| **Amp (Sourcegraph)** | 社区驱动 | 欢迎 PR | 欢迎社区贡献解析器。 |
| **Aider** | 社区驱动 | 欢迎 PR | 欢迎社区提交会话历史解析 (`.aider.chat.history.md`)。 |
| **GitHub Copilot CLI** | 社区驱动 | 欢迎 PR | 欢迎社区提交 (`~/.copilot-cli`)。 |
| **Cline / Roo Code** | 社区驱动 | 欢迎 PR | 欢迎社区提交贡献。 |
| **Cascade / Windsurf** | 社区驱动 | 欢迎 PR | 欢迎社区提交贡献。 |

> **社区治理原则：** 核心团队聚焦于日常高频使用的工具。对于生态中其他 AI 助手，非常欢迎社区提交 PR / Issue，只要符合零网络泄露、纯本地离线、高性能标准，团队将迅速审核并合并。

---

## 🚀 安装指南

根据你的开发环境选择最方便的安装方式：

### 1. 独立预编译二进制（最简 — 零依赖）

无需 Node.js, npm, Python 或 Rust 工具链。自动下载并安装预编译二进制文件。

* **macOS / Linux:**
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://github.com/FelixIsaac/tokenusage/releases/latest/download/tokenusage-installer.sh | sh
  ```
* **Windows (PowerShell):**
  ```powershell
  iwr https://github.com/FelixIsaac/tokenusage/releases/latest/download/tokenusage-installer.ps1 | iex
  ```

### 2. 🍺 Homebrew (macOS & Linux)

```bash
brew install FelixIsaac/tokenusage/tokenusage
```

### 3. 📦 Node.js / npm

```bash
npm install -g tokenusage
```

### 4. 🦀 Rust (Cargo / cargo-binstall)

```bash
# 预编译二进制快速安装
cargo binstall tokenusage --no-confirm

# 或从 Cargo 源码编译安装
cargo install tokenusage --bin tu
```

### 5. 🐍 Python (pip)

```bash
pip install tokenusage
```

### 6. 🛠️ 源码编译

```bash
git clone https://github.com/FelixIsaac/tokenusage.git
cd tokenusage
cargo install --path . --bin tu
```

---

## 💡 快速开始与常用命令

```bash
# 每日费用报表（默认）
tu

# 交互式 TUI 界面
tu --tui

# 指定 AI 助手报表与实时配额
tu codex daily
tu codex status                         # 快速探测官方 Codex 配额（零历史扫描）
tu blocks --official-limits-only --json  # 官方 Codex 配额 JSON 直出（<0.60s 极速）
tu claude weekly
tu antigravity
tu antigravity status                   # 实时套餐等级 + 会话/周配额百分比

# 过滤特定数据源
tu --only codex,gemini
tu weekly --sources claude,opencode

# 自动补全脚本配置
tu completions zsh > ~/.zsh/completion/_tu
tu completions bash > ~/.bash_completion.d/tu

# 低配额告警阈值设置
tu antigravity status --warn-threshold 15

# 实时进程监控（Token 界的 htop）
tu live
tu top

# 桌面 GUI 仪表盘
tu gui

# 碳足迹与能耗分析报表 (电能 kWh, 碳排放 CO2e, 冷却耗水 L)
tu carbon                 # 今日碳足迹报表
tu carbon weekly          # 本周碳足迹报表
tu carbon monthly         # 本月碳足迹报表
tu carbon all             # 历史全量碳足迹报表
tu carbon about           # 物理学计算公式与名词解释
tu carbon --region nordic # 切换电网区域 (us-east, us-west, eu-west, nordic, google-cfe 等)

# 生成分享卡片
tu img day
tu img week
```

---

## 📋 完整命令速查表

### 📊 周期性报表与用量聚合
| 命令 | 说明 |
| :--- | :--- |
| `tu` / `tu daily` | 每日 Token 用量与成本报表（默认命令） |
| `tu today` | 今日实时用量与模型消耗明细 |
| `tu weekly` (或 `tu week`) | 按 ISO 8601 自然周聚合（周一开始） |
| `tu monthly` | 按自然月聚合 |
| `tu session` | 按会话 Session ID 独立聚合 |
| `tu blocks` | 按 5 小时滚动限速窗口聚合 |
| `tu activity` | 真实编码活跃时长、Tokens/小时与主力编程语言 |
| `tu --tui` | 启用交互式 TUI 表格（固定表头、支持上下滚动） |
| `tu -i` / `tu --instances` | 显示按会话实例拆分的子明细 |
| `tu -p <项目名>` | 仅筛选特定项目路径的用量 |
| `tu --brief` | 单行极简概要输出（时间范围 · 总 Token · 成本 · 主力模型） |

### 🌱 碳足迹与能耗遥测
| 命令 | 说明 |
| :--- | :--- |
| `tu carbon` | 今日环境影响报表（电能 kWh、等效碳排 kg CO₂e、冷却耗水 L） |
| `tu carbon daily` | 每日碳足迹明细表 |
| `tu carbon weekly` | 本周碳足迹明细表 |
| `tu carbon monthly` | 本月碳足迹明细表 |
| `tu carbon all` | 历史所有会话的全量环境遥测分析 |
| `tu carbon about` | GPU 物理计算模型、能耗常数与 PUE/WUE 透明度声明 |
| `tu carbon --region <区域>` | 设定算力电网区域 (`us-east`, `us-west`, `us-avg`, `eu-west`, `nordic`, `google-cfe`, `global`) |
| `tu daily --carbon` | 在标准每日用量报表中附加碳排放与能耗数据列 |

### ⚡ 配额探测与官方额度
| 命令 | 说明 |
| :--- | :--- |
| `tu codex status` | **极速 OAuth 配额探测**（5小时/周限额、重置倒计时，零磁盘日志扫描） |
| `tu blocks --official-limits-only --json` | 官方 Codex 配额 JSON 数据流直出（<0.60s 快速旁路） |
| `tu antigravity status` | 实时探测 Antigravity 套餐等级与会话/周配额百分比 |
| `tu antigravity status --warn-threshold 15` | 当剩余额度低于 15% 时触发黄色告警提示 |
| `tu deepseek` | 查询 DeepSeek API 账户余额 (`DEEPSEEK_API_KEY`) |
| `tu openrouter` | 查询 OpenRouter API 账户可用额度 (`OPENROUTER_API_KEY`) |
| `tu grok` | 查询 Grok (xAI) API 额度与 CLI OAuth Proxy Token 状态 (`XAI_API_KEY`) |
| `tu kimi` | 查询 Kimi (Moonshot) API 账户余额 (`MOONSHOT_API_KEY`) |
| `tu anthropic-api` | 查询 Anthropic 官方 API 当日用量与费用 (`ANTHROPIC_API_KEY`) |

### 🖥️ 实时监控、TUI 与桌面 GUI
| 命令 | 说明 |
| :--- | :--- |
| `tu live` | 实时刷新交互式 TUI 监控仪表盘 |
| `tu top` | Token 进程监视器（类似 htop 的按会话实时监视） |
| `tu gui` | 原生桌面 GUI 客户端（基于 Iced 框架） |
| `tu img day` | 生成今日编码战报的高清 PNG 分享卡片 |
| `tu img week` | 生成本周编码战报的高清 PNG 分享卡片 |

### 🔧 终端集成、补全与诊断工具
| 命令 | 说明 |
| :--- | :--- |
| `tu statusline` | 为终端 Prompt / 状态栏挂钩输出单行状态组件 |
| `tu statusline init` | 一键自动配置 Claude Code 状态栏（智能合并 `~/.claude/settings.json`） |
| `tu completions <shell>` | 生成 Shell 自动补全脚本 (`bash`, `zsh`, `fish`, `powershell`, `elvish`) |
| `tu doctor` | 全面检查扫描路径、日志发现量、SQLite 缓存健康度与价格缓存 TTL |
| `tu parity` | 自动比对 `tu` 与 `@ccusage` 官方工具的解析与计算一致性 |

---

## ⚡ 性能基准测试

**测试环境:** Apple M3 Max, macOS 15.6.1 · `tu` v1.11.2 对比 `ccusage` v18.0.8

### Codex — 91 个 JSONL 文件, 1.7 GB (`~/.codex/sessions`)

| 模式 | `tu codex daily` | `bunx @ccusage/codex` | 加速倍数 |
| :--- | :---: | :---: | :---: |
| **冷启动 (重建缓存)** | **0.92s** | 20.76s | **22.6x** |
| **热启动 (5 次最佳)** | **0.15s** | 20.76s | **138x** |

### Claude — 1,521 个 JSONL 文件, 2.2 GB (`~/.claude/projects`)

| 模式 | `tu claude daily` | `bunx ccusage` | 加速倍数 |
| :--- | :---: | :---: | :---: |
| **冷启动 (重建缓存)** | **0.73s** | 17.15s | **23.5x** |
| **热启动 (5 次最佳)** | **0.08s** | 17.15s | **214x** |

---

## 📁 本地数据存储路径

| 服务商 / 数据源 | 默认本地日志路径 | 自定义参数 |
| :--- | :--- | :--- |
| **Claude Code** | `~/.claude/projects` | `--claude-projects-dir` |
| **OpenAI Codex** | `$CODEX_HOME/sessions` (或 `~/.codex/sessions`) | `--codex-sessions-dir` |
| **Antigravity / Gemini** | `~/.gemini/antigravity-cli/conversations/*.db` & `~/.gemini/tmp` | `--gemini-data-dir` |
| **OpenCode** | `$OPENCODE_DATA_DIR` (或 `~/.local/share/opencode`) | `--opencode-data-dir` |
| **Grok Build** | `~/.grok/logs` & `~/.grok/auth.json` | `--grok-log-dir` |
| **历史基准覆盖** | `~/.config/tokenusage/history_overrides.json` | `--no-history-overrides` |
| **历史聚合数据库** | `~/.config/tokenusage/history.db` | `--no-history-db` |

---

## ⚙️ 配置文件 (`tu.json`)

`tu` 会按如下顺序自动查找配置文件：
1. `./.tu/tu.json`
2. `~/.config/tu/tu.json`
3. `~/.config/tokenusage/tokenusage.json`

配置文件示例 (`~/.config/tu/tu.json`)：

```json
{
  "defaults": {
    "timezone": "Asia/Shanghai",
    "workers": 16,
    "compact": false
  },
  "commands": {
    "daily": {
      "instances": true
    },
    "live": {
      "sessionLength": 5,
      "refreshInterval": 1
    },
    "img": {
      "period": "daily",
      "bars": 24,
      "brand": "tokenusage",
      "brandUrl": "https://github.com/FelixIsaac/tokenusage"
    }
  }
}
```

---

## 🙏 致谢与贡献者声明

**tokenusage** 最初由 **[@hanbu97](https://github.com/hanbu97)** (`hanbu97/tokenusage`) 创建并构思架构。

本项目的所有核心最初灵感与初始解析框架均归功于原作者。本仓库 (**[FelixIsaac/tokenusage](https://github.com/FelixIsaac/tokenusage)**) 作为独立项目持续进行维护、性能优化与功能扩展，引入了全方位的多 Agent 支持、跨平台独立安装包与实时配额遥测功能。

---

## 📄 开源协议

基于 MIT 协议开源。详情参见 [`LICENSE`](./LICENSE)。
