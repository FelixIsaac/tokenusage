# Status line integration

`tu statusline` turns your token usage into a status bar. It reads the Claude
Code status-line **hook JSON** on stdin (session id, model, transcript path,
cost, context window) and prints cost/quota info that the editor's built-in
status widgets can't show.

## Quick setup: `tu statusline init`

Don't hand-edit JSON — run:

```bash
tu statusline init
```

It finds `~/.claude/settings.json`, backs it up, adds the `statusLine` block
(standalone mode), and asks you to confirm — **merging** so every other setting
is preserved. If a `statusLine` is already set by another tool (ccstatusline, a
custom script), it **won't overwrite it**; it stops and tells you how to compose
instead. Flags:

| Flag | What it does |
|---|---|
| *(none)* | Wire in standalone mode, with a confirm prompt |
| `--print` | Show the settings block, write nothing (safe preview — good for AI agents) |
| `--ccstatusline` | Print a copy-paste **prompt** to hand your AI assistant to integrate tu into your *existing* ccstatusline / custom line |
| `--yes` | Skip the confirm (non-interactive / agent use) |
| `--flags "<...>"` | Override the baked-in `--cache --refresh-interval 30` |

The two modes below are what `init` wires up — read on if you want the details
or prefer to edit by hand.

## 1. Standalone (simplest)

Point Claude Code's `statusLine.command` at `tu` and you're done — one line, zero
extra tooling. `tu statusline init` writes this for you; or in
`~/.claude/settings.json`:

```json
{
  "statusLine": {
    "type": "command",
    "command": "tu statusline --cache --refresh-interval 30"
  }
}
```

Output:

```
model claude-opus-4-8 | session $12.34 | today $22.41 | block $6.94 (1h 10m left) | ctx 90,000 (45%, low)
```

Add `--official-limits` for Codex/Claude 5h/week quota, `-B emoji` for a burn-rate
indicator. `--cache --refresh-interval N` means the expensive parse runs at most
once every `N` seconds; in between, repaints read the cache.

## 2. With [ccstatusline](https://github.com/sirmalloc/ccstatusline) (composable)

If you want styled, composable widgets, let `tu` *feed data* into your status
line instead of owning the whole line. The fastest way to wire this is to run
`tu statusline init --ccstatusline` and paste the prompt it prints to your AI
assistant — it'll edit your ccstatusline config for you. Under the hood,
`tu statusline` has three structured output modes:

| Mode | Example | Output |
|---|---|---|
| `--json` | `tu statusline --json` | `{"model":"claude-opus-4-8","today_cost_usd":22.18,"block_cost_usd":6.71,"block_remaining_min":70,"burn_status":"moderate","context_pct":45.0,...}` |
| `--field <name>` | `tu statusline --field today-cost` | `$22.18` |
| `--format <tmpl>` | `tu statusline --format "{today-cost} ⛽{burn-status}"` | `$22.18 ⛽moderate` |

**Field names:** `model`, `session-cost`, `today-cost`, `block-cost`,
`block-left`, `burn-hourly`, `burn-per-min`, `burn-status`, `ctx-pct`,
`ctx-level`. `--json` keys are the snake_case raw values (numbers, not formatted)
so a widget can style them itself.

### Performance — read this before wiring multiple widgets

Widgets don't make `tu` faster; **caching does.** Every mode honors
`--cache --refresh-interval N`, and they share **one** fields cache per session,
so the full parse happens at most once per interval no matter how many fields you
read.

The thing to avoid is **N process spawns per repaint** (one per widget, on every
message). Two good patterns:

- **Best — one call, read the file.** Have a single widget (or a pre-render
  step) run `tu statusline --json --cache --refresh-interval 30`, then have your
  other widgets read the cached JSON file directly (no `tu` spawn). The cache is
  shared across all your Claude sessions (keyed by session id), so only one
  triggers the reparse per interval.
- **Simple — one `--field` call per widget.** Each
  `tu statusline --field X --cache` spawns `tu`, but they share the fields cache,
  so only the first within the interval parses; the rest are cheap cache reads.
  Fine for a few widgets.

`tu`'s SQLite parse cache keeps that periodic reparse stable (~1s on a large
history) instead of a multi-second invalidation storm, so the refresh stays cheap.

## Recommendation

- Just want a quick cost line? Use **standalone** (`tu statusline --cache`).
- Already invested in **ccstatusline** and want styled, composable widgets? Use
  `--json` (parse once) or `--field` (one value per widget), and follow the
  caching patterns above. `tu` supplies the historical cost/quota/burn data that
  ccstatusline's built-ins don't track.
