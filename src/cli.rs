use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Deserialize;

use crate::heartbeat::{
    DEFAULT_HEARTBEAT_PULSE_SECS, DEFAULT_HEARTBEAT_TIMEOUT_SECS, HeartbeatEntityKind,
};

#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SortOrder {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum WeekStart {
    #[default]
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum VisualBurnRate {
    #[default]
    Off,
    Emoji,
    Text,
    EmojiText,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq, Deserialize)]
pub(crate) enum CostSource {
    #[default]
    #[value(name = "auto")]
    #[serde(rename = "auto")]
    Auto,
    #[value(name = "derived")]
    #[serde(rename = "derived")]
    Derived,
    #[value(name = "cc")]
    #[serde(rename = "cc")]
    Cc,
    #[value(name = "both")]
    #[serde(rename = "both")]
    Both,
}

#[derive(Debug, Parser)]
#[command(
    name = "tokenusage",
    version,
    about = "Multi-source token usage analyzer (Rust)",
    after_help = "GitHub:  https://github.com/hanbu97/tokenusage\n\
                  Issues:  https://github.com/hanbu97/tokenusage/issues\n\
                  \n\
                  If you find this tool useful, please consider giving it a star!"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    Daily(DailyArgs),
    #[command(about = "Daily coding activity view inferred from local usage")]
    Today(TodayArgs),
    #[command(about = "Coding activity view with per-day breakdowns")]
    Activity(ActivityArgs),
    #[command(about = "Native local heartbeat collector and stats")]
    Heartbeat(HeartbeatArgs),
    Codex(DailyArgs),
    Claude(DailyArgs),
    Antigravity(AntigravityArgs),
    Monthly(MonthlyArgs),
    Weekly(WeeklyArgs),
    Img(ImgArgs),
    Session(SessionArgs),
    Blocks(BlocksArgs),
    Live(LiveArgs),
    #[command(about = "Real-time per-session token viewer (htop for tokens)")]
    Top(TopArgs),
    Statusline(StatuslineArgs),
    Gui(GuiArgs),
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct CommonArgs {
    #[arg(long, short = 's', help = "Start date filter (YYYYMMDD or YYYY-MM-DD)")]
    pub(crate) since: Option<String>,
    #[arg(long, short = 'u', help = "End date filter (YYYYMMDD or YYYY-MM-DD)")]
    pub(crate) until: Option<String>,
    #[arg(long, short = 'j', help = "Output JSON report")]
    pub(crate) json: bool,
    #[arg(
        long,
        short = 'q',
        help = "Process JSON output with jq expression (implies --json)"
    )]
    pub(crate) jq: Option<String>,
    #[arg(long, short = 'd', help = "Show debug summary")]
    pub(crate) debug: bool,
    #[arg(long, default_value_t = 5, help = "Debug sample count")]
    pub(crate) debug_samples: usize,
    #[arg(long, short = 'o', value_enum, default_value_t = SortOrder::Asc)]
    pub(crate) order: SortOrder,
    #[arg(long, short = 'b', help = "Show per-model breakdown")]
    pub(crate) breakdown: bool,
    #[arg(long, short = 'O', help = "Use offline pricing behavior")]
    pub(crate) offline: bool,
    #[arg(
        long,
        short = 'z',
        help = "Timezone for date grouping (e.g. UTC, Asia/Tokyo)"
    )]
    pub(crate) timezone: Option<String>,
    #[arg(long, short = 'l', help = "Locale for date/time formatting")]
    pub(crate) locale: Option<String>,
    #[arg(long, help = "Path to config JSON")]
    pub(crate) config: Option<String>,
    #[arg(long, help = "Force compact table mode")]
    pub(crate) compact: bool,
    #[arg(long, help = "Worker thread count (default: CPU cores)")]
    pub(crate) workers: Option<usize>,
    #[arg(long, help = "Disable Claude source")]
    pub(crate) no_claude: bool,
    #[arg(long, help = "Disable Codex source")]
    pub(crate) no_codex: bool,
    #[arg(long, help = "Disable Antigravity quota probe")]
    pub(crate) no_antigravity: bool,
    #[arg(long = "claude-projects-dir", help = "Claude projects dir, repeatable")]
    pub(crate) claude_projects_dir: Vec<String>,
    #[arg(long = "codex-sessions-dir", help = "Codex sessions dir, repeatable")]
    pub(crate) codex_sessions_dir: Vec<String>,
    #[arg(
        long = "ignore-path",
        help = "Ignore paths containing this substring (repeatable)"
    )]
    pub(crate) ignore_path: Vec<String>,
    #[arg(long, help = "Disable built-in heavy directory ignore list")]
    pub(crate) no_default_ignores: bool,
    #[arg(long, help = "Disable incremental parse cache")]
    pub(crate) no_incremental_cache: bool,
    #[arg(long, help = "Rebuild incremental parse cache from scratch")]
    pub(crate) rebuild_cache: bool,
    #[arg(long, help = "Optional pricing override JSON file")]
    pub(crate) pricing_file: Option<String>,
    #[arg(long, help = "Enrich reports with locally inferred coding activity")]
    pub(crate) with_activity: bool,
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct DailyArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(long, help = "Interactive TUI (sticky header + scroll)")]
    pub(crate) tui: bool,
    #[arg(long, short = 'i', help = "Group by project/instance")]
    pub(crate) instances: bool,
    #[arg(long, short = 'p', help = "Filter specific project")]
    pub(crate) project: Option<String>,
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct TodayArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(long, short = 'p', help = "Filter specific project")]
    pub(crate) project: Option<String>,
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct ActivityArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(long, short = 'p', help = "Filter specific project")]
    pub(crate) project: Option<String>,
    #[arg(
        long,
        default_value_t = 7,
        help = "Default trailing days when --since/--until are omitted"
    )]
    pub(crate) days: u32,
    #[arg(long, default_value_t = 5, help = "Breakdown rows per section")]
    pub(crate) limit: usize,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct HeartbeatArgs {
    #[command(subcommand)]
    pub(crate) command: HeartbeatCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum HeartbeatCommand {
    Ping(HeartbeatPingArgs),
    Watch(HeartbeatWatchArgs),
    Stats(HeartbeatStatsArgs),
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct HeartbeatPingArgs {
    #[arg(help = "Entity path or identifier")]
    pub(crate) entity: Option<String>,
    #[arg(long, value_enum, default_value_t = HeartbeatEntityKind::File)]
    pub(crate) kind: HeartbeatEntityKind,
    #[arg(long, short = 'p', help = "Override project name")]
    pub(crate) project: Option<String>,
    #[arg(long, help = "Override language name")]
    pub(crate) language: Option<String>,
    #[arg(long, help = "Optional VCS branch name")]
    pub(crate) branch: Option<String>,
    #[arg(long, default_value = "manual", help = "Collector origin label")]
    pub(crate) origin: String,
    #[arg(long, help = "Mark this heartbeat as a write event")]
    pub(crate) write: bool,
    #[arg(long, help = "Override heartbeat timestamp (RFC3339)")]
    pub(crate) time: Option<String>,
    #[arg(long, default_value_t = DEFAULT_HEARTBEAT_PULSE_SECS, help = "Pulse length in seconds")]
    pub(crate) pulse_seconds: u16,
    #[arg(long, default_value_t = DEFAULT_HEARTBEAT_TIMEOUT_SECS, help = "Idle timeout in seconds")]
    pub(crate) timeout_seconds: u16,
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct HeartbeatWatchArgs {
    #[arg(
        value_name = "PATH",
        help = "Paths to watch recursively (default: current dir)"
    )]
    pub(crate) paths: Vec<String>,
    #[arg(long, short = 'p', help = "Override project name")]
    pub(crate) project: Option<String>,
    #[arg(long, default_value = "watch", help = "Collector origin label")]
    pub(crate) origin: String,
    #[arg(long, default_value_t = DEFAULT_HEARTBEAT_PULSE_SECS, help = "Minimum seconds between heartbeats per entity")]
    pub(crate) pulse_seconds: u16,
    #[arg(long, default_value_t = 750, help = "Watcher debounce in milliseconds")]
    pub(crate) debounce_ms: u64,
    #[arg(long, help = "Only emit write/modify heartbeats")]
    pub(crate) writes_only: bool,
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct HeartbeatStatsArgs {
    #[arg(long, short = 's', help = "Start date filter (YYYYMMDD or YYYY-MM-DD)")]
    pub(crate) since: Option<String>,
    #[arg(long, short = 'u', help = "End date filter (YYYYMMDD or YYYY-MM-DD)")]
    pub(crate) until: Option<String>,
    #[arg(
        long,
        default_value_t = 7,
        help = "Trailing days when no explicit date range is given"
    )]
    pub(crate) days: u32,
    #[arg(long, short = 'p', help = "Filter specific project")]
    pub(crate) project: Option<String>,
    #[arg(
        long,
        short = 'z',
        help = "Timezone for date grouping (e.g. UTC, Asia/Tokyo)"
    )]
    pub(crate) timezone: Option<String>,
    #[arg(long, short = 'j', help = "Output JSON report")]
    pub(crate) json: bool,
    #[arg(
        long,
        short = 'q',
        help = "Process JSON output with jq expression (implies --json)"
    )]
    pub(crate) jq: Option<String>,
    #[arg(long, default_value_t = 5, help = "Breakdown rows per section")]
    pub(crate) limit: usize,
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct AntigravityArgs {
    #[arg(long, short = 'j', help = "Output JSON report")]
    pub(crate) json: bool,
    #[arg(
        long,
        short = 'z',
        help = "Timezone for date grouping (e.g. UTC, Asia/Tokyo)"
    )]
    pub(crate) timezone: Option<String>,
    #[arg(long, help = "Path to config JSON")]
    pub(crate) config: Option<String>,
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct MonthlyArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct WeeklyArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(long, short = 'w', value_enum, default_value_t = WeekStart::Sunday)]
    pub(crate) start_of_week: WeekStart,
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct SessionArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(long, short = 'i', help = "Filter to specific session id")]
    pub(crate) id: Option<String>,
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct BlocksArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(long, short = 'a', help = "Show only active block")]
    pub(crate) active: bool,
    #[arg(long, short = 'r', help = "Show recent blocks")]
    pub(crate) recent: bool,
    #[arg(
        long,
        short = 't',
        help = "Token limit for block warnings (number or 'max')"
    )]
    pub(crate) token_limit: Option<String>,
    #[arg(long, short = 'n', default_value_t = 5, help = "Session length hours")]
    pub(crate) session_length: u32,
    #[arg(long, help = "Live monitor mode for active block")]
    pub(crate) live: bool,
    #[arg(long, default_value_t = 1, help = "Live refresh interval seconds")]
    pub(crate) refresh_interval: u64,
    #[arg(
        long,
        help = "Fetch official Codex/Claude 5h/weekly usage + plan via OAuth APIs (with CLI fallback)"
    )]
    pub(crate) official_limits: bool,
}

