<p align="center">
  <img src="assets/branding/tokenusage-logomark.svg" width="128" height="128" alt="tokenusage logo" />
</p>

<h1 align="center">tokenusage</h1>

<p align="center">
  <strong>不再毫无预警地被限速。0.08 秒掌握你的 AI 编码开销。</strong>
</p>

<p align="center">
  <a href="https://github.com/hanbu97/tokenusage/actions/workflows/ci.yml"><img src="https://github.com/hanbu97/tokenusage/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://crates.io/crates/tokenusage"><img src="https://img.shields.io/crates/v/tokenusage?color=orange" alt="crates.io" /></a>
  <a href="https://www.npmjs.com/package/tokenusage"><img src="https://img.shields.io/npm/v/tokenusage?color=red" alt="npm" /></a>
  <a href="https://pypi.org/project/tokenusage/"><img src="https://img.shields.io/pypi/v/tokenusage?color=blue" alt="PyPI" /></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="License" /></a>
</p>

<p align="center">
  <a href="./README.md">English</a> | 中文
</p>

---

### 一行安装

```bash
npm i -g tokenusage        # 或: cargo install tokenusage --bin tu
```

### 运行

```bash
tu                          # 0.08 秒出日报
```

---

<p align="center">
  解析 Claude 日志比 ccusage <strong>快 214 倍</strong> · 解析 Codex 日志<strong>快 138 倍</strong>（热缓存） · <a href="#性能基准">查看基准测试</a>
</p>

---

## 截图

<table align="center" width="100%">
  <tr>
    <td valign="top" width="50%">
      <code>tu</code> — 日报<br/>
      <p align="center">
        <a href="docs/images/cli-demo-padded.png"><img src="docs/images/thumbs/cli-demo-padded.png" alt="tu cli" height="220" loading="lazy" /></a>
      </p>
    </td>
    <td valign="top" width="50%">
      <code>tu gui</code> — 桌面仪表盘<br/>
      <p align="center">
        <a href="docs/images/gui-demo.png"><img src="docs/images/thumbs/gui-demo.png" alt="tu gui" height="220" loading="lazy" /></a>
      </p>
    </td>
  </tr>
  <tr>
    <td valign="top" width="50%">
      <code>tu img day</code> — 分享卡片<br/>
      <p align="center">
        <a href="docs/images/share-demo.png"><img src="docs/images/thumbs/share-demo.png" alt="tu img daily" height="260" loading="lazy" /></a>
      </p>
    </td>
    <td valign="top" width="50%">
      <code>tu img week</code> — 周报卡片<br/>
      <p align="center">
        <a href="docs/images/share-week-demo.png"><img src="docs/images/thumbs/share-week-demo.png" alt="tu img weekly" height="260" loading="lazy" /></a>
      </p>
    </td>
  </tr>
  <tr>
    <td valign="top" colspan="2">
      <code>tu live</code> — 实时 TUI 监控<br/>
      <p align="center">
        <a href="docs/images/live-demo.png"><img src="docs/images/thumbs/live-demo.png" alt="tu live" width="100%" loading="lazy" /></a>
      </p>
    </td>
  </tr>
</table>

## 为什么选 tokenusage

| 痛点 | tokenusage 方案 |
|---|---|
| 重构到一半被限速，毫无预警 | `tu live` 实时显示用量 |
| 不知道 AI 编码每天花多少钱 | `tu` 0.08 秒给出每日开销明细 |
| Codex 和 Claude 日志分散在不同目录 | 一个统一仪表盘，合并所有数据源 |
| 现有工具在大日志上很慢 | 比 ccusage 快 214 倍（Rust + 并行扫描 + 缓存） |
| 不想把日志上传到云端 | 100% 本地解析，数据不离开你的电脑 |
| 不只想看 token，还想看编码时间与效率 | 默认 `tu` 保持经典 token 报表；`--with-activity` 按需增加 coding time 与 tokens/hour |
| 想分享使用统计 | `tu img` 生成可分享的图片卡 |

## 安装

### npm（推荐）

```bash
npm install -g tokenusage
```

### cargo (crates.io)

```bash
cargo install tokenusage --bin tu
```

### pip (PyPI)

```bash
pip install tokenusage
```

### cargo-binstall（预编译二进制）

```bash
cargo binstall tokenusage --no-confirm
```

## 快速开始

