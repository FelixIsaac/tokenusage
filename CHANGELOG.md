# Changelog

All notable changes to this project are documented in this file.

## [Unreleased]

## [1.8.1] - 2026-06-11

### Fixed
- Interactive menu (bare `tu`) now opens in Git Bash / MINGW64. It was gated on
  `stdin().is_terminal()`, which returns false under MSYS pipe ptys even when the
  TUI works; now gated on `stdout().is_terminal()` only, matching `tu top`/`live`
  (crossterm reads key input via the console API, not std stdin).

## [1.8.0] - 2026-06-11

### Added
- **Interactive command menu.** Running bare `tu` on an interactive terminal now
  opens a category-grouped picker (arrow keys to move, type to filter, Enter to
  run, Esc to quit) instead of dumping all 22 commands. Power users are
  unaffected: `tu today`, `tu monthly`, etc. still run directly, and `tu` with
  any args (e.g. `tu --json`) or in a pipe/non-TTY keeps the daily-report
  default so scripts don't break.

## [1.7.0] - 2026-06-11

### Added
- `--brief`: a one-line headline (range · tokens · cost · top model) instead of
  the full table, for quick muscle-memory checks. Works across `today`, `daily`,
  `weekly`, `monthly`, `blocks`, `session`.

### Fixed
- The `unknown-pricing` stat (`--debug` / `doctor --pricing-debug`) is now
  recomputed against the *current* pricing table when serving cache hits
  (pre-filter, matching the parser) instead of replaying the cached count, so it
  no longer lags after a model becomes known/unknown.

## [1.6.2] - 2026-06-11

### Fixed
- Re-price-on-hydrate now mirrors the parser exactly for **unknown models**:
  an unknown model re-prices to $0 (matching the parser's `None => 0.0`) instead
  of keeping the stale cached cost, so cached and freshly-parsed events can't
  drift. Pinned by a test.

## [1.6.1] - 2026-06-11

### Fixed
- **Parse cache thrash.** The cache was nearly useless: entry count oscillated
  (e.g. 19k→1.9k) and warm runs hit periodic full-reparse storms. Two causes:
  (1) the cache-invalidation key embedded the entire pricing table, so every 6h
  OpenRouter refresh (or online/offline flip) wiped the cache — fixed by making
  the key pricing-independent and **re-pricing cached events at hydration** from
  their stored token counts (costs stay correct without tying cache validity to
  volatile pricing); (2) the eviction pass was global, so a scoped run (e.g.
  `tu codex daily`, or `doctor`'s opencode-only probe) evicted every other
  source's entries — fixed by **scoping eviction to roots actually scanned**.
  Result: cache stabilizes at ~all files, `doctor` no longer wipes it, warm
  `tu today` is a consistent ~1s. One-time rebuild on first run after upgrade.
- `doctor` cache entry-count now reads WAL-resident rows correctly.

## [1.6.0] - 2026-06-11

Performance and UX release. The headline is a SQLite-backed parse cache that
replaces the monolithic JSON cache (rewritten wholesale on every run), plus a
parallelized file scan and a categorized help menu.

### Added
- **Cost-centric insights**: `cache saved ~$X` (counterfactual net savings from
  prompt caching vs. paying full input price for cached reads, using canonical
  per-model rates, suppressed below 60% priced-token coverage), cache reuse-ratio
  warning on churn (<1×), and cost-concentration callout when a source's cost
  share outruns its token share by ≥15pp. All surfaced conditionally to keep the
  `Insights:` line signal-dense.
- **`tu doctor` cache health**: parse-cache path, size, and entry count, plus a
  `warnings:` section that flags only real problems (oversized cache, stale
  pricing cache, a source that discovered files but retained zero events).
- Every subcommand now has a one-line description; `--help` groups commands by
  category (Reporting / Live / Integration / Diagnostics / Balances).

### Changed
- **Parse cache is now SQLite** (`parse-cache-v3.db`) instead of
  `parse-cache-v2.json`. Saves are incremental — only changed rows are upserted
  and evicted rows deleted — so the per-run write cost drops from ~60 MB to a few
  KB while actively coding. WAL mode lets a `statusline` run alongside an
  interactive run without blocking. Existing users are migrated automatically:
  the first run imports `parse-cache-v2.json` into SQLite, then deletes it.
- The per-file fingerprint + cache-hydration scan over all discovered files is
  now parallelized with rayon (warm `tu today` ~2.5 s → ~1.65 s on a 19.6k-file
  install).
- Tables size columns to content instead of stretching to fill the terminal, and
  pick the richest layout that actually fits — cache columns now appear at ≥130
  cols instead of >160.

### Fixed
- `tu gui` no longer panics when the window is **closed**: iced now runs on the
  main thread outside the tokio runtime, so iced's internal runtime is no longer
  dropped inside an outer async context (upstream issue #2).

### Notes
- Deliberately did not add directory-mtime subtree skipping to discovery: a
  directory's mtime does not change when an existing append-only `.jsonl` grows,
  so it would silently miss new tokens. Per-file stat is required for correctness.

### Earlier (pre-1.6.0, previously unreleased)

#### Added
- Report insights pipeline for daily/weekly/monthly outputs, including cache/output share, efficiency metrics, peak periods, streaks, spikes, anomalies, and provider mix.
- GUI report caching for faster switching between `daily`, `weekly`, and `monthly` views.
- GUI insights summary cards for top source, top model, peak period, and anomaly.
- `today` supports explicit `--since` / `--until` ranges.

#### Changed
- CLI `Insights:` summary now includes spike/anomaly attribution with top source/model.
- Spike/anomaly attribution now includes top project/session per period when event context is available.
- TOTAL-row model summary now reports provider-aware model counts.

#### Fixed
- OpenCode ingestion improved for DB + legacy merge behavior and session id stability.
- `tu gui` startup panic on Windows async runtime teardown.
