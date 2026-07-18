#[cfg(feature = "cli")]
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Deserialize;

#[cfg(feature = "cli")]
use crate::heartbeat::{
    DEFAULT_HEARTBEAT_PULSE_SECS, DEFAULT_HEARTBEAT_TIMEOUT_SECS, HeartbeatEntityKind,
};
use crate::types::SourceKind;

// ---------------------------------------------------------------------------
// Shared enums — available with or without `cli` feature
// ---------------------------------------------------------------------------

#[cfg_attr(feature = "cli", derive(ValueEnum))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

#[cfg_attr(feature = "cli", derive(ValueEnum))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
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

#[cfg_attr(feature = "cli", derive(ValueEnum))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VisualBurnRate {
    #[default]
    Off,
    Emoji,
    Text,
    EmojiText,
}

#[cfg_attr(feature = "cli", derive(ValueEnum))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum CostSource {
    #[default]
    #[cfg_attr(feature = "cli", value(name = "auto"))]
    #[serde(rename = "auto")]
    Auto,
    #[cfg_attr(feature = "cli", value(name = "derived"))]
    #[serde(rename = "derived")]
    Derived,
    #[cfg_attr(feature = "cli", value(name = "cc"))]
    #[serde(rename = "cc")]
    Cc,
    #[cfg_attr(feature = "cli", value(name = "both"))]
    #[serde(rename = "both")]
    Both,
}

#[cfg_attr(feature = "cli", derive(ValueEnum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderArg {
    Claude,
    Codex,
    Gemini,
    Opencode,
    Grok,
}

impl ProviderArg {
    pub fn to_source_kind(self) -> SourceKind {
        match self {
            ProviderArg::Claude => SourceKind::Claude,
            ProviderArg::Codex => SourceKind::Codex,
            ProviderArg::Gemini => SourceKind::Gemini,
            ProviderArg::Opencode => SourceKind::OpenCode,
            ProviderArg::Grok => SourceKind::Grok,
        }
    }
}

// ---------------------------------------------------------------------------
// CommonArgs — shared struct, clap attributes conditional on `cli` feature
// ---------------------------------------------------------------------------

