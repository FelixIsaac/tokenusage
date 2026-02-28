# tokenusage (`tu`)

Fast **Rust** token usage tracker for **Claude Code** and **Codex**.

`tu` scans local session logs, merges multiple sources, estimates cost, and shows results in:
- CLI table (responsive columns)
- TUI (sticky header + scroll)
- GUI (`iced` + `tiny-skia`)
- Live session monitor with progress bars

This project is built for people searching for:
**Claude Code usage tracker**, **Codex usage monitor**, **LLM token cost dashboard**, **Rust TUI/GUI token analytics**.

## Quick Start

```bash
# Run daily report (default command)
tu

# Only Codex / only Claude
tu codex
tu claude

# Date filter
tu --since 2026-02-01 --until 2026-02-28

# Weekly / monthly
tu weekly --start-of-week monday
tu monthly

# Live block monitor
tu live
tu live codex
tu live claude

# GUI dashboard
tu gui
```

## Install

### From source

```bash
cargo install --path . --bin tu --force
```

### From crates.io (after publish)

```bash
cargo install tokenusage --bin tu
```

## What `tu` Tracks

- Input tokens
- Output tokens
- Cache create tokens
- Cache read tokens
- Total tokens
- Estimated cost (USD)
- Per-model merged view (`claude:<model>`, `codex:<model>`)

## Data Sources

By default, `tu` checks these directories and merges all valid logs:

- Claude:
  - `~/.config/claude/projects`
  - `~/.claude/projects`
- Codex:
  - `~/.codex/sessions`
  - `~/.config/codex/sessions`

You can override with:
- `--claude-projects-dir <PATH>` (repeatable)
- `--codex-sessions-dir <PATH>` (repeatable)

## Command Overview

```text
tu [daily|codex|claude|monthly|weekly|session|blocks|live|statusline|gui]
```

Useful commands:
- `tu daily --tui`: terminal table UI with sticky header
- `tu daily --json`
- `tu daily --jq '.rows[0]'`
- `tu blocks --active`
- `tu blocks --live`
- `tu live`: same intent as live block monitor
- `tu statusline`: statusline output (cache-aware)

## Config File

Config search order:
1. `./.tu/tu.json`
2. `~/.config/tu/tu.json`
3. `~/.config/tokenusage/tokenusage.json`

You can also pass explicit config:

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
    "weekly": {
      "startOfWeek": "monday"
    }
  }
}
```

## Pricing

- Uses OpenRouter model pricing when available.
- Pricing cache TTL: **6 hours**.
- Falls back to built-in offline rate table if network data is unavailable.
- Optional override file:

```bash
tu --pricing-file ./pricing.json
```

Use offline-only mode:

```bash
tu --offline
```

## Performance Notes

- Parallel file discovery via `ignore` crate
- Parallel parsing with `rayon` + worker threads + channels
- Incremental parse cache for repeated runs

Built-in heavy directory ignores include: `.git`, `node_modules`, `target`, `dist`, `build`, `.cache`, `venv`...

## Troubleshooting

- If no data appears, pass explicit roots:
  - `--claude-projects-dir ...`
  - `--codex-sessions-dir ...`
- If your terminal is narrow, `tu` auto-switches to compact columns.
- Use `--debug` to print parser stats.

## Development

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo check
```

## License

MIT. See [LICENSE](./LICENSE).
