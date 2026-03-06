<p align="center">
  <img src="assets/branding/tokenusage-logomark.svg" width="128" height="128" alt="tokenusage logo" />
</p>

<h1 align="center">tokenusage</h1>

<p align="center">
  <em>Fast Rust CLI/TUI/GUI token usage tracker for Codex, Claude Code, and Antigravity</em>
</p>

<p align="center">
  <a href="https://github.com/hanbu97/tokenusage/actions/workflows/ci.yml"><img src="https://github.com/hanbu97/tokenusage/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/hanbu97/tokenusage/actions/workflows/release.yml"><img src="https://github.com/hanbu97/tokenusage/actions/workflows/release.yml/badge.svg" alt="Release" /></a>
  <a href="https://crates.io/crates/tokenusage"><img src="https://img.shields.io/crates/v/tokenusage?color=orange" alt="crates.io" /></a>
  <a href="https://www.npmjs.com/package/tokenusage"><img src="https://img.shields.io/npm/v/tokenusage?color=red" alt="npm" /></a>
  <a href="https://pypi.org/project/tokenusage/"><img src="https://img.shields.io/pypi/v/tokenusage?color=blue" alt="PyPI" /></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="License" /></a>
</p>

<p align="center">
  <strong>214x faster</strong> than ccusage on Claude logs · <strong>138x faster</strong> on Codex logs (warm cache) · <a href="#benchmark-details">See benchmark</a>
</p>

<p align="center">
  <a href="#screenshots">Screenshots</a> · <a href="#install">Install</a> · <a href="#quick-start">Quick Start</a> · <a href="#why-tokenusage">Why</a> · <a href="#benchmark-details">Benchmark</a>
</p>

---

## Screenshots

<table align="center" width="100%">
  <tr>
    <td valign="top" width="50%">
      <code>tu</code><br/>
      <p align="center">
        <a href="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/cli-demo-padded.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/cli-demo-padded.png" alt="tu cli demo" height="220" loading="lazy" /></a>
      </p>
    </td>
    <td valign="top" width="50%">
      <code>tu gui</code><br/>
      <p align="center">
        <a href="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/gui-demo.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/gui-demo.png" alt="tu gui demo" height="220" loading="lazy" /></a>
      </p>
    </td>
  </tr>
  <tr>
    <td valign="top" width="50%">
      <code>tu img day</code><br/>
      <p align="center">
        <a href="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/share-demo.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/share-demo.png" alt="tu img daily demo" height="260" loading="lazy" /></a>
      </p>
    </td>
    <td valign="top" width="50%">
      <code>tu img week</code><br/>
      <p align="center">
        <a href="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/share-week-demo.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/share-week-demo.png" alt="tu img weekly demo" height="260" loading="lazy" /></a>
      </p>
    </td>
  </tr>
  <tr>
    <td valign="top" colspan="2">
      <code>tu live</code><br/>
      <p align="center">
        <a href="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/live-demo.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/live-demo.png" alt="tu live demo" width="100%" loading="lazy" /></a>
      </p>
    </td>
  </tr>
</table>

## Install

### cargo (crates.io)

```bash
cargo install tokenusage --bin tu
```

### npm

```bash
npm install -g tokenusage
```

### pip (PyPI)

```bash
pip install tokenusage
```

### cargo-binstall (prebuilt binary)

```bash
cargo binstall tokenusage --no-confirm
```

## Quick Start

```bash
# Daily report (default)
tu

# Source-specific
tu codex
tu claude
tu antigravity

# Date filter
tu --since 2026-02-01 --until 2026-02-28

# Weekly / monthly
tu weekly --start-of-week monday
tu monthly

# Live monitor (tabs: Codex / Claude / Antigravity)
tu live
tu live codex
tu live claude
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

## Why tokenusage

- Faster feedback loop: native Rust + parallel scan/parsing + incremental cache.
- One dashboard for Codex, Claude, and Antigravity, with merged totals and per-model breakdown.
- Share-ready image card (`tu img`) for posting your token/cost trend.
- Works in terminal and desktop GUI without sending your logs to a cloud service.

## FAQ

### Where does the data come from?

From local log directories and IDE probes:
- Claude: `~/.config/claude/projects`, `~/.claude/projects`
- Codex: `~/.codex/sessions`, `~/.config/codex/sessions`
- Antigravity: probed from running IDE language server (no log files needed)

You can override with `--claude-projects-dir` and `--codex-sessions-dir`.

### How is cost estimated?

`tu` uses OpenRouter pricing when available, caches it for 6 hours, and falls back to built-in offline rates when network pricing is unavailable.

### Is my data private?

Yes for usage logs: parsing is local. `tu` only requests pricing metadata unless you run `--offline`.

## Benchmark Details

**Setup:**

- Machine: Apple M3 Max, macOS 15.6.1
- `tu` version: `1.2.6` · `ccusage` version: `18.0.8` · `@ccusage/codex` version: `18.0.8`
- Default mode (no date filters, online pricing, network enabled)

**Codex** — 91 JSONL files, 1.7 GB (`~/.codex/sessions`)

| | `tu codex` | `bunx @ccusage/codex` | Speedup |
|---|---:|---:|---:|
| Cold (rebuild cache) | **0.92s** | 20.76s | **22.6x** |
| Warm (best of 5 / avg of 3) | **0.15s** | 20.76s | **138x** |

**Claude** — 1 521 JSONL files, 2.2 GB (`~/.claude/projects`)

| | `tu claude` | `bunx ccusage` | Speedup |
|---|---:|---:|---:|
| Cold (rebuild cache) | **0.73s** | 17.15s | **23.5x** |
| Warm (best of 5 / avg of 3) | **0.08s** | 17.15s | **214x** |

> Results vary by hardware, filesystem cache state, and log volume.

## Command Overview

```text
tu [daily|codex|claude|antigravity|monthly|weekly|img|session|blocks|live|statusline|gui]
```

Useful commands:
- `tu daily --tui`
- `tu daily --json`
- `tu daily --jq '.rows[0]'`
- `tu blocks --active`
- `tu blocks --live`
- `tu live`
- `tu img --output tokenusage-share.png` (today, hourly)
- `tu img --period weekly --output tokenusage-week.png` (7 days, daily)
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
      "brandUrl": "https://github.com/hanbu97/tokenusage"
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

## License

MIT. See [LICENSE](./LICENSE).
