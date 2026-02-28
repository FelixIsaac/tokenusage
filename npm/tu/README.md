# tokenusage (`tu`)

Blazing-fast Rust token usage tracker for **Claude Code** and **Codex**.

- Unified report across Claude + Codex
- Fast parallel scan + cache
- CLI table, TUI, GUI (`iced` + `tiny-skia`)
- Live monitor with progress bars

Repository: [github.com/hanbu97/tokenusage](https://github.com/hanbu97/tokenusage)
Crate: [crates.io/crates/tokenusage](https://crates.io/crates/tokenusage)

## Install

```bash
npm install -g tokenusage
```

## Quick Start

```bash
# Daily report (default)
tu

# Source-specific
tu codex
tu claude

# Date filter
tu --since 2026-02-01 --until 2026-02-28

# Weekly / monthly
tu weekly --start-of-week monday
tu monthly

# Live monitor
tu live
tu live codex
tu live claude

# GUI dashboard
tu gui
```

## Screenshots

![CLI demo](https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/cli-demo.png)
![GUI demo](https://raw.githubusercontent.com/hanbu97/tokenusage/main/docs/images/gui-demo.png)

## Why `tu`

- Real-world benchmark vs `ccusage` on local Codex logs:
- About **34.5x faster (cold)** and **131.1x faster (warm)**
- Rust-native startup, parallel parsing, incremental cache reuse

## Data Sources (default)

- Claude:
- `~/.config/claude/projects`
- `~/.claude/projects`
- Codex:
- `~/.codex/sessions`
- `~/.config/codex/sessions`

You can override with:

- `--claude-projects-dir <PATH>` (repeatable)
- `--codex-sessions-dir <PATH>` (repeatable)

## Config

Default config search:

1. `./.tu/tu.json`
2. `~/.config/tu/tu.json`
3. `~/.config/tokenusage/tokenusage.json`

Explicit config:

```bash
tu --config /path/to/tu.json
```

## License

MIT