#[cfg_attr(feature = "cli", derive(Args))]
#[derive(Debug, Clone, Default)]
pub struct CommonArgs {
    #[cfg_attr(
        feature = "cli",
        arg(long, short = 's', help = "Start date filter (YYYYMMDD or YYYY-MM-DD)")
    )]
    pub since: Option<String>,
    #[cfg_attr(
        feature = "cli",
        arg(long, short = 'u', help = "End date filter (YYYYMMDD or YYYY-MM-DD)")
    )]
    pub until: Option<String>,
    #[cfg_attr(feature = "cli", arg(long, short = 'j', help = "Output JSON report"))]
    pub json: bool,
    #[cfg_attr(
        feature = "cli",
        arg(
            long,
            short = 'q',
            help = "Process JSON output with jq expression (implies --json)"
        )
    )]
    pub jq: Option<String>,
    #[cfg_attr(feature = "cli", arg(long, short = 'd', help = "Show debug summary"))]
    pub debug: bool,
    #[cfg_attr(
        feature = "cli",
        arg(long, default_value_t = 5, help = "Debug sample count")
    )]
    pub debug_samples: usize,
    #[cfg_attr(feature = "cli", arg(long, short = 'o', value_enum, default_value_t = SortOrder::Asc))]
    pub order: SortOrder,
    #[cfg_attr(
        feature = "cli",
        arg(long, short = 'b', help = "Show per-model breakdown")
    )]
    pub breakdown: bool,
    #[cfg_attr(
        feature = "cli",
        arg(long, short = 'O', help = "Use offline pricing behavior")
    )]
    pub offline: bool,
    #[cfg_attr(
        feature = "cli",
        arg(long, help = "Print models missing pricing data (doctor only)")
    )]
    pub pricing_debug: bool,
    #[cfg_attr(
        feature = "cli",
        arg(
            long,
            short = 'z',
            help = "Timezone for date grouping (e.g. UTC, Asia/Tokyo)"
        )
    )]
    pub timezone: Option<String>,
    #[cfg_attr(
        feature = "cli",
        arg(long, short = 'l', help = "Locale for date/time formatting")
    )]
    pub locale: Option<String>,
    #[cfg_attr(feature = "cli", arg(long, help = "Path to config JSON"))]
    pub config: Option<String>,
    #[cfg_attr(feature = "cli", arg(long, help = "Force compact table mode"))]
    pub compact: bool,
    #[cfg_attr(
        feature = "cli",
        arg(long, help = "One-line headline summary instead of the full table")
    )]
    pub brief: bool,
    #[cfg_attr(
        feature = "cli",
        arg(long, help = "Worker thread count (default: CPU cores)")
    )]
    pub workers: Option<usize>,
    #[cfg_attr(feature = "cli", arg(long, help = "Disable Claude source"))]
    pub no_claude: bool,
    #[cfg_attr(feature = "cli", arg(long, help = "Disable Codex source"))]
    pub no_codex: bool,
    #[cfg_attr(feature = "cli", arg(long, help = "Disable Gemini CLI source"))]
    pub no_gemini: bool,
    #[cfg_attr(feature = "cli", arg(long, help = "Disable OpenCode source"))]
    pub no_opencode: bool,
    #[cfg_attr(feature = "cli", arg(long, help = "Disable Grok Build source"))]
    pub no_grok: bool,
    #[cfg_attr(feature = "cli", arg(long, help = "Disable Antigravity quota probe"))]
    pub no_antigravity: bool,
    #[cfg_attr(
        feature = "cli",
        arg(
            long = "only",
            value_enum,
            value_delimiter = ',',
            num_args = 1..,
            help = "Only include log providers (repeatable or comma-separated)"
        )
    )]
    pub only: Vec<ProviderArg>,
    #[cfg_attr(
        feature = "cli",
        arg(
            long = "sources",
            value_enum,
            value_delimiter = ',',
            num_args = 1..,
            help = "Alias of --only for log-provider selection"
        )
    )]
    pub sources: Vec<ProviderArg>,
    #[cfg_attr(
        feature = "cli",
        arg(long = "claude-projects-dir", help = "Claude projects dir, repeatable")
    )]
    pub claude_projects_dir: Vec<String>,
    #[cfg_attr(
        feature = "cli",
        arg(long = "codex-sessions-dir", help = "Codex sessions dir, repeatable")
    )]
    pub codex_sessions_dir: Vec<String>,
    #[cfg_attr(
        feature = "cli",
        arg(
            long = "gemini-data-dir",
            help = "Gemini CLI data dir (defaults to ~/.gemini/tmp), repeatable"
        )
    )]
    pub gemini_data_dir: Vec<String>,
    #[cfg_attr(
        feature = "cli",
        arg(
            long = "opencode-data-dir",
            help = "OpenCode data dir (defaults to ~/.local/share/opencode), repeatable"
        )
    )]
    pub opencode_data_dir: Vec<String>,
    #[cfg_attr(
        feature = "cli",
        arg(
            long = "grok-log-dir",
            help = "Grok Build log dir (defaults to ~/.grok/logs), repeatable"
        )
    )]
    pub grok_log_dir: Vec<String>,
    #[cfg_attr(
        feature = "cli",
        arg(
            long = "ignore-path",
            help = "Ignore paths containing this substring (repeatable)"
        )
    )]
    pub ignore_path: Vec<String>,
    #[cfg_attr(
        feature = "cli",
        arg(long, help = "Disable built-in heavy directory ignore list")
    )]
    pub no_default_ignores: bool,
    #[cfg_attr(feature = "cli", arg(long, help = "Disable incremental parse cache"))]
    pub no_incremental_cache: bool,
    #[cfg_attr(
        feature = "cli",
        arg(long, help = "Rebuild incremental parse cache from scratch")
    )]
    pub rebuild_cache: bool,
    #[cfg_attr(
        feature = "cli",
        arg(long, help = "Optional pricing override JSON file")
    )]
    pub pricing_file: Option<String>,
    #[cfg_attr(
        feature = "cli",
        arg(long, help = "Enrich reports with locally inferred coding activity")
    )]
    pub with_activity: bool,
    #[cfg_attr(feature = "cli", arg(long = "slow", short = 'S', num_args = 0..=1, default_missing_value = "60", help = "Slow output for long content (optional: ms per line, default 30)"))]
    pub slow: Option<u64>,
}

