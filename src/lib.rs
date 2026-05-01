//! # tokenusage
//!
//! Fast, zero-config token usage tracker for **Codex**, **Claude Code**,
//! **Gemini CLI**, **OpenCode**, and **Antigravity**. Parses local log files produced by AI coding assistants
//! and computes per-model, per-day, and per-session token counts with
//! estimated USD costs.
//!
//! This crate is **dual-use**:
//!
//! | Use case | Feature | What you get |
//! |----------|---------|--------------|
//! | CLI binary (`tu`) | `cli` (default) | Full terminal UI, TUI dashboard, image export, GUI |
//! | Library dependency | no default features | Programmatic access to parsed usage data |
//!
//! ## Quick start (library)
//!
//! Add `tokenusage` as a dependency **without** the default `cli` feature so
//! you only pull in the lightweight parsing core:
//!
//! ```toml
//! [dependencies]
//! tokenusage = { version = "1.5", default-features = false }
//! ```
//!
//! ### Fetch a full usage snapshot
//!
//! ```no_run
//! use tokenusage::{Config, ReportPeriod};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = Config::default();
//! let snapshot = tokenusage::usage_snapshot(config).await?;
//! for event in &snapshot.events {
//!     println!("{}: {} ({} tokens)",
//!         event.timestamp, event.model, event.usage.total_tokens());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Generate a daily cost report
//!
//! ```no_run
//! use tokenusage::{Config, ReportPeriod};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let report = tokenusage::daily_report(
//!     Config::default(),
//!     ReportPeriod::Daily,
//!     false,   // instances (per-session breakdown)
//!     None,    // project filter
//! ).await?;
//!
//! for row in &report.daily {
//!     println!("{}: {} total tokens, ${:.4}",
//!         row.date, row.totals.total_tokens, row.totals.cost_usd);
//! }
//! println!("Grand total: ${:.4}", report.totals.cost_usd);
//! # Ok(())
//! # }
//! ```
//!
//! ### Filter by date range
//!
//! ```no_run
//! use tokenusage::Config;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = Config {
//!     since: Some("2025-06-01".into()),
//!     until: Some("2025-06-30".into()),
//!     ..Config::default()
//! };
//! let events = tokenusage::load_events(config).await?;
//! println!("June events: {}", events.len());
//! # Ok(())
//! # }
//! ```
//!
//! ## Architecture overview
//!
//! ```text
//!  Log files on disk
//!    (Claude ~/.claude/projects/*/,  Codex ~/.codex/sessions/*)
//!         |
//!         v
//!  +--------------+     +--------------+     +---------------+
//!  |  Discovery   | --> |   Parsing    | --> |  Aggregation  |
//!  |  (sources)   |     | (rayon par.) |     | (reports/snap)|
//!  +--------------+     +--------------+     +---------------+
//!         |                    |                     |
//!    SourceConfig         UsageEvent           DailyReport
//!    DiscoveredFile       ParseStats          UsageSnapshot
//! ```
//!
//! 1. **Discovery** — Scans configured root directories for JSONL log files.
//! 2. **Parsing** — Parallel (rayon) extraction of [`UsageEvent`]s with
//!    incremental caching for repeated runs.
//! 3. **Aggregation** — Groups events into [`DailyReport`] rows or returns a
//!    raw [`UsageSnapshot`].
//!
//! ## Feature flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `cli`   | **yes** | Enables the `tu` binary and all terminal/GUI dependencies (clap, ratatui, iced, etc.) |
//!
//! When `cli` is disabled the crate compiles with only the core parsing and
//! reporting logic — no terminal, no GUI, no image export dependencies.

mod activity;
pub mod api;
mod cli;
#[cfg(feature = "cli")]
mod config;
#[cfg(feature = "cli")]
mod gui;
mod heartbeat;
#[cfg(feature = "cli")]
mod output;
mod pipeline;
#[cfg(feature = "cli")]
mod share;
mod types;

// ---------------------------------------------------------------------------
// Public re-exports for library consumers
// ---------------------------------------------------------------------------

pub use api::{Config, SortOrder, WeekStart};
pub use api::{
    daily_report, daily_report_with_week_start, load_events, parse_stats, usage_snapshot,
};
pub use pipeline::{ReportPeriod, TimeZoneMode, UsageSnapshot};
pub use types::{
    ActivitySummary, DailyReport, DailyRow, DateFilter, ParseStats, PricingRate, PricingTable,
    SourceKind, TokenCounts, UsageAccumulator, UsageEvent,
};

// ---------------------------------------------------------------------------
// CLI entry point — only available with `cli` feature
// ---------------------------------------------------------------------------

