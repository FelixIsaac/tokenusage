# Changelog

All notable changes to this project are documented in this file.

## [1.11.4] - 2026-08-27

### Added
- **Environmental Telemetry & Carbon Footprint Report (`tu carbon`):** Calculates IT + datacenter electrical energy (kWh), grid operational carbon emissions (kg CO₂e), cooling water draw (Liters), and eco-efficiency ratings across LLM token workloads.
- **Physics Engine & Data Provenance Framework:** Implemented EcoLogits/ML.ENERGY GPU physics formulas separating prefill matrix multiplication from autoregressive output decoding and KV prompt cache reuse. Includes transparency disclosures (`tu carbon about`).
- **Dynamic Multi-Tier "Wow Factor" Equivalences:** Dynamic human-scale comparisons that automatically scale depending on workload volume (Smartphone charges, Kettle boiling, EV driving, US Home Powering days, Transatlantic economy flights NYC ✈️ London, Datacenter cooling bathtubs, and Tree-years of forest CO₂ absorption).
- **Carbon Period Subcommands & All-Time Reporting:** `tu carbon today`, `tu carbon daily`, `tu carbon weekly`, `tu carbon monthly`, and `tu carbon all` (or `tu carbon all-time`).
- **Regional Grid Intensity Selection (`--region`):** Support for `us-east` (Virginia), `us-west` (Oregon hydro), `us-avg`, `eu-west` (France nuclear), `nordic` (Iceland/Norway hydro), `google-cfe` (24/7 CFE matched), and `global` grid regions.
- **Interactive TUI Menu Integration:** Added `carbon` directly to the interactive Ratatui command menu (`tu` bare launcher) and support for `--tui` interactive sticky header scrolling.
- **Grok 4.6 Pricing Rate:** Added official pricing prefix and `-build` alias normalization for `grok-4.6-build` and `grok-4.5-build`.
- **Ecosystem Roadmap & Governance Matrix:** Added comprehensive provider status, Cursor CLI priority roadmap, and community PR contribution policy.

### Fixed
- **Grok Build Session Discovery & Parsing:** Switched Grok source discovery from transient debug logs (`~/.grok/logs/unified.jsonl`) to persistent session directories (`~/.grok/sessions/*/*/updates.jsonl`), accurately capturing full multi-month history, exact model attributions, cached read tokens, and official `costUsdTicks`.
- **Grok Project Decoding:** Added URL decoding for Grok's working directory folder names (e.g. `%2FUsers%2Ffelix%2FProjects` -> `Projects`).

## [1.11.3] - 2026-08-09

### Added
- **Historical Baseline Overrides:** Support for `~/.config/tokenusage/history_overrides.json` to restore historical monthly metrics and model breakdowns if raw session log transcripts are purged from disk.
- **SQLite History Auto-Persistence (`history.db`):** Automatically upserts daily and monthly aggregated token counts and costs into SQLite at `~/.config/tokenusage/history.db` using `rusqlite`, permanently preserving historical usage metrics across future disk cleanups.

## [1.11.2] - 2026-08-04

### Added
- **Shell Auto-Completions:** `tu completions <shell>` subcommands for `bash`, `zsh`, `fish`, `powershell`, and `elvish` via `clap_complete`.
- **Low-Quota Warning Thresholds:** Added `--warn-threshold <PCT>` to `tu antigravity` and status probes to print warnings when remaining model quota drops below the specified threshold.
- **Native Windows Antigravity Support:** Cross-platform Windows process and port detection using PowerShell `Get-CimInstance Win32_Process` and `netstat -ano` listening port discovery.
- **Grok OAuth Proxy Token Fallback:** Auto-reads Grok CLI OAuth proxy tokens from `~/.grok/auth.json` with token auto-refresh when `XAI_API_KEY` is not set.
- **Standalone Binary Installers:** Pre-compiled cross-platform shell (`tokenusage-installer.sh`) and PowerShell (`tokenusage-installer.ps1`) installer scripts hosted directly on GitHub Releases.

## [Unreleased]

### Added
- **Google Antigravity** is now a first-class provider, not just a quota
  probe: `SourceKind::Gemini`'s display name changed to "Antigravity", and
  its real token/cost accounting is now parsed from Antigravity's own
  local SQLite conversation databases (`~/.gemini/antigravity-cli/conversations/*.db`,
  protobuf-encoded `gen_metadata` blobs — no published schema, so the
  field layout was reverse-engineered) alongside the existing
  `~/.gemini/tmp` and `~/.gemini/antigravity-cli/brain` transcript logs.
- `tu antigravity` / `tu agy` provider-first subcommands (`tu antigravity monthly`,
  `tu agy daily`, etc.) and `--only antigravity` / `--only agy` — matching
  the existing `tu claude` / `tu codex` / `tu opencode` UX.
- `tu antigravity status` restores access to the live plan-tier +
  session/weekly quota-% probe (previously the only thing `tu antigravity`
  did; now that name defaults to the usage table like the other
  providers, `status` reaches the same probe explicitly).