```bash
# 日报（默认）
tu                          # 经典合并 token 报表
tu --tui                    # 同一份报表的终端 UI

# 指定数据源
tu codex daily
tu claude daily
tu parity --provider claude --period daily --since 20260401 --until 20260401 --json
tu gemini
tu opencode
tu antigravity              # 每日用量/费用报表
tu antigravity status       # 实时套餐等级 + 会话/周配额百分比

# 在合并报表中按 provider 过滤
tu --only codex,gemini
tu weekly --sources claude,opencode

# 数据源诊断
tu doctor
tu doctor --only opencode --json

# 日期过滤
tu --since 2026-02-01 --until 2026-02-28

# 周报 / 月报
tu weekly --start-of-week monday
tu monthly

# 基于本地 AI 使用记录推断的时间视图
tu today
tu activity
tu activity --days 14
tu activity --project tokenusage

# 给合并后的 token 报表增加 activity 列（Coding / Tok/hr）
tu --with-activity
tu --with-activity --tui
tu live --with-activity

# 原生本地 heartbeat 采集层
tu heartbeat watch .
tu heartbeat stats
tu heartbeat ping src/main.rs --write

# 实时监控（标签页: Codex / Claude / Antigravity / OpenCode / Antigravity Quota）
tu live
tu live codex
tu live claude
tu live gemini
tu live opencode
tu live antigravity

# htop 风格的会话查看器
tu top
tu top --active-hours 12    # 显示最近 12 小时的活跃会话

# GUI 仪表盘
tu gui

# 生成分享图片
tu img
tu img day
tu img week
```

## 性能基准

**测试环境:**

- 机器: Apple M3 Max, macOS 15.6.1
- `tu` 版本: `1.2.6` · `ccusage` 版本: `18.0.8`
- 默认模式（无日期过滤，在线定价，网络启用）

**Claude** — 1,521 个 JSONL 文件, 2.2 GB

| | `tu claude daily` | `bunx ccusage` | 加速比 |
|---|---:|---:|---:|
| 冷启动（重建缓存） | **0.73s** | 17.15s | **23.5x** |
| 热缓存（5 次最佳 / 3 次均值） | **0.08s** | 17.15s | **214x** |

**Codex** — 91 个 JSONL 文件, 1.7 GB

| | `tu codex daily` | `bunx @ccusage/codex` | 加速比 |
|---|---:|---:|---:|
| 冷启动（重建缓存） | **0.92s** | 20.76s | **22.6x** |
| 热缓存（5 次最佳 / 3 次均值） | **0.15s** | 20.76s | **138x** |

> 结果因硬件、文件系统缓存状态和日志量而异。

详细对比请看 [tokenusage vs ccusage](docs/compare/tokenusage-vs-ccusage.md)。

## 常见问题

### 数据从哪来?

从本地日志目录和 IDE 探测：
- Claude: `~/.config/claude/projects`, `~/.claude/projects`
- Codex: `~/.codex/sessions`, `~/.config/codex/sessions`
- Gemini CLI: `~/.gemini/tmp`
- Antigravity: `~/.gemini/antigravity-cli/conversations/*.db`（承载真实 token/费用数据的 SQLite 会话数据库）以及 `~/.gemini/antigravity-cli/brain`（对话记录）。`tu antigravity status` 还会额外探测运行中 IDE 的语言服务器，获取实时套餐等级与会话/周配额百分比。
- OpenCode: `$OPENCODE_DATA_DIR` 或 `$XDG_DATA_HOME/opencode`（回退 `~/.local/share/opencode`）

可通过 `--claude-projects-dir`、`--codex-sessions-dir`、`--gemini-data-dir`、`--opencode-data-dir` 覆盖。

`--only` / `--sources` 作用于日志型 provider（`claude`、`codex`、`gemini`、`opencode`、`grok`）——`gemini`/`antigravity`/`agy` 是同一数据源的等价别名。

### 如何估算费用?

`tu` 优先使用 OpenRouter 在线定价（缓存 6 小时），网络不可用时回退到内置离线费率。

### `--with-activity` 是怎么工作的?

`tu` 会直接从本地机器推断编码活跃时间。默认情况下它会把相近的 AI usage 事件聚合成活跃窗口；如果你启用了原生 heartbeat 采集层（`tu heartbeat watch ...`），那么在 heartbeat 覆盖足够的日期上会优先使用 heartbeat，在其他日期回退到 token 事件推断。

基于这套本地 activity 信号，`tu` 会得到：

- coding time
- tokens per coding hour
- cost per coding hour
- project / language / source breakdowns

专门的时间视图命令 `tu today` / `tu activity` 会自动启用这一能力。`--with-activity` 会把同一套本地 activity 上下文合并进日/周/月报表和 `tu live`。

默认情况下，`tu` 会保持原来的合并 token 报表列布局。只有在显式启用 `--with-activity`，或使用专门的时间视图时，才会显示额外的 `Coding` / `Tok/hr` 列。

### 数据隐私安全吗?

日志解析完全在本地进行。`tu` 仅请求定价元数据，使用 `--offline` 可完全离线。

## 配置文件

配置搜索顺序：
1. `./.tu/tu.json`
2. `~/.config/tu/tu.json`
3. `~/.config/tokenusage/tokenusage.json`

指定配置文件：

```bash
tu --config /path/to/tu.json
```

示例：

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
      "brandUrl": "https://github.com/hanbu97/tokenusage"
    },
    "weekly": {
      "startOfWeek": "monday"
    }
  }
}
```

## 开发

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo check
```

## 许可证

MIT. 见 [LICENSE](./LICENSE)。
