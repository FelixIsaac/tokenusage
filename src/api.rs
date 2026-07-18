//! Public library API for programmatic access to token usage data.
//!
//! This module provides a **clap-free** interface for integrating tokenusage
//! into other Rust applications.  Use [`Config`] to configure source paths
//! and filters, then call one of the top-level functions to retrieve data.
//!
//! # Entry points
//!
//! | Function | Returns | Use when you want… |
//! |----------|---------|-------------------|
//! | [`usage_snapshot`] | [`UsageSnapshot`](crate::UsageSnapshot) | Raw events + parse stats + timezone |
//! | [`load_events`] | `Vec<`[`UsageEvent`]`>` | Just the event list |
//! | [`daily_report`] | [`DailyReport`] | Aggregated per-day / per-week / per-month rows with totals |
//! | [`daily_report_with_week_start`] | [`DailyReport`] | Same, but with a custom week start day |
//! | [`parse_stats`] | [`ParseStats`] | Parsing diagnostics only (files scanned, lines parsed, etc.) |
//!
//! # Examples
//!
//! ```no_run
//! use tokenusage::{Config, ReportPeriod, SortOrder};
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Snapshot: all events, default config
//! let config = Config::default();
//! let snapshot = tokenusage::usage_snapshot(config.clone()).await?;
//! println!("Found {} events", snapshot.events.len());
//!
//! // Daily report with date filter
//! let config = Config {
//!     since: Some("2025-06-01".into()),
//!     order: SortOrder::Desc,
//!     ..Config::default()
//! };
//! let report = tokenusage::daily_report(config, ReportPeriod::Daily, false, None).await?;
//! for row in &report.daily {
//!     println!("{}: {} tokens, ${:.4}", row.date, row.totals.total_tokens, row.totals.cost_usd);
//! }
//! # Ok(())
//! # }
//! ```

use anyhow::Result;

use crate::cli;
use crate::pipeline;
use crate::types::{DailyReport, ParseStats, UsageEvent};

/// Configuration for token usage analysis.
///
/// This is the library-friendly equivalent of CLI arguments.
/// All fields have sensible defaults; a `Config::default()` will discover
/// usage logs from standard locations for Claude Code, Codex, Gemini CLI,
/// and OpenCode.
///
/// # Default behaviour
///
/// - Scans default provider locations:
///   - Claude: `~/.claude/projects/*/`
///   - Codex: `~/.codex/sessions/`
///   - Gemini: `~/.gemini/tmp/`
///   - OpenCode: `~/.local/share/opencode/` (platform-dependent alternatives included)
/// - Uses the system local timezone for date grouping.
/// - Fetches live model pricing from the OpenRouter API; falls back to
///   built-in estimates if the network is unavailable.
/// - Enables incremental parse caching for fast repeated runs.
/// - Automatically detects CPU core count for parallel parsing.
///
/// # Examples
///
/// ```
/// use tokenusage::Config;
///
/// // Minimal — scan all defaults
/// let config = Config::default();
///
/// // Claude-only, offline pricing, date range
/// let config = Config {
///     no_codex: true,
///     offline: true,
///     since: Some("2025-06-01".into()),
///     until: Some("2025-06-30".into()),
///     ..Config::default()
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Start date filter (inclusive).
    ///
    /// Accepts `YYYYMMDD` or `YYYY-MM-DD`.  Events before this date are
    /// excluded.  `None` means no lower bound.
    pub since: Option<String>,

    /// End date filter (inclusive).
    ///
    /// Accepts `YYYYMMDD` or `YYYY-MM-DD`.  Events after this date are
    /// excluded.  `None` means no upper bound.
    pub until: Option<String>,

    /// Sort order for report rows.
    ///
    /// Controls whether daily/weekly/monthly rows are sorted by date
    /// ascending (oldest first) or descending (newest first).
    /// Defaults to [`SortOrder::Asc`].
    pub order: SortOrder,

    /// Use offline pricing only (skip the OpenRouter API fetch).
    ///
    /// When `true`, only the built-in fallback pricing table is used.
    /// Useful for air-gapped environments or when you want deterministic costs.
    pub offline: bool,

    /// IANA timezone name for date grouping.
    ///
    /// Examples: `"UTC"`, `"Asia/Tokyo"`, `"America/New_York"`.
    /// `None` defaults to the system local timezone.
    pub timezone: Option<String>,

    /// Worker thread count for parallel parsing.
    ///
    /// `None` auto-detects based on available CPU cores.  Set to `Some(1)`
    /// for single-threaded parsing (useful for debugging).
    pub workers: Option<usize>,

    /// Disable the Claude Code log source.
    ///
    /// When `true`, only Codex logs are scanned.
    pub no_claude: bool,

    /// Disable the Codex log source.
    ///
    /// When `true`, only Claude Code logs are scanned.
    pub no_codex: bool,
    /// Disable the Gemini CLI log source.
    pub no_gemini: bool,
    /// Disable the OpenCode log source.
    pub no_opencode: bool,
    /// Disable the Grok Build log source.
    pub no_grok: bool,

    /// Custom Claude projects directory paths.
    ///
    /// Overrides the default `~/.claude/projects/*/` discovery.
    /// Each string should be an absolute path.  Empty = use defaults.
    pub claude_projects_dir: Vec<String>,

    /// Custom Codex sessions directory paths.
    ///
    /// Overrides the default `~/.codex/sessions/` discovery.
    /// Each string should be an absolute path.  Empty = use defaults.
    pub codex_sessions_dir: Vec<String>,
    /// Custom Gemini data directory paths.
    pub gemini_data_dir: Vec<String>,
    /// Custom OpenCode data directory paths.
    pub opencode_data_dir: Vec<String>,
    /// Custom Grok Build log directory paths.
    pub grok_log_dir: Vec<String>,

    /// Path substring ignore rules.
    ///
    /// Any discovered file whose absolute path contains one of these
    /// substrings will be skipped during parsing.
    pub ignore_path: Vec<String>,

    /// Disable the built-in heavy directory ignore list.
    ///
    /// By default, directories like `.git`, `node_modules`, `target`, etc.
    /// are skipped during file discovery.  Set to `true` to scan everything.
    pub no_default_ignores: bool,

    /// Disable incremental parse caching.
    ///
    /// When `true`, all files are re-parsed from scratch on every run.
    /// The cache lives in the system cache directory.
    pub no_incremental_cache: bool,

    /// Rebuild the incremental parse cache from scratch.
    ///
    /// Like `no_incremental_cache`, but the rebuilt results are written
    /// back to the cache for future runs.
    pub rebuild_cache: bool,

    /// Path to a custom pricing JSON file.
    ///
    /// The file should contain a JSON object mapping model names to
    /// [`PricingRate`](crate::PricingRate) objects.  These override both
    /// the built-in and OpenRouter-fetched pricing.
    pub pricing_file: Option<String>,

    /// Enrich reports with locally inferred coding activity.
    ///
    /// When `true`, heartbeat data (if available) is used to annotate
    /// report rows with coding time, top project, and top language.
    pub with_activity: bool,
}