- Pricing for Gemini 2.5/3/3.1/3.6 Flash and Pro model variants, plus an
  alias fallback (`PricingTable::find_rate`) that strips Antigravity's
  internal reasoning-effort suffixes (`-low`/`-medium`/`-high`/`-xhigh`/`-none`)
  and maps known model bases to their public "-preview" pricing key — these
  were previously priced at $0.00 with no pricing-debug warning surfaced.

### Fixed
- `tu live`'s "Gemini" tab (renders the same Antigravity usage data as
  everywhere else) now reads "Antigravity"; the separate live-quota tab
  is now "Antigravity Quota" so the two don't collide.

## [1.11.1] - 2026-07-18

### Fixed
- `Cargo.toml`'s `repository` field still pointed at upstream
  (`hanbu97/tokenusage`), which cargo-dist bakes into every generated
  install script/Homebrew formula at release time — v1.11.0's install
  instructions 404'd because they pointed at upstream's releases,
  which don't have this fork's binaries. Now points at
  `FelixIsaac/tokenusage`. v1.11.0 release/tag were deleted;
  this supersedes it.

## [1.11.0] - 2026-07-18

### Added
- New log-based source: **Grok Build** (`SourceKind::Grok`), reading real
  per-turn token counts from `~/.grok/logs/unified.jsonl`
  (`--grok-log-dir` to override). Verified pricing for `grok-4.5`
  ($2/$0.5/$6 per M input/cached/output tokens, doubling above 200K).
  Grok's log has no per-turn model field, so events are labelled with
  a fixed fallback model rather than a per-event value; `tu parity`
  explicitly rejects Grok since no ccusage-family equivalent exists.
- cargo-dist release pipeline: prebuilt binaries for macOS (arm64/x64),
  Windows x64, and Linux x64/arm64, published to GitHub Releases and a
  Homebrew tap (`FelixIsaac/homebrew-tokenusage`).

## [1.10.1] - 2026-06-12

### Changed
- Bare-`tu` menu now has a dedicated **`statusline init`** entry (setup) separate
  from `statusline` (which just prints the rendered line) — picking it from the
  menu no longer dumps a raw status line. The menu dispatcher handles multi-token
  commands.

### Added
- Hidden `--settings-path <FILE>` on `tu statusline init` (testing seam) so the
  write/backup/merge path has real unit tests — on Windows `dirs::home_dir()`
  ignores `HOME`/`USERPROFILE`, so env redirection can't sandbox it.

## [1.10.0] - 2026-06-12

### Added
- **`tu statusline init`** — one command to wire tu into Claude Code's status
  line instead of hand-editing `~/.claude/settings.json`:
  - Safe by default: backs up the file, merges only the `statusLine` key
    (every other setting preserved), shows the change and asks to confirm.
  - **Never clobbers another tool's line.** If `statusLine` already belongs to
    ccstatusline / a custom script, it stops and points you at `--ccstatusline`
    (or `--yes` to force-replace).
  - `--print` — pure preview, writes nothing (predictable for AI agents to read
    and apply).
  - `--ccstatusline` — emits a copy-paste **prompt** to hand your AI assistant
    that integrates tu as a data source into your existing ccstatusline / custom
    line (using `--json`/`--field`), with the caching guidance baked in.
  - `--yes` skips the prompt (agent/non-interactive use); `--flags` overrides the
    baked-in `--cache --refresh-interval 30`.
  - A malformed `settings.json` is a hard stop — tu refuses to overwrite a file
    it couldn't parse.

## [1.9.1] - 2026-06-12

### Fixed
- `tu antigravity` on Windows now fails with a clear message (detection uses
  ps/lsof, macOS/Linux only) instead of a cryptic `/bin/ps` "path not found"
  error, and the interactive menu hides the `antigravity` entry on Windows.
  A real Windows port needs the `agy`-process + flagless-port discovery (see
  usage-tray-windows).

## [1.9.0] - 2026-06-12

### Added
- **Modular `tu statusline` output** for status-bar integrations (e.g. ccstatusline):
  - `--json` emits all values structured (model, session/today/block cost,
    block-remaining, burn rate/status, context %).
  - `--field <name>` emits a single formatted value (e.g. `today-cost` -> `$22.18`;
    names: model, session-cost, today-cost, block-cost, block-left, burn-hourly,
    burn-per-min, burn-status, ctx-pct, ctx-level).
  - `--format "<template>"` substitutes `{field}` placeholders into a custom line.
  - All three render from one shared per-session fields cache, so N widgets trigger
    at most one parse per `--refresh-interval`. The default full-line output is
    unchanged. See docs/statusline.md.

## [1.8.3] - 2026-06-12

### Changed
- The interactive menu's **Balances** section now lists only providers whose API
  key is actually configured (`DEEPSEEK_API_KEY`, `OPENROUTER_API_KEY`,
  `XAI_API_KEY`, `MOONSHOT_API_KEY`, `ANTHROPIC_API_KEY`/`ANTHROPIC_ADMIN_KEY` —
  each provider's documented convention), instead of always showing all six.
  Antigravity (a local probe, no key) is always shown.

## [1.8.2] - 2026-06-11

### Fixed
- Bare `tu` now actually opens the interactive menu. `normalize_cli_args` injects
  `daily` for a bare invocation, so the menu's `cli.command.is_none()` check was
  always false; gate on raw args (`env::args().nth(1).is_none()`) instead.

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