impl CommonArgs {
    pub fn selected_sources(&self) -> Vec<SourceKind> {
        let mut selected = Vec::new();
        for provider in self.only.iter().chain(self.sources.iter()) {
            let kind = provider.to_source_kind();
            if !selected.contains(&kind) {
                selected.push(kind);
            }
        }
        selected
    }
}

// ---------------------------------------------------------------------------
// CLI-only types — only available when `cli` feature is enabled
// ---------------------------------------------------------------------------

#[cfg(feature = "cli")]
#[derive(Debug, Parser)]
#[command(
    name = "tokenusage",
    version,
    about = "Multi-source token usage analyzer (Rust)",
    after_help = "Commands by category:\n\
                  \x20 Reporting     today daily weekly monthly activity blocks session\n\
                  \x20 Live          live top gui\n\
                  \x20 Integration   statusline img heartbeat\n\
                  \x20 Diagnostics   doctor parity\n\
                  \x20 Balances      antigravity deepseek openrouter grok kimi anthropic-api\n\
                  \n\
                  Examples:\n\
                  tu\n\
                  tu --since 2026-04-01 --until 2026-04-30\n\
                  tu codex daily\n\
                  tu claude weekly --since 2026-04-01\n\
                  tu opencode doctor --json\n\
                  tu --only codex,gemini\n\
                  \n\
                  Default data locations (override with flags):\n\
                  - Claude:   ~/.claude/projects        (--claude-projects-dir)\n\
                  - Codex:    $CODEX_HOME/sessions      (--codex-sessions-dir)\n\
                  - Gemini:   ~/.gemini/tmp             (--gemini-data-dir)\n\
                  - OpenCode: $OPENCODE_DATA_DIR or ~/.local/share/opencode (--opencode-data-dir)\n\
                  \n\
                  GitHub:  https://github.com/hanbu97/tokenusage\n\
                  Issues:  https://github.com/hanbu97/tokenusage/issues"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[cfg(feature = "cli")]