/// Sort order for report rows.
///
/// Determines the chronological ordering of [`DailyRow`](crate::DailyRow)
/// entries in a [`DailyReport`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortOrder {
    /// Oldest date first (default).
    #[default]
    Asc,
    /// Newest date first.
    Desc,
}

/// Week start day for weekly report grouping.
///
/// Only relevant when calling [`daily_report_with_week_start`] with
/// [`ReportPeriod::Weekly`](crate::ReportPeriod::Weekly).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WeekStart {
    /// Sunday (default, ISO-like US convention).
    #[default]
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

/// Collect an aggregated usage report grouped by day, week, or month.
///
/// This is the primary high-level function for generating tabular reports.
///
/// # Arguments
///
/// * `config` — Controls which sources to scan, date filters, pricing, etc.
/// * `period` — Grouping granularity: [`ReportPeriod::Daily`](crate::ReportPeriod::Daily),
///   [`Weekly`](crate::ReportPeriod::Weekly), or [`Monthly`](crate::ReportPeriod::Monthly).
/// * `instances` — If `true`, report rows include per-session (instance)
///   breakdowns instead of aggregating all sessions into one row per period.
/// * `project` — Optional project name filter. Only events from matching
///   projects are included.
///
/// # Returns
///
/// A [`DailyReport`] containing a `daily` vec of per-period
/// rows, a `totals` summary, and [`ParseStats`] diagnostics.
///
/// # Example
///
/// ```no_run
/// use tokenusage::{Config, ReportPeriod};
///
/// # async fn example() -> anyhow::Result<()> {
/// let report = tokenusage::daily_report(
///     Config::default(),
///     ReportPeriod::Monthly,
///     false,
///     None,
/// ).await?;
/// println!("Total cost: ${:.4}", report.totals.cost_usd);
/// # Ok(())
/// # }
/// ```
pub async fn daily_report(
    config: Config,
    period: pipeline::ReportPeriod,
    instances: bool,
    project: Option<String>,
) -> Result<DailyReport> {
    daily_report_with_week_start(config, period, instances, project, WeekStart::default()).await
}

/// Collect a usage report with a custom week start day.
///
/// Identical to [`daily_report`] but allows specifying which day of the week
/// begins a new "week" when using [`ReportPeriod::Weekly`](crate::ReportPeriod::Weekly).
///
/// # Example
///
/// ```no_run
/// use tokenusage::{Config, ReportPeriod, WeekStart};
///
/// # async fn example() -> anyhow::Result<()> {
/// let report = tokenusage::daily_report_with_week_start(
///     Config::default(),
///     ReportPeriod::Weekly,
///     false,
///     None,
///     WeekStart::Monday,  // ISO 8601 weeks
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn daily_report_with_week_start(
    config: Config,
    period: pipeline::ReportPeriod,
    instances: bool,
    project: Option<String>,
    start_of_week: WeekStart,
) -> Result<DailyReport> {
    let common = config_to_common_args(&config);
    let cli_week = to_cli_week_start(start_of_week);
    pipeline::collect_report(common, period, instances, project, cli_week).await
}

