<p align="center">
  <img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/assets/branding/tokenusage-logomark.svg" width="128" height="128" alt="tokenusage logo" />
</p>

<h1 align="center">tokenusage</h1>

<p align="center">
  <strong>Stop getting throttled without warning. Know your AI coding costs in 0.08s.</strong>
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
  English | <a href="./README.zh-cn.md">中文</a>
</p>

---

### Install in one line

```bash
# macOS / Linux (Zero dependencies):
curl --proto '=https' --tlsv1.2 -sSf https://github.com/FelixIsaac/tokenusage/releases/latest/download/tokenusage-installer.sh | sh

# Or Homebrew:
brew install FelixIsaac/tokenusage/tokenusage
```

### Run it

```bash
tu                          # daily cost report in 0.08s
```

---

<p align="center">
  <strong>214x faster</strong> than ccusage on Claude logs · <strong>138x faster</strong> on Codex logs · <a href="#benchmark-details">See benchmark</a>
</p>

<p align="center">
  If <code>tokenusage</code> saves you time, <a href="https://github.com/FelixIsaac/tokenusage">give it a star</a> — it directly helps other Codex and Claude users find it.
</p>

---

## Screenshots

<table align="center" width="100%">
  <tr>
    <td valign="top" width="50%">
      <code>tu</code> — daily report<br/>
      <p align="center">
        <a href="https://github.com/hanbu97/tokenusage/blob/main/docs/images/cli-demo-padded.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/cli-demo-padded.png" alt="tu cli demo" height="220" loading="lazy" /></a>
      </p>
    </td>
    <td valign="top" width="50%">
      <code>tu gui</code> — desktop dashboard<br/>
      <p align="center">
        <a href="https://github.com/hanbu97/tokenusage/blob/main/docs/images/gui-demo.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/gui-demo.png" alt="tu gui demo" height="220" loading="lazy" /></a>
      </p>
    </td>
  </tr>
  <tr>
    <td valign="top" width="50%">
      <code>tu img day</code> — shareable card<br/>
      <p align="center">
        <a href="https://github.com/hanbu97/tokenusage/blob/main/docs/images/share-demo.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/share-demo.png" alt="tu img daily demo" height="260" loading="lazy" /></a>
      </p>
    </td>
    <td valign="top" width="50%">
      <code>tu img week</code> — weekly card<br/>
      <p align="center">
        <a href="https://github.com/hanbu97/tokenusage/blob/main/docs/images/share-week-demo.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/share-week-demo.png" alt="tu img weekly demo" height="260" loading="lazy" /></a>
      </p>
    </td>
  </tr>
  <tr>
    <td valign="top" colspan="2">
      <code>tu live</code> — real-time TUI monitor<br/>
      <p align="center">
        <a href="https://github.com/hanbu97/tokenusage/blob/main/docs/images/live-demo.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/live-demo.png" alt="tu live demo" width="100%" loading="lazy" /></a>
      </p>
    </td>
  </tr>
</table>

## Why tokenusage

| Problem | tokenusage solution |
|---|---|
| Hit rate limits mid-refactor, no warning | `tu live` shows usage in real-time |
| No idea what AI coding costs per day | `tu` gives daily cost breakdown in 0.08s |
| Codex / Claude / Gemini / OpenCode logs in separate places | One merged dashboard across all sources |
| Existing tools are slow on large logs | 214x faster than ccusage (Rust + parallel scan + cache) |
| Don't want to upload logs to a cloud | 100% local parsing, no data leaves your machine |
| Want coding-time context, not just raw tokens | `tu` keeps the classic token table by default; `--with-activity` opt-in adds coding time and tokens/hour |
| Want to share usage stats | `tu img` generates shareable image cards |

## Supported Providers & Feature Matrix

| Provider | Data Source | Local Parsing | Real-time TUI (`tu live`/`top`) | Quota / Balance Probe | Command Shortcut |
| :--- | :--- | :---: | :---: | :---: | :--- |
| **Claude Code** | Log / JSON | ✅ | ✅ | ✅ (`tu statusline`) | `tu claude` |
| **OpenAI Codex** | Log / JSON | ✅ | ✅ | — | `tu codex` |
| **Antigravity / AGY** | Protobuf (`*.db`) | ✅ | ✅ | ✅ (`tu antigravity status`) | `tu antigravity` / `tu agy` |
| **Gemini CLI** | Log / JSON | ✅ | ✅ | — | `tu gemini` |
| **OpenCode** | Log / JSON | ✅ | ✅ | — | `tu opencode` |
| **Grok (xAI)** | Log / OAuth Proxy | ✅ | ✅ | ✅ (`tu grok`) | `tu grok` |
| **DeepSeek API** | API Balance | — | — | ✅ (`tu deepseek`) | `tu deepseek` |
| **OpenRouter API** | API Balance | — | — | ✅ (`tu openrouter`) | `tu openrouter` |
| **Kimi (Moonshot)** | API Balance | — | — | ✅ (`tu kimi`) | `tu kimi` |
| **Anthropic API** | API Usage | — | — | ✅ (`tu anthropic-api`) | `tu anthropic-api` |