#[cfg(feature = "cli")]
use crate::cli::{Cli, Commands, DailyArgs, normalize_cli_args};
#[cfg(feature = "cli")]
use anyhow::Result;
#[cfg(feature = "cli")]
use clap::Parser;

/// Run the CLI application.
///
/// This is the main entry point for the `tu` binary.  It parses command-line
/// arguments, applies config-file overrides, and dispatches to the appropriate
/// subcommand.
///
/// Only available when the `cli` feature is enabled (default).
#[cfg(feature = "cli")]
pub async fn run() -> Result<()> {
    let cli = Cli::parse_from(normalize_cli_args(std::env::args().collect()));

    let command = cli.command.unwrap_or(Commands::Daily(DailyArgs::default()));
    let command = config::apply_config(command)?;

    let throttle_ms = extract_throttle(&command);
    if throttle_ms > 0 && std::env::var("_TU_THROTTLE_ACTIVE").is_err() {
        run_with_throttle(throttle_ms)
    } else {
        dispatch(command).await
    }
}

#[cfg(feature = "cli")]
fn extract_throttle(cmd: &Commands) -> u64 {
    match cmd {
        Commands::Daily(a) | Commands::Doctor(a) => a.common.slow,
        Commands::Parity(a) => a.common.slow,
        Commands::Today(a) => a.common.slow,
        Commands::Activity(a) => a.common.slow,
        Commands::Monthly(a) => a.common.slow,
        Commands::Weekly(a) => a.common.slow,
        Commands::Img(a) => a.common.slow,
        Commands::Session(a) => a.common.slow,
        Commands::Blocks(a) => a.common.slow,
        Commands::Live(a) => a.common.slow,
        Commands::Top(a) => a.common.slow,
        Commands::Statusline(a) => a.common.slow,
        Commands::Gui(a) => a.common.slow,
        Commands::Heartbeat(a) => match &a.command {
            cli::HeartbeatCommand::Stats(s) => s.slow,
            _ => None,
        },
        _ => None,
    }
    .unwrap_or(0)
}

#[cfg(feature = "cli")]
fn run_with_throttle(delay_ms: u64) -> Result<()> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    let exe = std::env::current_exe()?;
    let args = args_without_slow();
    let delay = Duration::from_millis(delay_ms);

    let mut child = Command::new(exe)
        .args(&args)
        .env("_TU_THROTTLE_ACTIVE", "1")
        .env("CLICOLOR_FORCE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let mut out = std::io::stdout().lock();

    for line in reader.lines().map_while(Result::ok) {
        let _ = writeln!(out, "{}", line);
        let _ = out.flush();
        std::thread::sleep(delay);
    }

    let status = child.wait()?;
    if !status.success() {
        anyhow::bail!("child exited with {}", status);
    }
    Ok(())
}

#[cfg(feature = "cli")]
fn args_without_slow() -> Vec<String> {
    let mut result = Vec::new();
    let mut skip_next = false;
    for arg in std::env::args().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--slow" || arg == "-S" {
            skip_next = true;
            continue;
        }
        if arg.starts_with("--slow=") {
            continue;
        }
        result.push(arg);
    }
    result
}

#[cfg(feature = "cli")]
async fn dispatch(command: Commands) -> Result<()> {
    match command {
        Commands::Daily(args) => pipeline::run_daily(args).await,
        Commands::Today(args) => pipeline::run_today(args).await,
        Commands::Activity(args) => pipeline::run_activity(args).await,
        Commands::Heartbeat(args) => heartbeat::run(args).await,
        Commands::Doctor(args) => pipeline::run_doctor(args).await,
        Commands::Parity(args) => pipeline::run_parity(args).await,
        Commands::Antigravity(args) => pipeline::run_antigravity(args).await,
        Commands::Monthly(args) => pipeline::run_monthly(args).await,
        Commands::Weekly(args) => pipeline::run_weekly(args).await,
        Commands::Img(args) => share::run_share(args).await,
        Commands::Session(args) => pipeline::run_session(args).await,
        Commands::Blocks(args) => pipeline::run_blocks(args).await,
        Commands::Live(args) => pipeline::run_blocks(args.into()).await,
        Commands::Top(args) => pipeline::run_top(args).await,
        Commands::Statusline(args) => pipeline::run_statusline(args).await,
        Commands::Gui(args) => {
            let handle = std::thread::spawn(move || gui::run_gui(args));
            match handle.join() {
                Ok(result) => result,
                Err(_) => anyhow::bail!("GUI thread panicked"),
            }
        }
    }
}