/// Load all parsed usage events matching the given config.
///
/// Returns a flat `Vec` of every [`UsageEvent`] that
/// passed the date and source filters.  Events are sorted by timestamp.
///
/// This is a convenience wrapper around [`usage_snapshot`] that discards
/// the parse stats and timezone metadata.
///
/// # Example
///
/// ```no_run
/// use tokenusage::Config;
///
/// # async fn example() -> anyhow::Result<()> {
/// let events = tokenusage::load_events(Config::default()).await?;
/// for e in &events {
///     println!("[{}] {} → {} in/out", e.source.as_str(), e.model, e.usage.total_tokens());
/// }
/// # Ok(())
/// # }
/// ```
pub async fn load_events(config: Config) -> Result<Vec<UsageEvent>> {
    let snapshot = usage_snapshot(config).await?;
    Ok(snapshot.events)
}

/// Collect a full usage snapshot (events + parse stats + timezone).
///
/// This is the lowest-level data retrieval function.  It returns:
///
/// - **`events`** — All parsed [`UsageEvent`]s, sorted by timestamp.
/// - **`stats`** — [`ParseStats`] with file/line counters and error counts.
/// - **`timezone`** — The [`TimeZoneMode`](crate::TimeZoneMode) used for date grouping.
///
/// # Example
///
/// ```no_run
/// use tokenusage::Config;
///
/// # async fn example() -> anyhow::Result<()> {
/// let snap = tokenusage::usage_snapshot(Config::default()).await?;
/// println!("Scanned {} files, parsed {} lines",
///     snap.stats.files_discovered, snap.stats.lines_parsed);
/// println!("Total events: {}", snap.events.len());
/// # Ok(())
/// # }
/// ```
pub async fn usage_snapshot(config: Config) -> Result<pipeline::UsageSnapshot> {
    let common = config_to_common_args(&config);
    pipeline::collect_usage_snapshot(common).await
}

/// Get parse statistics without loading full event data.
///
/// Useful for diagnostics — tells you how many files were found, how many
/// lines were parsed vs. skipped, and counts of various error categories.
///
/// # Example
///
/// ```no_run
/// use tokenusage::Config;
///
/// # async fn example() -> anyhow::Result<()> {
/// let stats = tokenusage::parse_stats(Config::default()).await?;
/// println!("Files: {}, Parsed lines: {}, Invalid JSON: {}",
///     stats.files_discovered, stats.lines_parsed, stats.lines_invalid_json);
/// # Ok(())
/// # }
/// ```
pub async fn parse_stats(config: Config) -> Result<ParseStats> {
    let snapshot = usage_snapshot(config).await?;
    Ok(snapshot.stats)
}

// ---------------------------------------------------------------------------
// Internal conversion helpers
// ---------------------------------------------------------------------------

fn config_to_common_args(config: &Config) -> cli::CommonArgs {
    cli::CommonArgs {
        since: config.since.clone(),
        until: config.until.clone(),
        json: false,
        jq: None,
        debug: false,
        debug_samples: 5,
        order: match config.order {
            SortOrder::Asc => cli::SortOrder::Asc,
            SortOrder::Desc => cli::SortOrder::Desc,
        },
        breakdown: false,
        offline: config.offline,
        pricing_debug: false,
        timezone: config.timezone.clone(),
        locale: None,
        config: None,
        compact: false,
        brief: false,
        workers: config.workers,
        no_claude: config.no_claude,
        no_codex: config.no_codex,
        no_gemini: config.no_gemini,
        no_opencode: config.no_opencode,
        no_grok: config.no_grok,
        no_antigravity: true,
        only: Vec::new(),
        sources: Vec::new(),
        claude_projects_dir: config.claude_projects_dir.clone(),
        codex_sessions_dir: config.codex_sessions_dir.clone(),
        gemini_data_dir: config.gemini_data_dir.clone(),
        opencode_data_dir: config.opencode_data_dir.clone(),
        grok_log_dir: config.grok_log_dir.clone(),
        ignore_path: config.ignore_path.clone(),
        no_default_ignores: config.no_default_ignores,
        no_incremental_cache: config.no_incremental_cache,
        rebuild_cache: config.rebuild_cache,
        pricing_file: config.pricing_file.clone(),
        with_activity: config.with_activity,
        slow: None,
    }
}

fn to_cli_week_start(ws: WeekStart) -> cli::WeekStart {
    match ws {
        WeekStart::Sunday => cli::WeekStart::Sunday,
        WeekStart::Monday => cli::WeekStart::Monday,
        WeekStart::Tuesday => cli::WeekStart::Tuesday,
        WeekStart::Wednesday => cli::WeekStart::Wednesday,
        WeekStart::Thursday => cli::WeekStart::Thursday,
        WeekStart::Friday => cli::WeekStart::Friday,
        WeekStart::Saturday => cli::WeekStart::Saturday,
    }
}