#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    // --- Reporting (display_order 1-7) ---
    #[command(about = "Per-day token usage and cost", display_order = 2)]
    Daily(DailyArgs),
    #[command(about = "Today's token usage and coding activity", display_order = 1)]
    Today(TodayArgs),
    #[command(about = "Coding-activity view with per-day breakdowns", display_order = 5)]
    Activity(ActivityArgs),
    #[command(about = "Per-month token usage", display_order = 4)]
    Monthly(MonthlyArgs),
    #[command(about = "Per-week token usage", alias = "week", display_order = 3)]
    Weekly(WeeklyArgs),
    #[command(about = "Usage grouped by 5-hour billing blocks", display_order = 6)]
    Blocks(BlocksArgs),
    #[command(about = "Per-session token usage", display_order = 7)]
    Session(SessionArgs),

    // --- Live / interactive (display_order 10-12) ---
    #[command(about = "Live-updating usage view", display_order = 10)]
    Live(LiveArgs),
    #[command(
        about = "Real-time per-session token viewer (htop for tokens)",
        display_order = 11
    )]
    Top(TopArgs),
    #[command(about = "Desktop GUI (Iced)", display_order = 12)]
    Gui(GuiArgs),

    // --- Integration (display_order 20-22) ---
    #[command(
        about = "One-line status for the Claude Code statusLine hook",
        display_order = 20
    )]
    Statusline(StatuslineArgs),
    #[command(about = "Render a usage report as a PNG", display_order = 21)]
    Img(ImgArgs),
    #[command(about = "Local editor-activity heartbeat collector and stats", display_order = 22)]
    Heartbeat(HeartbeatArgs),

    // --- Diagnostics (display_order 30-31) ---
    #[command(about = "Inspect roots, files, cache and pricing health", display_order = 30)]
    Doctor(DailyArgs),
    #[command(about = "Compare tu totals against ccusage", display_order = 31)]
    Parity(ParityArgs),

    // --- Balances / quota (display_order 40-45) ---
    #[command(about = "Show Antigravity plan and usage limits", display_order = 40)]
    Antigravity(AntigravityArgs),
    #[command(
        about = "Show DeepSeek API credit balance (DEEPSEEK_API_KEY)",
        display_order = 41
    )]
    Deepseek(DeepseekArgs),
    #[command(
        about = "Show OpenRouter API credit balance (OPENROUTER_API_KEY)",
        display_order = 42
    )]
    Openrouter(OpenrouterArgs),
    #[command(about = "Show Grok (xAI) credit balance (XAI_API_KEY)", display_order = 43)]
    Grok(GrokArgs),
    #[command(
        about = "Show Kimi (Moonshot) credit balance (MOONSHOT_API_KEY)",
        display_order = 44
    )]
    Kimi(KimiArgs),
    #[command(
        about = "Show Anthropic API usage today (ANTHROPIC_API_KEY / ANTHROPIC_ADMIN_KEY)",
        display_order = 45
    )]
    AnthropicApi(AnthropicApiArgs),
}

#[cfg(feature = "cli")]
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

#[cfg(feature = "cli")]
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct TodayArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(long, short = 'p', help = "Filter specific project")]
    pub(crate) project: Option<String>,
}

#[cfg(feature = "cli")]
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

#[cfg(feature = "cli")]
#[derive(Debug, Args, Clone)]
pub(crate) struct HeartbeatArgs {
    #[command(subcommand)]
    pub(crate) command: HeartbeatCommand,
}

#[cfg(feature = "cli")]
#[derive(Debug, Subcommand, Clone)]
pub(crate) enum HeartbeatCommand {
    Ping(HeartbeatPingArgs),
    Watch(HeartbeatWatchArgs),
    Stats(HeartbeatStatsArgs),
}

#[cfg(feature = "cli")]
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

#[cfg(feature = "cli")]
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

#[cfg(feature = "cli")]
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
    #[arg(
        long = "slow",
        short = 'S',
        num_args = 0..=1,
        default_missing_value = "60",
        help = "Slow output for long content (optional: ms per line, default 60)"
    )]
    pub(crate) slow: Option<u64>,
}

#[cfg(feature = "cli")]
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

#[cfg(feature = "cli")]
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct DeepseekArgs {
    #[arg(long, short = 'j', help = "Output JSON report")]
    pub(crate) json: bool,
}

#[cfg(feature = "cli")]
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct OpenrouterArgs {
    #[arg(long, short = 'j', help = "Output JSON report")]
    pub(crate) json: bool,
}

#[cfg(feature = "cli")]
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct GrokArgs {
    #[arg(long, short = 'j', help = "Output JSON report")]
    pub(crate) json: bool,
}

#[cfg(feature = "cli")]
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct KimiArgs {
    #[arg(long, short = 'j', help = "Output JSON report")]
    pub(crate) json: bool,
}

#[cfg(feature = "cli")]
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct AnthropicApiArgs {
    #[arg(long, short = 'j', help = "Output JSON report")]
    pub(crate) json: bool,
}