#[derive(Debug, Args, Clone, Default)]
#[command(
    after_help = "Source shortcuts:\n  tu live codex   (equivalent to: tu live --no-claude)\n  tu live claude  (equivalent to: tu live --no-codex)"
)]
pub(crate) struct LiveArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(
        long,
        short = 't',
        help = "Token limit for block warnings (number or 'max')"
    )]
    pub(crate) token_limit: Option<String>,
    #[arg(long, short = 'n', default_value_t = 5, help = "Session length hours")]
    pub(crate) session_length: u32,
    #[arg(long, default_value_t = 1, help = "Live refresh interval seconds")]
    pub(crate) refresh_interval: u64,
}

impl From<LiveArgs> for BlocksArgs {
    fn from(value: LiveArgs) -> Self {
        Self {
            common: value.common,
            active: false,
            recent: false,
            token_limit: value.token_limit,
            session_length: value.session_length,
            live: true,
            refresh_interval: value.refresh_interval,
            official_limits: true,
        }
    }
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct TopArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(long, default_value_t = 2, help = "Refresh interval seconds")]
    pub(crate) refresh_interval: u64,
    #[arg(
        long,
        short = 'n',
        default_value_t = 50,
        help = "Max sessions to display"
    )]
    pub(crate) limit: usize,
    #[arg(
        long,
        default_value_t = 3,
        help = "Active window in hours (0 = show all)"
    )]
    pub(crate) active_hours: u64,
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct StatuslineArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(
        long,
        help = "Fetch official Codex/Claude 5h/week usage and plan via OAuth APIs (with CLI fallback)"
    )]
    pub(crate) official_limits: bool,
    #[arg(long, default_value_t = true, help = "Enable statusline cache")]
    pub(crate) cache: bool,
    #[arg(long, default_value_t = 1, help = "Cache refresh interval seconds")]
    pub(crate) refresh_interval: u64,
    #[arg(
        long,
        short = 'B',
        value_enum,
        default_value_t = VisualBurnRate::Off,
        help = "Burn-rate visual style"
    )]
    pub(crate) visual_burn_rate: VisualBurnRate,
    #[arg(
        long,
        value_enum,
        default_value_t = CostSource::Auto,
        help = "Session cost source: auto|derived|cc|both"
    )]
    pub(crate) cost_source: CostSource,
    #[arg(
        long,
        default_value_t = 50,
        help = "Context low threshold percentage (0-100)"
    )]
    pub(crate) context_low_threshold: u8,
    #[arg(
        long,
        default_value_t = 80,
        help = "Context medium threshold percentage (0-100)"
    )]
    pub(crate) context_medium_threshold: u8,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum GuiPeriod {
    #[default]
    Daily,
    Monthly,
    Weekly,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ImgPeriod {
    #[value(alias = "day")]
    Daily,
    #[value(alias = "week")]
    Weekly,
    #[default]
    #[value(hide = true)]
    Both,
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct GuiArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(long, value_enum, default_value_t = GuiPeriod::Daily)]
    pub(crate) period: GuiPeriod,
    #[arg(
        long,
        short = 'i',
        help = "Group by project/instance (daily mode only)"
    )]
    pub(crate) instances: bool,
    #[arg(long, short = 'p', help = "Filter specific project (daily mode only)")]
    pub(crate) project: Option<String>,
    #[arg(long, short = 'w', value_enum, default_value_t = WeekStart::Sunday)]
    pub(crate) start_of_week: WeekStart,
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct ImgArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(long, value_enum, default_value_t = ImgPeriod::Both)]
    pub(crate) period: ImgPeriod,
    #[arg(long, short = 'p', help = "Filter specific project (daily mode only)")]
    pub(crate) project: Option<String>,
    #[arg(
        long,
        short = 'f',
        default_value = "tokenusage-share.png",
        help = "Output image file path"
    )]
    pub(crate) output: String,
    #[arg(long, default_value_t = 1600, help = "Image width in pixels")]
    pub(crate) width: u32,
    #[arg(long, default_value_t = 900, help = "Image height in pixels")]
    pub(crate) height: u32,
    #[arg(long, help = "Portrait social card layout (9:16 style)")]
    pub(crate) portrait: bool,
    #[arg(long, default_value_t = 56, help = "Max periods rendered in charts")]
    pub(crate) bars: usize,
    #[arg(long, default_value = "tokenusage", help = "Brand label on the card")]
    pub(crate) brand: String,
    #[arg(
        long,
        default_value = "https://github.com/hanbu97/tokenusage",
        help = "Brand URL shown on the card"
    )]
    pub(crate) brand_url: String,
    #[arg(long, help = "Optional logo image path (SVG/PNG/JPG/WebP)")]
    pub(crate) logo: Option<String>,
}

