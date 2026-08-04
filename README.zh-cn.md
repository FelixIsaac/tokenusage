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

| 服务商 / 助手 | 数据源类型 | 本地日志解析 | 实时 TUI (`tu live`/`top`) | 配额/余额探测 | 快捷命令 |
| :--- | :--- | :---: | :---: | :---: | :--- |
| **Claude Code** | 日志 / JSON | ✅ | ✅ | ✅ (`tu statusline`) | `tu claude` |
| **OpenAI Codex** | 日志 / JSON | ✅ | ✅ | — | `tu codex` |
| **Antigravity / AGY** | Protobuf (`*.db`) | ✅ | ✅ | ✅ (`tu antigravity status`) | `tu antigravity` / `tu agy` |
| **Gemini CLI** | 日志 / JSON | ✅ | ✅ | — | `tu gemini` |
| **OpenCode** | 日志 / JSON | ✅ | ✅ | — | `tu opencode` |
| **Grok (xAI)** | 日志 / OAuth 代理 | ✅ | ✅ | ✅ (`tu grok`) | `tu grok` |
| **DeepSeek API** | API 余额 | — | — | ✅ (`tu deepseek`) | `tu deepseek` |
| **OpenRouter API** | API 余额 | — | — | ✅ (`tu openrouter`) | `tu openrouter` |
| **Kimi (Moonshot)** | API 余额 | — | — | ✅ (`tu kimi`) | `tu kimi` |
| **Anthropic API** | API 用量 | — | — | ✅ (`tu anthropic-api`) | `tu anthropic-api` |

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

# 指定 AI 助手报表
tu codex daily
tu claude weekly
tu antigravity
tu antigravity status      # 实时套餐等级 + 会话/周配额百分比

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

# 生成分享卡片
tu img day
tu img week
```

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

## 🙏 致谢与贡献者声明

**tokenusage** 最初由 **[@hanbu97](https://github.com/hanbu97)** (`hanbu97/tokenusage`) 创建并构思架构。

本项目的所有核心最初灵感与初始解析框架均归功于原作者。本仓库 (**[FelixIsaac/tokenusage](https://github.com/FelixIsaac/tokenusage)**) 作为独立项目持续进行维护、性能优化与功能扩展，引入了全方位的多 Agent 支持、跨平台独立安装包与实时配额遥测功能。

---

## 📄 开源协议

基于 MIT 协议开源。详情参见 [`LICENSE`](./LICENSE)。