#[cfg(feature = "cli")]
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct MonthlyArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[cfg(feature = "cli")]
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct WeeklyArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(long, short = 'w', value_enum, default_value_t = WeekStart::Sunday)]
    pub(crate) start_of_week: WeekStart,
}

#[cfg(feature = "cli")]
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct SessionArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(long, short = 'i', help = "Filter to specific session id")]
    pub(crate) id: Option<String>,
}

#[cfg(feature = "cli")]
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
    #[arg(long, help = "Run non-interactive startup/data smoke check and exit")]
    pub(crate) smoke_check: bool,
    #[arg(
        long,
        help = "Fetch official Codex/Claude 5h/weekly usage + plan via OAuth APIs (with CLI fallback)"
    )]
    pub(crate) official_limits: bool,
}

#[cfg(feature = "cli")]
#[derive(Debug, Args, Clone, Default)]
#[command(
    after_help = "Source shortcuts:\n  tu live codex       (equivalent to: tu live --no-claude --no-gemini --no-opencode)\n  tu live claude      (equivalent to: tu live --no-codex --no-gemini --no-opencode)\n  tu live gemini      (equivalent to: tu live --no-codex --no-claude --no-opencode)\n  tu live opencode    (equivalent to: tu live --no-codex --no-claude --no-gemini)\n  tu live antigravity (alias supported; keeps all sources enabled)"
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
    #[arg(long, help = "Run non-interactive startup/data smoke check and exit")]
    pub(crate) smoke_check: bool,
}

#[cfg(feature = "cli")]
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
            smoke_check: value.smoke_check,
            official_limits: true,
        }
    }
}

#[cfg(feature = "cli")]
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
    #[arg(long, help = "Run non-interactive startup/data smoke check and exit")]
    pub(crate) smoke_check: bool,
}

#[cfg(feature = "cli")]
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
    #[arg(
        long,
        value_name = "NAME",
        help = "Emit a single field (e.g. today-cost, block-left) instead of the full line"
    )]
    pub(crate) field: Option<String>,
    #[arg(
        long,
        value_name = "TEMPLATE",
        help = "Emit a custom line, substituting {field} placeholders (e.g. \"{today-cost} {block-left}\")"
    )]
    pub(crate) format: Option<String>,
    #[command(subcommand)]
    pub(crate) action: Option<StatuslineAction>,
}

#[cfg(feature = "cli")]
#[derive(Debug, Subcommand, Clone)]
pub(crate) enum StatuslineAction {
    /// Wire tu into your Claude Code status line (or print the config/integration prompt)
    Init(StatuslineInitArgs),
}

#[cfg(feature = "cli")]
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct StatuslineInitArgs {
    #[arg(
        long,
        help = "Print the settings block (or integration prompt) without writing any files"
    )]
    pub(crate) print: bool,
    #[arg(
        long,
        help = "Emit a copy-paste prompt to hand your AI assistant for integrating tu into an existing ccstatusline / custom statusline"
    )]
    pub(crate) ccstatusline: bool,
    #[arg(
        long,
        short = 'y',
        help = "Skip the confirmation prompt (non-interactive / agent use)"
    )]
    pub(crate) yes: bool,
    #[arg(
        long,
        value_name = "FLAGS",
        help = "Flags to bake into the standalone command (default: \"--cache --refresh-interval 30\")"
    )]
    pub(crate) flags: Option<String>,
    #[arg(long, hide = true, value_name = "FILE")]
    /// Override the settings.json path (testing only).
    pub(crate) settings_path: Option<String>,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum GuiPeriod {
    #[default]
    Daily,
    Monthly,
    Weekly,
}

#[cfg(feature = "cli")]
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

#[cfg(feature = "cli")]
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

#[cfg(feature = "cli")]
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

