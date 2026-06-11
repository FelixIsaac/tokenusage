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
mod insights;
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

pub use insights::ReportInsights;

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

/// Synchronous binary entry point.
///
/// The GUI must own the main thread and run **outside** a tokio runtime: iced
/// spins up its own runtime, and dropping that runtime from inside an outer
/// runtime (the old `#[tokio::main]`) panics on window close
/// (`Cannot drop a runtime in a context where blocking is not allowed`).
/// So we parse first, run the GUI directly on the main thread, and only build a
/// tokio runtime for the async (non-GUI) commands.
#[cfg(feature = "cli")]
pub fn run_blocking() -> Result<()> {
    use std::io::IsTerminal;

    let cli = Cli::parse_from(normalize_cli_args(std::env::args().collect()));

    // Bare `tu` (no subcommand, no flags) on an interactive terminal opens the
    // command menu. Anything with args (e.g. `tu --json`) keeps the daily
    // default; a pipe/non-TTY also keeps it so automation isn't broken.
    if cli.command.is_none()
        && std::env::args().nth(1).is_none()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
    {
        match output::run_command_menu()? {
            Some(name) => {
                let exe = std::env::current_exe()?;
                let status = std::process::Command::new(exe).arg(&name).status()?;
                std::process::exit(status.code().unwrap_or(0));
            }
            None => return Ok(()),
        }
    }

    let command = cli.command.unwrap_or(Commands::Daily(DailyArgs::default()));
    let command = config::apply_config(command)?;

    let throttle_ms = extract_throttle(&command);
    if throttle_ms > 0 && std::env::var("_TU_THROTTLE_ACTIVE").is_err() {
        return run_with_throttle(throttle_ms);
    }

    if let Commands::Gui(args) = command {
        return gui::run_gui(args);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(dispatch(command))
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
        Commands::Deepseek(args) => pipeline::run_deepseek(args).await,
        Commands::Openrouter(args) => pipeline::run_openrouter(args).await,
        Commands::Grok(args) => pipeline::run_grok(args).await,
        Commands::Kimi(args) => pipeline::run_kimi(args).await,
        Commands::AnthropicApi(args) => pipeline::run_anthropic_api(args).await,
        Commands::Monthly(args) => pipeline::run_monthly(args).await,
        Commands::Weekly(args) => pipeline::run_weekly(args).await,
        Commands::Img(args) => share::run_share(args).await,
        Commands::Session(args) => pipeline::run_session(args).await,
        Commands::Blocks(args) => pipeline::run_blocks(args).await,
        Commands::Live(args) => pipeline::run_blocks(args.into()).await,
        Commands::Top(args) => pipeline::run_top(args).await,
        Commands::Statusline(args) => pipeline::run_statusline(args).await,
        Commands::Gui(args) => gui::run_gui(args),
    }
}
