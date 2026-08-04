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

## ⚡ Overview

**tokenusage** (`tu`) is a blazing-fast, 100% local CLI, TUI, and GUI dashboard for monitoring AI coding tool usage and token costs. 

Parsing local logs in under **0.08 seconds** (214x faster than alternatives), `tu` unifies usage metrics across **Claude Code, OpenAI Codex, Antigravity/AGY, Gemini CLI, OpenCode, Grok, DeepSeek, OpenRouter, Kimi, and Anthropic API** into a single local control plane.

---

## 📸 Interface Screenshots

<table align="center" width="100%">
  <tr>
    <td valign="top" width="50%">
      <code>tu</code> — Daily Cost Report<br/>
      <p align="center">
        <a href="https://github.com/hanbu97/tokenusage/blob/main/docs/images/cli-demo-padded.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/cli-demo-padded.png" alt="tu cli demo" height="220" loading="lazy" /></a>
      </p>
    </td>
    <td valign="top" width="50%">
      <code>tu gui</code> — Desktop Dashboard<br/>
      <p align="center">
        <a href="https://github.com/hanbu97/tokenusage/blob/main/docs/images/gui-demo.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/gui-demo.png" alt="tu gui demo" height="220" loading="lazy" /></a>
      </p>
    </td>
  </tr>
  <tr>
    <td valign="top" width="50%">
      <code>tu img day</code> — Daily Social Card<br/>
      <p align="center">
        <a href="https://github.com/hanbu97/tokenusage/blob/main/docs/images/share-demo.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/share-demo.png" alt="tu img daily demo" height="260" loading="lazy" /></a>
      </p>
    </td>
    <td valign="top" width="50%">
      <code>tu img week</code> — Weekly Social Card<br/>
      <p align="center">
        <a href="https://github.com/hanbu97/tokenusage/blob/main/docs/images/share-week-demo.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/share-week-demo.png" alt="tu img weekly demo" height="260" loading="lazy" /></a>
      </p>
    </td>
  </tr>
  <tr>
    <td valign="top" colspan="2">
      <code>tu live</code> — Real-time TUI Monitor<br/>
      <p align="center">
        <a href="https://github.com/hanbu97/tokenusage/blob/main/docs/images/live-demo.png"><img src="https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/thumbs/live-demo.png" alt="tu live demo" width="100%" loading="lazy" /></a>
      </p>
    </td>
  </tr>
</table>

---

## 🎯 Why tokenusage?

| Pain Point | tokenusage (`tu`) Solution |
| :--- | :--- |
| **Throttled mid-refactor without warning** | `tu live` shows real-time usage & quota remaining |
| **No idea what AI coding costs daily** | `tu` gives an instant cost breakdown in 0.08s |
| **Logs scattered across multiple AI tools** | One merged dashboard across 10 AI providers |
| **Existing log tools are slow on large logs** | 214x faster than ccusage (Rust + Rayon + SQLite WAL) |
| **Privacy concerns with cloud tracking** | 100% local parsing, zero telemetry leaves your machine |
| **Need active coding time context** | `--with-activity` adds coding hours and tokens/hour context |
| **Sharing progress with team/socials** | `tu img` generates high-res shareable image cards |

---

## 🤖 Supported Providers & Feature Matrix

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

---

## 🚀 Installation

Choose the installation method for your environment:

### 1. Standalone Installer (Simplest — Zero Dependencies)

No Node.js, npm, Python, or Rust toolchain required. Downloads pre-compiled binary directly.

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

---

## 💡 Quick Start & Usage Examples

```bash
# Daily cost report (Default)
tu

# Interactive TUI mode
tu --tui

# Provider-specific commands
tu codex daily
tu claude weekly
tu antigravity
tu antigravity status      # Live plan tier + session/weekly quota %

# Filter specific sources
tu --only codex,gemini
tu weekly --sources claude,opencode

# Shell completions setup
tu completions zsh > ~/.zsh/completion/_tu
tu completions bash > ~/.bash_completion.d/tu

# Quota warning threshold alerts
tu antigravity status --warn-threshold 15

# Real-time process monitor (htop for AI tokens)
tu live
tu top

# Desktop GUI dashboard
tu gui

# Generate shareable card
tu img day
tu img week
```

---

## ⚡ Performance Benchmarks

**Setup:** Apple M3 Max, macOS 15.6.1 · `tu` v1.11.2 vs `ccusage` v18.0.8

### Codex — 91 JSONL files, 1.7 GB (`~/.codex/sessions`)

| Mode | `tu codex daily` | `bunx @ccusage/codex` | Speedup |
| :--- | :---: | :---: | :---: |
| **Cold (rebuild cache)** | **0.92s** | 20.76s | **22.6x** |
| **Warm (best of 5)** | **0.15s** | 20.76s | **138x** |

### Claude — 1,521 JSONL files, 2.2 GB (`~/.claude/projects`)

| Mode | `tu claude daily` | `bunx ccusage` | Speedup |
| :--- | :---: | :---: | :---: |
| **Cold (rebuild cache)** | **0.73s** | 17.15s | **23.5x** |
| **Warm (best of 5)** | **0.08s** | 17.15s | **214x** |

---

## 📁 Local Data Locations

| Provider | Default Log Location | Override Flag |
| :--- | :--- | :--- |
| **Claude Code** | `~/.claude/projects` | `--claude-projects-dir` |
| **OpenAI Codex** | `$CODEX_HOME/sessions` (or `~/.codex/sessions`) | `--codex-sessions-dir` |
| **Antigravity / Gemini** | `~/.gemini/antigravity-cli/conversations/*.db` & `~/.gemini/tmp` | `--gemini-data-dir` |
| **OpenCode** | `$OPENCODE_DATA_DIR` (or `~/.local/share/opencode`) | `--opencode-data-dir` |
| **Grok Build** | `~/.grok/logs` & `~/.grok/auth.json` | `--grok-log-dir` |

---

## ⚙️ Configuration File (`tu.json`)

`tu` looks for configuration in:
1. `./.tu/tu.json`
2. `~/.config/tu/tu.json`
3. `~/.config/tokenusage/tokenusage.json`

Example `~/.config/tu/tu.json`:

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

## 🙏 Acknowledgements & Credits

**tokenusage** was originally created by **[@hanbu97](https://github.com/hanbu97)** (`hanbu97/tokenusage`), who designed the original architecture and foundation for local AI log parsing. 

All core credit for creating the initial tool belongs to the original author. This repository (**[FelixIsaac/tokenusage](https://github.com/FelixIsaac/tokenusage)**) actively maintains, optimizes, and expands the project with multi-agent support, cross-platform installers, and real-time quota telemetry.

---

## 📄 License

Distributed under the MIT License. See [`LICENSE`](https://github.com/hanbu97/tokenusage/blob/main/LICENSE) for details.
