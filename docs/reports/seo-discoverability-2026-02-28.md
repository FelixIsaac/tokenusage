# Tokenusage SEO & Discoverability Validation Report

Date: 2026-02-28 (Asia/Shanghai)
Release target: 1.1.x patch
Published version: 1.1.5

## 1) Unified Metadata Lexicon

Canonical keyword set (human-readable):
- codex usage
- claude code usage
- token usage tracker
- rust cli
- tui
- gui

Platform mapping:
- crates.io keywords (slug + max-5 constraint):
  - codex-usage
  - claude-code-usage
  - token-usage-tracker
  - rust-cli
  - tui-gui
- npm keywords:
  - codex usage
  - claude code usage
  - token usage tracker
  - rust cli
  - tui
  - gui

## 2) Metadata Changes Applied

### Cargo.toml
- version: 1.1.5
- description: Fast Rust CLI/TUI/GUI token usage tracker for Codex usage and Claude Code usage.
- keywords: [codex-usage, claude-code-usage, token-usage-tracker, rust-cli, tui-gui]
- categories: [command-line-utilities, development-tools]
- homepage: https://github.com/hanbu97/tokenusage
- repository: https://github.com/hanbu97/tokenusage
- documentation: https://docs.rs/tokenusage

### npm/tu/package.json
- version: 1.1.5
- description: Fast Rust CLI/TUI/GUI token usage tracker for Codex usage and Claude Code usage.
- keywords: [codex usage, claude code usage, token usage tracker, rust cli, tui, gui]
- homepage: https://github.com/hanbu97/tokenusage
- repository: git+https://github.com/hanbu97/tokenusage.git
- bugs: https://github.com/hanbu97/tokenusage/issues

### README first sentence consistency
- Root README first sentence: Fast Rust CLI/TUI/GUI token usage tracker for Codex usage and Claude Code usage.
- npm README first sentence: Fast Rust CLI/TUI/GUI token usage tracker for Codex usage and Claude Code usage.
- Cargo / npm description: same sentence as above.

Consistency status: PASS

## 3) Publish Execution

- crates.io publish command: `cargo publish --allow-dirty`
- npm publish command: `cd npm/tu && npm publish`
- Result: `tokenusage` v1.1.5 published on both registries.

## 4) Post-Publish Verification

### crates.io (`cargo info tokenusage`)
Command run from `/tmp` (to avoid local package shadowing):

```bash
cargo info tokenusage
```

Observed:
- version: 1.1.5
- description: Fast Rust CLI/TUI/GUI token usage tracker for Codex usage and Claude Code usage.
- keywords: codex-usage, claude-code-usage, token-usage-tracker, rust-cli, tui-gui
- homepage/repository/documentation: all match expected

Verification status: PASS

### npm (`npm view tokenusage@latest ...`)
Command:

```bash
npm view tokenusage@latest version description keywords homepage repository bugs --json --registry=https://registry.npmjs.org/
```

Observed:
- version: 1.1.5
- description: Fast Rust CLI/TUI/GUI token usage tracker for Codex usage and Claude Code usage.
- keywords: codex usage, claude code usage, token usage tracker, rust cli, tui, gui
- homepage/repository/bugs: all match expected shape

Verification status: PASS

## 5) 7-Day Metrics Snapshot

Metric window:
- npm downloads API window (if available): last-week
- GitHub stars/week: last 7 days from 2026-02-28T15:26:15Z cutoff

Current snapshot:
- npm weekly downloads: unavailable (downloads API currently returns `{"error":"package tokenusage not found"}`)
- crates total downloads: 0
- GitHub stars/week: 1

Data sources:
- npm downloads API: `https://api.npmjs.org/downloads/point/last-week/tokenusage`
- crates API: `https://crates.io/api/v1/crates/tokenusage`
- GitHub API stargazers: `https://api.github.com/repos/hanbu97/tokenusage/stargazers`

## 6) Next Keyword Iteration Suggestions

- Keep the current six-core keywords stable for at least 14 days to avoid noisy attribution.
- In next patch cycle, test one long-tail swap on npm only:
  - candidate add: `llm usage tracker`
  - candidate add: `codex token usage`
  - candidate remove (if needed): `gui` (lowest intent specificity)
- If GitHub/README traffic becomes primary, add one natural-language sentence in README intro containing:
  - "codex usage tracker"
  - "claude code usage tracker"
  (without adding more package keywords).

## 7) Acceptance Check

- Three-end description consistency: PASS
- Core search term coverage: PASS
- CLI verification commands return expected metadata: PASS
