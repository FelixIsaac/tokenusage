# AGENTS.md — Developer & AI Agent Guidelines

Welcome AI agents and human contributors! This file defines the repository architecture, coding invariants, testing workflows, and release automation guidelines for **tokenusage** (`tu`).

---

## 🎯 Mission & Core Philosophy

**tokenusage** (`tu`) is a blazing-fast, 100% local telemetry aggregator, cost calculator, and real-time rate-limit shield for AI coding assistants.

1. **Zero-Cloud Local Privacy:** 100% local log and database parsing. No user transcripts or token logs ever leave the local machine.
2. **Blazing Speed (< 0.08s):** Rayon-parallel file scanning and SQLite WAL caching (`parse-cache-v3.db`) deliver reports 214x faster than alternative tools.
3. **Multi-Agent Parity:** Equal first-class support across all **10 AI providers** (Claude Code, OpenAI Codex, Antigravity/AGY, Gemini CLI, OpenCode, Grok, DeepSeek API, OpenRouter API, Kimi Moonshot, Anthropic API).

---

## 🏗️ Repository Architecture

```
tokenusage/
├── Cargo.toml               # Rust package & dependency manifest (version source of truth)
├── Cargo.lock               # Lockfile
├── src/
│   ├── main.rs              # Binary entrypoint
│   ├── lib.rs               # Library root & CLI subcommand dispatch logic
│   ├── cli.rs               # Clap CLI argument structures & subcommand definitions
│   ├── config.rs            # Configuration file loading (~/.config/tu/tu.json)
│   ├── types.rs             # Core data types (TokenCounts, PricingTable, ProviderArg, SourceKind)
│   └── pipeline/
│       ├── mod.rs           # Pipeline module root
│       ├── parsing.rs       # JSONL / Protobuf / Log parsing algorithms
│       ├── official.rs      # API & OAuth token probes (Grok, Antigravity, DeepSeek, etc.)
│       └── sqlite.rs        # SQLite parse cache engine (parse-cache-v3.db)
├── npm/tu/                  # npm wrapper package
├── pypi/tu/                 # PyPI wrapper package
├── scripts/                 # Sync & release helper scripts
│   ├── sync-npm-readme.sh
│   ├── sync-pypi-readme.sh
│   ├── sync-npm-version.sh
│   └── sync-pypi-version.sh
├── .github/workflows/
│   ├── ci.yml               # Format, Clippy linting, and unit test suite
│   └── release.yml          # cargo-dist release workflow for GitHub Releases & Homebrew
├── README.md                # Main documentation (English)
├── README.zh-cn.md          # Main documentation (Chinese)
├── CHANGELOG.md             # Version release history
└── ROADMAP.md               # Strategic product roadmap
```

---

## ⚠️ Critical Invariants for AI Agents

When implementing new features or fixing bugs in this repository, strictly observe the following rules:

1. **Do Not Break Multi-Provider Parity:**
   * If adding a CLI flag or feature (e.g. `--warn-threshold`), ensure it applies across all relevant providers.
   * If adding a new subcommand shortcut, update `ProviderArg` in `src/types.rs`, `src/cli.rs`, `README.md`, and `README.zh-cn.md`.

2. **Always Sync npm and PyPI Package Files:**
   * Version bumps in `Cargo.toml` MUST be synced to `npm/tu/package.json` and `pypi/tu/tokenusage/__init__.py`.
   * After modifying `README.md`, ALWAYS run:
     ```bash
     ./scripts/sync-npm-readme.sh && ./scripts/sync-pypi-readme.sh && ./scripts/sync-npm-version.sh && ./scripts/sync-pypi-version.sh
     ```

3. **Strict Code Quality & Clippy Verification:**
   * Before committing, run and pass:
     ```bash
     cargo fmt
     cargo clippy --all-targets --all-features
     cargo test
     ```
   * Avoid `is_some()` followed by `.unwrap()` — use `if let Some(...)` or `match` constructs.

4. **Non-Interactive Automated Commits:**
   * When creating git commits or tags via automated tools/scripts, pass `--no-gpg-sign` to `git commit` and `--no-sign` to `git tag` to avoid gpg passphrase prompts blocking non-interactive execution.

---

## 🧪 Developer Commands

| Task | Command |
| :--- | :--- |
| **Run Unit Tests** | `cargo test` |
| **Run Clippy Lints** | `cargo clippy --all-targets --all-features` |
| **Format Code** | `cargo fmt` |
| **Check Build** | `cargo check` |
| **Sync All Package Files** | `./scripts/sync-npm-readme.sh && ./scripts/sync-pypi-readme.sh && ./scripts/sync-npm-version.sh && ./scripts/sync-pypi-version.sh` |
| **Dry Run npm Package** | `cd npm/tu && npm pack --dry-run` |