pub(crate) fn normalize_cli_args(mut argv: Vec<String>) -> Vec<String> {
    normalize_live_shortcuts(&mut argv);
    normalize_img_shortcuts(&mut argv);

    let first = argv.get(1).map(String::as_str);
    let should_insert_daily = match first {
        None => true,
        Some(
            "-h" | "--help" | "-V" | "--version" | "help" | "daily" | "today" | "activity"
            | "heartbeat" | "monthly" | "weekly" | "img" | "session" | "blocks" | "live" | "top"
            | "statusline" | "gui" | "codex" | "claude" | "antigravity",
        ) => false,
        Some(arg) if arg.starts_with('-') => true,
        Some(_) => false,
    };

    if should_insert_daily {
        argv.insert(1, "daily".to_string());
    }

    argv
}

fn normalize_live_shortcuts(argv: &mut Vec<String>) {
    if argv.get(1).map(String::as_str) != Some("live") {
        return;
    }

    let Some(selector) = argv.get(2).map(String::as_str) else {
        return;
    };

    match selector {
        "codex" => {
            argv.remove(2);
            argv.retain(|arg| arg != "--no-codex");
            if !argv.iter().any(|arg| arg == "--no-claude") {
                argv.insert(2, "--no-claude".to_string());
            }
        }
        "claude" => {
            argv.remove(2);
            argv.retain(|arg| arg != "--no-claude");
            if !argv.iter().any(|arg| arg == "--no-codex") {
                argv.insert(2, "--no-codex".to_string());
            }
        }
        "antigravity" => {
            // Keep log sources (codex/claude) active for block tracking,
            // just ensure antigravity probe is not disabled.
            argv.remove(2);
            argv.retain(|arg| arg != "--no-antigravity");
        }
        _ => {}
    }
}

fn normalize_img_shortcuts(argv: &mut Vec<String>) {
    if argv.get(1).map(String::as_str) != Some("img") {
        return;
    }
    let Some(selector) = argv.get(2).map(String::as_str) else {
        return;
    };
    let period = match selector {
        "daily" | "day" => "daily",
        "weekly" | "week" => "weekly",
        _ => return,
    };
    argv.remove(2);
    argv.insert(2, "--period".to_string());
    argv.insert(3, period.to_string());
}
