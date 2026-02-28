use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Deserialize;

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
    about = "Multi-source token usage analyzer (Rust)"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    Daily(DailyArgs),
    Codex(DailyArgs),
    Claude(DailyArgs),
    Monthly(MonthlyArgs),
    Weekly(WeeklyArgs),
    Session(SessionArgs),
    Blocks(BlocksArgs),
    Live(LiveArgs),
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
        }
    }
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct StatuslineArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
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

pub(crate) fn normalize_cli_args(mut argv: Vec<String>) -> Vec<String> {
    normalize_live_shortcuts(&mut argv);

    let first = argv.get(1).map(String::as_str);
    let should_insert_daily = match first {
        None => true,
        Some(
            "-h" | "--help" | "-V" | "--version" | "help" | "daily" | "monthly" | "weekly"
            | "session" | "blocks" | "live" | "statusline" | "gui" | "codex" | "claude",
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
        _ => {}
    }
}