## Features & Project Extensions

**[FelixIsaac/tokenusage](https://github.com/FelixIsaac/tokenusage)** was originally created by @hanbu97 and is actively maintained and expanded as an independent standalone project. Key extensions include (see [CHANGELOG.md](CHANGELOG.md) for detail):

**Performance & correctness**
- **SQLite parse cache** (`parse-cache-v3.db`) replacing the rewrite-everything JSON
  cache — incremental upserts, WAL mode (a `statusline` run can read while an
  interactive run writes), and a rayon-parallel file scan. Auto-migrates from the old
  JSON cache on first run.
- **Cache-thrash fix**: the cache key is pricing-independent and cached events are
  **re-priced at hydration**, so the 6-hourly OpenRouter pricing refresh no longer wipes
  the cache; eviction is scoped to the roots actually scanned, so a single-source run
  (e.g. `tu codex daily`) no longer evicts the others.
- `tu gui` no longer panics on window **close** (iced runs outside the tokio runtime —
  upstream issue #2).

**UX**
- **Interactive menu** on bare `tu` (category-grouped, type-to-filter, arrow-keys);
  `tu <cmd>`, piped, and non-TTY invocations are unaffected.
- **`tu statusline init`** — wires tu into Claude Code's status line with a safe JSON
  merge + backup + conflict guard (won't clobber another tool's line); `--print`
  (no-write preview), `--ccstatusline` (integration prompt for an existing status line),
  `--yes`. Plus modular `tu statusline --json` / `--field` / `--format` for ccstatusline
  widgets, sharing one per-session cache. See [docs/statusline.md](https://github.com/hanbu97/tokenusage/blob/main/docs/statusline.md).
- **`--brief`** — one-line headline (range · tokens · cost · top model) across
  today/daily/weekly/monthly/blocks/session.
- **Cost-centric insights** (cache `$` saved, reuse ratio, cost-concentration) and
  **`tu doctor` cache/pricing health** with a real-problems-only warnings section.
- Menu **Balances** lists only providers whose API key is configured.

**Platform**
- `tu antigravity status` on Windows fails with a clear, actionable message instead of a
  cryptic `ps`/`lsof` error (a real Windows port is tracked separately).

## Install

Choose the installation method for your environment:

### 1. 🚀 One-Line Standalone Installer (Simplest — Zero Dependencies)

No Node.js, npm, Python, or Rust toolchain required. Downloads the pre-compiled binary for your system directly.

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
# Pre-compiled binary install (Fast)
cargo binstall tokenusage --no-confirm

# Or build from source via Cargo
cargo install tokenusage --bin tu
```

### 5. 🐍 Python (pip)

```bash
pip install tokenusage
```

### 6. 🛠️ Build from Source

```bash
git clone https://github.com/FelixIsaac/tokenusage.git
cd tokenusage
cargo install --path . --bin tu
```

## Quick Start

```bash
# Daily report (default)
tu                          # classic merged token report
tu --tui                    # same report in terminal UI

# Source-specific
tu codex daily
tu claude daily
tu gemini
tu opencode
tu antigravity              # daily usage/cost table
tu antigravity status       # live plan tier + session/weekly quota %

# Provider selection (merged commands)
tu --only codex,gemini
tu weekly --sources claude,opencode

# Data source diagnostics
tu doctor
tu doctor --only opencode --json

# Date filter
tu --since 2026-02-01 --until 2026-02-28

# Weekly / monthly
tu weekly --start-of-week monday
tu monthly

# Native time views inferred from local AI usage
tu today
tu today --since 2026-05-01 --until 2026-05-05
tu activity
tu activity --days 14
tu activity --project tokenusage

# Add locally inferred coding activity columns (Coding / Tok/hr)
tu --with-activity
tu --with-activity --tui
tu live --with-activity

# Native local heartbeat collector
tu heartbeat watch .
tu heartbeat stats
tu heartbeat ping src/main.rs --write

# Live monitor (tabs: Codex / Claude / Antigravity / OpenCode / Antigravity Quota)
tu live
tu live codex
tu live claude
tu live gemini
tu live opencode
tu live antigravity

# Real-time per-session viewer (htop for tokens)
tu top
tu top --active-hours 12    # show sessions active in last 12h
tu top --active-hours 0     # show all sessions

# GUI dashboard
tu gui

# Share image card (for social posting)
tu img
tu img day
tu img week
```

### Data locations (defaults)

- Claude Code: `~/.claude/projects` (override with `--claude-projects-dir`)
- Codex CLI: `$CODEX_HOME/sessions` (fallback `~/.codex/sessions`, override with `--codex-sessions-dir`)
- Gemini CLI / Antigravity: `~/.gemini/tmp` (legacy Gemini CLI jsonl transcripts), plus `~/.gemini/antigravity-cli/conversations/*.db` (Antigravity's SQLite conversation databases, which carry the real token/cost accounting) and `~/.gemini/antigravity-cli/brain` (transcript logs) — override any of these with `--gemini-data-dir`
- OpenCode: `$OPENCODE_DATA_DIR` or `$XDG_DATA_HOME/opencode` (fallback `~/.local/share/opencode`, override with `--opencode-data-dir`)
- Grok Build: `~/.grok/logs` (override with `--grok-log-dir`)

`--only` / `--sources` target log-based providers (`claude`, `codex`, `gemini`, `opencode`, `grok`) — `gemini`/`antigravity`/`agy` are equivalent aliases for the same source. Note: `tu grok` is the pre-existing xAI balance-check subcommand (needs `XAI_API_KEY`) — Grok's log-based usage data is accessed via `--sources grok`/`--only grok`, not a `tu grok daily`-style shortcut like the other four sources have, since the bare word `grok` is already claimed by that balance command.

## Benchmark Details

**Setup:**

- Machine: Apple M3 Max, macOS 15.6.1
- `tu` version: `1.2.6` · `ccusage` version: `18.0.8` · `@ccusage/codex` version: `18.0.8`
- Default mode (no date filters, online pricing, network enabled)

**Codex** — 91 JSONL files, 1.7 GB (`~/.codex/sessions`)

| | `tu codex daily` | `bunx @ccusage/codex` | Speedup |
|---|---:|---:|---:|
| Cold (rebuild cache) | **0.92s** | 20.76s | **22.6x** |
| Warm (best of 5 / avg of 3) | **0.15s** | 20.76s | **138x** |

**Claude** — 1 521 JSONL files, 2.2 GB (`~/.claude/projects`)

| | `tu claude daily` | `bunx ccusage` | Speedup |
|---|---:|---:|---:|
| Cold (rebuild cache) | **0.73s** | 17.15s | **23.5x** |
| Warm (best of 5 / avg of 3) | **0.08s** | 17.15s | **214x** |

> Results vary by hardware, filesystem cache state, and log volume.

For a detailed feature comparison, see [tokenusage vs ccusage](https://github.com/hanbu97/tokenusage/blob/main/docs/compare/tokenusage-vs-ccusage.md).

## FAQ

### Where does the data come from?

From local log directories and IDE probes:
- Claude: `~/.config/claude/projects`, `~/.claude/projects`
- Codex: `~/.codex/sessions`, `~/.config/codex/sessions`
- Gemini CLI: `~/.gemini/tmp`
- Antigravity: `~/.gemini/antigravity-cli/conversations/*.db` (SQLite conversation databases carrying real token/cost accounting) and `~/.gemini/antigravity-cli/brain` (transcript logs). `tu antigravity status` additionally probes the running IDE's language server for your live plan tier and session/weekly quota %.
- OpenCode: `$OPENCODE_DATA_DIR` or `~/.local/share/opencode`
- Grok Build: `~/.grok/logs`

You can override with `--claude-projects-dir`, `--codex-sessions-dir`, `--gemini-data-dir`, `--opencode-data-dir`, and `--grok-log-dir`.

OpenCode merge semantics:
- `tu` ingests both `opencode.db` and legacy `storage/message/**/*.json` when both exist.
- Cross-source dedupe prefers stable OpenCode message ids (`msg_*`) so DB and legacy copies of the same assistant message are counted once.
- `tu doctor --only opencode --json` reports DB/legacy discovered and retained counts to debug overlap.

### How is cost estimated?

`tu` uses OpenRouter pricing when available, caches it for 6 hours, and falls back to built-in offline rates when network pricing is unavailable.

### How does `--with-activity` work?

`tu` infers coding activity locally from your machine. By default it clusters nearby AI usage events into active windows. If you enable the native heartbeat collector (`tu heartbeat watch ...`), `tu` will prefer heartbeat-backed activity on days with sufficient heartbeat coverage and fall back to token-event inference elsewhere.

From that local activity signal, `tu` derives:

- coding time
- tokens per coding hour
- cost per coding hour
- project / language / source breakdowns

The dedicated time views (`tu today`, `tu activity`) enable this automatically. `--with-activity` adds the same local activity context to daily/weekly/monthly reports and `tu live`.

By default, `tu` keeps the original merged token report layout. The extra `Coding` / `Tok/hr` columns only appear when activity context is explicitly enabled with `--with-activity`, or when you use the dedicated time views.

### What are `Insights` in reports?

`tu` now computes derived analytics for daily/weekly/monthly reports and includes them in both CLI and JSON output:

- cache/output token share
- unit efficiency (`$/1M tok`, `tok/$`)
- top source and top model concentration
- streaks and average active-day usage
- peak period, token spikes, robust anomaly detection
- provider mix by tokens/cost

Spike and anomaly entries include top source/model attribution, and when available, top project/session attribution for faster root-cause checks on heavy periods. In `tu gui`, the same insights are surfaced as summary cards.

### Is my data private?

Yes for usage logs: parsing is local. `tu` only requests pricing metadata unless you run `--offline`.

### Why did historical tokens drop after a machine cleanup?

`tu` only counts what still exists in local provider logs. If old session files were deleted/rotated by the provider, OS cleanup, or storage migration, past token totals can drop and cannot be reconstructed exactly from pricing/cache data alone.

Use `tu doctor --json` to verify current discovered roots/files. Keep periodic backups of provider log dirs (`~/.claude/projects`, `~/.codex/sessions`, `~/.gemini/tmp`, OpenCode data dir) if long-term parity matters.

## Command Overview

```text
tu [daily|today|activity|heartbeat|doctor|parity|antigravity|monthly|weekly|img|session|blocks|live|top|statusline|gui]

Provider-first grammar:
- `tu <provider> <report>` e.g. `tu codex daily`, `tu claude weekly`, `tu opencode doctor`
```

Useful commands:
- `tu --tui`
- `tu --with-activity --tui`
- `tu daily --tui`
- `tu daily --json`
- `tu daily --jq '.rows[0]'`
- `tu today`
- `tu activity --days 14`
- `tu heartbeat watch .`
- `tu heartbeat stats`
- `tu blocks --active`
- `tu blocks --live`
- `tu live`
- `tu img --output tokenusage-share.png` (today, hourly)
- `tu img --period weekly --output tokenusage-week.png` (7 days, daily)
- `tu parity --provider claude --period daily --since 20260401 --until 20260401 --json`
- `tu img --logo ./logo.png --brand-url tokenusage.dev`
- `tu statusline`

## Config File

Config search order:
1. `./.tu/tu.json`
2. `~/.config/tu/tu.json`
3. `~/.config/tokenusage/tokenusage.json`

Use an explicit config file:

```bash
tu --config /path/to/tu.json
```

Example:

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
    },
    "weekly": {
      "startOfWeek": "monday"
    }
  }
}
```

## Pricing

```bash
tu --pricing-file ./pricing.json
```

Offline-only mode:

```bash
tu --offline
```

## Demo Dataset (No Real Data)

```bash
python3 examples/demo/generate_demo_data.py
tu daily --config ./examples/demo/tu.demo.json --since 2026-02-09 --until 2026-02-28
tu live --config ./examples/demo/tu.demo.json
tu gui --config ./examples/demo/tu.demo.json --since 2026-02-09 --until 2026-02-28
tu img --config ./examples/demo/tu.demo.json --since 2026-02-28 --until 2026-02-28 --output ./docs/images/share-demo.png
tu img --config ./examples/demo/tu.demo.json --period weekly --since 2026-02-22 --until 2026-02-28 --output ./docs/images/share-week-demo.png
```

## Development

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo check
```

Release notes: see [CHANGELOG.md](CHANGELOG.md).

## License

MIT. See [LICENSE](https://github.com/hanbu97/tokenusage/blob/main/LICENSE).