#[cfg(feature = "cli")]
pub(crate) fn normalize_cli_args(mut argv: Vec<String>) -> Vec<String> {
    normalize_provider_first(&mut argv);
    normalize_live_shortcuts(&mut argv);
    normalize_img_shortcuts(&mut argv);

    let first = argv.get(1).map(String::as_str);
    let should_insert_daily = match first {
        None => true,
        Some(
            "-h" | "--help" | "-V" | "--version" | "help" | "daily" | "today" | "activity"
            | "heartbeat" | "doctor" | "parity" | "monthly" | "weekly" | "week" | "img" | "session"
            | "blocks" | "live" | "top" | "statusline" | "gui" | "antigravity",
        ) => false,
        Some(arg) if arg.starts_with('-') => true,
        Some(_) => false,
    };

    if should_insert_daily {
        argv.insert(1, "daily".to_string());
    }

    argv
}

#[cfg(feature = "cli")]
#[derive(Debug, Args, Clone)]
pub(crate) struct ParityArgs {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[arg(
        long,
        value_enum,
        default_value_t = ProviderArg::Claude,
        help = "Provider to compare"
    )]
    pub(crate) provider: ProviderArg,
    #[arg(
        long,
        value_enum,
        default_value_t = ParityPeriod::Daily,
        help = "Period to compare"
    )]
    pub(crate) period: ParityPeriod,
    #[arg(
        long,
        value_enum,
        default_value_t = ParityOpencodeScope::Merged,
        help = "OpenCode parity compare scope (merged matches tu default; legacy-only matches @ccusage/opencode ingestion scope)"
    )]
    pub(crate) opencode_scope: ParityOpencodeScope,
    #[arg(
        long,
        default_value_t = 0,
        help = "Maximum allowed absolute token delta for totals"
    )]
    pub(crate) max_token_delta: u64,
    #[arg(
        long,
        default_value_t = 0.0,
        help = "Maximum allowed absolute USD delta for totals"
    )]
    pub(crate) max_cost_delta: f64,
    #[arg(
        long,
        default_value_t = false,
        help = "Exit with error when parity thresholds are exceeded"
    )]
    pub(crate) fail_on_delta: bool,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ParityPeriod {
    #[default]
    Daily,
    Weekly,
    Monthly,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ParityOpencodeScope {
    #[default]
    Merged,
    LegacyOnly,
}

#[cfg(feature = "cli")]
fn normalize_provider_first(argv: &mut Vec<String>) {
    let Some(provider) = argv.get(1).map(|s| s.to_ascii_lowercase()) else {
        return;
    };
    if !matches!(
        provider.as_str(),
        "claude" | "codex" | "gemini" | "opencode"
    ) {
        return;
    }

    let report = argv
        .get(2)
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| "daily".to_string());
    if argv.get(2).is_some_and(|arg| arg.starts_with('-')) {
        argv.insert(2, "daily".to_string());
    }

    let target = match report.as_str() {
        "daily" | "weekly" | "monthly" | "live" | "top" | "statusline" | "img" | "blocks"
        | "gui" | "session" | "doctor" | "today" | "activity" => report,
        _ => "daily".to_string(),
    };

    argv.remove(1);
    if argv.get(1).is_some_and(|v| v != &target) {
        argv.insert(1, target);
    } else if argv.get(1).is_none() {
        argv.push(target);
    }

    if !argv.iter().any(|arg| arg == "--only") && !argv.iter().any(|arg| arg == "--sources") {
        argv.insert(2, "--only".to_string());
        argv.insert(3, provider);
    }
}

