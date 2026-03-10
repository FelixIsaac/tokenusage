//! Public library API for programmatic access to token usage data.
//!
//! This module provides a clap-free interface for integrating tokenusage
//! into other Rust applications. Use [`Config`] to configure source paths
//! and filters, then call [`load_events`], [`daily_report`], or
//! [`usage_snapshot`] to retrieve data.
//!
//! # Examples
//!
//! ```no_run
//! use tokenusage::{Config, ReportPeriod, SortOrder};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = Config::default();
//! let snapshot = tokenusage::usage_snapshot(config.clone()).await?;
//! println!("Found {} events", snapshot.events.len());
//!
//! let report = tokenusage::daily_report(config, ReportPeriod::Daily, false, None).await?;
//! for row in &report.daily {
//!     println!("{}: {} tokens, ${:.2}", row.date, row.totals.total_tokens, row.totals.cost_usd);
//! }
//! # Ok(())
//! # }
//! ```

use anyhow::Result;

use crate::cli;
use crate::pipeline;
use crate::types::{DailyReport, UsageEvent, ParseStats};

/// Configuration for token usage analysis.
///
/// This is the library-friendly equivalent of CLI arguments.
/// All fields have sensible defaults; a `Config::default()` will discover
/// usage logs from standard locations for both Claude and Codex.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Start date filter (YYYYMMDD or YYYY-MM-DD).
    pub since: Option<String>,
    /// End date filter (YYYYMMDD or YYYY-MM-DD).
    pub until: Option<String>,
    /// Sort order for report rows.
    pub order: SortOrder,
    /// Use offline pricing (skip network fetch).
    pub offline: bool,
    /// Timezone for date grouping (e.g. "UTC", "Asia/Tokyo"). None = local.
    pub timezone: Option<String>,
    /// Worker thread count. None = auto (CPU cores).
    pub workers: Option<usize>,
    /// Disable Claude source.
    pub no_claude: bool,
    /// Disable Codex source.
    pub no_codex: bool,
    /// Custom Claude projects directories (empty = use defaults).
    pub claude_projects_dir: Vec<String>,
    /// Custom Codex sessions directories (empty = use defaults).
    pub codex_sessions_dir: Vec<String>,
    /// Paths containing these substrings will be ignored.
    pub ignore_path: Vec<String>,
    /// Disable built-in heavy directory ignore list.
    pub no_default_ignores: bool,
    /// Disable incremental parse cache.
    pub no_incremental_cache: bool,
    /// Rebuild incremental parse cache from scratch.
    pub rebuild_cache: bool,
    /// Optional pricing override JSON file path.
    pub pricing_file: Option<String>,
    /// Enrich reports with locally inferred coding activity.
    pub with_activity: bool,
}

/// Sort order for report rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

/// Week start day.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WeekStart {
    #[default]
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

/// Collect a daily/weekly/monthly usage report.
pub async fn daily_report(
    config: Config,
    period: pipeline::ReportPeriod,
    instances: bool,
    project: Option<String>,
) -> Result<DailyReport> {
    daily_report_with_week_start(config, period, instances, project, WeekStart::default()).await
}

/// Collect a usage report with a custom week start day.
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
pub async fn load_events(config: Config) -> Result<Vec<UsageEvent>> {
    let snapshot = usage_snapshot(config).await?;
    Ok(snapshot.events)
}

/// Collect a full usage snapshot (events + stats + timezone).
pub async fn usage_snapshot(config: Config) -> Result<pipeline::UsageSnapshot> {
    let common = config_to_common_args(&config);
    pipeline::collect_usage_snapshot(common).await
}

/// Get parse statistics without full event data.
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
        timezone: config.timezone.clone(),
        locale: None,
        config: None,
        compact: false,
        workers: config.workers,
        no_claude: config.no_claude,
        no_codex: config.no_codex,
        no_antigravity: true,
        claude_projects_dir: config.claude_projects_dir.clone(),
        codex_sessions_dir: config.codex_sessions_dir.clone(),
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