#[cfg(feature = "cli")]
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
            if !argv.iter().any(|arg| arg == "--no-gemini") {
                argv.insert(2, "--no-gemini".to_string());
            }
            if !argv.iter().any(|arg| arg == "--no-opencode") {
                argv.insert(2, "--no-opencode".to_string());
            }
        }
        "claude" => {
            argv.remove(2);
            argv.retain(|arg| arg != "--no-claude");
            if !argv.iter().any(|arg| arg == "--no-codex") {
                argv.insert(2, "--no-codex".to_string());
            }
            if !argv.iter().any(|arg| arg == "--no-gemini") {
                argv.insert(2, "--no-gemini".to_string());
            }
            if !argv.iter().any(|arg| arg == "--no-opencode") {
                argv.insert(2, "--no-opencode".to_string());
            }
        }
        "gemini" => {
            argv.remove(2);
            argv.retain(|arg| arg != "--no-gemini");
            if !argv.iter().any(|arg| arg == "--no-codex") {
                argv.insert(2, "--no-codex".to_string());
            }
            if !argv.iter().any(|arg| arg == "--no-claude") {
                argv.insert(2, "--no-claude".to_string());
            }
            if !argv.iter().any(|arg| arg == "--no-opencode") {
                argv.insert(2, "--no-opencode".to_string());
            }
        }
        "opencode" => {
            argv.remove(2);
            argv.retain(|arg| arg != "--no-opencode");
            if !argv.iter().any(|arg| arg == "--no-codex") {
                argv.insert(2, "--no-codex".to_string());
            }
            if !argv.iter().any(|arg| arg == "--no-claude") {
                argv.insert(2, "--no-claude".to_string());
            }
            if !argv.iter().any(|arg| arg == "--no-gemini") {
                argv.insert(2, "--no-gemini".to_string());
            }
        }
        "antigravity" => {
            argv.remove(2);
            argv.retain(|arg| arg != "--no-antigravity");
        }
        _ => {}
    }
}

#[cfg(feature = "cli")]
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

#[cfg(test)]
mod tests {
    use super::normalize_cli_args;

    #[test]
    fn provider_first_daily_is_rewritten() {
        let argv = vec![
            "tu".to_string(),
            "codex".to_string(),
            "daily".to_string(),
            "--json".to_string(),
        ];
        let out = normalize_cli_args(argv);
        assert_eq!(out[1], "daily");
        assert_eq!(out[2], "--only");
        assert_eq!(out[3], "codex");
    }

    #[test]
    fn provider_first_weekly_is_rewritten() {
        let argv = vec![
            "tu".to_string(),
            "claude".to_string(),
            "weekly".to_string(),
            "--since".to_string(),
            "20260401".to_string(),
        ];
        let out = normalize_cli_args(argv);
        assert_eq!(out[1], "weekly");
        assert_eq!(out[2], "--only");
        assert_eq!(out[3], "claude");
    }

    #[test]
    fn provider_first_live_is_rewritten() {
        let argv = vec!["tu".to_string(), "gemini".to_string(), "live".to_string()];
        let out = normalize_cli_args(argv);
        assert_eq!(out[1], "live");
        assert_eq!(out[2], "--only");
        assert_eq!(out[3], "gemini");
    }

    #[test]
    fn provider_first_doctor_is_rewritten() {
        let argv = vec![
            "tu".to_string(),
            "opencode".to_string(),
            "doctor".to_string(),
            "--json".to_string(),
        ];
        let out = normalize_cli_args(argv);
        assert_eq!(out[1], "doctor");
        assert_eq!(out[2], "--only");
        assert_eq!(out[3], "opencode");
    }

    #[test]
    fn provider_first_defaults_to_daily_when_missing_report() {
        let argv = vec!["tu".to_string(), "codex".to_string(), "--json".to_string()];
        let out = normalize_cli_args(argv);
        assert_eq!(out[1], "daily");
        assert_eq!(out[2], "--only");
        assert_eq!(out[3], "codex");
    }

    #[test]
    fn provider_first_monthly_top_statusline_img_blocks_gui_are_rewritten() {
        for report in ["monthly", "top", "statusline", "img", "blocks", "gui"] {
            let argv = vec!["tu".to_string(), "codex".to_string(), report.to_string()];
            let out = normalize_cli_args(argv);
            assert_eq!(out[1], report);
            assert_eq!(out[2], "--only");
            assert_eq!(out[3], "codex");
        }
    }
}
