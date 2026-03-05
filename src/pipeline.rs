use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use crossbeam_channel::{Receiver, bounded};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ignore::WalkBuilder;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color as TuiColor, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph, Wrap};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::fs;

use crate::cli::{
    AntigravityArgs, BlocksArgs, CommonArgs, CostSource, DailyArgs, MonthlyArgs, SessionArgs,
    SortOrder, StatuslineArgs, VisualBurnRate, WeekStart, WeeklyArgs,
};
use crate::output::{print_report_table_with_options, run_report_tui};
use crate::types::{
    CodexParseState, CodexRawUsage, DailyReport, DailyRow, DateFilter, DiscoveredFile,
    LEGACY_CODEX_FALLBACK_MODEL, ParseLineResult, ParseStats, ParseStatsAtomic, ParsedLine,
    PricingRate, PricingTable, SourceConfig, SourceKind, TokenCounts, UsageAccumulator, UsageEvent,
};

#[derive(Debug, Clone)]
pub(crate) enum TimeZoneMode {
    Local,
    Utc,
    Named(Tz),
}

impl TimeZoneMode {
    pub(crate) fn date_of(&self, ts: DateTime<Utc>) -> NaiveDate {
        match self {
            TimeZoneMode::Local => ts.with_timezone(&Local).date_naive(),
            TimeZoneMode::Utc => ts.date_naive(),
            TimeZoneMode::Named(tz) => ts.with_timezone(tz).date_naive(),
        }
    }

    pub(crate) fn hour_of(&self, ts: DateTime<Utc>) -> u32 {
        match self {
            TimeZoneMode::Local => ts.with_timezone(&Local).hour(),
            TimeZoneMode::Utc => ts.hour(),
            TimeZoneMode::Named(tz) => ts.with_timezone(tz).hour(),
        }
    }

    pub(crate) fn now_date(&self) -> NaiveDate {
        self.date_of(Utc::now())
    }
}

#[derive(Debug, Default, Clone)]
struct GroupAggregate {
    totals: UsageAccumulator,
    by_model: HashMap<String, UsageAccumulator>,
    by_source: HashMap<SourceKind, UsageAccumulator>,
    last_activity: Option<DateTime<Utc>>,
    project: Option<String>,
}

impl GroupAggregate {
    fn add_event(&mut self, event: &UsageEvent) {
        self.totals.add(event.usage);
        let merged_model = event.model.clone();
        self.by_model
            .entry(merged_model)
            .or_default()
            .add(event.usage);
        self.by_source
            .entry(event.source)
            .or_default()
            .add(event.usage);

        self.last_activity = match self.last_activity {
            Some(ts) if ts >= event.timestamp => Some(ts),
            _ => Some(event.timestamp),
        };

        if self.project.is_none() {
            self.project = event.project.clone();
        }
    }
}

#[derive(Debug)]
struct LoadedUsage {
    events: Vec<UsageEvent>,
    stats: ParseStats,
}

#[derive(Debug)]
struct ParsedUsageOutput {
    loaded: LoadedUsage,
    cache_dirty: bool,
}

#[derive(Debug)]
struct LiveUsageRuntime {
    filter: DateFilter,
    sources: Vec<SourceConfig>,
    ignore_rules: PathIgnoreRules,
    pricing: Arc<PricingTable>,
    worker_count: usize,
    cache_enabled: bool,
    cache_store: IncrementalCacheStore,
    cache_path: Option<PathBuf>,
    cache_dirty: bool,
    files_cache: Vec<DiscoveredFile>,
    last_discovery_at: Instant,
    discovery_interval: Duration,
    last_sources_refresh_at: Instant,
    sources_refresh_interval: Duration,
    last_cache_flush_at: Instant,
}

pub(crate) struct UsageSnapshot {
    pub(crate) events: Vec<UsageEvent>,
    pub(crate) stats: ParseStats,
    pub(crate) timezone: TimeZoneMode,
}

#[derive(Debug, Serialize)]
struct SessionJsonRow {
    session_id: String,
    project: Option<String>,
    last_activity: String,
    totals: TokenCounts,
    models: BTreeMap<String, TokenCounts>,
    sources: BTreeMap<String, TokenCounts>,
}

#[derive(Debug, Serialize)]
struct SessionJsonReport {
    sessions: Vec<SessionJsonRow>,
    totals: TokenCounts,
    stats: ParseStats,
}

#[derive(Debug, Serialize)]
struct BlockJsonRow {
    id: String,
    start_time: String,
    end_time: String,
    is_active: bool,
    totals: TokenCounts,
    models: BTreeMap<String, TokenCounts>,
    percent_of_limit: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct MembershipSourceEstimate {
    source: String,
    estimated_plan: String,
    estimated_window_tokens: u64,
    observed_peak_window_tokens: u64,
    observed_p95_window_tokens: u64,
    observed_total_tokens: u64,
    completed_blocks: usize,
    confidence: f64,
}

#[derive(Debug, Clone, Serialize)]
struct MembershipEstimate {
    estimated_plan: String,
    estimated_window_tokens: u64,
    observed_peak_window_tokens: u64,
    observed_p95_window_tokens: u64,
    observed_total_tokens: u64,
    completed_blocks: usize,
    confidence: f64,
    source_breakdown: Vec<MembershipSourceEstimate>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TokenLimitSource {
    Explicit,
    HistoricalMax,
    EstimatedFromLogs,
    Unset,
}

#[derive(Debug, Serialize)]
struct BlockJsonReport {
    blocks: Vec<BlockJsonRow>,
    totals: TokenCounts,
    stats: ParseStats,
    token_limit: Option<u64>,
    token_limit_source: TokenLimitSource,
    membership_estimate: Option<MembershipEstimate>,
    official_codex: Option<OfficialCodexSnapshot>,
    official_claude: Option<OfficialClaudeSnapshot>,
    official_antigravity: Option<OfficialAntigravitySnapshot>,
}

#[derive(Debug, Clone)]
struct BlockReportBuildOptions {
    order: SortOrder,
    recent_only: bool,
    active_only: bool,
    window_secs: i64,
    token_limit: Option<u64>,
    token_limit_source: TokenLimitSource,
    membership_estimate: Option<MembershipEstimate>,
    official_codex: Option<OfficialCodexSnapshot>,
    official_claude: Option<OfficialClaudeSnapshot>,
    official_antigravity: Option<OfficialAntigravitySnapshot>,
    now: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
struct OfficialCodexSnapshot {
    plan_type: Option<String>,
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
    primary_window_mins: Option<i64>,
    secondary_window_mins: Option<i64>,
    primary_resets_at: Option<i64>,
    secondary_resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
struct OfficialClaudeSnapshot {
    plan_type: Option<String>,
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
    primary_window_mins: Option<i64>,
    secondary_window_mins: Option<i64>,
    primary_resets_at: Option<i64>,
    secondary_resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
struct AntigravityModelQuotaSnapshot {
    label: String,
    model_id: String,
    remaining_fraction: Option<f64>,
    reset_time: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
struct OfficialAntigravitySnapshot {
    plan_type: Option<String>,
    account_email: Option<String>,
    models: Vec<AntigravityModelQuotaSnapshot>,
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
    tertiary_used_percent: Option<f64>,
    primary_label: Option<String>,
    secondary_label: Option<String>,
    tertiary_label: Option<String>,
    primary_resets_at: Option<i64>,
    secondary_resets_at: Option<i64>,
    tertiary_resets_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct StatuslineHookInput {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    transcript_path: Option<String>,
    #[serde(default)]
    model: Option<StatuslineHookModel>,
    #[serde(default)]
    cost: Option<StatuslineHookCost>,
    #[serde(default)]
    context_window: Option<StatuslineHookContext>,
}

#[derive(Debug, Deserialize)]
struct StatuslineHookModel {
    #[serde(default)]
    id: Option<String>,
    #[serde(default, alias = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StatuslineHookCost {
    #[serde(default, alias = "totalCostUsd", alias = "total", alias = "total_cost")]
    total_cost_usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct StatuslineHookContext {
    #[serde(default, alias = "totalInputTokens")]
    total_input_tokens: Option<u64>,
    #[serde(default, alias = "contextWindowSize")]
    context_window_size: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StatuslineCacheEntry {
    updated_unix: u64,
    transcript_path: Option<String>,
    transcript_mtime_unix: Option<u64>,
    line: String,
}

#[derive(Debug)]
struct ActiveBlockSummary {
    totals: TokenCounts,
    remaining_minutes: i64,
    burn: Option<BurnRateSummary>,
    dominant_source: Option<SourceKind>,
}

#[derive(Debug, Clone, Copy)]
struct LimitDisplayContext<'a> {
    token_limit: Option<u64>,
    token_limit_source: TokenLimitSource,
    membership_estimate: Option<&'a MembershipEstimate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveTab {
    Overview,
    Codex,
    Claude,
    Antigravity,
}

const ALL_LIVE_TABS: &[LiveTab] = &[
    LiveTab::Overview,
    LiveTab::Codex,
    LiveTab::Claude,
    LiveTab::Antigravity,
];

impl LiveTab {
    fn label(self) -> &'static str {
        match self {
            LiveTab::Overview => "Overview",
            LiveTab::Codex => "Codex",
            LiveTab::Claude => "Claude",
            LiveTab::Antigravity => "Antigravity",
        }
    }

    fn next(self) -> Self {
        let idx = ALL_LIVE_TABS.iter().position(|&t| t == self).unwrap_or(0);
        ALL_LIVE_TABS[(idx + 1) % ALL_LIVE_TABS.len()]
    }

    fn prev(self) -> Self {
        let idx = ALL_LIVE_TABS.iter().position(|&t| t == self).unwrap_or(0);
        ALL_LIVE_TABS[(idx + ALL_LIVE_TABS.len() - 1) % ALL_LIVE_TABS.len()]
    }
}

enum LiveInputEvent {
    Exit,
    SwitchTab(LiveTab),
    Tick,
}

#[derive(Debug, Clone)]
struct LiveFrameContext<'a> {
    now: DateTime<Utc>,
    window_secs: i64,
    elapsed_secs: i64,
    tz: &'a TimeZoneMode,
    now_text: String,
    block_start_text: String,
    block_end_text: String,
    limit: LimitDisplayContext<'a>,
    official_codex: Option<&'a OfficialCodexSnapshot>,
    official_claude: Option<&'a OfficialClaudeSnapshot>,
    official_antigravity: Option<&'a OfficialAntigravitySnapshot>,
    selected_source: Option<SourceKind>,
    today_totals: TokenCounts,
    last_30d_totals: TokenCounts,
    last_30d_active_days: u32,
    active: Option<&'a ActiveBlockSummary>,
    active_tab: LiveTab,
}

impl<'a> LiveFrameContext<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        now: DateTime<Utc>,
        tz: &'a TimeZoneMode,
        block_start_unix: i64,
        block_end_unix: i64,
        limit: LimitDisplayContext<'a>,
        official_codex: Option<&'a OfficialCodexSnapshot>,
        official_claude: Option<&'a OfficialClaudeSnapshot>,
        official_antigravity: Option<&'a OfficialAntigravitySnapshot>,
        selected_source: Option<SourceKind>,
        today_totals: TokenCounts,
        last_30d_totals: TokenCounts,
        last_30d_active_days: u32,
        active: Option<&'a ActiveBlockSummary>,
        active_tab: LiveTab,
    ) -> Self {
        let now_unix = now.timestamp();
        let block_start = DateTime::from_timestamp(block_start_unix, 0).unwrap_or(now);
        let block_end = DateTime::from_timestamp(block_end_unix, 0)
            .unwrap_or(block_start + chrono::TimeDelta::seconds(5 * 3600));
        let window_secs = (block_end_unix - block_start_unix).max(1);

        Self {
            now,
            window_secs,
            elapsed_secs: (now_unix - block_start_unix).clamp(0, window_secs.max(1)),
            tz,
            now_text: format_display_datetime(now, tz),
            block_start_text: format_display_datetime(block_start, tz),
            block_end_text: format_display_datetime(block_end, tz),
            limit,
            official_codex,
            official_claude,
            official_antigravity,
            selected_source,
            today_totals,
            last_30d_totals,
            last_30d_active_days,
            active,
            active_tab,
        }
    }
}

#[derive(Debug)]
struct BurnRateSummary {
    cost_per_hour: f64,
    tokens_per_minute: f64,
    status: BurnStatus,
}

#[derive(Debug, Clone, Copy)]
enum BurnStatus {
    Normal,
    Moderate,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReportPeriod {
    Daily,
    Monthly,
    Weekly,
}

const DEFAULT_IGNORED_DIR_NAMES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    "__pycache__",
    ".venv",
    "venv",
];
const INCREMENTAL_CACHE_VERSION: u32 = 2;
const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const OPENROUTER_PRICING_CACHE_VERSION: u32 = 1;
const OPENROUTER_PRICING_CACHE_TTL_SECS: u64 = 6 * 60 * 60;
const CLAUDE_RECENT_DEDUPE_KEYS_LIMIT: usize = 8192;
const CLAUDE_OAUTH_REFRESH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CLAUDE_OAUTH_REFRESH_URL: &str = "https://platform.claude.com/v1/oauth/token";

#[derive(Debug, Clone, Copy)]
enum TokenLimitMode {
    Exact(u64),
    MaxHistorical,
}

#[derive(Debug, Clone)]
struct PathIgnoreRules {
    ignored_dir_names: HashSet<String>,
    path_fragments: Vec<String>,
}

impl PathIgnoreRules {
    fn from_common(common: &CommonArgs) -> Self {
        let ignored_dir_names = if common.no_default_ignores {
            HashSet::new()
        } else {
            DEFAULT_IGNORED_DIR_NAMES
                .iter()
                .map(|name| (*name).to_string())
                .collect()
        };

        let path_fragments = common
            .ignore_path
            .iter()
            .filter_map(|raw| normalize_ignore_fragment(raw))
            .collect();

        Self {
            ignored_dir_names,
            path_fragments,
        }
    }

    fn should_skip_dir(&self, path: &Path) -> bool {
        if let Some(name) = path.file_name().and_then(|s| s.to_str())
            && self.ignored_dir_names.contains(name)
        {
            return true;
        }

        self.matches_fragment(path)
    }

    fn should_skip_path(&self, path: &Path) -> bool {
        self.matches_fragment(path)
    }

    fn matches_fragment(&self, path: &Path) -> bool {
        if self.path_fragments.is_empty() {
            return false;
        }

        let normalized = path.to_string_lossy().replace('\\', "/");
        self.path_fragments
            .iter()
            .any(|fragment| normalized.contains(fragment))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
struct FileFingerprint {
    size: u64,
    modified_unix_secs: i64,
    modified_unix_nanos: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedUsageEvent {
    timestamp: DateTime<Utc>,
    model: String,
    usage: UsageAccumulator,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CachedFileStats {
    lines_total: usize,
    lines_invalid_json: usize,
    lines_missing_usage: usize,
    lines_unknown_pricing: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedFileEntry {
    fingerprint: FileFingerprint,
    stats: CachedFileStats,
    events: Vec<CachedUsageEvent>,
    #[serde(default)]
    parsed_offset: u64,
    #[serde(default)]
    codex_last_model: Option<String>,
    #[serde(default)]
    codex_last_totals: Option<CodexRawUsage>,
    #[serde(default)]
    claude_recent_keys: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct IncrementalCacheStore {
    version: u32,
    pricing_key: String,
    files: HashMap<String, CachedFileEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenRouterPricingCacheStore {
    version: u32,
    fetched_unix: u64,
    exact: HashMap<String, PricingRate>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModelsResponse {
    data: Vec<OpenRouterModelEntry>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModelEntry {
    id: String,
    pricing: OpenRouterPricingEntry,
}

#[derive(Debug, Deserialize)]
struct OpenRouterPricingEntry {
    #[serde(default)]
    prompt: Option<OpenRouterNumber>,
    #[serde(default)]
    completion: Option<OpenRouterNumber>,
    #[serde(default)]
    input_cache_read: Option<OpenRouterNumber>,
    #[serde(default)]
    input_cache_write: Option<OpenRouterNumber>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum OpenRouterNumber {
    Number(f64),
    String(String),
}

impl IncrementalCacheStore {
    fn new(pricing_key: String) -> Self {
        Self {
            version: INCREMENTAL_CACHE_VERSION,
            pricing_key,
            files: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct FileParseJob {
    file: DiscoveredFile,
    cache_key: String,
    fingerprint: FileFingerprint,
    strategy: ParseStrategy,
}

#[derive(Debug, Clone)]
enum ParseStrategy {
    Full,
    Incremental { base_cache: CachedFileEntry },
}

#[derive(Debug, Default)]
struct WorkerParseOutput {
    events: Vec<UsageEvent>,
    cache_updates: Vec<(String, CachedFileEntry)>,
}

#[derive(Debug)]
struct ParsedFileOutput {
    events: Vec<UsageEvent>,
    cache_entry: CachedFileEntry,
}

#[derive(Debug, Default)]
struct ClaudeDedupeState {
    seen_keys: HashSet<String>,
    insertion_order: VecDeque<String>,
}

impl ClaudeDedupeState {
    fn with_seed(seed: Vec<String>) -> Self {
        let mut out = Self::default();
        for key in seed {
            out.insert(key);
        }
        out
    }

    fn insert(&mut self, key: String) -> bool {
        if !self.seen_keys.insert(key.clone()) {
            return false;
        }
        self.insertion_order.push_back(key);
        while self.insertion_order.len() > CLAUDE_RECENT_DEDUPE_KEYS_LIMIT {
            if let Some(old) = self.insertion_order.pop_front() {
                self.seen_keys.remove(&old);
            }
        }
        true
    }

    fn snapshot(&self) -> Vec<String> {
        self.insertion_order.iter().cloned().collect()
    }
}

pub(crate) async fn run_daily(args: DailyArgs) -> Result<()> {
    let use_json = should_emit_json(&args.common);

    if use_json && args.tui {
        bail!("--json/--jq and --tui cannot be used together");
    }

    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let loaded = load_usage(&args.common, &tz).await?;

    let events = loaded
        .events
        .into_iter()
        .filter(|e| match args.project.as_deref() {
            Some(project_filter) => e
                .project
                .as_deref()
                .is_some_and(|p| p.contains(project_filter)),
            None => true,
        })
        .collect::<Vec<_>>();

    let rows = build_group_rows(&events, &args.common.order, |event| {
        let day = local_date(event.timestamp, &tz);
        let day_key = day.format("%Y-%m-%d").to_string();
        if args.instances {
            match event.project.as_deref() {
                Some(project) => format!("{project} | {day_key}"),
                None => format!("- | {day_key}"),
            }
        } else {
            day_key
        }
    });

    let report = build_report_from_rows(rows, loaded.stats);

    if use_json {
        emit_json(&report, args.common.jq.as_deref())
    } else if args.tui {
        run_report_tui(&report)
    } else {
        print_report_table_with_options(&report, args.common.compact, args.common.breakdown);
        print_debug(&report.stats, &args.common);
        Ok(())
    }
}

pub(crate) async fn run_monthly(args: MonthlyArgs) -> Result<()> {
    let use_json = should_emit_json(&args.common);
    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let loaded = load_usage(&args.common, &tz).await?;

    let rows = build_group_rows(&loaded.events, &args.common.order, |event| {
        let day = local_date(event.timestamp, &tz);
        format!("{:04}-{:02}", day.year(), day.month())
    });

    let report = build_report_from_rows(rows, loaded.stats);

    if use_json {
        #[derive(Serialize)]
        struct MonthlyOut {
            monthly: Vec<DailyRow>,
            totals: TokenCounts,
            stats: ParseStats,
        }
        let out = MonthlyOut {
            monthly: report.daily,
            totals: report.totals,
            stats: report.stats,
        };
        emit_json(&out, args.common.jq.as_deref())
    } else {
        let show = DailyReport {
            daily: report.daily,
            totals: report.totals,
            stats: report.stats,
        };
        print_report_table_with_options(&show, args.common.compact, args.common.breakdown);
        print_debug(&show.stats, &args.common);
        Ok(())
    }
}

pub(crate) async fn run_weekly(args: WeeklyArgs) -> Result<()> {
    let use_json = should_emit_json(&args.common);
    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let loaded = load_usage(&args.common, &tz).await?;

    let rows = build_group_rows(&loaded.events, &args.common.order, |event| {
        let day = local_date(event.timestamp, &tz);
        let start = week_start(day, args.start_of_week);
        format!("{}", start.format("%Y-%m-%d"))
    });

    let report = build_report_from_rows(rows, loaded.stats);

    if use_json {
        #[derive(Serialize)]
        struct WeeklyOut {
            weekly: Vec<DailyRow>,
            totals: TokenCounts,
            stats: ParseStats,
        }
        let out = WeeklyOut {
            weekly: report.daily,
            totals: report.totals,
            stats: report.stats,
        };
        emit_json(&out, args.common.jq.as_deref())
    } else {
        let show = DailyReport {
            daily: report.daily,
            totals: report.totals,
            stats: report.stats,
        };
        print_report_table_with_options(&show, args.common.compact, args.common.breakdown);
        print_debug(&show.stats, &args.common);
        Ok(())
    }
}

pub(crate) async fn run_antigravity(args: AntigravityArgs) -> Result<()> {
    let tz = parse_timezone_mode(args.timezone.as_deref())?;
    let snapshot = fetch_antigravity_official_limits()
        .await
        .context("Failed to probe Antigravity language server")?;

    if args.json {
        emit_json(&snapshot, None)?;
    } else {
        let plan = snapshot.plan_type.as_deref().unwrap_or("unknown");
        if let Some(email) = snapshot.account_email.as_deref() {
            println!("Antigravity  plan={plan}  email={email}");
        } else {
            println!("Antigravity  plan={plan}");
        }
        println!();

        if snapshot.models.is_empty() {
            println!("  (no model quotas available)");
        } else {
            // Show all models: those with quota data first, then unknowns
            let ordered = select_antigravity_models(&snapshot.models);
            let now = Utc::now();

            // Collect models already shown via priority selection
            let shown_labels: HashSet<&str> =
                ordered.iter().map(|m| m.label.as_str()).collect();

            // Remaining models not in the priority list
            let rest: Vec<_> = snapshot
                .models
                .iter()
                .filter(|m| !shown_labels.contains(m.label.as_str()))
                .collect();

            for model in ordered.iter().chain(rest.into_iter()) {
                if let Some(frac) = model.remaining_fraction {
                    let remaining_pct = frac * 100.0;
                    let used_pct = 100.0 - remaining_pct;
                    let bar = quota_bar(remaining_pct);
                    let mut line = format!(
                        "  {:<32} {bar}  {remaining_pct:5.1}% remaining  ({used_pct:.1}% used)",
                        model.label
                    );
                    if let Some(reset_ts) = model.reset_time {
                        let reset_text = format_reset_timestamp(reset_ts, &tz);
                        let eta_text = format_time_until_reset_short(reset_ts, now);
                        line.push_str(&format!("  resets {reset_text} (in {eta_text})"));
                    }
                    println!("{line}");
                } else {
                    let bar = "\x1b[90m[░░░░░░░░░░░░░░░░░░░░]\x1b[0m";
                    let mut line =
                        format!("  {:<32} {bar}  quota not reported", model.label);
                    if let Some(reset_ts) = model.reset_time {
                        let reset_text = format_reset_timestamp(reset_ts, &tz);
                        let eta_text = format_time_until_reset_short(reset_ts, now);
                        line.push_str(&format!("  resets {reset_text} (in {eta_text})"));
                    }
                    println!("{line}");
                }
            }
        }
    }

    Ok(())
}

fn quota_bar(remaining_pct: f64) -> String {
    let width: usize = 20;
    let filled = ((remaining_pct / 100.0) * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    let color = if remaining_pct >= 40.0 {
        "\x1b[32m" // green
    } else if remaining_pct >= 15.0 {
        "\x1b[33m" // yellow
    } else {
        "\x1b[31m" // red
    };
    format!(
        "{color}[{}{}]\x1b[0m",
        "█".repeat(filled),
        "░".repeat(empty)
    )
}

pub(crate) async fn run_session(args: SessionArgs) -> Result<()> {
    let use_json = should_emit_json(&args.common);
    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let loaded = load_usage(&args.common, &tz).await?;

    let mut grouped: HashMap<String, GroupAggregate> = HashMap::new();
    for event in loaded.events {
        if let Some(id_filter) = args.id.as_deref()
            && !event.session.contains(id_filter)
        {
            continue;
        }
        grouped
            .entry(event.session.clone())
            .or_default()
            .add_event(&event);
    }

    let mut sessions = grouped
        .into_iter()
        .map(|(session, agg)| {
            let models = agg
                .by_model
                .into_iter()
                .map(|(model, totals)| (model, totals.to_counts()))
                .collect::<BTreeMap<_, _>>();
            let sources = agg
                .by_source
                .into_iter()
                .map(|(source, totals)| (source.as_str().to_string(), totals.to_counts()))
                .collect::<BTreeMap<_, _>>();
            let last_activity = agg
                .last_activity
                .unwrap_or(DateTime::from_timestamp(0, 0).unwrap_or_else(Utc::now));

            SessionJsonRow {
                session_id: session,
                project: agg.project,
                last_activity: format_display_datetime(last_activity, &tz),
                totals: agg.totals.to_counts(),
                models,
                sources,
            }
        })
        .collect::<Vec<_>>();

    sessions.sort_by(|a, b| a.last_activity.cmp(&b.last_activity));
    if args.common.order == SortOrder::Desc {
        sessions.reverse();
    }

    let totals = sessions
        .iter()
        .fold(TokenCounts::default(), |mut acc, row| {
            acc.add_assign(row.totals.clone());
            acc
        });

    let json_report = SessionJsonReport {
        sessions,
        totals,
        stats: loaded.stats,
    };

    if use_json {
        emit_json(&json_report, args.common.jq.as_deref())
    } else {
        let rows = json_report
            .sessions
            .iter()
            .map(|row| DailyRow {
                date: shorten_session_id(&row.session_id),
                totals: row.totals.clone(),
                models: row.models.clone(),
                sources: row.sources.clone(),
            })
            .collect::<Vec<_>>();

        let show = DailyReport {
            daily: rows,
            totals: json_report.totals,
            stats: json_report.stats,
        };
        print_report_table_with_options(&show, args.common.compact, args.common.breakdown);
        print_debug(&show.stats, &args.common);
        Ok(())
    }
}

pub(crate) async fn run_blocks(args: BlocksArgs) -> Result<()> {
    if args.session_length == 0 {
        bail!("--session-length must be greater than 0");
    }
    if args.refresh_interval == 0 {
        bail!("--refresh-interval must be greater than 0");
    }

    let use_json = should_emit_json(&args.common);
    if args.live && use_json {
        bail!("--live cannot be used together with --json/--jq");
    }

    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let window_secs = i64::from(args.session_length) * 3600;
    let token_limit_mode = parse_token_limit_mode(args.token_limit.as_deref())?;
    // For live mode, skip blocking initial fetch — go straight to TUI and fetch in background.
    if args.live {
        return run_blocks_live(
            &args,
            &tz,
            window_secs,
            token_limit_mode,
            None,
            None,
            None,
        )
        .await;
    }

    let (official_codex, official_claude, official_antigravity) = if args.official_limits {
        let (codex, claude, antigravity, errors) = fetch_selected_official_limits(&args.common).await;
        for error in errors {
            eprintln!("{error}");
        }
        (codex, claude, antigravity)
    } else {
        (None, None, None)
    };

    let loaded = load_usage(&args.common, &tz).await?;
    let now = Utc::now();
    let membership_estimate = estimate_membership_from_logs(&loaded.events, now, window_secs);
    let inferred_limit = membership_estimate
        .as_ref()
        .map(|estimate| estimate.estimated_window_tokens);
    let resolved_from_mode =
        resolve_token_limit(token_limit_mode, &loaded.events, now, window_secs);
    let resolved_limit = resolved_from_mode.or(inferred_limit);
    let token_limit_source =
        resolve_token_limit_source(token_limit_mode, resolved_from_mode, inferred_limit);
    let json_report = build_block_json_report(
        loaded,
        &tz,
        BlockReportBuildOptions {
            order: args.common.order,
            recent_only: args.recent,
            active_only: args.active,
            window_secs,
            token_limit: resolved_limit,
            token_limit_source,
            membership_estimate: membership_estimate.clone(),
            official_codex: official_codex.clone(),
            official_claude: official_claude.clone(),
            official_antigravity: official_antigravity.clone(),
            now,
        },
    );

    if use_json {
        emit_json(&json_report, args.common.jq.as_deref())
    } else {
        let rows = json_report
            .blocks
            .iter()
            .map(|row| DailyRow {
                date: row.start_time.clone(),
                totals: row.totals.clone(),
                models: row.models.clone(),
                sources: BTreeMap::new(),
            })
            .collect::<Vec<_>>();

        let show = DailyReport {
            daily: rows,
            totals: json_report.totals,
            stats: json_report.stats,
        };

        print_report_table_with_options(&show, args.common.compact, args.common.breakdown);
        print_membership_estimate(
            &json_report.membership_estimate,
            resolved_limit,
            token_limit_source,
            json_report.official_codex.as_ref(),
            json_report.official_claude.as_ref(),
            json_report.official_antigravity.as_ref(),
            &tz,
        );
        print_debug(&show.stats, &args.common);
        Ok(())
    }
}

async fn fetch_selected_official_limits(
    common: &CommonArgs,
) -> (
    Option<OfficialCodexSnapshot>,
    Option<OfficialClaudeSnapshot>,
    Option<OfficialAntigravitySnapshot>,
    Vec<String>,
) {
    let codex_enabled = !common.no_codex;
    let claude_enabled = !common.no_claude;
    let antigravity_enabled = !common.no_antigravity;
    let (codex_result, claude_result, antigravity_result) = tokio::join!(
        async {
            if codex_enabled {
                Some(fetch_codex_official_limits().await)
            } else {
                None
            }
        },
        async {
            if claude_enabled {
                Some(fetch_claude_official_limits().await)
            } else {
                None
            }
        },
        async {
            if antigravity_enabled {
                Some(fetch_antigravity_official_limits().await)
            } else {
                None
            }
        }
    );

    let mut errors = Vec::new();

    let codex = match codex_result {
        Some(Ok(snapshot)) => Some(snapshot),
        Some(Err(error)) => {
            errors.push(format!("official: failed to fetch Codex limits ({error})"));
            None
        }
        None => None,
    };

    let claude = match claude_result {
        Some(Ok(snapshot)) => Some(snapshot),
        Some(Err(error)) => {
            errors.push(format!("official: failed to fetch Claude limits ({error})"));
            None
        }
        None => None,
    };

    let antigravity = match antigravity_result {
        Some(Ok(snapshot)) => Some(snapshot),
        Some(Err(_)) => None, // Silently skip — Antigravity may not be running
        None => None,
    };

    (codex, claude, antigravity, errors)
}

fn parse_token_limit_mode(raw: Option<&str>) -> Result<Option<TokenLimitMode>> {
    let Some(value) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };

    if value.eq_ignore_ascii_case("max") {
        return Ok(Some(TokenLimitMode::MaxHistorical));
    }

    let parsed = u64::from_str(value)
        .with_context(|| format!("Invalid --token-limit value: {value}. Use a number or 'max'"))?;
    Ok(Some(TokenLimitMode::Exact(parsed)))
}

fn resolve_token_limit(
    mode: Option<TokenLimitMode>,
    events: &[UsageEvent],
    now: DateTime<Utc>,
    window_secs: i64,
) -> Option<u64> {
    match mode {
        None => None,
        Some(TokenLimitMode::Exact(value)) => Some(value),
        Some(TokenLimitMode::MaxHistorical) => max_completed_block_tokens(events, now, window_secs),
    }
}

fn resolve_token_limit_source(
    mode: Option<TokenLimitMode>,
    resolved_from_mode: Option<u64>,
    inferred_limit: Option<u64>,
) -> TokenLimitSource {
    match mode {
        Some(TokenLimitMode::Exact(_)) => TokenLimitSource::Explicit,
        Some(TokenLimitMode::MaxHistorical) => {
            if resolved_from_mode.is_some() {
                TokenLimitSource::HistoricalMax
            } else if inferred_limit.is_some() {
                TokenLimitSource::EstimatedFromLogs
            } else {
                TokenLimitSource::Unset
            }
        }
        None => {
            if inferred_limit.is_some() {
                TokenLimitSource::EstimatedFromLogs
            } else {
                TokenLimitSource::Unset
            }
        }
    }
}

fn estimate_membership_from_logs(
    events: &[UsageEvent],
    now: DateTime<Utc>,
    window_secs: i64,
) -> Option<MembershipEstimate> {
    if window_secs <= 0 {
        return None;
    }

    let now_unix = now.timestamp();
    let active_start_unix = now_unix - now_unix.rem_euclid(window_secs);

    let mut per_block_totals: HashMap<i64, u64> = HashMap::new();
    let mut per_source_block_totals: HashMap<(SourceKind, i64), u64> = HashMap::new();
    let mut observed_total_tokens: u64 = 0;
    let mut observed_total_by_source: HashMap<SourceKind, u64> = HashMap::new();

    for event in events {
        let tokens = event.usage.total_tokens();
        observed_total_tokens = observed_total_tokens.saturating_add(tokens);
        observed_total_by_source
            .entry(event.source)
            .and_modify(|total| *total = total.saturating_add(tokens))
            .or_insert(tokens);

        let unix = event.timestamp.timestamp();
        let block_start_unix = unix - unix.rem_euclid(window_secs);
        if block_start_unix == active_start_unix {
            continue;
        }

        per_block_totals
            .entry(block_start_unix)
            .and_modify(|total| *total = total.saturating_add(tokens))
            .or_insert(tokens);
        per_source_block_totals
            .entry((event.source, block_start_unix))
            .and_modify(|total| *total = total.saturating_add(tokens))
            .or_insert(tokens);
    }

    let combined_block_samples = per_block_totals
        .into_values()
        .filter(|tokens| *tokens > 0)
        .collect::<Vec<_>>();
    if combined_block_samples.is_empty() {
        return None;
    }

    let combined = build_membership_source_estimate(
        "all".to_string(),
        None,
        combined_block_samples,
        observed_total_tokens,
    )?;

    let mut source_breakdown = Vec::new();
    for source in [SourceKind::Claude, SourceKind::Codex] {
        let samples = per_source_block_totals
            .iter()
            .filter_map(|((kind, _), tokens)| (*kind == source).then_some(*tokens))
            .filter(|tokens| *tokens > 0)
            .collect::<Vec<_>>();
        if samples.is_empty() {
            continue;
        }

        let observed_total = observed_total_by_source.get(&source).copied().unwrap_or(0);
        if let Some(estimate) = build_membership_source_estimate(
            source.as_str().to_string(),
            Some(source),
            samples,
            observed_total,
        ) {
            source_breakdown.push(estimate);
        }
    }

    Some(MembershipEstimate {
        estimated_plan: combined.estimated_plan,
        estimated_window_tokens: combined.estimated_window_tokens,
        observed_peak_window_tokens: combined.observed_peak_window_tokens,
        observed_p95_window_tokens: combined.observed_p95_window_tokens,
        observed_total_tokens: combined.observed_total_tokens,
        completed_blocks: combined.completed_blocks,
        confidence: combined.confidence,
        source_breakdown,
    })
}

fn build_membership_source_estimate(
    source: String,
    source_kind: Option<SourceKind>,
    mut completed_block_samples: Vec<u64>,
    observed_total_tokens: u64,
) -> Option<MembershipSourceEstimate> {
    if completed_block_samples.is_empty() {
        return None;
    }

    completed_block_samples.sort_unstable();
    let observed_peak_window_tokens = *completed_block_samples.last()?;
    let observed_p95_window_tokens = percentile_nearest_rank(&completed_block_samples, 0.95);
    let estimated_window_tokens = observed_peak_window_tokens.max(
        ((observed_p95_window_tokens as f64) * 1.05)
            .round()
            .max(observed_p95_window_tokens as f64) as u64,
    );
    let completed_blocks = completed_block_samples.len();
    let confidence = estimate_membership_confidence(
        completed_blocks,
        observed_peak_window_tokens,
        observed_p95_window_tokens,
    );
    let estimated_plan = classify_estimated_plan(source_kind, estimated_window_tokens).to_string();

    Some(MembershipSourceEstimate {
        source,
        estimated_plan,
        estimated_window_tokens,
        observed_peak_window_tokens,
        observed_p95_window_tokens,
        observed_total_tokens,
        completed_blocks,
        confidence,
    })
}

fn percentile_nearest_rank(sorted_values: &[u64], percentile: f64) -> u64 {
    if sorted_values.is_empty() {
        return 0;
    }
    let p = percentile.clamp(0.0, 1.0);
    let rank = ((sorted_values.len() as f64) * p).ceil() as usize;
    let index = rank
        .saturating_sub(1)
        .min(sorted_values.len().saturating_sub(1));
    sorted_values[index]
}

fn estimate_membership_confidence(
    completed_blocks: usize,
    peak_tokens: u64,
    p95_tokens: u64,
) -> f64 {
    let mut confidence: f64 = if completed_blocks < 3 {
        0.35
    } else if completed_blocks < 8 {
        0.55
    } else if completed_blocks < 20 {
        0.7
    } else {
        0.8
    };

    if p95_tokens > 0 {
        let spread = peak_tokens as f64 / p95_tokens as f64;
        if spread <= 1.15 {
            confidence += 0.1;
        } else if spread <= 1.35 {
            confidence += 0.05;
        } else {
            confidence -= 0.05;
        }
    }

    confidence.clamp(0.2, 0.95)
}

fn classify_estimated_plan(
    source: Option<SourceKind>,
    estimated_window_tokens: u64,
) -> &'static str {
    match source {
        Some(SourceKind::Claude) => {
            if estimated_window_tokens < 180_000_000 {
                "claude_pro"
            } else if estimated_window_tokens < 900_000_000 {
                "claude_max_5x"
            } else if estimated_window_tokens < 3_600_000_000 {
                "claude_max_20x"
            } else {
                "claude_max_20x_or_enterprise"
            }
        }
        Some(SourceKind::Codex) => {
            if estimated_window_tokens < 120_000_000 {
                "codex_plus_or_business"
            } else if estimated_window_tokens < 720_000_000 {
                "codex_pro"
            } else {
                "codex_pro_with_credits"
            }
        }
        None => {
            if estimated_window_tokens < 240_000_000 {
                "mixed_standard"
            } else if estimated_window_tokens < 1_200_000_000 {
                "mixed_high"
            } else {
                "mixed_very_high"
            }
        }
    }
}

fn display_plan_label(raw: &str) -> &'static str {
    match raw {
        "claude_pro" => "Claude Pro",
        "claude_max_5x" => "Claude Max 5x",
        "claude_max_20x" => "Claude Max 20x",
        "claude_max_20x_or_enterprise" => "Claude Max 20x / Enterprise",
        "codex_plus_or_business" => "Codex Plus/Business",
        "codex_pro" => "Codex Pro",
        "codex_pro_with_credits" => "Codex Pro + credits",
        "mixed_standard" => "Mixed Standard",
        "mixed_high" => "Mixed High",
        "mixed_very_high" => "Mixed Very High",
        _ => "Unknown",
    }
}

fn resolve_display_limit(
    base_limit: u64,
    projected_tokens: u64,
    token_limit_source: TokenLimitSource,
    membership_estimate: Option<&MembershipEstimate>,
) -> (u64, Vec<&'static str>) {
    if base_limit == 0
        || projected_tokens <= base_limit
        || token_limit_source != TokenLimitSource::EstimatedFromLogs
    {
        return (base_limit, Vec::new());
    }

    let mut effective_limit = base_limit;
    let mut plan_key = inferred_plan_key(membership_estimate);
    let mut promoted = Vec::new();

    for _ in 0..3 {
        if projected_tokens <= effective_limit {
            break;
        }

        let Some(current_plan) = plan_key else {
            break;
        };
        let Some((next_plan, multiplier)) = next_plan_transition(current_plan) else {
            break;
        };

        let next_limit = ((effective_limit as f64) * multiplier).ceil() as u64;
        if next_limit <= effective_limit {
            break;
        }

        effective_limit = next_limit;
        promoted.push(display_plan_label(next_plan));
        plan_key = Some(next_plan);
    }

    (effective_limit, promoted)
}

fn inferred_plan_key(estimate: Option<&MembershipEstimate>) -> Option<&str> {
    let estimate = estimate?;
    if estimate.source_breakdown.len() == 1 {
        return estimate
            .source_breakdown
            .first()
            .map(|entry| entry.estimated_plan.as_str());
    }

    Some(estimate.estimated_plan.as_str())
}

fn next_plan_transition(plan: &str) -> Option<(&'static str, f64)> {
    match plan {
        "claude_pro" => Some(("claude_max_5x", 5.0)),
        "claude_max_5x" => Some(("claude_max_20x", 4.0)),
        "codex_plus_or_business" => Some(("codex_pro", 6.0)),
        "mixed_standard" => Some(("mixed_high", 4.0)),
        "mixed_high" => Some(("mixed_very_high", 3.0)),
        _ => None,
    }
}

fn max_completed_block_tokens(
    events: &[UsageEvent],
    now: DateTime<Utc>,
    window_secs: i64,
) -> Option<u64> {
    if window_secs <= 0 {
        return None;
    }

    let now_unix = now.timestamp();
    let active_start_unix = now_unix - now_unix.rem_euclid(window_secs);
    let mut totals_by_block = HashMap::<i64, u64>::new();

    for event in events {
        let unix = event.timestamp.timestamp();
        let block_start_unix = unix - unix.rem_euclid(window_secs);
        if block_start_unix == active_start_unix {
            continue;
        }

        totals_by_block
            .entry(block_start_unix)
            .and_modify(|total| *total += event.usage.total_tokens())
            .or_insert_with(|| event.usage.total_tokens());
    }

    totals_by_block.into_values().max()
}

fn build_block_json_report(
    loaded: LoadedUsage,
    tz: &TimeZoneMode,
    options: BlockReportBuildOptions,
) -> BlockJsonReport {
    let BlockReportBuildOptions {
        order,
        recent_only,
        active_only,
        window_secs,
        token_limit,
        token_limit_source,
        membership_estimate,
        official_codex,
        official_claude,
        official_antigravity,
        now,
    } = options;
    let mut grouped: HashMap<i64, GroupAggregate> = HashMap::new();
    for event in loaded.events {
        let unix = event.timestamp.timestamp();
        let block_start = unix - unix.rem_euclid(window_secs);
        grouped.entry(block_start).or_default().add_event(&event);
    }

    let mut grouped_blocks = grouped.into_iter().collect::<Vec<_>>();
    if recent_only {
        let recent_cutoff = now - chrono::TimeDelta::days(3);
        grouped_blocks.retain(|(start_unix, _)| {
            DateTime::from_timestamp(*start_unix, 0)
                .map(|dt| dt >= recent_cutoff)
                .unwrap_or(true)
        });
    }

    let mut blocks = grouped_blocks
        .into_iter()
        .map(|(start_unix, agg)| {
            let start = DateTime::from_timestamp(start_unix, 0).unwrap_or_else(Utc::now);
            let end = start + chrono::TimeDelta::seconds(window_secs);
            let is_active = now >= start && now < end;
            let percent_of_limit = token_limit.map(|limit| {
                if limit == 0 {
                    0.0
                } else {
                    (agg.totals.total_tokens() as f64 / limit as f64) * 100.0
                }
            });

            let row = BlockJsonRow {
                id: format!("{}", start.format("%Y%m%d%H")),
                start_time: format_display_datetime(start, tz),
                end_time: format_display_datetime(end, tz),
                is_active,
                totals: agg.totals.to_counts(),
                models: agg
                    .by_model
                    .into_iter()
                    .map(|(model, totals)| (model, totals.to_counts()))
                    .collect::<BTreeMap<_, _>>(),
                percent_of_limit,
            };

            (start_unix, row)
        })
        .collect::<Vec<_>>();

    if active_only {
        blocks.retain(|(_, row)| row.is_active);
    }

    blocks.sort_by_key(|(start_unix, _)| *start_unix);
    if order == SortOrder::Desc {
        blocks.reverse();
    }

    let blocks = blocks.into_iter().map(|(_, row)| row).collect::<Vec<_>>();
    let totals = blocks.iter().fold(TokenCounts::default(), |mut acc, row| {
        acc.add_assign(row.totals.clone());
        acc
    });

    BlockJsonReport {
        blocks,
        totals,
        stats: loaded.stats,
        token_limit,
        token_limit_source,
        membership_estimate,
        official_codex,
        official_claude,
        official_antigravity,
    }
}

async fn run_blocks_live(
    args: &BlocksArgs,
    tz: &TimeZoneMode,
    window_secs: i64,
    token_limit_mode: Option<TokenLimitMode>,
    mut official_codex: Option<OfficialCodexSnapshot>,
    mut official_claude: Option<OfficialClaudeSnapshot>,
    mut official_antigravity: Option<OfficialAntigravitySnapshot>,
) -> Result<()> {
    if !std::io::stdout().is_terminal() {
        bail!("--live requires an interactive terminal");
    }

    let refresh_every = args.refresh_interval.max(1);
    let mut session = BlocksLiveSession::enter()?;
    let mut live_runtime = LiveUsageRuntime::new(&args.common, refresh_every).await?;
    let mut last_official_refresh = Instant::now();
    let mut active_tab = LiveTab::Overview;
    #[allow(unused_assignments)]
    let mut last_data_refresh = Instant::now();
    let mut pending_official_task: Option<
        tokio::task::JoinHandle<(
            Option<OfficialCodexSnapshot>,
            Option<OfficialClaudeSnapshot>,
            Option<OfficialAntigravitySnapshot>,
            Vec<String>,
        )>,
    > = None;

    loop {
        let now = Utc::now();
        live_runtime.maybe_refresh_sources(&args.common).await?;

        let source_hint = select_live_source(
            &args.common,
            None,
            official_codex.as_ref(),
            official_claude.as_ref(),
        );
        let (block_start_unix, block_end_unix, live_window_secs) = resolve_live_block_bounds(
            now,
            window_secs,
            source_hint,
            official_codex.as_ref(),
            official_claude.as_ref(),
        );

        // Non-blocking official limits fetch: spawn a background task and check
        // completion on each frame instead of blocking the render loop.
        let should_refresh_official = args.official_limits
            && pending_official_task.is_none()
            && (official_codex.is_none()
                || official_claude.is_none()
                || official_antigravity.is_none()
                || last_official_refresh.elapsed() >= Duration::from_secs(30));
        if should_refresh_official {
            let common = args.common.clone();
            pending_official_task =
                Some(tokio::spawn(
                    async move { fetch_selected_official_limits(&common).await },
                ));
        }

        // Check if the background task has completed (non-blocking).
        if let Some(ref task) = pending_official_task {
            if task.is_finished() {
                if let Some(task) = pending_official_task.take() {
                    if let Ok((codex, claude, antigravity, _errors)) = task.await {
                        if codex.is_some() {
                            official_codex = codex;
                        }
                        if claude.is_some() {
                            official_claude = claude;
                        }
                        if antigravity.is_some() {
                            official_antigravity = antigravity;
                        }
                        last_official_refresh = Instant::now();
                    }
                }
            }
        }

        let loaded = live_runtime.load(tz);
        let membership_estimate =
            estimate_membership_from_logs(&loaded.events, now, live_window_secs);
        let inferred_limit = membership_estimate
            .as_ref()
            .map(|estimate| estimate.estimated_window_tokens);
        let resolved_from_mode =
            resolve_token_limit(token_limit_mode, &loaded.events, now, live_window_secs);
        let token_limit = resolved_from_mode.or(inferred_limit);
        let token_limit_source =
            resolve_token_limit_source(token_limit_mode, resolved_from_mode, inferred_limit);
        let active =
            active_block_summary_for_bounds(&loaded.events, now, block_start_unix, block_end_unix);
        let selected_source = select_live_source(
            &args.common,
            active.as_ref(),
            official_codex.as_ref(),
            official_claude.as_ref(),
        );
        let (today_totals, last_30d_totals, last_30d_active_days) =
            aggregate_recent_costs(&loaded.events, now, tz, selected_source);
        let mut frame_context = LiveFrameContext::new(
            now,
            tz,
            block_start_unix,
            block_end_unix,
            LimitDisplayContext {
                token_limit,
                token_limit_source,
                membership_estimate: membership_estimate.as_ref(),
            },
            official_codex.as_ref(),
            official_claude.as_ref(),
            official_antigravity.as_ref(),
            selected_source,
            today_totals,
            last_30d_totals,
            last_30d_active_days,
            active.as_ref(),
            active_tab,
        );

        frame_context.active_tab = active_tab;
        render_blocks_live_frame(&mut session, &frame_context)?;
        last_data_refresh = Instant::now();

        // Fast inner loop: poll input at ~50ms intervals, re-render on tab switch
        // immediately. Only break out to refresh data when the refresh timer fires.
        loop {
            let until_refresh = Duration::from_secs(refresh_every)
                .saturating_sub(last_data_refresh.elapsed());
            if until_refresh.is_zero() {
                break; // Time to refresh data
            }
            let poll_for = until_refresh.min(Duration::from_millis(50));
            match poll_live_input(poll_for, active_tab)? {
                LiveInputEvent::Exit => {
                    live_runtime.flush_cache(true);
                    return Ok(()); // Exit directly with cache flush
                }
                LiveInputEvent::SwitchTab(tab) => {
                    active_tab = tab;
                    let mut ctx = frame_context.clone();
                    ctx.active_tab = active_tab;
                    render_blocks_live_frame(&mut session, &ctx)?;
                }
                LiveInputEvent::Tick => {
                    // Check non-blocking official task completion mid-frame.
                    if let Some(ref task) = pending_official_task {
                        if task.is_finished() {
                            if let Some(task) = pending_official_task.take() {
                                if let Ok((codex, claude, antigravity, _errors)) = task.await {
                                    if codex.is_some() {
                                        official_codex = codex;
                                    }
                                    if claude.is_some() {
                                        official_claude = claude;
                                    }
                                    if antigravity.is_some() {
                                        official_antigravity = antigravity;
                                    }
                                    last_official_refresh = Instant::now();
                                }
                            }
                            // Official data changed — force an immediate data refresh.
                            break;
                        }
                    }
                }
            }
        }
    }
}

fn select_live_source(
    common: &CommonArgs,
    active: Option<&ActiveBlockSummary>,
    official_codex: Option<&OfficialCodexSnapshot>,
    official_claude: Option<&OfficialClaudeSnapshot>,
) -> Option<SourceKind> {
    if common.no_codex && !common.no_claude {
        return Some(SourceKind::Claude);
    }
    if common.no_claude && !common.no_codex {
        return Some(SourceKind::Codex);
    }
    if let Some(source) = active.and_then(|v| v.dominant_source) {
        return Some(source);
    }
    // When both sources exist, return None to show combined data.
    // Only pick a single source when it's the sole provider.
    match (official_codex.is_some(), official_claude.is_some()) {
        (true, false) => Some(SourceKind::Codex),
        (false, true) => Some(SourceKind::Claude),
        _ => None,
    }
}

fn resolve_live_block_bounds(
    now: DateTime<Utc>,
    default_window_secs: i64,
    source_hint: Option<SourceKind>,
    official_codex: Option<&OfficialCodexSnapshot>,
    official_claude: Option<&OfficialClaudeSnapshot>,
) -> (i64, i64, i64) {
    let fallback = {
        let now_unix = now.timestamp();
        let start = now_unix - now_unix.rem_euclid(default_window_secs.max(1));
        let end = start + default_window_secs.max(1);
        (start, end, default_window_secs.max(1))
    };

    let (reset_at, window_secs) = match source_hint {
        Some(SourceKind::Codex) => {
            let Some(snapshot) = official_codex else {
                return fallback;
            };
            let reset = snapshot.primary_resets_at;
            let window = snapshot
                .primary_window_mins
                .map(|mins| mins.saturating_mul(60))
                .unwrap_or(default_window_secs);
            (reset, window)
        }
        Some(SourceKind::Claude) => {
            let Some(snapshot) = official_claude else {
                return fallback;
            };
            let reset = snapshot.primary_resets_at;
            let window = snapshot
                .primary_window_mins
                .map(|mins| mins.saturating_mul(60))
                .unwrap_or(default_window_secs);
            (reset, window)
        }
        None => return fallback,
    };

    let Some(mut end_unix) = reset_at else {
        return fallback;
    };
    let window_secs = window_secs.max(1);
    let now_unix = now.timestamp();

    if end_unix <= now_unix {
        let steps = (now_unix - end_unix).div_euclid(window_secs) + 1;
        end_unix = end_unix.saturating_add(steps.saturating_mul(window_secs));
    } else if end_unix - now_unix > window_secs {
        let steps = (end_unix - now_unix - 1).div_euclid(window_secs);
        end_unix = end_unix.saturating_sub(steps.saturating_mul(window_secs));
    }

    let start_unix = end_unix.saturating_sub(window_secs);
    if now_unix < start_unix || now_unix >= end_unix {
        return fallback;
    }
    (start_unix, end_unix, window_secs)
}

fn aggregate_recent_costs(
    events: &[UsageEvent],
    now: DateTime<Utc>,
    tz: &TimeZoneMode,
    source: Option<SourceKind>,
) -> (TokenCounts, TokenCounts, u32) {
    let today = local_date(now, tz);
    let last_30d_start = today
        .checked_sub_signed(chrono::TimeDelta::days(29))
        .unwrap_or(today);
    let mut today_totals = TokenCounts::default();
    let mut last_30d_totals = TokenCounts::default();
    let mut active_days = HashSet::new();

    for event in events {
        if source.is_some_and(|selected| event.source != selected) {
            continue;
        }
        let day = local_date(event.timestamp, tz);
        let counts = event.usage.to_counts();
        if day == today {
            today_totals.add_assign(counts.clone());
        }
        if day >= last_30d_start && day <= today {
            if counts.total_tokens > 0 {
                active_days.insert(day);
            }
            last_30d_totals.add_assign(counts);
        }
    }

    (today_totals, last_30d_totals, active_days.len() as u32)
}

struct BlocksLiveSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl BlocksLiveSession {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(out);
        let mut terminal = Terminal::new(backend)?;
        terminal.hide_cursor()?;
        Ok(Self { terminal })
    }
}

impl Drop for BlocksLiveSession {
    fn drop(&mut self) {
        let _ = self.terminal.show_cursor();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn render_blocks_live_frame(
    session: &mut BlocksLiveSession,
    context: &LiveFrameContext<'_>,
) -> Result<()> {
    session
        .terminal
        .draw(|frame| draw_blocks_live_tui(frame, context))?;
    Ok(())
}

fn draw_blocks_live_tui(frame: &mut ratatui::Frame<'_>, context: &LiveFrameContext<'_>) {
    let root = frame.area();
    let preferred_official = preferred_official_for_live(context);
    let progress_height: u16 = match context.active_tab {
        LiveTab::Overview => {
            // Count how many sources have data
            let n = [
                context.official_codex.is_some(),
                context.official_claude.is_some(),
                context.official_antigravity.is_some(),
            ]
            .iter()
            .filter(|&&v| v)
            .count() as u16;
            // 2 lines per source (label + bar), plus time bar (2 lines)
            2 + n * 2
        }
        LiveTab::Codex => {
            if context
                .official_codex
                .and_then(|s| s.secondary_used_percent)
                .is_some()
            {
                6
            } else {
                4
            }
        }
        LiveTab::Claude => {
            if context
                .official_claude
                .and_then(|s| s.secondary_used_percent)
                .is_some()
            {
                6
            } else {
                4
            }
        }
        LiveTab::Antigravity => 0,
    };
    let tab_bar_height = 1u16;
    let info_height = 1u16;
    let [tab_area, info_area, progress_area, body_area] = Layout::vertical([
        Constraint::Length(tab_bar_height),
        Constraint::Length(info_height),
        Constraint::Length(progress_height),
        Constraint::Min(4),
    ])
    .margin(1)
    .areas(root);

    let mode_text = if preferred_official.is_some() {
        "official"
    } else {
        "estimated"
    };
    let plan_text = preferred_official
        .and_then(LiveOfficialRef::plan_type)
        .unwrap_or("unknown");

    // Tab bar at top
    render_live_tab_bar(frame, tab_area, context.active_tab);

    // Condensed info line below tabs
    let info_line = if root.width >= 100 {
        Line::from(vec![
            Span::styled(
                context.now_text.clone(),
                Style::default().fg(TuiColor::DarkGray),
            ),
            Span::styled("  |  ", Style::default().fg(TuiColor::DarkGray)),
            Span::styled(
                format!("block {} -> {}", context.block_start_text, context.block_end_text),
                Style::default().fg(TuiColor::DarkGray),
            ),
            Span::styled("  |  ", Style::default().fg(TuiColor::DarkGray)),
            Span::raw(format!("{mode_text} · plan {plan_text}")),
            Span::styled("  |  ", Style::default().fg(TuiColor::DarkGray)),
            Span::styled(
                "q/Esc exit · Tab/←→ switch",
                Style::default().fg(TuiColor::DarkGray),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                context.now_text.clone(),
                Style::default().fg(TuiColor::DarkGray),
            ),
            Span::styled("  |  ", Style::default().fg(TuiColor::DarkGray)),
            Span::raw(format!("{mode_text} · {plan_text}")),
            Span::styled("  |  ", Style::default().fg(TuiColor::DarkGray)),
            Span::styled(
                "q · Tab switch",
                Style::default().fg(TuiColor::DarkGray),
            ),
        ])
    };
    frame.render_widget(Paragraph::new(info_line), info_area);

    match context.active_tab {
        LiveTab::Overview => {
            render_live_overview_bars(frame, progress_area, context);
            render_live_overview_body(frame, body_area, context);
        }
        LiveTab::Codex => {
            render_live_progress_bars_for(frame, progress_area, context, Some(SourceKind::Codex));
            render_live_source_detail(frame, body_area, context, SourceKind::Codex);
        }
        LiveTab::Claude => {
            render_live_progress_bars_for(frame, progress_area, context, Some(SourceKind::Claude));
            render_live_source_detail(frame, body_area, context, SourceKind::Claude);
        }
        LiveTab::Antigravity => {
            render_live_antigravity_tab(frame, body_area, context);
        }
    }
}

fn render_live_tab_bar(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    active: LiveTab,
) {
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(
            "tu live",
            Style::default()
                .fg(TuiColor::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
    ];
    for (i, tab) in ALL_LIVE_TABS.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        let label = format!(" {} ", tab.label());
        if *tab == active {
            spans.push(Span::styled(
                label,
                Style::default()
                    .fg(TuiColor::Black)
                    .bg(TuiColor::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                label,
                Style::default().fg(TuiColor::DarkGray),
            ));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_live_antigravity_tab(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    let Some(ag) = context.official_antigravity else {
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "Antigravity language server not detected",
                Style::default()
                    .fg(TuiColor::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from("Make sure Antigravity IDE is running with its language server."),
        ])
        .wrap(Wrap { trim: true });
        frame.render_widget(msg, area);
        return;
    };

    let mut lines: Vec<Line<'static>> = Vec::new();

    let plan = ag.plan_type.as_deref().unwrap_or("unknown");
    let email = ag.account_email.as_deref().unwrap_or("—");
    lines.push(Line::from(vec![
        Span::styled(
            "Antigravity",
            Style::default()
                .fg(TuiColor::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  plan {plan}  |  {email}")),
    ]));
    lines.push(Line::from(""));

    let now = Utc::now();
    let ordered = select_antigravity_models(&ag.models);
    let shown: HashSet<String> = ordered.iter().map(|m| m.label.clone()).collect();
    let rest: Vec<_> = ag
        .models
        .iter()
        .filter(|m| !shown.contains(&m.label))
        .collect();

    // Draw each model with a gauge-style bar
    for model in ordered.iter().chain(rest.into_iter()) {
        lines.push(Line::from(vec![Span::styled(
            format!("  {}", model.label),
            Style::default().add_modifier(Modifier::BOLD),
        )]));

        if let Some(frac) = model.remaining_fraction {
            let remaining_pct = frac * 100.0;
            let used_pct = 100.0 - remaining_pct;
            let color = if remaining_pct >= 40.0 {
                TuiColor::Green
            } else if remaining_pct >= 15.0 {
                TuiColor::Yellow
            } else {
                TuiColor::Red
            };

            let bar_width = (area.width as usize).saturating_sub(6).min(60);
            let filled = ((used_pct / 100.0) * bar_width as f64).round() as usize;
            let empty = bar_width.saturating_sub(filled);
            let bar = format!("{}{}",
                "█".repeat(filled),
                "░".repeat(empty),
            );

            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(bar, Style::default().fg(color)),
                Span::styled(
                    format!(" {remaining_pct:.0}% left"),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ]));

            if let Some(reset_ts) = model.reset_time {
                let eta = format_time_until_reset_short(reset_ts, now);
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("resets in {eta}"),
                        Style::default().fg(TuiColor::DarkGray),
                    ),
                ]));
            }
        } else {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("quota not reported", Style::default().fg(TuiColor::DarkGray)),
            ]));
        }
        lines.push(Line::from(""));
    }

    let body = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(body, area);
}

fn render_live_overview_bars(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    let mut constraints: Vec<Constraint> = Vec::new();
    // Time bar: label + gauge
    constraints.push(Constraint::Length(1));
    constraints.push(Constraint::Length(1));

    struct SourceBar {
        label: String,
        used: f64,
        reset_text: Option<String>,
    }
    let mut source_bars: Vec<SourceBar> = Vec::new();

    if let Some(codex) = context.official_codex {
        if let Some(used) = codex.primary_used_percent {
            let reset = codex
                .primary_resets_at
                .map(|r| format_time_until_reset_short(r, context.now));
            source_bars.push(SourceBar {
                label: "Codex".to_string(),
                used,
                reset_text: reset,
            });
            constraints.push(Constraint::Length(1));
            constraints.push(Constraint::Length(1));
        }
    }
    if let Some(claude) = context.official_claude {
        if let Some(used) = claude.primary_used_percent {
            let reset = claude
                .primary_resets_at
                .map(|r| format_time_until_reset_short(r, context.now));
            source_bars.push(SourceBar {
                label: "Claude".to_string(),
                used,
                reset_text: reset,
            });
            constraints.push(Constraint::Length(1));
            constraints.push(Constraint::Length(1));
        }
    }
    if let Some(ag) = context.official_antigravity {
        if let Some(used) = ag.primary_used_percent {
            let reset = ag
                .primary_resets_at
                .map(|r| format_time_until_reset_short(r, context.now));
            source_bars.push(SourceBar {
                label: ag
                    .primary_label
                    .as_deref()
                    .unwrap_or("Antigravity")
                    .to_string(),
                used,
                reset_text: reset,
            });
            constraints.push(Constraint::Length(1));
            constraints.push(Constraint::Length(1));
        }
    }

    let rows = Layout::vertical(constraints).split(area);

    // Time bar
    let time_ratio = if context.window_secs > 0 {
        (context.elapsed_secs as f64 / context.window_secs as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let time_title = Paragraph::new(Line::from(vec![
        Span::styled("Time ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(
            "{} / {}",
            format_hours_minutes(context.elapsed_secs / 60),
            format_hours_minutes(context.window_secs / 60)
        )),
    ]));
    frame.render_widget(time_title, rows[0]);
    let time_gauge = Gauge::default()
        .style(live_gauge_track_style())
        .gauge_style(
            Style::default()
                .fg(TuiColor::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(time_ratio)
        .label(format!("{:.1}%", time_ratio * 100.0));
    frame.render_widget(time_gauge, rows[1]);

    // Source bars
    for (i, bar) in source_bars.iter().enumerate() {
        let row_base = 2 + i * 2;
        let title = Paragraph::new(Line::from(vec![Span::styled(
            &bar.label,
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        frame.render_widget(title, rows[row_base]);

        let mut label = format!("{:.1}% used", bar.used);
        if let Some(reset) = &bar.reset_text {
            label.push_str(&format!(" | resets in {reset}"));
        }
        let gauge = Gauge::default()
            .style(live_gauge_track_style())
            .gauge_style(
                Style::default()
                    .fg(used_gauge_color(bar.used))
                    .add_modifier(Modifier::BOLD),
            )
            .ratio((bar.used / 100.0).clamp(0.0, 1.0))
            .label(label);
        frame.render_widget(gauge, rows[row_base + 1]);
    }
}

fn render_live_overview_body(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Today summary
    lines.push(Line::from(vec![Span::styled(
        "Today (all sources)",
        Style::default()
            .fg(TuiColor::Cyan)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(vec![
        Span::raw(live_key_label("Tokens")),
        Span::styled(
            format_u64(context.today_totals.total_tokens),
            Style::default()
                .fg(TuiColor::LightCyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw(live_key_label("Cost")),
        Span::styled(
            format_usd(context.today_totals.cost_usd),
            Style::default()
                .fg(TuiColor::Green)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // 30-day avg
    if context.last_30d_active_days > 0 {
        let avg_cost =
            context.last_30d_totals.cost_usd / context.last_30d_active_days as f64;
        let avg_tokens =
            context.last_30d_totals.total_tokens / context.last_30d_active_days as u64;
        lines.push(Line::from(vec![
            Span::raw(live_key_label("30d avg/day")),
            Span::raw(format!(
                "{} · {} tokens",
                format_usd(avg_cost),
                format_u64(avg_tokens)
            )),
        ]));
    }

    lines.push(Line::from(""));

    // Per-source summaries
    if let Some(codex) = context.official_codex {
        let plan = codex.plan_type.as_deref().unwrap_or("?");
        lines.push(Line::from(vec![Span::styled(
            format!("Codex  (plan {plan})"),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        if let Some(primary) = codex.primary_used_percent {
            lines.push(live_key_value_line("  Session", format!("{primary:.1}% used"), used_gauge_color(primary)));
        }
        if let Some(weekly) = codex.secondary_used_percent {
            lines.push(live_key_value_line("  Weekly", format!("{weekly:.1}% used"), used_gauge_color(weekly)));
        }
        lines.push(Line::from(""));
    }

    if let Some(claude) = context.official_claude {
        let plan = claude.plan_type.as_deref().unwrap_or("?");
        lines.push(Line::from(vec![Span::styled(
            format!("Claude  (plan {plan})"),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        if let Some(primary) = claude.primary_used_percent {
            lines.push(live_key_value_line("  Session", format!("{primary:.1}% used"), used_gauge_color(primary)));
        }
        if let Some(weekly) = claude.secondary_used_percent {
            lines.push(live_key_value_line("  Weekly", format!("{weekly:.1}% used"), used_gauge_color(weekly)));
        }
        lines.push(Line::from(""));
    }

    if let Some(ag) = context.official_antigravity {
        let plan = ag.plan_type.as_deref().unwrap_or("?");
        lines.push(Line::from(vec![Span::styled(
            format!("Antigravity  (plan {plan})"),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        let ordered = select_antigravity_models(&ag.models);
        for model in ordered.iter().take(3) {
            if let Some(frac) = model.remaining_fraction {
                let remaining = frac * 100.0;
                let color = if remaining >= 40.0 {
                    TuiColor::Green
                } else if remaining >= 15.0 {
                    TuiColor::Yellow
                } else {
                    TuiColor::Red
                };
                lines.push(live_key_value_line(
                    &format!("  {}", model.label),
                    format!("{remaining:.0}% left"),
                    color,
                ));
            }
        }
        lines.push(Line::from(""));
    }

    if context.official_codex.is_none()
        && context.official_claude.is_none()
        && context.official_antigravity.is_none()
    {
        lines.push(Line::from(vec![Span::styled(
            "No official limits available",
            Style::default().fg(TuiColor::Yellow),
        )]));
    }

    let body = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(body, area);
}

fn render_live_source_detail(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
    source: SourceKind,
) {
    let has_official = match source {
        SourceKind::Codex => context.official_codex.is_some(),
        SourceKind::Claude => context.official_claude.is_some(),
    };

    if !has_official {
        let label = source.as_str();
        let lower = label.to_lowercase();
        let mut lines = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                format!("{label} official limits not available"),
                Style::default()
                    .fg(TuiColor::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
        ];
        match source {
            SourceKind::Claude => {
                lines.push(Line::from(
                    "Fetching via Claude CLI probe... (may take ~15s on first load)",
                ));
                lines.push(Line::from(
                    "If this persists, ensure `claude` CLI is installed and accessible.",
                ));
            }
            SourceKind::Codex => {
                lines.push(Line::from(format!(
                    "Run `tu live {lower}` after using {label} to fetch limits.",
                )));
            }
        }

        // Still show today's usage from logs below the warning
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Usage from logs",
            Style::default()
                .fg(TuiColor::Cyan)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(vec![
            Span::raw(live_key_label("Today tokens")),
            Span::styled(
                format_u64(context.today_totals.total_tokens),
                Style::default()
                    .fg(TuiColor::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw(live_key_label("Today cost")),
            Span::styled(
                format_usd(context.today_totals.cost_usd),
                Style::default()
                    .fg(TuiColor::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        let msg = Paragraph::new(lines).wrap(Wrap { trim: true });
        frame.render_widget(msg, area);
        return;
    }

    render_live_body(frame, area, context);
}

fn render_live_progress_bars_for(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
    source_override: Option<SourceKind>,
) {
    let preferred_official = match source_override {
        Some(SourceKind::Codex) => context.official_codex.map(LiveOfficialRef::Codex),
        Some(SourceKind::Claude) => context.official_claude.map(LiveOfficialRef::Claude),
        None => preferred_official_for_live(context),
    };
    let show_weekly = preferred_official
        .and_then(LiveOfficialRef::secondary_used_percent)
        .is_some();
    let constraints = if show_weekly {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]
    } else {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]
    };
    let rows = Layout::vertical(constraints).split(area);
    let time_label_area = rows[0];
    let time_area = rows[1];

    let time_ratio = if context.window_secs > 0 {
        (context.elapsed_secs as f64 / context.window_secs as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let time_pct = time_ratio * 100.0;
    let time_label = if area.width >= 96 {
        format!(
            "{} / {} ({time_pct:.1}%)",
            format_hours_minutes(context.elapsed_secs / 60),
            format_hours_minutes(context.window_secs / 60)
        )
    } else {
        format!("{time_pct:.1}%")
    };
    let time_title = Paragraph::new(Line::from(vec![
        Span::styled("Time ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(
            "{} / {}",
            format_hours_minutes(context.elapsed_secs / 60),
            format_hours_minutes(context.window_secs / 60)
        )),
    ]));
    frame.render_widget(time_title, time_label_area);
    let time_gauge = Gauge::default()
        .style(live_gauge_track_style())
        .gauge_style(
            Style::default()
                .fg(TuiColor::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(time_ratio)
        .label(time_label);
    frame.render_widget(time_gauge, time_area);

    if let Some(official) = preferred_official
        && let Some(primary_used) = official.primary_used_percent()
    {
        let primary_title = Paragraph::new(Line::from(vec![Span::styled(
            format!("Session ({})", official.provider_label()),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        frame.render_widget(primary_title, rows[2]);

        let mut primary_label = format!("{primary_used:.1}% used");
        if let Some(resets_at) = official.primary_resets_at() {
            let eta_text = format_time_until_reset_short(resets_at, Utc::now());
            let local_reset = format_reset_timestamp(resets_at, context.tz);
            primary_label.push_str(&format!(" | resets in {eta_text} ({local_reset})"));
        }
        let primary_gauge = Gauge::default()
            .style(live_gauge_track_style())
            .gauge_style(
                Style::default()
                    .fg(used_gauge_color(primary_used))
                    .add_modifier(Modifier::BOLD),
            )
            .ratio((primary_used / 100.0).clamp(0.0, 1.0))
            .label(primary_label);
        frame.render_widget(primary_gauge, rows[3]);

        if show_weekly && let Some(weekly_used) = official.secondary_used_percent() {
            let weekly_title = Paragraph::new(Line::from(vec![Span::styled(
                format!("Weekly ({})", official.provider_label()),
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            frame.render_widget(weekly_title, rows[4]);

            let mut weekly_label = format!("{weekly_used:.1}% used");
            if let Some(resets_at) = official.secondary_resets_at() {
                let eta_text = format_time_until_reset_short(resets_at, Utc::now());
                let local_reset = format_reset_timestamp(resets_at, context.tz);
                weekly_label.push_str(&format!(" | resets in {eta_text} ({local_reset})"));
            }
            let weekly_gauge = Gauge::default()
                .style(live_gauge_track_style())
                .gauge_style(
                    Style::default()
                        .fg(used_gauge_color(weekly_used))
                        .add_modifier(Modifier::BOLD),
                )
                .ratio((weekly_used / 100.0).clamp(0.0, 1.0))
                .label(weekly_label);
            frame.render_widget(weekly_gauge, rows[5]);
        }
        return;
    }

    let blended = blended_projection(context);
    let current_tokens = context
        .active
        .map(|active_block| active_block.totals.total_tokens)
        .unwrap_or(0);
    let projected_tokens = blended
        .map(|projection| projection.projected_tokens_end)
        .or_else(|| {
            context
                .active
                .map(|active_block| projected_end(active_block).0)
        })
        .unwrap_or(0);

    let (limit_ratio, limit_label, limit_color, promoted) = match context.limit.token_limit {
        Some(0) => (0.0, "disabled (0)".to_string(), TuiColor::DarkGray, false),
        Some(limit) => {
            let (effective_limit, promotions) = resolve_display_limit(
                limit,
                projected_tokens,
                context.limit.token_limit_source,
                context.limit.membership_estimate,
            );
            let current_pct = (current_tokens as f64 / effective_limit as f64) * 100.0;
            let projected_pct = (projected_tokens as f64 / effective_limit as f64) * 100.0;
            let (status, status_color) = limit_status(projected_pct);
            let ratio = (projected_pct / 100.0).clamp(0.0, 1.0);
            let limit_prefix = match context.limit.token_limit_source {
                TokenLimitSource::EstimatedFromLogs => "est limit",
                _ => "limit",
            };
            let promoted = !promotions.is_empty();
            let label = if area.width >= 120 && promoted {
                format!(
                    "{limit_prefix} {} (auto from {}) | current {:.1}% | projected {:.1}% ({status})",
                    format_u64(effective_limit),
                    format_u64(limit),
                    current_pct,
                    projected_pct
                )
            } else if area.width >= 120 {
                format!(
                    "{limit_prefix} {} | current {:.1}% | projected {:.1}% ({status})",
                    format_u64(effective_limit),
                    current_pct,
                    projected_pct
                )
            } else {
                format!(
                    "cur {:.1}% proj {:.1}% ({status})",
                    current_pct, projected_pct
                )
            };
            (ratio, label, status_color, promoted)
        }
        None => (
            0.0,
            "not set (--token-limit <n|max>)".to_string(),
            TuiColor::DarkGray,
            false,
        ),
    };
    let limit_title_text = match context.limit.token_limit_source {
        TokenLimitSource::EstimatedFromLogs if promoted => "Estimated limit (tiered)",
        TokenLimitSource::EstimatedFromLogs => "Estimated limit (from logs)",
        TokenLimitSource::HistoricalMax => "Limit (historical max)",
        TokenLimitSource::Explicit => "Limit (explicit)",
        TokenLimitSource::Unset => "Limit",
    };
    let limit_title = Paragraph::new(Line::from(vec![Span::styled(
        limit_title_text,
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    frame.render_widget(limit_title, rows[2]);

    let limit_gauge = Gauge::default()
        .style(live_gauge_track_style())
        .gauge_style(
            Style::default()
                .fg(limit_color)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(limit_ratio)
        .label(limit_label);
    frame.render_widget(limit_gauge, rows[3]);
}

#[derive(Clone, Copy)]
enum LiveOfficialRef<'a> {
    Codex(&'a OfficialCodexSnapshot),
    Claude(&'a OfficialClaudeSnapshot),
    Antigravity(&'a OfficialAntigravitySnapshot),
}

impl<'a> LiveOfficialRef<'a> {
    fn provider_label(self) -> &'static str {
        match self {
            LiveOfficialRef::Codex(_) => "Codex",
            LiveOfficialRef::Claude(_) => "Claude",
            LiveOfficialRef::Antigravity(_) => "Antigravity",
        }
    }

    fn plan_type(self) -> Option<&'a str> {
        match self {
            LiveOfficialRef::Codex(snapshot) => snapshot.plan_type.as_deref(),
            LiveOfficialRef::Claude(snapshot) => snapshot.plan_type.as_deref(),
            LiveOfficialRef::Antigravity(snapshot) => snapshot.plan_type.as_deref(),
        }
    }

    fn primary_used_percent(self) -> Option<f64> {
        match self {
            LiveOfficialRef::Codex(snapshot) => snapshot.primary_used_percent,
            LiveOfficialRef::Claude(snapshot) => snapshot.primary_used_percent,
            LiveOfficialRef::Antigravity(snapshot) => snapshot.primary_used_percent,
        }
    }

    fn secondary_used_percent(self) -> Option<f64> {
        match self {
            LiveOfficialRef::Codex(snapshot) => snapshot.secondary_used_percent,
            LiveOfficialRef::Claude(snapshot) => snapshot.secondary_used_percent,
            LiveOfficialRef::Antigravity(snapshot) => snapshot.secondary_used_percent,
        }
    }

    fn secondary_window_mins(self) -> Option<i64> {
        match self {
            LiveOfficialRef::Codex(snapshot) => snapshot.secondary_window_mins,
            LiveOfficialRef::Claude(snapshot) => snapshot.secondary_window_mins,
            LiveOfficialRef::Antigravity(_) => None,
        }
    }

    fn primary_resets_at(self) -> Option<i64> {
        match self {
            LiveOfficialRef::Codex(snapshot) => snapshot.primary_resets_at,
            LiveOfficialRef::Claude(snapshot) => snapshot.primary_resets_at,
            LiveOfficialRef::Antigravity(snapshot) => snapshot.primary_resets_at,
        }
    }

    fn secondary_resets_at(self) -> Option<i64> {
        match self {
            LiveOfficialRef::Codex(snapshot) => snapshot.secondary_resets_at,
            LiveOfficialRef::Claude(snapshot) => snapshot.secondary_resets_at,
            LiveOfficialRef::Antigravity(snapshot) => snapshot.secondary_resets_at,
        }
    }
}

fn preferred_official_for_live<'a>(
    context: &'a LiveFrameContext<'a>,
) -> Option<LiveOfficialRef<'a>> {
    if let Some(source) = context.selected_source {
        return match source {
            SourceKind::Codex => context.official_codex.map(LiveOfficialRef::Codex),
            SourceKind::Claude => context.official_claude.map(LiveOfficialRef::Claude),
        };
    }

    // Antigravity is shown when it's the only available provider or alongside others
    if context.official_codex.is_none()
        && context.official_claude.is_none()
        && let Some(ag) = context.official_antigravity
    {
        return Some(LiveOfficialRef::Antigravity(ag));
    }

    match (context.official_codex, context.official_claude) {
        (Some(codex), None) => Some(LiveOfficialRef::Codex(codex)),
        (None, Some(claude)) => Some(LiveOfficialRef::Claude(claude)),
        (Some(codex), Some(claude)) => {
            match context.active.and_then(|active| active.dominant_source) {
                Some(SourceKind::Claude) => Some(LiveOfficialRef::Claude(claude)),
                Some(SourceKind::Codex) => Some(LiveOfficialRef::Codex(codex)),
                None => Some(LiveOfficialRef::Codex(codex)),
            }
        }
        (None, None) => None,
    }
}

fn used_gauge_color(used_percent: f64) -> TuiColor {
    if used_percent >= 85.0 {
        TuiColor::Red
    } else if used_percent >= 60.0 {
        TuiColor::Yellow
    } else {
        TuiColor::Green
    }
}

fn live_gauge_track_style() -> Style {
    Style::default().bg(TuiColor::Rgb(42, 46, 64))
}

fn render_live_body(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    if area.width >= 128 {
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)])
                .spacing(2)
                .areas(area);
        let left = Paragraph::new(live_current_lines(context)).wrap(Wrap { trim: true });
        let right = Paragraph::new(live_limit_lines(context)).wrap(Wrap { trim: true });
        frame.render_widget(left, left_area);
        frame.render_widget(right, right_area);
        return;
    }

    let mut lines = live_current_lines(context);
    lines.push(Line::from(""));
    lines.extend(live_limit_lines(context));
    let body = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(body, area);
}

#[derive(Clone, Copy)]
struct TodayProjection {
    tokens_per_hour: f64,
    cost_per_hour: f64,
    projected_tokens_end_of_day: u64,
    projected_cost_end_of_day: f64,
}

fn day_elapsed_seconds(now: DateTime<Utc>, tz: &TimeZoneMode) -> f64 {
    match tz {
        TimeZoneMode::Local => {
            let local = now.with_timezone(&Local);
            (i64::from(local.hour()) * 3600
                + i64::from(local.minute()) * 60
                + i64::from(local.second()))
            .max(1) as f64
        }
        TimeZoneMode::Utc => {
            (i64::from(now.hour()) * 3600 + i64::from(now.minute()) * 60 + i64::from(now.second()))
                .max(1) as f64
        }
        TimeZoneMode::Named(zone) => {
            let zoned = now.with_timezone(zone);
            (i64::from(zoned.hour()) * 3600
                + i64::from(zoned.minute()) * 60
                + i64::from(zoned.second()))
            .max(1) as f64
        }
    }
}

fn day_progress_ratio(now: DateTime<Utc>, tz: &TimeZoneMode) -> f64 {
    (day_elapsed_seconds(now, tz) / (24.0 * 3600.0)).clamp(0.0, 1.0)
}

const LIVE_KEY_COL_WIDTH: usize = 18;

fn live_key_label(key: &str) -> String {
    format!("{:<width$}", format!("{key}:"), width = LIVE_KEY_COL_WIDTH)
}

fn today_projection(context: &LiveFrameContext<'_>) -> Option<TodayProjection> {
    let elapsed_secs = day_elapsed_seconds(context.now, context.tz);
    if elapsed_secs < 10.0 * 60.0 {
        return None;
    }

    let tokens_per_sec = context.today_totals.total_tokens as f64 / elapsed_secs;
    let cost_per_sec = context.today_totals.cost_usd / elapsed_secs;
    let full_day_secs = 24.0 * 3600.0;
    Some(TodayProjection {
        tokens_per_hour: tokens_per_sec * 3600.0,
        cost_per_hour: cost_per_sec * 3600.0,
        projected_tokens_end_of_day: (tokens_per_sec * full_day_secs)
            .round()
            .max(context.today_totals.total_tokens as f64)
            as u64,
        projected_cost_end_of_day: (cost_per_sec * full_day_secs)
            .max(context.today_totals.cost_usd),
    })
}

#[derive(Clone, Copy)]
struct BlendedProjection {
    tokens_per_minute: f64,
    cost_per_hour: f64,
    projected_tokens_end: u64,
    projected_cost_end: f64,
    short_weight: f64,
    today_weight: f64,
    long_weight: f64,
}

#[derive(Clone, Copy)]
struct RateComponent {
    tokens_per_minute: f64,
    cost_per_minute: f64,
}

fn blended_projection(context: &LiveFrameContext<'_>) -> Option<BlendedProjection> {
    let block_ratio = if context.window_secs > 0 {
        (context.elapsed_secs as f64 / context.window_secs as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let day_ratio = day_progress_ratio(context.now, context.tz);

    let short_component = context.active.and_then(|active| {
        active.burn.as_ref().map(|burn| RateComponent {
            tokens_per_minute: burn.tokens_per_minute.max(0.0),
            cost_per_minute: (burn.cost_per_hour / 60.0).max(0.0),
        })
    });
    let short_score = if short_component.is_some() {
        0.15 + 1.35 * block_ratio
    } else {
        0.0
    };

    let today_component = today_projection(context).map(|projection| RateComponent {
        tokens_per_minute: (projection.tokens_per_hour / 60.0).max(0.0),
        cost_per_minute: (projection.cost_per_hour / 60.0).max(0.0),
    });
    let today_score = if today_component.is_some() {
        0.25 + 0.85 * day_ratio.sqrt()
    } else {
        0.0
    };

    let active_days = context.last_30d_active_days.max(1) as f64;
    let long_tokens_per_day = context.last_30d_totals.total_tokens as f64 / active_days;
    let long_cost_per_day = context.last_30d_totals.cost_usd / active_days;
    let long_component = if long_tokens_per_day > 0.0 || long_cost_per_day > 0.0 {
        Some(RateComponent {
            tokens_per_minute: (long_tokens_per_day / 1440.0).max(0.0),
            cost_per_minute: (long_cost_per_day / 1440.0).max(0.0),
        })
    } else {
        None
    };
    let long_score = if long_component.is_some() {
        (1.20 - 0.95 * block_ratio).clamp(0.10, 2.0)
    } else {
        0.0
    };

    let total_score = short_score + today_score + long_score;
    if total_score <= f64::EPSILON {
        return None;
    }

    let short_weight = short_score / total_score;
    let today_weight = today_score / total_score;
    let long_weight = long_score / total_score;

    let (short_tokens, short_cost) = short_component
        .map(|component| (component.tokens_per_minute, component.cost_per_minute))
        .unwrap_or((0.0, 0.0));
    let (today_tokens, today_cost) = today_component
        .map(|component| (component.tokens_per_minute, component.cost_per_minute))
        .unwrap_or((0.0, 0.0));
    let (long_tokens, long_cost) = long_component
        .map(|component| (component.tokens_per_minute, component.cost_per_minute))
        .unwrap_or((0.0, 0.0));

    let blended_tokens_per_minute =
        short_tokens * short_weight + today_tokens * today_weight + long_tokens * long_weight;
    let blended_cost_per_minute =
        short_cost * short_weight + today_cost * today_weight + long_cost * long_weight;

    let (current_tokens, current_cost, remaining_minutes) = context
        .active
        .map(|active| {
            (
                active.totals.total_tokens,
                active.totals.cost_usd,
                active.remaining_minutes.max(0),
            )
        })
        .unwrap_or_else(|| {
            (
                0,
                0.0,
                ((context.window_secs - context.elapsed_secs).max(0) / 60),
            )
        });

    let projected_tokens_end = (current_tokens as f64
        + blended_tokens_per_minute * remaining_minutes as f64)
        .round()
        .max(current_tokens as f64) as u64;
    let projected_cost_end =
        (current_cost + blended_cost_per_minute * remaining_minutes as f64).max(current_cost);

    Some(BlendedProjection {
        tokens_per_minute: blended_tokens_per_minute,
        cost_per_hour: blended_cost_per_minute * 60.0,
        projected_tokens_end,
        projected_cost_end,
        short_weight,
        today_weight,
        long_weight,
    })
}

fn live_key_value_line(
    key: impl AsRef<str>,
    value: impl Into<String>,
    value_color: TuiColor,
) -> Line<'static> {
    Line::from(vec![
        Span::raw(live_key_label(key.as_ref())),
        Span::styled(
            value.into(),
            Style::default()
                .fg(value_color)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn live_current_lines(context: &LiveFrameContext<'_>) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![Span::styled(
        "Current",
        Style::default()
            .fg(TuiColor::Cyan)
            .add_modifier(Modifier::BOLD),
    )])];
    let source_label = context
        .selected_source
        .map(SourceKind::as_str)
        .unwrap_or("all");
    lines.push(live_key_value_line(
        "Source",
        source_label,
        TuiColor::LightCyan,
    ));

    let Some(active_block) = context.active else {
        lines.push(Line::from("No active usage in this 5h window yet."));
        lines.push(live_key_value_line(
            "Today",
            format!(
                "{} · {} tokens",
                format_usd(context.today_totals.cost_usd),
                format_u64(context.today_totals.total_tokens)
            ),
            TuiColor::Green,
        ));
        return lines;
    };

    lines.push(live_key_value_line(
        "5h now",
        format!(
            "{} tokens | {}",
            format_u64(active_block.totals.total_tokens),
            format_usd(active_block.totals.cost_usd)
        ),
        TuiColor::LightCyan,
    ));

    if let Some(burn) = active_block.burn.as_ref() {
        lines.push(Line::from(vec![
            Span::raw(live_key_label("Burn avg")),
            Span::styled(
                format!(
                    "{} tokens/min | {}/hr",
                    format_u64(burn.tokens_per_minute.round().max(0.0) as u64),
                    format_usd(burn.cost_per_hour)
                ),
                Style::default()
                    .fg(TuiColor::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" | "),
            Span::styled(
                burn_status_text(burn.status),
                Style::default()
                    .fg(burn_status_color(burn.status))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    if let Some(projection) = today_projection(context) {
        lines.push(live_key_value_line(
            "Today EOD(avg)",
            format!(
                "{} tokens | {}",
                format_u64(projection.projected_tokens_end_of_day),
                format_usd(projection.projected_cost_end_of_day)
            ),
            TuiColor::LightCyan,
        ));
        lines.push(Line::from(vec![
            Span::raw(live_key_label("Today avg rate")),
            Span::styled(
                format!(
                    "{}/hr | {}/hr",
                    format_u64(projection.tokens_per_hour.round().max(0.0) as u64),
                    format_usd(projection.cost_per_hour)
                ),
                Style::default()
                    .fg(TuiColor::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    lines.push(live_key_value_line(
        "Today",
        format!(
            "{} · {} tokens",
            format_usd(context.today_totals.cost_usd),
            format_u64(context.today_totals.total_tokens)
        ),
        TuiColor::Green,
    ));
    lines.push(live_key_value_line(
        "Last 30d",
        format!(
            "{} · {} tokens",
            format_usd(context.last_30d_totals.cost_usd),
            format_u64(context.last_30d_totals.total_tokens)
        ),
        TuiColor::Green,
    ));

    lines
}

fn live_limit_lines(context: &LiveFrameContext<'_>) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![Span::styled(
        "Forecast",
        Style::default()
            .fg(TuiColor::Cyan)
            .add_modifier(Modifier::BOLD),
    )])];

    if let Some(official) = preferred_official_for_live(context) {
        if let Some(secondary_used) = official.secondary_used_percent() {
            if let Some((pace_line, pace_color)) = weekly_pace_line(
                secondary_used,
                official.secondary_window_mins(),
                official.secondary_resets_at(),
                context.now,
            ) {
                lines.push(live_key_value_line("Pace", pace_line, pace_color));
            }

            if let Some((runout_line, runout_color)) = weekly_runout_local_line(
                secondary_used,
                official.secondary_window_mins(),
                official.secondary_resets_at(),
                context.now,
                context.tz,
            ) {
                lines.push(live_key_value_line(
                    "Weekly runout",
                    runout_line,
                    runout_color,
                ));
            }
        } else if let Some(primary_used) = official.primary_used_percent() {
            let session_color = used_gauge_color(primary_used);
            lines.push(live_key_value_line(
                "Session trend",
                format!("{:.1}% used", primary_used),
                session_color,
            ));
        }
    } else {
        lines.push(live_key_value_line(
            "Official",
            "limits unavailable",
            TuiColor::Yellow,
        ));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "Projection",
        Style::default()
            .fg(TuiColor::DarkGray)
            .add_modifier(Modifier::BOLD),
    )]));

    let limit_ctx = LimitDisplayContext {
        token_limit: context.limit.token_limit,
        token_limit_source: context.limit.token_limit_source,
        membership_estimate: context.limit.membership_estimate,
    };
    let blended = blended_projection(context).unwrap_or_else(|| {
        let (current_tokens, current_cost) = context
            .active
            .map(|active| (active.totals.total_tokens, active.totals.cost_usd))
            .unwrap_or((0, 0.0));
        BlendedProjection {
            tokens_per_minute: 0.0,
            cost_per_hour: 0.0,
            projected_tokens_end: current_tokens,
            projected_cost_end: current_cost,
            short_weight: 0.0,
            today_weight: 0.0,
            long_weight: 1.0,
        }
    });

    lines.push(Line::from(vec![
        Span::raw(live_key_label("Rate blend")),
        Span::styled(
            format!(
                "{} tokens/min | {}/hr",
                format_u64(blended.tokens_per_minute.round().max(0.0) as u64),
                format_usd(blended.cost_per_hour)
            ),
            Style::default()
                .fg(TuiColor::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw(live_key_label("Weights")),
        Span::raw("short "),
        Span::styled(
            format!("{:.0}%", blended.short_weight * 100.0),
            Style::default()
                .fg(TuiColor::LightCyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" · today "),
        Span::styled(
            format!("{:.0}%", blended.today_weight * 100.0),
            Style::default()
                .fg(TuiColor::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" · 30d "),
        Span::styled(
            format!("{:.0}%", blended.long_weight * 100.0),
            Style::default()
                .fg(TuiColor::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(live_key_value_line(
        "5h projected end",
        format!(
            "{} | {}",
            format_u64(blended.projected_tokens_end),
            format_usd(blended.projected_cost_end)
        ),
        TuiColor::LightCyan,
    ));

    let current_tokens = context
        .active
        .map(|active_block| active_block.totals.total_tokens)
        .unwrap_or(0);
    if let Some(limit) = limit_ctx.token_limit {
        if limit > 0 {
            let (effective_limit, _promotions) = resolve_display_limit(
                limit,
                blended.projected_tokens_end,
                limit_ctx.token_limit_source,
                limit_ctx.membership_estimate,
            );
            let current_pct = (current_tokens as f64 / effective_limit as f64) * 100.0;
            let projected_pct =
                (blended.projected_tokens_end as f64 / effective_limit as f64) * 100.0;
            let (status, status_color) = limit_status(projected_pct);
            lines.push(Line::from(vec![
                Span::raw(live_key_label("5h limit")),
                Span::styled(
                    format_u64(effective_limit),
                    Style::default()
                        .fg(TuiColor::LightCyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" | current "),
                Span::styled(
                    format!("{current_pct:.1}%"),
                    Style::default()
                        .fg(used_gauge_color(current_pct))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" | projected "),
                Span::styled(
                    format!("{projected_pct:.1}%"),
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" | "),
                Span::styled(
                    status,
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    } else if let Some(estimate) = limit_ctx.membership_estimate {
        lines.push(Line::from(vec![
            Span::raw(live_key_label("Est. 5h limit")),
            Span::styled(
                format_u64(estimate.estimated_window_tokens),
                Style::default()
                    .fg(TuiColor::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ("),
            Span::styled(
                format!("{:.0}%", estimate.confidence * 100.0),
                Style::default()
                    .fg(TuiColor::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" confidence)"),
        ]));
    }

    if let Some(ag) = context.official_antigravity {
        lines.push(Line::from(""));
        let plan = ag.plan_type.as_deref().unwrap_or("unknown");
        lines.push(Line::from(vec![Span::styled(
            format!("Antigravity ({plan})"),
            Style::default()
                .fg(TuiColor::Green)
                .add_modifier(Modifier::BOLD),
        )]));
        let now = Utc::now();
        let ordered = select_antigravity_models(&ag.models);
        let shown: HashSet<String> = ordered.iter().map(|m| m.label.clone()).collect();
        let rest: Vec<_> = ag
            .models
            .iter()
            .filter(|m| !shown.contains(&m.label))
            .collect();
        for model in ordered.iter().chain(rest.into_iter()) {
            let (pct_text, color) = if let Some(frac) = model.remaining_fraction {
                let remaining = frac * 100.0;
                let used = 100.0 - remaining;
                let color = if remaining >= 40.0 {
                    TuiColor::Green
                } else if remaining >= 15.0 {
                    TuiColor::Yellow
                } else {
                    TuiColor::Red
                };
                (format!("{remaining:.0}% left ({used:.0}% used)"), color)
            } else {
                ("quota n/a".to_string(), TuiColor::DarkGray)
            };
            let mut spans = vec![
                Span::raw(format!("  {:<26} ", model.label)),
                Span::styled(
                    pct_text,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ];
            if let Some(reset_ts) = model.reset_time {
                let eta = format_time_until_reset_short(reset_ts, now);
                spans.push(Span::raw(format!(" ({eta})")));
            }
            lines.push(Line::from(spans));
        }
    }

    lines
}

fn weekly_pace_line(
    used_percent: f64,
    window_mins: Option<i64>,
    resets_at: Option<i64>,
    now: DateTime<Utc>,
) -> Option<(String, TuiColor)> {
    let window_secs = window_mins?.max(1) * 60;
    let reset_unix = resets_at?;
    let reset_dt = DateTime::from_timestamp(reset_unix, 0)?;
    let remaining_secs = (reset_dt - now).num_seconds().clamp(0, window_secs);
    let elapsed_secs = (window_secs - remaining_secs).clamp(0, window_secs);
    let elapsed_secs_f = elapsed_secs.max(1) as f64;
    let elapsed_pct = (elapsed_secs_f / window_secs as f64) * 100.0;
    let delta = used_percent - elapsed_pct;
    if delta.abs() < 3.0 {
        return Some(("On pace · Lasts to reset".to_string(), TuiColor::Green));
    }

    if delta > 0.0 {
        let mut suffix = String::new();
        if used_percent > 0.0 {
            let used_per_sec = used_percent / elapsed_secs as f64;
            if used_per_sec.is_finite() && used_per_sec > 0.0 {
                let secs_to_full = ((100.0 - used_percent).max(0.0) / used_per_sec).round() as i64;
                if secs_to_full > 0 && secs_to_full < remaining_secs {
                    suffix = format!(" · Runs out in {}", format_hours_minutes(secs_to_full / 60));
                } else if remaining_secs > 0 {
                    suffix = " · Lasts to reset".to_string();
                }
            }
        }
        let color = if delta >= 20.0 {
            TuiColor::Red
        } else {
            TuiColor::Yellow
        };
        Some((format!("Behind (-{:.1}%){}", delta.abs(), suffix), color))
    } else {
        Some((
            format!("Ahead (+{:.1}%) · Lasts to reset", delta.abs(),),
            TuiColor::Green,
        ))
    }
}

fn weekly_runout_local_line(
    used_percent: f64,
    window_mins: Option<i64>,
    resets_at: Option<i64>,
    now: DateTime<Utc>,
    tz: &TimeZoneMode,
) -> Option<(String, TuiColor)> {
    let window_secs = window_mins?.max(1) * 60;
    let reset_unix = resets_at?;
    let reset_dt = DateTime::from_timestamp(reset_unix, 0)?;
    let remaining_secs = (reset_dt - now).num_seconds().clamp(0, window_secs);
    let elapsed_secs = (window_secs - remaining_secs).clamp(0, window_secs);
    let elapsed_secs_f = elapsed_secs.max(1) as f64;
    let observed_used_per_sec = (used_percent / elapsed_secs_f).max(0.0);
    let baseline_used_per_sec = 100.0 / window_secs as f64;
    let observed_weight = (elapsed_secs_f / (6.0 * 3600.0)).clamp(0.15, 0.9);
    let blended_used_per_sec =
        baseline_used_per_sec * (1.0 - observed_weight) + observed_used_per_sec * observed_weight;
    if !blended_used_per_sec.is_finite() || blended_used_per_sec <= 0.0 {
        return Some(("Lasts to reset".to_string(), TuiColor::Green));
    }
    let secs_to_full = ((100.0 - used_percent).max(0.0) / blended_used_per_sec).round() as i64;
    if secs_to_full <= 0 {
        return Some(("Now".to_string(), TuiColor::Red));
    }

    let predicted = now + chrono::TimeDelta::seconds(secs_to_full);
    if predicted < reset_dt {
        let local = format_display_datetime(predicted, tz);
        let eta = format_hours_minutes((secs_to_full / 60).max(0));
        let color = if secs_to_full <= 24 * 3600 {
            TuiColor::Red
        } else {
            TuiColor::Yellow
        };
        return Some((format!("{local} (in {eta})"), color));
    }

    Some((
        format!(
            "Lasts to reset ({})",
            format_reset_timestamp(reset_unix, tz)
        ),
        TuiColor::Green,
    ))
}

fn projected_end(active_block: &ActiveBlockSummary) -> (u64, f64) {
    let current_tokens = active_block.totals.total_tokens;
    let current_cost = active_block.totals.cost_usd;
    let Some(burn) = active_block.burn.as_ref() else {
        return (current_tokens, current_cost);
    };

    let projected_tokens = (current_tokens as f64
        + burn.tokens_per_minute * active_block.remaining_minutes.max(0) as f64)
        .round()
        .max(current_tokens as f64) as u64;
    let projected_cost = (current_cost
        + (burn.cost_per_hour / 60.0) * active_block.remaining_minutes.max(0) as f64)
        .max(current_cost);
    (projected_tokens, projected_cost)
}

fn limit_status(projected_pct: f64) -> (&'static str, TuiColor) {
    if projected_pct >= 100.0 {
        ("EXCEEDS", TuiColor::Red)
    } else if projected_pct >= 80.0 {
        ("WARNING", TuiColor::Yellow)
    } else {
        ("OK", TuiColor::Green)
    }
}

fn burn_status_text(status: BurnStatus) -> &'static str {
    match status {
        BurnStatus::Normal => "Normal",
        BurnStatus::Moderate => "Moderate",
        BurnStatus::High => "High",
    }
}

fn burn_status_color(status: BurnStatus) -> TuiColor {
    match status {
        BurnStatus::Normal => TuiColor::Green,
        BurnStatus::Moderate => TuiColor::Yellow,
        BurnStatus::High => TuiColor::Red,
    }
}

fn poll_live_input(timeout: Duration, current_tab: LiveTab) -> Result<LiveInputEvent> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let poll_for = remaining.min(Duration::from_millis(50));
        if !event::poll(poll_for)? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Ok(LiveInputEvent::Exit),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(LiveInputEvent::Exit);
            }
            KeyCode::Tab | KeyCode::Right => {
                return Ok(LiveInputEvent::SwitchTab(current_tab.next()));
            }
            KeyCode::BackTab | KeyCode::Left => {
                return Ok(LiveInputEvent::SwitchTab(current_tab.prev()));
            }
            _ => {}
        }
    }

    Ok(LiveInputEvent::Tick)
}

pub(crate) async fn run_statusline(args: StatuslineArgs) -> Result<()> {
    if args.context_low_threshold >= args.context_medium_threshold {
        bail!(
            "--context-low-threshold ({}) must be less than --context-medium-threshold ({})",
            args.context_low_threshold,
            args.context_medium_threshold
        );
    }
    if args.context_medium_threshold > 100 {
        bail!("--context-medium-threshold must be <= 100");
    }

    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let hook = read_statusline_hook_input()?;
    let session_id = hook
        .as_ref()
        .and_then(|h| h.session_id.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let cache_path = statusline_cache_path(session_id);

    if args.cache
        && let Some(cached) = read_statusline_cache(
            &cache_path,
            args.refresh_interval,
            hook.as_ref().and_then(|h| h.transcript_path.as_deref()),
        )
    {
        print!("{cached}");
        return Ok(());
    }

    let loaded = load_usage(&args.common, &tz).await?;
    let today = local_date(Utc::now(), &tz);

    let today_totals = loaded
        .events
        .iter()
        .filter(|e| local_date(e.timestamp, &tz) == today)
        .fold(TokenCounts::default(), |mut acc, e| {
            acc.add_assign(e.usage.to_counts());
            acc
        });

    let session_totals = session_id.and_then(|id| aggregate_session_totals(&loaded.events, id));
    let block_summary = active_block_summary(&loaded.events, Utc::now(), 5 * 3600);
    let (official_codex, official_claude, official_antigravity) = if args.official_limits {
        let (codex, claude, antigravity, errors) =
            fetch_selected_official_limits(&args.common).await;
        for error in errors {
            eprintln!("{error}");
        }
        (codex, claude, antigravity)
    } else {
        (None, None, None)
    };
    let line = build_statusline_line(
        &args,
        hook.as_ref(),
        session_totals.as_ref(),
        &today_totals,
        block_summary.as_ref(),
        official_codex.as_ref(),
        official_claude.as_ref(),
        official_antigravity.as_ref(),
        &tz,
    );

    println!("{line}");

    if args.cache {
        write_statusline_cache(
            &cache_path,
            &line,
            hook.as_ref().and_then(|h| h.transcript_path.as_deref()),
        );
    }

    Ok(())
}

pub(crate) async fn collect_report(
    common: CommonArgs,
    period: ReportPeriod,
    instances: bool,
    project: Option<String>,
    start_of_week: WeekStart,
) -> Result<DailyReport> {
    let tz = parse_timezone_mode(common.timezone.as_deref())?;
    let loaded = load_usage(&common, &tz).await?;

    let rows = match period {
        ReportPeriod::Daily => {
            let events = loaded
                .events
                .into_iter()
                .filter(|event| match project.as_deref() {
                    Some(project_filter) => event
                        .project
                        .as_deref()
                        .is_some_and(|p| p.contains(project_filter)),
                    None => true,
                })
                .collect::<Vec<_>>();

            build_group_rows(&events, &common.order, |event| {
                let day = local_date(event.timestamp, &tz);
                let day_key = day.format("%Y-%m-%d").to_string();
                if instances {
                    match event.project.as_deref() {
                        Some(project_name) => format!("{project_name} | {day_key}"),
                        None => format!("- | {day_key}"),
                    }
                } else {
                    day_key
                }
            })
        }
        ReportPeriod::Monthly => build_group_rows(&loaded.events, &common.order, |event| {
            let day = local_date(event.timestamp, &tz);
            format!("{:04}-{:02}", day.year(), day.month())
        }),
        ReportPeriod::Weekly => build_group_rows(&loaded.events, &common.order, |event| {
            let day = local_date(event.timestamp, &tz);
            let start = week_start(day, start_of_week);
            start.format("%Y-%m-%d").to_string()
        }),
    };

    Ok(build_report_from_rows(rows, loaded.stats))
}

pub(crate) async fn collect_usage_snapshot(common: CommonArgs) -> Result<UsageSnapshot> {
    let timezone = parse_timezone_mode(common.timezone.as_deref())?;
    let loaded = load_usage(&common, &timezone).await?;
    Ok(UsageSnapshot {
        events: loaded.events,
        stats: loaded.stats,
        timezone,
    })
}

fn read_statusline_hook_input() -> Result<Option<StatuslineHookInput>> {
    if std::io::stdin().is_terminal() {
        return Ok(None);
    }

    let mut stdin = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin)
        .context("Failed to read statusline stdin")?;

    let raw = stdin.trim();
    if raw.is_empty() {
        return Ok(None);
    }

    let hook = serde_json::from_str::<StatuslineHookInput>(raw)
        .context("Invalid statusline stdin JSON payload")?;
    Ok(Some(hook))
}

fn statusline_cache_path(session_id: Option<&str>) -> PathBuf {
    let suffix = session_id
        .map(sanitize_cache_key)
        .unwrap_or_else(|| "global".to_string());
    std::env::temp_dir().join(format!("tu_statusline_cache_{suffix}.json"))
}

fn sanitize_cache_key(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
        if out.len() >= 96 {
            break;
        }
    }
    if out.is_empty() {
        "session".to_string()
    } else {
        out
    }
}

fn read_statusline_cache(
    cache_path: &Path,
    refresh_interval: u64,
    transcript_path: Option<&str>,
) -> Option<String> {
    let raw = std::fs::read_to_string(cache_path).ok()?;
    let entry = serde_json::from_str::<StatuslineCacheEntry>(&raw).ok()?;

    let now = unix_now_secs();
    if now.saturating_sub(entry.updated_unix) >= refresh_interval {
        return None;
    }

    if let Some(path) = transcript_path {
        let current_mtime = file_mtime_unix(path);
        if entry.transcript_path.as_deref() != Some(path)
            || entry.transcript_mtime_unix != current_mtime
        {
            return None;
        }
    }

    Some(format!("{}\n", entry.line))
}

fn write_statusline_cache(cache_path: &Path, line: &str, transcript_path: Option<&str>) {
    let entry = StatuslineCacheEntry {
        updated_unix: unix_now_secs(),
        transcript_path: transcript_path.map(ToString::to_string),
        transcript_mtime_unix: transcript_path.and_then(file_mtime_unix),
        line: line.to_string(),
    };

    if let Ok(serialized) = serde_json::to_string(&entry) {
        let _ = std::fs::write(cache_path, serialized);
    }
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn file_mtime_unix(path: &str) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn aggregate_session_totals(events: &[UsageEvent], session_id: &str) -> Option<TokenCounts> {
    let totals = events
        .iter()
        .filter(|event| session_id_matches(&event.session, session_id))
        .fold(TokenCounts::default(), |mut acc, event| {
            acc.add_assign(event.usage.to_counts());
            acc
        });

    if totals.total_tokens == 0 && totals.cost_usd <= 0.0 {
        None
    } else {
        Some(totals)
    }
}

fn session_id_matches(candidate: &str, query: &str) -> bool {
    candidate == query || candidate.ends_with(query) || candidate.contains(query)
}

fn active_block_summary(
    events: &[UsageEvent],
    now: DateTime<Utc>,
    block_window_secs: i64,
) -> Option<ActiveBlockSummary> {
    if block_window_secs <= 0 {
        return None;
    }

    let now_unix = now.timestamp();
    let block_start_unix = now_unix - now_unix.rem_euclid(block_window_secs);
    let block_end_unix = block_start_unix + block_window_secs;

    active_block_summary_for_bounds(events, now, block_start_unix, block_end_unix)
}

fn active_block_summary_for_bounds(
    events: &[UsageEvent],
    now: DateTime<Utc>,
    block_start_unix: i64,
    block_end_unix: i64,
) -> Option<ActiveBlockSummary> {
    if block_end_unix <= block_start_unix {
        return None;
    }
    let now_unix = now.timestamp();

    let mut selected = events
        .iter()
        .filter(|event| {
            let ts = event.timestamp.timestamp();
            ts >= block_start_unix && ts < block_end_unix
        })
        .collect::<Vec<_>>();

    if selected.is_empty() {
        return None;
    }

    selected.sort_by_key(|event| event.timestamp);

    let totals = selected
        .iter()
        .fold(TokenCounts::default(), |mut acc, event| {
            acc.add_assign(event.usage.to_counts());
            acc
        });

    let mut source_totals: HashMap<SourceKind, u64> = HashMap::new();
    for event in &selected {
        let entry = source_totals.entry(event.source).or_insert(0);
        *entry = entry.saturating_add(event.usage.to_counts().total_tokens);
    }
    let dominant_source = source_totals
        .into_iter()
        .max_by_key(|(_, tokens)| *tokens)
        .map(|(source, _)| source);

    let burn = {
        let first = selected.first().map(|event| event.timestamp);
        let last = selected.last().map(|event| event.timestamp);

        match (first, last) {
            (Some(first_ts), Some(last_ts)) => {
                let minutes = (last_ts - first_ts).num_minutes();
                if minutes > 0 {
                    let tokens_per_minute = totals.total_tokens as f64 / minutes as f64;
                    let non_cache_tokens = totals.input_tokens.saturating_add(totals.output_tokens);
                    let indicator = non_cache_tokens as f64 / minutes as f64;
                    let status = if indicator < 2000.0 {
                        BurnStatus::Normal
                    } else if indicator < 5000.0 {
                        BurnStatus::Moderate
                    } else {
                        BurnStatus::High
                    };

                    Some(BurnRateSummary {
                        cost_per_hour: totals.cost_usd / minutes as f64 * 60.0,
                        tokens_per_minute,
                        status,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    };

    Some(ActiveBlockSummary {
        totals,
        remaining_minutes: ((block_end_unix - now_unix) / 60).max(0),
        burn,
        dominant_source,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_statusline_line(
    args: &StatuslineArgs,
    hook: Option<&StatuslineHookInput>,
    session_totals: Option<&TokenCounts>,
    today_totals: &TokenCounts,
    block: Option<&ActiveBlockSummary>,
    official_codex: Option<&OfficialCodexSnapshot>,
    official_claude: Option<&OfficialClaudeSnapshot>,
    official_antigravity: Option<&OfficialAntigravitySnapshot>,
    tz: &TimeZoneMode,
) -> String {
    let model_name = hook
        .and_then(|h| h.model.as_ref())
        .and_then(|m| m.display_name.as_deref().or(m.id.as_deref()))
        .unwrap_or("unknown");

    let cc_cost = hook
        .and_then(|h| h.cost.as_ref())
        .and_then(|c| c.total_cost_usd);
    let derived_cost = session_totals.map(|t| t.cost_usd);
    let session_text = match args.cost_source {
        CostSource::Auto => format_usd(cc_cost.or(derived_cost).unwrap_or(0.0)),
        CostSource::Derived => format_usd(derived_cost.unwrap_or(0.0)),
        CostSource::Cc => format_usd(cc_cost.unwrap_or(0.0)),
        CostSource::Both => format!(
            "{} hook / {} derived",
            format_usd(cc_cost.unwrap_or(0.0)),
            format_usd(derived_cost.unwrap_or(0.0))
        ),
    };

    let block_text = if let Some(block) = block {
        format!(
            "{} ({})",
            format_usd(block.totals.cost_usd),
            format_remaining_minutes(block.remaining_minutes)
        )
    } else {
        "n/a".to_string()
    };

    let mut parts = vec![
        format!("model {model_name}"),
        format!(
            "session {} | today {} | block {}",
            session_text,
            format_usd(today_totals.cost_usd),
            block_text
        ),
    ];

    if let Some(burn) = block.and_then(|b| b.burn.as_ref())
        && args.visual_burn_rate != VisualBurnRate::Off
    {
        let emoji = match burn.status {
            BurnStatus::Normal => "🟢",
            BurnStatus::Moderate => "⚠️",
            BurnStatus::High => "🚨",
        };
        let label = match burn.status {
            BurnStatus::Normal => "Normal",
            BurnStatus::Moderate => "Moderate",
            BurnStatus::High => "High",
        };

        let extra = match args.visual_burn_rate {
            VisualBurnRate::Off => String::new(),
            VisualBurnRate::Emoji => format!(" {emoji}"),
            VisualBurnRate::Text => format!(" ({label})"),
            VisualBurnRate::EmojiText => format!(" {emoji} ({label})"),
        };
        parts.push(format!(
            "burn {}/hr, {}/min{}",
            format_usd(burn.cost_per_hour),
            format_u64(burn.tokens_per_minute.round() as u64),
            extra
        ));
    }

    if let Some(context) = hook.and_then(|h| h.context_window.as_ref()) {
        let input = context.total_input_tokens.unwrap_or(0);
        let limit = context.context_window_size.unwrap_or(0);
        if limit > 0 {
            let pct = (input as f64 / limit as f64) * 100.0;
            let level = if pct < f64::from(args.context_low_threshold) {
                "low"
            } else if pct < f64::from(args.context_medium_threshold) {
                "medium"
            } else {
                "high"
            };
            parts.push(format!("ctx {} ({pct:.0}%, {level})", format_u64(input)));
        }
    }

    if let Some(official) = official_codex {
        parts.push(build_statusline_official_codex_segment(official, tz));
    }
    if let Some(official) = official_claude {
        parts.push(build_statusline_official_claude_segment(official, tz));
    }
    if let Some(official) = official_antigravity {
        parts.push(build_statusline_official_antigravity_segment(official, tz));
    }

    parts.join(" | ")
}

fn build_statusline_official_codex_segment(
    official: &OfficialCodexSnapshot,
    tz: &TimeZoneMode,
) -> String {
    let plan = official.plan_type.as_deref().unwrap_or("unknown");
    let now = Utc::now();
    let mut parts = vec![format!("official {plan}")];

    if let Some(primary_used) = official.primary_used_percent {
        let remaining = (100.0 - primary_used).clamp(0.0, 100.0);
        let mut entry = format!("5h {:.1}% left", remaining);
        if let Some(resets_at) = official.primary_resets_at {
            let reset_text = format_reset_timestamp(resets_at, tz);
            let eta_text = format_time_until_reset_short(resets_at, now);
            entry.push_str(&format!(" (reset {reset_text}, in {eta_text})"));
        }
        parts.push(entry);
    }

    if let Some(secondary_used) = official.secondary_used_percent {
        let remaining = (100.0 - secondary_used).clamp(0.0, 100.0);
        let mut entry = format!("wk {:.1}% left", remaining);
        if let Some(resets_at) = official.secondary_resets_at {
            let reset_text = format_reset_timestamp(resets_at, tz);
            let eta_text = format_time_until_reset_short(resets_at, now);
            entry.push_str(&format!(" (reset {reset_text}, in {eta_text})"));
        }
        parts.push(entry);
    }

    parts.join(" ")
}

fn build_statusline_official_claude_segment(
    official: &OfficialClaudeSnapshot,
    tz: &TimeZoneMode,
) -> String {
    let plan = official.plan_type.as_deref().unwrap_or("unknown");
    let now = Utc::now();
    let mut parts = vec![format!("official claude {plan}")];

    if let Some(primary_used) = official.primary_used_percent {
        let remaining = (100.0 - primary_used).clamp(0.0, 100.0);
        let mut entry = format!("5h {:.1}% left", remaining);
        if let Some(resets_at) = official.primary_resets_at {
            let reset_text = format_reset_timestamp(resets_at, tz);
            let eta_text = format_time_until_reset_short(resets_at, now);
            entry.push_str(&format!(" (reset {reset_text}, in {eta_text})"));
        }
        parts.push(entry);
    }

    if let Some(secondary_used) = official.secondary_used_percent {
        let remaining = (100.0 - secondary_used).clamp(0.0, 100.0);
        let mut entry = format!("wk {:.1}% left", remaining);
        if let Some(resets_at) = official.secondary_resets_at {
            let reset_text = format_reset_timestamp(resets_at, tz);
            let eta_text = format_time_until_reset_short(resets_at, now);
            entry.push_str(&format!(" (reset {reset_text}, in {eta_text})"));
        }
        parts.push(entry);
    }

    parts.join(" ")
}

fn build_statusline_official_antigravity_segment(
    official: &OfficialAntigravitySnapshot,
    tz: &TimeZoneMode,
) -> String {
    let plan = official.plan_type.as_deref().unwrap_or("unknown");
    let now = Utc::now();
    let mut parts = vec![format!("antigravity {plan}")];

    let slots: &[(Option<f64>, Option<&str>, Option<i64>)] = &[
        (
            official.primary_used_percent,
            official.primary_label.as_deref(),
            official.primary_resets_at,
        ),
        (
            official.secondary_used_percent,
            official.secondary_label.as_deref(),
            official.secondary_resets_at,
        ),
        (
            official.tertiary_used_percent,
            official.tertiary_label.as_deref(),
            official.tertiary_resets_at,
        ),
    ];

    for (used_opt, label, resets_at) in slots {
        if let Some(used) = used_opt {
            let tag = label.unwrap_or("model");
            let remaining = (100.0 - used).clamp(0.0, 100.0);
            let mut entry = format!("{tag} {remaining:.1}% left");
            if let Some(resets_at) = resets_at {
                let reset_text = format_reset_timestamp(*resets_at, tz);
                let eta_text = format_time_until_reset_short(*resets_at, now);
                entry.push_str(&format!(" (reset {reset_text}, in {eta_text})"));
            }
            parts.push(entry);
        }
    }

    parts.join(" ")
}

fn format_remaining_minutes(minutes: i64) -> String {
    format!("{} left", format_hours_minutes(minutes))
}

fn format_hours_minutes(minutes: i64) -> String {
    let safe = minutes.max(0);
    let hrs = safe / 60;
    let mins = safe % 60;
    if hrs > 0 {
        format!("{hrs}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

fn official_window_details(
    window_mins: Option<i64>,
    resets_at: Option<i64>,
    tz: &TimeZoneMode,
) -> Vec<String> {
    let mut details = Vec::new();
    if let Some(mins) = window_mins {
        details.push(format!("window={mins}m"));
    }
    if let Some(resets_at) = resets_at {
        details.push(format!("resets={}", format_reset_timestamp(resets_at, tz)));
    }
    details
}

fn format_reset_timestamp(unix_secs: i64, tz: &TimeZoneMode) -> String {
    DateTime::from_timestamp(unix_secs, 0)
        .map(|ts| format_display_datetime(ts, tz))
        .unwrap_or_else(|| format!("unix:{unix_secs}"))
}

fn format_time_until_reset_short(resets_at: i64, now: DateTime<Utc>) -> String {
    let delta_secs = (resets_at - now.timestamp()).max(0);
    let minutes = delta_secs / 60;
    let days = minutes / (24 * 60);
    if days > 0 {
        let hours = (minutes % (24 * 60)) / 60;
        if hours > 0 {
            format!("{days}d {hours}h")
        } else {
            format!("{days}d")
        }
    } else {
        format_hours_minutes(minutes)
    }
}

#[derive(Debug, Deserialize)]
struct CodexAuthFile {
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    #[serde(default)]
    tokens: Option<CodexAuthTokens>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexAuthTokens {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexUsageResponse {
    #[serde(default)]
    plan_type: Option<String>,
    #[serde(default)]
    rate_limit: Option<CodexRateLimitDetails>,
}

#[derive(Debug, Deserialize)]
struct CodexRateLimitDetails {
    #[serde(default)]
    primary_window: Option<CodexUsageWindow>,
    #[serde(default)]
    secondary_window: Option<CodexUsageWindow>,
}

#[derive(Debug, Deserialize)]
struct CodexUsageWindow {
    #[serde(default)]
    used_percent: Option<f64>,
    #[serde(default)]
    reset_at: Option<i64>,
    #[serde(default)]
    limit_window_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CodexRefreshResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Debug)]
enum CodexOAuthFetchError {
    Unauthorized,
    Other(anyhow::Error),
}

#[derive(Debug, Deserialize)]
struct ClaudeCredentialsFile {
    #[serde(rename = "claudeAiOauth", alias = "claude_ai_oauth")]
    claude_ai_oauth: Option<ClaudeOAuthTokens>,
}

#[derive(Debug, Clone, Deserialize)]
struct ClaudeOAuthTokens {
    #[serde(rename = "accessToken", alias = "access_token")]
    access_token: Option<String>,
    #[serde(rename = "refreshToken", alias = "refresh_token")]
    refresh_token: Option<String>,
    #[serde(rename = "expiresAt", alias = "expires_at")]
    expires_at: Option<f64>,
    #[serde(rename = "rateLimitTier", alias = "rate_limit_tier")]
    rate_limit_tier: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeOAuthUsageResponse {
    #[serde(default)]
    five_hour: Option<ClaudeOAuthWindow>,
    #[serde(default)]
    seven_day: Option<ClaudeOAuthWindow>,
    #[serde(default)]
    seven_day_oauth_apps: Option<ClaudeOAuthWindow>,
    #[serde(default)]
    seven_day_opus: Option<ClaudeOAuthWindow>,
    #[serde(default)]
    seven_day_sonnet: Option<ClaudeOAuthWindow>,
}

#[derive(Debug, Deserialize)]
struct ClaudeOAuthWindow {
    #[serde(default)]
    utilization: Option<f64>,
    #[serde(default)]
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeRefreshResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Debug)]
enum ClaudeOAuthFetchError {
    Unauthorized,
    Other(anyhow::Error),
}

#[derive(Debug, Deserialize)]
struct RpcEnvelope {
    id: Option<serde_json::Value>,
    result: Option<serde_json::Value>,
    error: Option<RpcErrorObject>,
}

#[derive(Debug, Deserialize)]
struct RpcErrorObject {
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RpcRateLimitsReadResult {
    #[serde(rename = "rateLimits")]
    rate_limits: Option<RpcRateLimits>,
}

#[derive(Debug, Deserialize)]
struct RpcRateLimits {
    primary: Option<RpcRateLimitWindow>,
    secondary: Option<RpcRateLimitWindow>,
    #[serde(rename = "planType")]
    plan_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RpcRateLimitWindow {
    #[serde(rename = "usedPercent")]
    used_percent: Option<f64>,
    #[serde(rename = "windowDurationMins")]
    window_duration_mins: Option<i64>,
    #[serde(rename = "resetsAt")]
    resets_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RpcAccountReadResult {
    account: Option<RpcAccount>,
}

#[derive(Debug, Deserialize)]
struct RpcAccount {
    #[serde(rename = "planType")]
    plan_type: Option<String>,
}

async fn fetch_codex_official_limits() -> Result<OfficialCodexSnapshot> {
    match fetch_codex_official_limits_via_oauth().await {
        Ok(snapshot) => Ok(snapshot),
        Err(oauth_error) => {
            let fallback = tokio::task::spawn_blocking(fetch_codex_official_limits_blocking)
                .await
                .context("codex app-server task join failed")?;
            fallback.with_context(|| format!("oauth failed first: {oauth_error}"))
        }
    }
}

async fn fetch_codex_official_limits_via_oauth() -> Result<OfficialCodexSnapshot> {
    let (auth_path, mut tokens) = load_codex_auth_tokens()?;

    match fetch_codex_usage_with_access_token(&tokens.access_token, tokens.account_id.as_deref())
        .await
    {
        Ok(snapshot) => Ok(snapshot),
        Err(CodexOAuthFetchError::Unauthorized) => {
            let refresh = tokens
                .refresh_token
                .as_deref()
                .filter(|v| !v.is_empty())
                .context("Codex OAuth token unauthorized and no refresh token available")?;
            let refreshed = refresh_codex_access_token(refresh).await?;
            tokens.access_token = refreshed.0;
            tokens.refresh_token = Some(refreshed.1);
            tokens.id_token = refreshed.2;
            let _ = save_codex_auth_tokens(&auth_path, &tokens);
            fetch_codex_usage_with_access_token(&tokens.access_token, tokens.account_id.as_deref())
                .await
                .map_err(|err| match err {
                    CodexOAuthFetchError::Unauthorized => {
                        anyhow::anyhow!("Codex OAuth remained unauthorized after refresh")
                    }
                    CodexOAuthFetchError::Other(error) => error,
                })
        }
        Err(CodexOAuthFetchError::Other(error)) => Err(error),
    }
}

fn load_codex_auth_tokens() -> Result<(PathBuf, CodexAuthTokens)> {
    let auth_path = codex_auth_path().context("Failed to resolve Codex auth path")?;
    let raw = std::fs::read(&auth_path)
        .with_context(|| format!("Failed to read Codex auth file: {}", auth_path.display()))?;
    let parsed: CodexAuthFile =
        serde_json::from_slice(&raw).context("Invalid Codex auth.json format")?;

    if let Some(api_key) = parsed.openai_api_key {
        let trimmed = api_key.trim();
        if !trimmed.is_empty() {
            return Ok((
                auth_path,
                CodexAuthTokens {
                    access_token: trimmed.to_string(),
                    refresh_token: None,
                    id_token: None,
                    account_id: None,
                },
            ));
        }
    }

    let Some(tokens) = parsed.tokens else {
        bail!("Codex auth.json missing tokens");
    };
    if tokens.access_token.trim().is_empty() {
        bail!("Codex auth.json missing access_token");
    }
    Ok((auth_path, tokens))
}

fn save_codex_auth_tokens(path: &Path, tokens: &CodexAuthTokens) -> Result<()> {
    let existing = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let mut root = existing;
    if !root.is_object() {
        root = serde_json::json!({});
    }
    let obj = root
        .as_object_mut()
        .context("Codex auth root must be object")?;
    let tokens_value = obj
        .entry("tokens".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !tokens_value.is_object() {
        *tokens_value = serde_json::json!({});
    }
    let token_obj = tokens_value
        .as_object_mut()
        .context("Codex auth tokens must be object")?;
    token_obj.insert(
        "access_token".to_string(),
        serde_json::Value::String(tokens.access_token.clone()),
    );
    if let Some(refresh) = tokens.refresh_token.as_ref().filter(|v| !v.is_empty()) {
        token_obj.insert(
            "refresh_token".to_string(),
            serde_json::Value::String(refresh.clone()),
        );
    }
    if let Some(id_token) = tokens.id_token.as_ref().filter(|v| !v.is_empty()) {
        token_obj.insert(
            "id_token".to_string(),
            serde_json::Value::String(id_token.clone()),
        );
    }
    if let Some(account_id) = tokens.account_id.as_ref().filter(|v| !v.is_empty()) {
        token_obj.insert(
            "account_id".to_string(),
            serde_json::Value::String(account_id.clone()),
        );
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let bytes = serde_json::to_vec_pretty(&root).context("Failed to serialize Codex auth file")?;
    std::fs::write(path, bytes)
        .with_context(|| format!("Failed to write Codex auth file: {}", path.display()))?;
    Ok(())
}

fn codex_auth_path() -> Option<PathBuf> {
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        let trimmed = codex_home.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed).join("auth.json"));
        }
    }
    Some(dirs::home_dir()?.join(".codex").join("auth.json"))
}

fn codex_config_path() -> Option<PathBuf> {
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        let trimmed = codex_home.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed).join("config.toml"));
        }
    }
    Some(dirs::home_dir()?.join(".codex").join("config.toml"))
}

fn resolve_codex_usage_base_url() -> String {
    let mut base = "https://chatgpt.com/backend-api".to_string();
    if let Some(config_path) = codex_config_path()
        && let Ok(contents) = std::fs::read_to_string(config_path)
        && let Some(parsed) = parse_chatgpt_base_url(&contents)
    {
        base = parsed;
    }

    while base.ends_with('/') {
        base.pop();
    }
    if (base.starts_with("https://chatgpt.com") || base.starts_with("https://chat.openai.com"))
        && !base.contains("/backend-api")
    {
        base.push_str("/backend-api");
    }
    base
}

fn parse_chatgpt_base_url(contents: &str) -> Option<String> {
    for raw_line in contents.lines() {
        let line = raw_line
            .split('#')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '=');
        let key = parts.next()?.trim();
        let value = parts.next()?.trim();
        if key != "chatgpt_base_url" {
            continue;
        }
        let unquoted = value
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .to_string();
        if !unquoted.is_empty() {
            return Some(unquoted);
        }
    }
    None
}

async fn fetch_codex_usage_with_access_token(
    access_token: &str,
    account_id: Option<&str>,
) -> std::result::Result<OfficialCodexSnapshot, CodexOAuthFetchError> {
    let base = resolve_codex_usage_base_url();
    let path = if base.contains("/backend-api") {
        "/wham/usage"
    } else {
        "/api/codex/usage"
    };
    let url = format!("{base}{path}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| CodexOAuthFetchError::Other(anyhow::Error::new(error)))?;

    let mut request = client
        .get(url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .header("User-Agent", "tokenusage");
    if let Some(account_id) = account_id.filter(|v| !v.is_empty()) {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    let response = request
        .send()
        .await
        .map_err(|error| CodexOAuthFetchError::Other(anyhow::Error::new(error)))?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(CodexOAuthFetchError::Unauthorized);
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(CodexOAuthFetchError::Other(anyhow::anyhow!(
            "Codex usage API returned {status}: {body}"
        )));
    }

    let usage: CodexUsageResponse = response
        .json()
        .await
        .map_err(|error| CodexOAuthFetchError::Other(anyhow::Error::new(error)))?;
    Ok(OfficialCodexSnapshot {
        plan_type: usage.plan_type,
        primary_used_percent: usage.rate_limit.as_ref().and_then(|r| {
            r.primary_window
                .as_ref()
                .and_then(|window| window.used_percent)
                .map(normalize_official_used_percent)
        }),
        secondary_used_percent: usage.rate_limit.as_ref().and_then(|r| {
            r.secondary_window
                .as_ref()
                .and_then(|window| window.used_percent)
                .map(normalize_official_used_percent)
        }),
        primary_window_mins: usage.rate_limit.as_ref().and_then(|r| {
            r.primary_window
                .as_ref()
                .and_then(|window| window.limit_window_seconds)
                .map(|secs| secs / 60)
        }),
        secondary_window_mins: usage.rate_limit.as_ref().and_then(|r| {
            r.secondary_window
                .as_ref()
                .and_then(|window| window.limit_window_seconds)
                .map(|secs| secs / 60)
        }),
        primary_resets_at: usage
            .rate_limit
            .as_ref()
            .and_then(|r| r.primary_window.as_ref().and_then(|window| window.reset_at)),
        secondary_resets_at: usage.rate_limit.as_ref().and_then(|r| {
            r.secondary_window
                .as_ref()
                .and_then(|window| window.reset_at)
        }),
    })
}

async fn refresh_codex_access_token(
    refresh_token: &str,
) -> Result<(String, String, Option<String>)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client for Codex refresh")?;
    let response = client
        .post("https://auth.openai.com/oauth/token")
        .json(&serde_json::json!({
            "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "scope": "openid profile email"
        }))
        .send()
        .await
        .context("Codex refresh request failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Codex refresh failed ({status}): {body}");
    }
    let payload: CodexRefreshResponse = response
        .json()
        .await
        .context("Invalid Codex refresh response")?;
    Ok((
        payload.access_token,
        payload
            .refresh_token
            .unwrap_or_else(|| refresh_token.to_string()),
        payload.id_token,
    ))
}

async fn fetch_claude_official_limits() -> Result<OfficialClaudeSnapshot> {
    // Primary approach: CLI PTY probe (run `claude`, send `/usage`, parse output).
    match fetch_claude_limits_via_cli().await {
        Ok(snapshot) => return Ok(snapshot),
        Err(cli_err) => {
            eprintln!("claude cli probe failed: {cli_err:#}");
        }
    }

    // Fallback: OAuth via ~/.claude/.credentials.json (may not exist on newer installs).
    let (credentials_path, mut tokens) = load_claude_oauth_tokens()?;
    let current_access_token = tokens.access_token.clone().unwrap_or_default();
    match fetch_claude_usage_with_access_token(
        &current_access_token,
        tokens.rate_limit_tier.as_deref(),
    )
    .await
    {
        Ok(snapshot) => Ok(snapshot),
        Err(ClaudeOAuthFetchError::Unauthorized) => {
            let refresh = tokens
                .refresh_token
                .as_deref()
                .filter(|v| !v.is_empty())
                .context("Claude OAuth token unauthorized and no refresh token available")?;
            let refreshed = refresh_claude_access_token(refresh).await?;
            tokens.access_token = Some(refreshed.0);
            if let Some(refresh_token) = refreshed.1 {
                tokens.refresh_token = Some(refresh_token);
            }
            if let Some(expires_at) = refreshed.2 {
                tokens.expires_at = Some(expires_at as f64);
            }
            let _ = save_claude_oauth_tokens(&credentials_path, &tokens);
            let refreshed_access_token = tokens.access_token.clone().unwrap_or_default();
            fetch_claude_usage_with_access_token(
                &refreshed_access_token,
                tokens.rate_limit_tier.as_deref(),
            )
            .await
            .map_err(|err| match err {
                ClaudeOAuthFetchError::Unauthorized => {
                    anyhow::anyhow!("Claude OAuth remained unauthorized after refresh")
                }
                ClaudeOAuthFetchError::Other(error) => error,
            })
        }
        Err(ClaudeOAuthFetchError::Other(error)) => Err(error),
    }
}

// ---------------------------------------------------------------------------
// Claude CLI PTY probe — run `claude` in a pseudo-terminal, send `/usage`,
// parse the rendered TUI text to extract session/weekly usage percentages.
// ---------------------------------------------------------------------------

async fn fetch_claude_limits_via_cli() -> Result<OfficialClaudeSnapshot> {
    let claude_bin = resolve_claude_binary()?;
    let bin = claude_bin.clone();
    let raw_output = tokio::task::spawn_blocking(move || claude_pty_capture(&bin))
        .await
        .context("claude pty task join failed")??;
    let clean = strip_ansi_codes(&raw_output);
    let debug = std::env::var("TU_DEBUG_CLI").is_ok();
    if debug {
        eprintln!("--- claude cli CLEAN output ({} bytes) ---", clean.len());
        eprintln!("{clean}");
        eprintln!("--- end claude cli output ---");
    }

    // Retry once if output doesn't look relevant (CodexBar approach).
    let looks_relevant = {
        let compact: String = clean.to_ascii_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
        compact.contains("currentsession") || compact.contains("currentweek")
            || compact.contains("loadingusage") || compact.contains("failedtoloadusagedata")
    };
    if !looks_relevant {
        if debug {
            eprintln!("[pty] output looked like startup; retrying once...");
        }
        let bin2 = claude_bin;
        let raw2 = tokio::task::spawn_blocking(move || claude_pty_capture(&bin2))
            .await
            .context("claude pty retry join failed")??;
        let clean2 = strip_ansi_codes(&raw2);
        if debug {
            eprintln!("--- claude cli RETRY CLEAN output ({} bytes) ---", clean2.len());
            eprintln!("{clean2}");
            eprintln!("--- end claude cli retry output ---");
        }
        return parse_claude_usage_text(&clean2);
    }

    parse_claude_usage_text(&clean)
}

fn resolve_claude_binary() -> Result<String> {
    // Check common locations.
    if let Ok(path) = std::env::var("CLAUDE_BIN") {
        if Path::new(&path).is_file() {
            return Ok(path);
        }
    }
    // `which claude`
    let output = Command::new("which")
        .arg("claude")
        .output()
        .context("failed to run `which claude`")?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() && Path::new(&path).is_file() {
            return Ok(path);
        }
    }
    // Well-known paths.
    for candidate in &[
        dirs::home_dir().map(|h| h.join(".bun/bin/claude")),
        dirs::home_dir().map(|h| h.join(".npm-global/bin/claude")),
        Some(PathBuf::from("/usr/local/bin/claude")),
    ] {
        if let Some(p) = candidate {
            if p.is_file() {
                return Ok(p.to_string_lossy().into_owned());
            }
        }
    }
    bail!("Claude CLI binary not found. Install it or set CLAUDE_BIN env var.")
}

/// Open a PTY via `portable-pty`, spawn `claude --allowed-tools ""`,
/// send `/usage`, and collect the rendered TUI output.
fn claude_pty_capture(binary: &str) -> Result<String> {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let debug = std::env::var("TU_DEBUG_CLI").is_ok();

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 50,
            cols: 160,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("openpty failed")?;

    let mut cmd = CommandBuilder::new(binary);
    cmd.args(["--allowed-tools", ""]);

    // Remove all Claude-related env vars to avoid nested-session detection.
    for (key, _) in std::env::vars() {
        if key == "CLAUDECODE"
            || key.starts_with("ANTHROPIC_")
            || key.starts_with("CLAUDE_CODE")
        {
            cmd.env_remove(&key);
        }
    }
    cmd.env("TERM", "xterm-256color");
    if let Some(home) = dirs::home_dir() {
        cmd.cwd(&home);
    }

    let mut child = pair.slave.spawn_command(cmd).context("failed to spawn claude CLI")?;
    // Drop slave side in parent so reads on master detect EOF properly.
    drop(pair.slave);

    let reader = pair.master.try_clone_reader().context("clone pty reader")?;
    let mut writer = pair.master.take_writer().context("take pty writer")?;

    // Spawn a background reader thread. The PTY reader can block, so we
    // funnel all data into a channel that the main thread polls.
    let (tx, rx) = bounded::<Vec<u8>>(256);
    let reader_thread = thread::spawn(move || {
        let mut reader = reader;
        let mut buf = vec![0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Helper: drain all currently available chunks from the channel.
    let drain_rx = |rx: &Receiver<Vec<u8>>| -> String {
        let mut out = Vec::new();
        while let Ok(chunk) = rx.try_recv() {
            out.extend_from_slice(&chunk);
        }
        String::from_utf8_lossy(&out).into_owned()
    };

    // Helper: write to PTY master.
    let write_pty = |writer: &mut Box<dyn Write + Send>, text: &str| -> Result<()> {
        writer
            .write_all(text.as_bytes())
            .context("write to PTY failed")?;
        writer.flush().ok();
        Ok(())
    };

    // Wait for TUI to fully initialize. The Claude TUI needs time to render
    // the initial prompt. We look for the prompt character (❯) as a signal.
    let mut startup_buf = String::new();
    let startup_deadline = Instant::now() + Duration::from_secs(15);
    let mut last_startup_data = Instant::now();
    let mut sent_trust_accept = false;
    let mut saw_prompt = false;

    while Instant::now() < startup_deadline {
        let chunk = drain_rx(&rx);
        if !chunk.is_empty() {
            startup_buf.push_str(&chunk);
            last_startup_data = Instant::now();

            let clean = strip_ansi_codes(&startup_buf);
            let clean_lower = clean.to_ascii_lowercase();
            // Also check raw text (ANSI stripped may lose spaces between words).
            let raw_lower = startup_buf.to_ascii_lowercase();

            // Handle trust/safety prompts.
            // Claude CLI asks "Is this a project you created or one you trust?"
            // ANSI stripping may remove spaces, so match generously.
            let is_trust_dialog = (clean_lower.contains("trust")
                || raw_lower.contains("trust"))
                && (clean_lower.contains("enter to confirm")
                    || raw_lower.contains("enter to confirm")
                    || clean_lower.contains("entertoconfirm"));
            if !sent_trust_accept && is_trust_dialog {
                if debug {
                    eprintln!("[pty] detected trust/safety prompt, accepting...");
                }
                // Send Enter to confirm the default selection (❯ 1. Yes).
                let _ = write_pty(&mut writer, "\r");
                thread::sleep(Duration::from_millis(500));
                let _ = write_pty(&mut writer, "\r");
                sent_trust_accept = true;
                startup_buf.clear();
                last_startup_data = Instant::now();
                continue;
            }

            // Respond to command palette hints (compact match — no spaces).
            let compact_lower: String = clean_lower.chars().filter(|c| !c.is_whitespace()).collect();
            if compact_lower.contains("showplan") {
                let _ = write_pty(&mut writer, "\r");
            }

            // The Claude TUI shows ❯ when ready for input (main prompt, not
            // inside a trust/selection dialog — those also use ❯ for selection).
            let has_dialog = clean_lower.contains("trust")
                || raw_lower.contains("trust")
                || clean_lower.contains("entertoconfirm")
                || raw_lower.contains("enter to confirm");
            let is_main_prompt =
                (clean.contains('❯') || clean.contains("> ")) && !has_dialog;
            if is_main_prompt {
                saw_prompt = true;
                // Give a short settle after seeing prompt.
                thread::sleep(Duration::from_millis(300));
                let extra = drain_rx(&rx);
                if !extra.is_empty() {
                    startup_buf.push_str(&extra);
                }
                break;
            }
        }

        // Extended idle: wait longer (4s) since TUI startup can have gaps.
        if !startup_buf.is_empty()
            && Instant::now().duration_since(last_startup_data) > Duration::from_secs(4)
        {
            break;
        }

        thread::sleep(Duration::from_millis(80));
    }

    if debug {
        let clean = strip_ansi_codes(&startup_buf);
        eprintln!(
            "[pty] startup done ({} bytes, prompt={saw_prompt}), clean tail: {:?}",
            startup_buf.len(),
            clean.chars().rev().take(300).collect::<String>().chars().rev().collect::<String>()
        );
    }

    // Send /usage command.
    write_pty(&mut writer, "/usage\r")?;
    if debug {
        eprintln!("[pty] sent /usage command");
    }

    // Read output until we see usage data or timeout.
    // CodexBar approach: stop on specific labels, send Enter periodically to help TUI render,
    // settle for 2s after detecting stop patterns.
    let mut buffer = String::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last_enter_at = Instant::now();
    let send_enter_every = Duration::from_millis(800);
    let mut stopped_early = false;

    while Instant::now() < deadline {
        let chunk = drain_rx(&rx);
        if !chunk.is_empty() {
            buffer.push_str(&chunk);

            // Auto-respond to command palette prompts (compact match — no spaces).
            let chunk_compact: String = strip_ansi_codes(&chunk)
                .to_ascii_lowercase()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            if chunk_compact.contains("showplan") {
                let _ = write_pty(&mut writer, "\r");
            }

            // Check stop conditions using compact text (no whitespace).
            let clean_compact: String = strip_ansi_codes(&buffer)
                .to_ascii_lowercase()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();

            // Stop patterns (inspired by CodexBar): once we see these, the panel is rendering.
            let has_stop = clean_compact.contains("currentsession")
                || clean_compact.contains("currentweek")
                || clean_compact.contains("failedtoloadusagedata");
            if has_stop {
                stopped_early = true;
                break;
            }
        }

        // Send Enter periodically to help TUI render (CodexBar does this for /usage).
        if Instant::now().duration_since(last_enter_at) >= send_enter_every {
            let _ = write_pty(&mut writer, "\r");
            last_enter_at = Instant::now();
        }

        thread::sleep(Duration::from_millis(60));
    }

    // Settle: after detecting stop patterns, wait 2s more to capture the full panel.
    if stopped_early {
        let settle_deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < settle_deadline {
            let extra = drain_rx(&rx);
            if !extra.is_empty() {
                buffer.push_str(&extra);
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    if debug {
        eprintln!("[pty] read loop done, buffer len = {}", buffer.len());
    }

    // Clean up: send /exit and kill.
    let _ = write_pty(&mut writer, "/exit\r");
    thread::sleep(Duration::from_millis(200));
    let _ = child.kill();
    let _ = child.wait();
    drop(writer);
    drop(pair.master);
    let _ = reader_thread.join();

    if buffer.is_empty() {
        bail!("Claude CLI produced no output (timed out)");
    }
    Ok(buffer)
}

/// Strip ANSI escape codes from PTY output.
fn strip_ansi_codes(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // ESC sequence.
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                // Read until terminating letter.
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() || c == '~' {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                // OSC sequence: ESC ] ... ST (or BEL).
                chars.next();
                for c in chars.by_ref() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            } else {
                // Other ESC sequences — skip next char.
                chars.next();
            }
        } else if ch == '\r' {
            // Ignore carriage returns.
            continue;
        } else {
            result.push(ch);
        }
    }
    result
}

/// Parse the cleaned `/usage` text to extract session/weekly usage percentages.
/// NOTE: Because TUI PTY output has ANSI cursor-movement sequences, when
/// stripped the words often run together (e.g. "Currentsession38%used").
/// We handle both spaced and compact forms.
fn parse_claude_usage_text(text: &str) -> Result<OfficialClaudeSnapshot> {
    // Step 1: Trim to latest usage panel (like CodexBar's trimToLatestUsagePanel).
    // Find the last "Settings:" header containing "Usage" to skip startup fragments.
    let panel_text = trim_to_latest_usage_panel(text).unwrap_or(text);

    // Step 2: Build line-based search context.
    // Normalize each line by collapsing whitespace to single space (CodexBar approach).
    let lines: Vec<&str> = panel_text.lines().collect();
    let normalized_lines: Vec<String> = lines
        .iter()
        .map(|l| normalize_for_label_search(l))
        .collect();

    // Compact form for quick label existence checks.
    let compact: String = panel_text
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    // Step 3: Extract percentages with label-based line search.
    let mut session_pct = extract_pct_by_label("current session", &lines, &normalized_lines);
    let mut weekly_pct = extract_pct_by_label("current week", &lines, &normalized_lines);

    // Fallback: ordered percent scraping (CodexBar does this when labels match
    // but surrounding layout moved the percentages).
    let has_weekly_label = compact.contains("currentweek");
    if session_pct.is_none() || (has_weekly_label && weekly_pct.is_none()) {
        let ordered = all_percents_from_lines(&lines);
        if session_pct.is_none() {
            session_pct = ordered.first().copied();
        }
        if has_weekly_label && weekly_pct.is_none() {
            weekly_pct = ordered.get(1).copied();
        }
    }

    if session_pct.is_none() {
        bail!("Could not find 'Current session' usage in Claude CLI output");
    }

    // Step 4: Extract reset times.
    let session_reset = extract_reset_by_label("current session", &lines, &normalized_lines);
    let weekly_reset = if has_weekly_label {
        extract_reset_by_label("current week", &lines, &normalized_lines)
    } else {
        None
    };

    // Step 5: Extract plan type.
    let plan_type = extract_claude_plan_from_compact(&compact);

    Ok(OfficialClaudeSnapshot {
        plan_type,
        primary_used_percent: session_pct,
        secondary_used_percent: weekly_pct,
        primary_window_mins: Some(5 * 60),
        secondary_window_mins: Some(7 * 24 * 60),
        primary_resets_at: session_reset
            .as_deref()
            .and_then(parse_claude_reset_to_unix),
        secondary_resets_at: weekly_reset
            .as_deref()
            .and_then(parse_claude_reset_to_unix),
    })
}

/// Trim to the latest "Settings: ... Usage ..." panel in the output.
/// This skips startup fragments (status bar, logo, etc.) that may contain stray percent values.
fn trim_to_latest_usage_panel(text: &str) -> Option<&str> {
    // Find the last "Settings:" header.
    let settings_pos = text.to_ascii_lowercase().rfind("settings:")?;
    let tail = &text[settings_pos..];
    let tail_lower = tail.to_ascii_lowercase();
    // Must contain "Usage" tab indicator.
    if !tail_lower.contains("usage") {
        return None;
    }
    // Must have percent values with usage keywords, or "loading usage".
    let has_percent = tail_lower.contains('%');
    let has_usage_words = tail_lower.contains("used")
        || tail_lower.contains("left")
        || tail_lower.contains("remaining")
        || tail_lower.contains("available");
    let has_loading = tail_lower.contains("loading usage");
    if (has_percent && has_usage_words) || has_loading {
        Some(tail)
    } else {
        None
    }
}

/// Normalize text for label search: lowercase, collapse whitespace to single space.
fn normalize_for_label_search(text: &str) -> String {
    text.to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extract percent "remaining" from lines near a label.
/// Returns the "used" percent (if text says "XX% used", returns XX;
/// if text says "XX% remaining/left", returns 100-XX).
fn extract_pct_by_label(label: &str, lines: &[&str], normalized: &[String]) -> Option<f64> {
    let norm_label = normalize_for_label_search(label);
    for (idx, norm_line) in normalized.iter().enumerate() {
        if !norm_line.contains(&norm_label) {
            continue;
        }
        // Scan up to 12 lines from the label for a percent value.
        for candidate in lines.iter().skip(idx).take(12) {
            if let Some((pct, is_used)) = percent_from_line(candidate) {
                return if is_used {
                    Some(pct)
                } else {
                    Some(100.0 - pct)
                };
            }
        }
    }
    None
}

/// Extract a percent value from a single line.
/// Returns (value, is_used_not_remaining).
/// Skips status-bar context lines (containing | with model names).
fn percent_from_line(line: &str) -> Option<(f64, bool)> {
    // Skip status-bar context lines (e.g. "opus | 0% | sonnet").
    if line.contains('|') {
        let lower = line.to_ascii_lowercase();
        if ["opus", "sonnet", "haiku", "default"]
            .iter()
            .any(|m| lower.contains(m))
        {
            return None;
        }
    }

    // Find XX% pattern (allow Unicode whitespace before %).
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            // Skip optional whitespace/NBSP before %.
            let mut j = i;
            while j < bytes.len() {
                if bytes[j] == b' ' {
                    j += 1;
                } else if j + 1 < bytes.len() && bytes[j] == 0xc2 && bytes[j + 1] == 0xa0 {
                    j += 2; // U+00A0 NBSP
                } else {
                    break;
                }
            }
            if j < bytes.len() && bytes[j] == b'%' {
                let num_str = &line[start..i];
                if let Ok(val) = num_str.parse::<f64>() {
                    let clamped = val.clamp(0.0, 100.0);
                    let lower = line.to_ascii_lowercase();
                    let is_used = if ["used", "spent", "consumed"]
                        .iter()
                        .any(|k| lower.contains(k))
                    {
                        true
                    } else if ["remaining", "left", "available"]
                        .iter()
                        .any(|k| lower.contains(k))
                    {
                        false
                    } else {
                        // Default: Claude CLI shows "XX% used".
                        true
                    };
                    return Some((clamped, is_used));
                }
            }
        }
        i += 1;
    }
    None
}

/// Collect all percent values from the text (ordered, for fallback).
fn all_percents_from_lines(lines: &[&str]) -> Vec<f64> {
    lines
        .iter()
        .filter_map(|l| percent_from_line(l).map(|(pct, is_used)| if is_used { pct } else { 100.0 - pct }))
        .collect()
}

/// Extract "Resets ..." text near a label using line-based normalized search.
fn extract_reset_by_label(label: &str, lines: &[&str], normalized: &[String]) -> Option<String> {
    let norm_label = normalize_for_label_search(label);
    for (idx, norm_line) in normalized.iter().enumerate() {
        if !norm_line.contains(&norm_label) {
            continue;
        }
        // Scan up to 14 lines from the label for "Resets".
        for scan_line in lines.iter().skip(idx).take(14) {
            let scan_norm = normalize_for_label_search(scan_line);
            // Stop if we hit another "current" section.
            if scan_norm.starts_with("current ") && !scan_norm.contains(&norm_label) {
                break;
            }
            if scan_norm.contains("reset") || scan_norm.contains("reses") {
                if let Some(time_str) = extract_time_string(scan_line) {
                    return Some(time_str);
                }
            }
        }
    }
    None
}

/// Extract a time-like string from a line (e.g. "4:59pm", "2pm", "Mar 5, 3pm").
fn extract_time_string(line: &str) -> Option<String> {
    // Find patterns like "HH:MMam/pm" or "Ham/pm" in the raw text.
    let lower = line.to_ascii_lowercase();

    // Look for "H:MMam/pm" pattern.
    for (i, _) in lower.char_indices() {
        let rest = &lower[i..];
        if rest.starts_with(|c: char| c.is_ascii_digit()) {
            // Try to match time pattern.
            let mut end = i;
            // Digits.
            while end < lower.len() && lower.as_bytes()[end].is_ascii_digit() {
                end += 1;
            }
            // Optional colon + digits.
            if end < lower.len() && lower.as_bytes()[end] == b':' {
                end += 1;
                while end < lower.len() && lower.as_bytes()[end].is_ascii_digit() {
                    end += 1;
                }
            }
            // am/pm.
            let after = &lower[end..];
            if after.starts_with("am") || after.starts_with("pm") {
                let time_part = &line[i..end + 2];
                // Check for timezone in parens after.
                let remaining = line[end + 2..].trim();
                if remaining.starts_with('(') {
                    if let Some(close) = remaining.find(')') {
                        return Some(format!("{} {}", time_part, &remaining[..=close]));
                    }
                }
                return Some(time_part.to_string());
            }
        }
    }
    None
}

/// Try to parse "4:59pm (Asia/Shanghai)" or "2pm (Asia/Shanghai)" into a Unix timestamp.
fn parse_claude_reset_to_unix(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    // Extract timezone if present in parens.
    let (time_part, tz_str) = if let Some(paren_start) = text.find('(') {
        let tz = text[paren_start + 1..]
            .trim_end_matches(')')
            .trim()
            .to_string();
        (text[..paren_start].trim(), Some(tz))
    } else {
        (text, None)
    };

    let now = Local::now();

    // Try parsing with timezone.
    let tz: Option<Tz> = tz_str.as_deref().and_then(|s| s.parse::<Tz>().ok());

    // Try time-only formats: "4:59pm", "4:59PM", "16:59", "2pm".
    for fmt in &["%I:%M%p", "%I:%M%P", "%I:%M %p", "%H:%M", "%I%p", "%I%P", "%I %p"] {
        if let Ok(time) = chrono::NaiveTime::parse_from_str(time_part, fmt) {
            let today = now.date_naive();
            let dt = today.and_time(time);
            let ts = if let Some(tz) = tz {
                tz.from_local_datetime(&dt).single()?.timestamp()
            } else {
                now.timezone().from_local_datetime(&dt).single()?.timestamp()
            };
            if ts <= now.timestamp() {
                return Some(ts + 86400);
            }
            return Some(ts);
        }
    }

    // Try date+time formats.
    for fmt in &[
        "%b %d, %I:%M%p",
        "%b %d, %I:%M %p",
        "%b %d %I:%M%p",
        "%b %d, %H:%M",
    ] {
        let with_year = format!("{} {}", now.format("%Y"), time_part);
        let fmt_with_year = format!("%Y {fmt}");
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&with_year, &fmt_with_year) {
            let ts = if let Some(tz) = tz {
                tz.from_local_datetime(&dt).single()?.timestamp()
            } else {
                now.timezone().from_local_datetime(&dt).single()?.timestamp()
            };
            return Some(ts);
        }
    }

    None
}

/// Extract Claude plan name from compact text.
fn extract_claude_plan_from_compact(compact: &str) -> Option<String> {
    for (pattern, label) in &[
        ("claudemax", "Claude Max"),
        ("claudepro", "Claude Pro"),
        ("claudeteam", "Claude Team"),
        ("claudeenterprise", "Claude Enterprise"),
    ] {
        if compact.contains(*pattern) {
            return Some(label.to_string());
        }
    }
    None
}

fn load_claude_oauth_tokens() -> Result<(PathBuf, ClaudeOAuthTokens)> {
    let path = dirs::home_dir()
        .map(|home| home.join(".claude").join(".credentials.json"))
        .context("Failed to resolve Claude credentials path")?;
    let body = std::fs::read(&path)
        .with_context(|| format!("Failed to read Claude credentials file: {}", path.display()))?;
    let parsed: ClaudeCredentialsFile =
        serde_json::from_slice(&body).context("Invalid Claude .credentials.json format")?;
    let Some(tokens) = parsed.claude_ai_oauth else {
        bail!("Claude credentials missing claudeAiOauth payload");
    };
    let access_token = tokens
        .access_token
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if access_token.is_empty() {
        bail!("Claude credentials missing access token");
    }
    Ok((
        path,
        ClaudeOAuthTokens {
            access_token: Some(access_token.to_string()),
            ..tokens
        },
    ))
}

fn save_claude_oauth_tokens(path: &Path, tokens: &ClaudeOAuthTokens) -> Result<()> {
    let existing = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let mut root = existing;
    if !root.is_object() {
        root = serde_json::json!({});
    }
    let obj = root
        .as_object_mut()
        .context("Claude credentials root must be object")?;
    let oauth_value = obj
        .entry("claudeAiOauth".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !oauth_value.is_object() {
        *oauth_value = serde_json::json!({});
    }
    let oauth = oauth_value
        .as_object_mut()
        .context("claudeAiOauth must be object")?;

    if let Some(access) = tokens.access_token.as_ref().filter(|v| !v.is_empty()) {
        oauth.insert(
            "accessToken".to_string(),
            serde_json::Value::String(access.clone()),
        );
    }
    if let Some(refresh) = tokens.refresh_token.as_ref().filter(|v| !v.is_empty()) {
        oauth.insert(
            "refreshToken".to_string(),
            serde_json::Value::String(refresh.clone()),
        );
    }
    if let Some(expires_at) = tokens.expires_at {
        if let Some(number) = serde_json::Number::from_f64(expires_at) {
            oauth.insert("expiresAt".to_string(), serde_json::Value::Number(number));
        }
    }
    if let Some(tier) = tokens.rate_limit_tier.as_ref().filter(|v| !v.is_empty()) {
        oauth.insert(
            "rateLimitTier".to_string(),
            serde_json::Value::String(tier.clone()),
        );
    }

    let bytes =
        serde_json::to_vec_pretty(&root).context("Failed to serialize Claude credentials file")?;
    std::fs::write(path, bytes).with_context(|| {
        format!(
            "Failed to write Claude credentials file: {}",
            path.display()
        )
    })?;
    Ok(())
}

async fn fetch_claude_usage_with_access_token(
    access_token: &str,
    rate_limit_tier: Option<&str>,
) -> std::result::Result<OfficialClaudeSnapshot, ClaudeOAuthFetchError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| ClaudeOAuthFetchError::Other(anyhow::Error::new(error)))?;
    let response = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "tokenusage")
        .send()
        .await
        .map_err(|error| ClaudeOAuthFetchError::Other(anyhow::Error::new(error)))?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(ClaudeOAuthFetchError::Unauthorized);
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ClaudeOAuthFetchError::Other(anyhow::anyhow!(
            "Claude usage API returned {status}: {body}"
        )));
    }

    let usage: ClaudeOAuthUsageResponse = response
        .json()
        .await
        .map_err(|error| ClaudeOAuthFetchError::Other(anyhow::Error::new(error)))?;

    let weekly = usage
        .seven_day
        .or(usage.seven_day_oauth_apps)
        .or(usage.seven_day_opus)
        .or(usage.seven_day_sonnet);
    Ok(OfficialClaudeSnapshot {
        plan_type: infer_claude_plan_label(rate_limit_tier),
        primary_used_percent: usage
            .five_hour
            .as_ref()
            .and_then(|window| window.utilization)
            .map(normalize_official_used_percent),
        secondary_used_percent: weekly
            .as_ref()
            .and_then(|window| window.utilization)
            .map(normalize_official_used_percent),
        primary_window_mins: Some(5 * 60),
        secondary_window_mins: Some(7 * 24 * 60),
        primary_resets_at: usage
            .five_hour
            .as_ref()
            .and_then(|window| parse_iso8601_to_unix(window.resets_at.as_deref())),
        secondary_resets_at: weekly
            .as_ref()
            .and_then(|window| parse_iso8601_to_unix(window.resets_at.as_deref())),
    })
}

fn infer_claude_plan_label(rate_limit_tier: Option<&str>) -> Option<String> {
    let tier = rate_limit_tier?.trim();
    if tier.is_empty() {
        return None;
    }
    let normalized = tier.to_ascii_lowercase();
    let label = if normalized.contains("enterprise") {
        "Claude Enterprise"
    } else if normalized.contains("team") {
        "Claude Team"
    } else if normalized.contains("max") {
        "Claude Max"
    } else if normalized.contains("pro") {
        "Claude Pro"
    } else {
        tier
    };
    Some(label.to_string())
}

fn normalize_official_used_percent(raw: f64) -> f64 {
    if raw < 1.0 {
        (raw * 100.0).clamp(0.0, 100.0)
    } else {
        raw.clamp(0.0, 100.0)
    }
}

fn parse_iso8601_to_unix(raw: Option<&str>) -> Option<i64> {
    let text = raw?.trim();
    if text.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(text)
        .ok()
        .or_else(|| DateTime::parse_from_str(text, "%+").ok())
        .map(|ts| ts.timestamp())
}

async fn refresh_claude_access_token(
    refresh_token: &str,
) -> Result<(String, Option<String>, Option<i64>)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client for Claude refresh")?;
    let response = client
        .post(CLAUDE_OAUTH_REFRESH_URL)
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLAUDE_OAUTH_REFRESH_CLIENT_ID),
        ])
        .send()
        .await
        .context("Claude refresh request failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Claude refresh failed ({status}): {body}");
    }
    let payload: ClaudeRefreshResponse = response
        .json()
        .await
        .context("Invalid Claude refresh response")?;
    let expires_at_ms = payload
        .expires_in
        .map(|seconds| (Utc::now().timestamp() + seconds).saturating_mul(1000));
    Ok((payload.access_token, payload.refresh_token, expires_at_ms))
}

// ---------------------------------------------------------------------------
// Antigravity local probe — detect language_server_macos process, find port,
// query the Connect-protocol gRPC endpoints for quota data.
// ---------------------------------------------------------------------------

async fn fetch_antigravity_official_limits() -> Result<OfficialAntigravitySnapshot> {
    let (pid, csrf_token, extension_port) = detect_antigravity_process().await?;
    let ports = antigravity_listening_ports(pid).await?;
    let connect_port = antigravity_find_working_port(&ports, &csrf_token).await?;
    let ctx = AntigravityRequestContext {
        https_port: connect_port,
        http_port: extension_port,
        csrf_token,
    };

    let snapshot = match antigravity_fetch_user_status(&ctx).await {
        Ok(snap) => snap,
        Err(_) => antigravity_fetch_command_model_configs(&ctx).await?,
    };
    Ok(snapshot)
}

struct AntigravityRequestContext {
    https_port: u16,
    http_port: Option<u16>,
    csrf_token: String,
}

async fn detect_antigravity_process() -> Result<(u32, String, Option<u16>)> {
    let output = tokio::task::spawn_blocking(|| {
        Command::new("/bin/ps")
            .args(["-ax", "-o", "pid=,command="])
            .output()
            .context("failed to run ps for antigravity detection")
    })
    .await
    .context("ps task join failed")??;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (pid_str, cmd) = match trimmed.split_once(|c: char| c.is_whitespace()) {
            Some((p, c)) => (p.trim(), c),
            None => continue,
        };
        let lower = cmd.to_ascii_lowercase();
        if !lower.contains("language_server_macos") {
            continue;
        }
        let is_antigravity = (lower.contains("--app_data_dir") && lower.contains("antigravity"))
            || lower.contains("/antigravity/");
        if !is_antigravity {
            continue;
        }
        let pid: u32 = match pid_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let csrf = match extract_flag_value("--csrf_token", cmd) {
            Some(v) => v,
            None => continue,
        };
        let ext_port = extract_flag_value("--extension_server_port", cmd)
            .and_then(|v| v.parse::<u16>().ok());
        return Ok((pid, csrf, ext_port));
    }
    bail!("Antigravity language server not detected")
}

fn extract_flag_value(flag: &str, command: &str) -> Option<String> {
    let idx = command.find(flag)?;
    let after = &command[idx + flag.len()..];
    let after = after.strip_prefix('=').unwrap_or_else(|| after.trim_start());
    let value = after.split_whitespace().next()?;
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

async fn antigravity_listening_ports(pid: u32) -> Result<Vec<u16>> {
    let lsof = ["/usr/sbin/lsof", "/usr/bin/lsof"]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .context("lsof not available for antigravity port detection")?;

    let lsof = lsof.to_string();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(&lsof)
            .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-a", "-p", &pid.to_string()])
            .output()
            .context("failed to run lsof for antigravity")
    })
    .await
    .context("lsof task join failed")??;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut ports = std::collections::BTreeSet::new();
    for line in stdout.lines() {
        // Match lines like: ... TCP *:42150 (LISTEN)
        if let Some(listen_idx) = line.find("(LISTEN)") {
            let before = &line[..listen_idx];
            if let Some(colon_idx) = before.rfind(':') {
                let port_str = before[colon_idx + 1..].trim();
                if let Ok(port) = port_str.parse::<u16>() {
                    ports.insert(port);
                }
            }
        }
    }

    if ports.is_empty() {
        bail!("Antigravity process has no listening ports");
    }
    Ok(ports.into_iter().collect())
}

async fn antigravity_find_working_port(ports: &[u16], csrf_token: &str) -> Result<u16> {
    let unleash_body = serde_json::json!({
        "context": {
            "properties": {
                "devMode": "false",
                "extensionVersion": "unknown",
                "hasAnthropicModelAccess": "true",
                "ide": "antigravity",
                "ideVersion": "unknown",
                "installationId": "tokenusage",
                "language": "UNSPECIFIED",
                "os": "macos",
                "requestedModelId": "MODEL_UNSPECIFIED",
            }
        }
    });
    let path = "/exa.language_server_pb.LanguageServerService/GetUnleashData";

    for &port in ports {
        let url = format!("https://127.0.0.1:{port}{path}");
        let result = antigravity_post(&url, csrf_token, &unleash_body).await;
        if result.is_ok() {
            return Ok(port);
        }
    }
    bail!("no working Antigravity API port found among {} candidates", ports.len())
}

fn antigravity_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .no_proxy()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .context("failed to build HTTP client for antigravity")
}

async fn antigravity_post(
    url: &str,
    csrf_token: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    let client = antigravity_http_client()?;
    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .header("X-Codeium-Csrf-Token", csrf_token)
        .json(body)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        bail!("Antigravity HTTP {status}: {text}");
    }
    response
        .json()
        .await
        .context("invalid JSON from antigravity")
}

async fn antigravity_post_with_http_fallback(
    ctx: &AntigravityRequestContext,
    path: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    let https_url = format!("https://127.0.0.1:{}{path}", ctx.https_port);
    match antigravity_post(&https_url, &ctx.csrf_token, body).await {
        Ok(v) => Ok(v),
        Err(https_err) => {
            if let Some(http_port) = ctx.http_port {
                if http_port != ctx.https_port {
                    let http_url = format!("http://127.0.0.1:{http_port}{path}");
                    return antigravity_post(&http_url, &ctx.csrf_token, body).await;
                }
            }
            Err(https_err)
        }
    }
}

fn antigravity_default_request_body() -> serde_json::Value {
    serde_json::json!({
        "metadata": {
            "ideName": "antigravity",
            "extensionName": "antigravity",
            "ideVersion": "unknown",
            "locale": "en",
        }
    })
}

async fn antigravity_fetch_user_status(
    ctx: &AntigravityRequestContext,
) -> Result<OfficialAntigravitySnapshot> {
    let path = "/exa.language_server_pb.LanguageServerService/GetUserStatus";
    let body = antigravity_default_request_body();
    let resp = antigravity_post_with_http_fallback(ctx, path, &body).await?;
    parse_antigravity_user_status(&resp)
}

async fn antigravity_fetch_command_model_configs(
    ctx: &AntigravityRequestContext,
) -> Result<OfficialAntigravitySnapshot> {
    let path = "/exa.language_server_pb.LanguageServerService/GetCommandModelConfigs";
    let body = antigravity_default_request_body();
    let resp = antigravity_post_with_http_fallback(ctx, path, &body).await?;
    parse_antigravity_command_model_configs(&resp)
}

fn parse_antigravity_user_status(
    resp: &serde_json::Value,
) -> Result<OfficialAntigravitySnapshot> {
    // Check for error code
    if let Some(code) = resp.get("code") {
        let is_ok = match code {
            serde_json::Value::Number(n) => n.as_i64() == Some(0),
            serde_json::Value::String(s) => {
                let l = s.to_ascii_lowercase();
                l == "ok" || l == "success" || l == "0"
            }
            _ => true,
        };
        if !is_ok {
            bail!("Antigravity API error code: {code}");
        }
    }

    let user_status = resp
        .get("userStatus")
        .context("Antigravity response missing userStatus")?;

    let email = user_status
        .get("email")
        .and_then(|v| v.as_str())
        .map(String::from);

    let plan_name = user_status
        .pointer("/planStatus/planInfo")
        .and_then(|info| {
            let candidates = [
                "planDisplayName",
                "displayName",
                "productName",
                "planName",
                "planShortName",
            ];
            candidates
                .iter()
                .filter_map(|key| info.get(key)?.as_str())
                .find(|s| !s.trim().is_empty())
                .map(String::from)
        });

    let model_configs = user_status
        .pointer("/cascadeModelConfigData/clientModelConfigs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let models = parse_antigravity_model_quotas(&model_configs);
    Ok(build_antigravity_snapshot(plan_name, email, models))
}

fn parse_antigravity_command_model_configs(
    resp: &serde_json::Value,
) -> Result<OfficialAntigravitySnapshot> {
    if let Some(code) = resp.get("code") {
        let is_ok = match code {
            serde_json::Value::Number(n) => n.as_i64() == Some(0),
            serde_json::Value::String(s) => {
                let l = s.to_ascii_lowercase();
                l == "ok" || l == "success" || l == "0"
            }
            _ => true,
        };
        if !is_ok {
            bail!("Antigravity API error code: {code}");
        }
    }

    let model_configs = resp
        .get("clientModelConfigs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let models = parse_antigravity_model_quotas(&model_configs);
    Ok(build_antigravity_snapshot(None, None, models))
}

fn parse_antigravity_model_quotas(
    configs: &[serde_json::Value],
) -> Vec<AntigravityModelQuotaSnapshot> {
    configs
        .iter()
        .filter_map(|config| {
            let label = config.get("label")?.as_str()?.to_string();
            let model_id = config
                .pointer("/modelOrAlias/model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let quota = config.get("quotaInfo")?;
            let remaining_fraction = quota.get("remainingFraction").and_then(|v| v.as_f64());
            let reset_time = quota
                .get("resetTime")
                .and_then(|v| v.as_str())
                .and_then(parse_antigravity_reset_time);
            Some(AntigravityModelQuotaSnapshot {
                label,
                model_id,
                remaining_fraction,
                reset_time,
            })
        })
        .collect()
}

fn parse_antigravity_reset_time(value: &str) -> Option<i64> {
    // Try as ISO 8601 first
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(dt.timestamp());
    }
    // Try as seconds since epoch
    if let Ok(secs) = value.parse::<f64>() {
        return Some(secs as i64);
    }
    None
}

fn build_antigravity_snapshot(
    plan_type: Option<String>,
    account_email: Option<String>,
    models: Vec<AntigravityModelQuotaSnapshot>,
) -> OfficialAntigravitySnapshot {
    let ordered = select_antigravity_models(&models);

    let (primary_used, primary_label, primary_resets) = ordered
        .first()
        .map(|m| {
            let used = m.remaining_fraction.map(|f| (1.0 - f) * 100.0);
            (used, Some(m.label.clone()), m.reset_time)
        })
        .unwrap_or((None, None, None));

    let (secondary_used, secondary_label, secondary_resets) = ordered
        .get(1)
        .map(|m| {
            let used = m.remaining_fraction.map(|f| (1.0 - f) * 100.0);
            (used, Some(m.label.clone()), m.reset_time)
        })
        .unwrap_or((None, None, None));

    let (tertiary_used, tertiary_label, tertiary_resets) = ordered
        .get(2)
        .map(|m| {
            let used = m.remaining_fraction.map(|f| (1.0 - f) * 100.0);
            (used, Some(m.label.clone()), m.reset_time)
        })
        .unwrap_or((None, None, None));

    OfficialAntigravitySnapshot {
        plan_type,
        account_email,
        models,
        primary_used_percent: primary_used,
        secondary_used_percent: secondary_used,
        tertiary_used_percent: tertiary_used,
        primary_label,
        secondary_label,
        tertiary_label,
        primary_resets_at: primary_resets,
        secondary_resets_at: secondary_resets,
        tertiary_resets_at: tertiary_resets,
    }
}

/// Select and prioritise models the same way CodexBar does:
/// 1. Claude (non-thinking)  2. Gemini Pro Low  3. Gemini Flash
/// Fallback: all models sorted by remaining % ascending.
fn select_antigravity_models(
    models: &[AntigravityModelQuotaSnapshot],
) -> Vec<AntigravityModelQuotaSnapshot> {
    let mut ordered = Vec::new();

    if let Some(m) = models.iter().find(|m| {
        let l = m.label.to_ascii_lowercase();
        l.contains("claude") && !l.contains("thinking")
    }) {
        ordered.push(m.clone());
    }
    if let Some(m) = models.iter().find(|m| {
        let l = m.label.to_ascii_lowercase();
        l.contains("pro") && l.contains("low")
    }) {
        if !ordered.iter().any(|o| o.label == m.label) {
            ordered.push(m.clone());
        }
    }
    if let Some(m) = models.iter().find(|m| {
        let l = m.label.to_ascii_lowercase();
        l.contains("gemini") && l.contains("flash")
    }) {
        if !ordered.iter().any(|o| o.label == m.label) {
            ordered.push(m.clone());
        }
    }

    if ordered.is_empty() {
        let mut all: Vec<_> = models.to_vec();
        all.sort_by(|a, b| {
            let ra = a.remaining_fraction.unwrap_or(0.0);
            let rb = b.remaining_fraction.unwrap_or(0.0);
            ra.partial_cmp(&rb).unwrap_or(std::cmp::Ordering::Equal)
        });
        return all;
    }
    ordered
}

fn fetch_codex_official_limits_blocking() -> Result<OfficialCodexSnapshot> {
    let rpc_script = r#"(
exec </dev/null;
printf '{"id":1,"method":"initialize","params":{"clientInfo":{"name":"tu","version":"official-limits"}}}\n';
sleep 0.3;
printf '{"method":"initialized","params":{}}\n';
sleep 0.3;
printf '{"id":2,"method":"account/rateLimits/read","params":{}}\n';
sleep 1.2;
printf '{"id":4,"method":"account/rateLimits/read","params":{}}\n';
sleep 1.2;
printf '{"id":3,"method":"account/read","params":{}}\n';
sleep 1.8;
) | script -q /dev/null codex -s read-only -a untrusted app-server"#;

    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 0..3 {
        let output = match Command::new("/bin/sh")
            .arg("-lc")
            .arg(rpc_script)
            .output()
            .context("failed to run codex app-server probe via script")
        {
            Ok(output) => output,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };

        let mut raw = String::new();
        raw.push_str(&String::from_utf8_lossy(&output.stdout));
        raw.push('\n');
        raw.push_str(&String::from_utf8_lossy(&output.stderr));

        match parse_codex_official_snapshot(&raw) {
            Ok(snapshot) => return Ok(snapshot),
            Err(error) => {
                last_error = Some(error);
                if attempt < 2 {
                    thread::sleep(Duration::from_millis(250));
                }
            }
        }
    }

    if let Some(error) = last_error {
        return Err(error);
    }
    bail!("codex app-server probe failed with unknown error")
}

fn parse_codex_official_snapshot(raw: &str) -> Result<OfficialCodexSnapshot> {
    let mut rate_limits_value: Option<serde_json::Value> = None;
    let mut account_value: Option<serde_json::Value> = None;

    for chunk in extract_json_objects(raw) {
        let Ok(envelope) = serde_json::from_str::<RpcEnvelope>(&chunk) else {
            continue;
        };

        if let Some(error) = envelope.error {
            if let Some(message) = error.message {
                bail!("codex app-server error: {message}");
            }
            bail!("codex app-server returned error");
        }

        match rpc_envelope_id(&envelope) {
            Some(2) | Some(4) => rate_limits_value = envelope.result,
            Some(3) => account_value = envelope.result,
            _ => {}
        }
    }

    let rate_limits_value =
        rate_limits_value.context("codex app-server missing rateLimits response")?;
    let rate_limits: RpcRateLimitsReadResult = serde_json::from_value(rate_limits_value)
        .context("invalid account/rateLimits/read response")?;
    let account = account_value
        .and_then(|value| serde_json::from_value::<RpcAccountReadResult>(value).ok())
        .and_then(|res| res.account);

    let limits = rate_limits
        .rate_limits
        .context("rateLimits missing from Codex response")?;
    let primary_used_percent = limits
        .primary
        .as_ref()
        .and_then(|window| window.used_percent)
        .map(normalize_official_used_percent);
    let secondary_used_percent = limits
        .secondary
        .as_ref()
        .and_then(|window| window.used_percent)
        .map(normalize_official_used_percent);
    let primary_window_mins = limits
        .primary
        .as_ref()
        .and_then(|window| window.window_duration_mins);
    let secondary_window_mins = limits
        .secondary
        .as_ref()
        .and_then(|window| window.window_duration_mins);
    let primary_resets_at = limits.primary.as_ref().and_then(|window| window.resets_at);
    let secondary_resets_at = limits
        .secondary
        .as_ref()
        .and_then(|window| window.resets_at);

    Ok(OfficialCodexSnapshot {
        plan_type: limits
            .plan_type
            .or_else(|| account.and_then(|acc| acc.plan_type)),
        primary_used_percent,
        secondary_used_percent,
        primary_window_mins,
        secondary_window_mins,
        primary_resets_at,
        secondary_resets_at,
    })
}

fn extract_json_objects(raw: &str) -> Vec<String> {
    let mut objects = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;

    for ch in raw.chars() {
        if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
            continue;
        }

        if depth == 0 {
            if ch == '{' {
                current.clear();
                current.push(ch);
                depth = 1;
                in_string = false;
                escape = false;
            }
            continue;
        }

        current.push(ch);

        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    objects.push(current.clone());
                    current.clear();
                }
            }
            _ => {}
        }
    }

    objects
}

fn rpc_envelope_id(envelope: &RpcEnvelope) -> Option<i64> {
    envelope.id.as_ref().and_then(|id| {
        if let Some(v) = id.as_i64() {
            Some(v)
        } else {
            id.as_u64().map(|v| v as i64)
        }
    })
}

fn print_membership_estimate(
    estimate: &Option<MembershipEstimate>,
    token_limit: Option<u64>,
    token_limit_source: TokenLimitSource,
    official_codex: Option<&OfficialCodexSnapshot>,
    official_claude: Option<&OfficialClaudeSnapshot>,
    official_antigravity: Option<&OfficialAntigravitySnapshot>,
    tz: &TimeZoneMode,
) {
    if estimate.is_none()
        && official_codex.is_none()
        && official_claude.is_none()
        && official_antigravity.is_none()
    {
        return;
    }

    if let Some(estimate) = estimate {
        println!();
        println!(
            "estimate: plan={} | window_limit={} tokens | confidence={:.0}% | completed_blocks={} | observed_total={}",
            estimate.estimated_plan,
            format_u64(estimate.estimated_window_tokens),
            estimate.confidence * 100.0,
            estimate.completed_blocks,
            format_u64(estimate.observed_total_tokens),
        );
        println!(
            "estimate: peak_window={} | p95_window={} | limit_source={}",
            format_u64(estimate.observed_peak_window_tokens),
            format_u64(estimate.observed_p95_window_tokens),
            token_limit_source_label(token_limit_source),
        );

        if token_limit_source == TokenLimitSource::EstimatedFromLogs
            && let Some(limit) = token_limit
        {
            println!(
                "estimate: using inferred token limit {} (set --token-limit to override)",
                format_u64(limit)
            );
        }

        for source in &estimate.source_breakdown {
            println!(
                "estimate:{} plan={} | limit={} | confidence={:.0}% | blocks={} | total={}",
                source.source,
                source.estimated_plan,
                format_u64(source.estimated_window_tokens),
                source.confidence * 100.0,
                source.completed_blocks,
                format_u64(source.observed_total_tokens),
            );
        }
    }

    if let Some(official) = official_codex {
        println!();
        let plan = official.plan_type.as_deref().unwrap_or("unknown");
        println!("official: codex plan={plan}");
        if let Some(primary_used) = official.primary_used_percent {
            let details = official_window_details(
                official.primary_window_mins,
                official.primary_resets_at,
                tz,
            );
            let detail_text = if details.is_empty() {
                String::new()
            } else {
                format!(" {}", details.join(" "))
            };
            println!(
                "official: 5h used={:.1}% remaining={:.1}%{}",
                primary_used,
                (100.0 - primary_used).clamp(0.0, 100.0),
                detail_text
            );
        }
        if let Some(secondary_used) = official.secondary_used_percent {
            let details = official_window_details(
                official.secondary_window_mins,
                official.secondary_resets_at,
                tz,
            );
            let detail_text = if details.is_empty() {
                String::new()
            } else {
                format!(" {}", details.join(" "))
            };
            println!(
                "official: weekly used={:.1}% remaining={:.1}%{}",
                secondary_used,
                (100.0 - secondary_used).clamp(0.0, 100.0),
                detail_text
            );
        }
    }

    if let Some(official) = official_claude {
        println!();
        let plan = official.plan_type.as_deref().unwrap_or("unknown");
        println!("official: claude plan={plan}");
        if let Some(primary_used) = official.primary_used_percent {
            let details = official_window_details(
                official.primary_window_mins,
                official.primary_resets_at,
                tz,
            );
            let detail_text = if details.is_empty() {
                String::new()
            } else {
                format!(" {}", details.join(" "))
            };
            println!(
                "official: claude 5h used={:.1}% remaining={:.1}%{}",
                primary_used,
                (100.0 - primary_used).clamp(0.0, 100.0),
                detail_text
            );
        }
        if let Some(secondary_used) = official.secondary_used_percent {
            let details = official_window_details(
                official.secondary_window_mins,
                official.secondary_resets_at,
                tz,
            );
            let detail_text = if details.is_empty() {
                String::new()
            } else {
                format!(" {}", details.join(" "))
            };
            println!(
                "official: claude weekly used={:.1}% remaining={:.1}%{}",
                secondary_used,
                (100.0 - secondary_used).clamp(0.0, 100.0),
                detail_text
            );
        }
    }

    if let Some(official) = official_antigravity {
        println!();
        let plan = official.plan_type.as_deref().unwrap_or("unknown");
        if let Some(email) = official.account_email.as_deref() {
            println!("official: antigravity plan={plan} email={email}");
        } else {
            println!("official: antigravity plan={plan}");
        }
        let labels = [
            ("primary", official.primary_label.as_deref(), official.primary_used_percent, official.primary_resets_at),
            ("secondary", official.secondary_label.as_deref(), official.secondary_used_percent, official.secondary_resets_at),
            ("tertiary", official.tertiary_label.as_deref(), official.tertiary_used_percent, official.tertiary_resets_at),
        ];
        for (slot, label, used_opt, reset_opt) in labels {
            if let Some(used) = used_opt {
                let tag = label.unwrap_or(slot);
                let mut entry = format!(
                    "official: antigravity {tag} used={used:.1}% remaining={:.1}%",
                    (100.0 - used).clamp(0.0, 100.0)
                );
                if let Some(resets_at) = reset_opt {
                    let reset_text = format_reset_timestamp(resets_at, tz);
                    let eta_text = format_time_until_reset_short(resets_at, Utc::now());
                    entry.push_str(&format!(" (reset {reset_text}, in {eta_text})"));
                }
                println!("{entry}");
            }
        }
    }
}

fn token_limit_source_label(source: TokenLimitSource) -> &'static str {
    match source {
        TokenLimitSource::Explicit => "explicit",
        TokenLimitSource::HistoricalMax => "historical_max",
        TokenLimitSource::EstimatedFromLogs => "estimated_from_logs",
        TokenLimitSource::Unset => "unset",
    }
}

fn should_emit_json(common: &CommonArgs) -> bool {
    common.json || common.jq.is_some()
}

fn print_debug(stats: &ParseStats, common: &CommonArgs) {
    if !common.debug {
        return;
    }

    eprintln!(
        "debug: files={} open_failed={} lines_total={} parsed={} filtered={} invalid_json={} missing_usage={} unknown_pricing={}",
        stats.files_discovered,
        stats.files_open_failed,
        stats.lines_total,
        stats.lines_parsed,
        stats.lines_filtered,
        stats.lines_invalid_json,
        stats.lines_missing_usage,
        stats.lines_unknown_pricing,
    );
}

fn emit_json<T: Serialize>(value: &T, jq: Option<&str>) -> Result<()> {
    let pretty = serde_json::to_string_pretty(value)?;
    if let Some(query) = jq {
        let mut child = Command::new("jq")
            .arg(query)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("Failed to run jq. Please ensure jq is installed.")?;

        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(pretty.as_bytes())?;
        }

        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!("jq failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        print!("{}", String::from_utf8_lossy(&out.stdout));
        return Ok(());
    }

    println!("{pretty}");
    Ok(())
}

fn build_report_from_rows(rows: Vec<DailyRow>, stats: ParseStats) -> DailyReport {
    let totals = rows.iter().fold(TokenCounts::default(), |mut acc, row| {
        acc.add_assign(row.totals.clone());
        acc
    });

    DailyReport {
        daily: rows,
        totals,
        stats,
    }
}

fn build_group_rows<F>(events: &[UsageEvent], order: &SortOrder, mut key_fn: F) -> Vec<DailyRow>
where
    F: FnMut(&UsageEvent) -> String,
{
    let mut groups: HashMap<String, GroupAggregate> = HashMap::new();
    for event in events {
        let key = key_fn(event);
        groups.entry(key).or_default().add_event(event);
    }

    let mut rows = groups
        .into_iter()
        .map(|(key, agg)| DailyRow {
            date: key,
            totals: agg.totals.to_counts(),
            models: agg
                .by_model
                .into_iter()
                .map(|(model, totals)| (model, totals.to_counts()))
                .collect::<BTreeMap<_, _>>(),
            sources: agg
                .by_source
                .into_iter()
                .map(|(source, totals)| (source.as_str().to_string(), totals.to_counts()))
                .collect::<BTreeMap<_, _>>(),
        })
        .collect::<Vec<_>>();

    rows.sort_by(|a, b| a.date.cmp(&b.date));
    if *order == SortOrder::Desc {
        rows.reverse();
    }

    rows
}

fn parse_timezone_mode(input: Option<&str>) -> Result<TimeZoneMode> {
    let Some(raw) = input else {
        return Ok(TimeZoneMode::Local);
    };

    if raw.eq_ignore_ascii_case("local") {
        return Ok(TimeZoneMode::Local);
    }
    if raw.eq_ignore_ascii_case("utc") {
        return Ok(TimeZoneMode::Utc);
    }

    let tz = Tz::from_str(raw)
        .with_context(|| format!("Invalid timezone: {raw}. Use e.g. UTC or Asia/Tokyo"))?;
    Ok(TimeZoneMode::Named(tz))
}

fn local_date(ts: DateTime<Utc>, tz: &TimeZoneMode) -> NaiveDate {
    tz.date_of(ts)
}

fn format_display_datetime(ts: DateTime<Utc>, tz: &TimeZoneMode) -> String {
    match tz {
        TimeZoneMode::Local => ts
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        TimeZoneMode::Utc => ts.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        TimeZoneMode::Named(tz) => ts
            .with_timezone(tz)
            .format("%Y-%m-%d %H:%M:%S %Z")
            .to_string(),
    }
}

fn week_start(day: NaiveDate, start: WeekStart) -> NaiveDate {
    let week_start_num = match start {
        WeekStart::Sunday => 0,
        WeekStart::Monday => 1,
        WeekStart::Tuesday => 2,
        WeekStart::Wednesday => 3,
        WeekStart::Thursday => 4,
        WeekStart::Friday => 5,
        WeekStart::Saturday => 6,
    };

    let current = day.weekday().num_days_from_sunday() as i64;
    let target = week_start_num as i64;
    let diff = (7 + current - target) % 7;
    day - chrono::TimeDelta::days(diff)
}

fn shorten_session_id(session_id: &str) -> String {
    if session_id.chars().count() <= 24 {
        return session_id.to_string();
    }
    let tail = session_id
        .chars()
        .rev()
        .take(22)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("..{tail}")
}

fn format_u64(value: u64) -> String {
    let raw = value.to_string();
    let mut out = String::with_capacity(raw.len() + (raw.len() / 3));
    let total = raw.len();
    for (idx, ch) in raw.chars().enumerate() {
        out.push(ch);
        let remain = total.saturating_sub(idx + 1);
        if remain > 0 && remain.is_multiple_of(3) {
            out.push(',');
        }
    }
    out
}

fn format_usd(value: f64) -> String {
    format!("${value:.2}")
}

fn parse_common_filter(common: &CommonArgs) -> Result<DateFilter> {
    let filter = DateFilter {
        since: parse_date_filter(common.since.as_deref())?,
        until: parse_date_filter(common.until.as_deref())?,
    };
    if let (Some(since), Some(until)) = (filter.since, filter.until)
        && since > until
    {
        bail!("--since must be earlier than or equal to --until");
    }
    Ok(filter)
}

fn worker_count_from_common(common: &CommonArgs) -> usize {
    common.workers.unwrap_or_else(|| {
        thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    })
}

#[allow(clippy::too_many_arguments)]
fn parse_files_with_cache(
    files: &[DiscoveredFile],
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: Arc<PricingTable>,
    worker_count: usize,
    cache_enabled: bool,
    cache_store: &mut IncrementalCacheStore,
    sort_events: bool,
) -> ParsedUsageOutput {
    let stats = Arc::new(ParseStatsAtomic::default());
    stats.files_discovered.store(files.len(), Ordering::Relaxed);

    let mut cache_dirty = false;
    let mut seen_cache_keys = HashSet::with_capacity(files.len());
    let mut parse_jobs = Vec::new();
    let mut events = Vec::new();

    for file in files {
        let key = cache_file_key(&file.path);
        seen_cache_keys.insert(key.clone());

        let Some(fingerprint) = read_file_fingerprint(&file.path) else {
            parse_jobs.push(FileParseJob {
                file: file.clone(),
                cache_key: key,
                fingerprint: FileFingerprint {
                    size: 0,
                    modified_unix_secs: 0,
                    modified_unix_nanos: 0,
                },
                strategy: ParseStrategy::Full,
            });
            continue;
        };

        if cache_enabled && let Some(cached) = cache_store.files.get(&key) {
            if cached.fingerprint == fingerprint {
                events.extend(hydrate_cached_events(
                    file, cached, filter, timezone, &stats,
                ));
                continue;
            }
            if can_incremental_parse(cached, fingerprint) {
                parse_jobs.push(FileParseJob {
                    file: file.clone(),
                    cache_key: key,
                    fingerprint,
                    strategy: ParseStrategy::Incremental {
                        base_cache: cached.clone(),
                    },
                });
                continue;
            }
        }

        parse_jobs.push(FileParseJob {
            file: file.clone(),
            cache_key: key,
            fingerprint,
            strategy: ParseStrategy::Full,
        });
    }

    let parsed = parse_files_concurrently(
        parse_jobs,
        worker_count.max(1),
        filter,
        timezone.clone(),
        pricing,
        stats.clone(),
    );
    events.extend(parsed.events);

    if cache_enabled {
        for (key, entry) in parsed.cache_updates {
            cache_store.files.insert(key, entry);
            cache_dirty = true;
        }

        let before = cache_store.files.len();
        cache_store
            .files
            .retain(|path, _| seen_cache_keys.contains(path));
        if cache_store.files.len() != before {
            cache_dirty = true;
        }
    }

    if sort_events {
        events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    }

    ParsedUsageOutput {
        loaded: LoadedUsage {
            events,
            stats: stats.snapshot(),
        },
        cache_dirty,
    }
}

impl LiveUsageRuntime {
    async fn new(common: &CommonArgs, refresh_every: u64) -> Result<Self> {
        let filter = parse_common_filter(common)?;
        let sources = build_sources(common).await?;
        if sources.is_empty() {
            bail!(
                "No valid source directories found. Please provide --claude-projects-dir/--codex-sessions-dir."
            );
        }

        let pricing = Arc::new(load_pricing(common.pricing_file.as_deref(), common.offline).await?);
        let pricing_key = pricing_cache_key(&pricing);
        let ignore_rules = PathIgnoreRules::from_common(common);
        let worker_count = worker_count_from_common(common);

        let cache_enabled = !common.no_incremental_cache;
        let cache_path = incremental_cache_path();
        let mut cache_store = if cache_enabled {
            match cache_path.as_ref() {
                Some(path) => load_incremental_cache(path, &pricing_key),
                None => IncrementalCacheStore::new(pricing_key.clone()),
            }
        } else {
            IncrementalCacheStore::new(pricing_key.clone())
        };
        if common.rebuild_cache {
            cache_store = IncrementalCacheStore::new(pricing_key);
        }

        let files_cache = discover_files(&sources, &ignore_rules, filter);
        let now = Instant::now();
        let discovery_interval =
            Duration::from_secs((refresh_every.saturating_mul(3)).clamp(2, 12));

        Ok(Self {
            filter,
            sources,
            ignore_rules,
            pricing,
            worker_count,
            cache_enabled,
            cache_store,
            cache_path,
            cache_dirty: common.rebuild_cache,
            files_cache,
            last_discovery_at: now,
            discovery_interval,
            last_sources_refresh_at: now,
            sources_refresh_interval: Duration::from_secs(60),
            last_cache_flush_at: now,
        })
    }

    async fn maybe_refresh_sources(&mut self, common: &CommonArgs) -> Result<()> {
        if self.last_sources_refresh_at.elapsed() < self.sources_refresh_interval {
            return Ok(());
        }
        self.last_sources_refresh_at = Instant::now();
        let refreshed = build_sources(common).await?;
        if refreshed.is_empty() || refreshed == self.sources {
            return Ok(());
        }

        self.sources = refreshed;
        self.files_cache.clear();
        self.last_discovery_at = Instant::now() - self.discovery_interval;
        Ok(())
    }

    fn maybe_refresh_discovery(&mut self) {
        if !self.files_cache.is_empty()
            && self.last_discovery_at.elapsed() < self.discovery_interval
        {
            return;
        }
        self.files_cache = discover_files(&self.sources, &self.ignore_rules, self.filter);
        self.last_discovery_at = Instant::now();
    }

    fn load(&mut self, timezone: &TimeZoneMode) -> LoadedUsage {
        self.maybe_refresh_discovery();
        let parsed = parse_files_with_cache(
            &self.files_cache,
            self.filter,
            timezone,
            self.pricing.clone(),
            self.worker_count,
            self.cache_enabled,
            &mut self.cache_store,
            false,
        );
        self.cache_dirty |= parsed.cache_dirty;
        self.flush_cache(false);
        parsed.loaded
    }

    fn flush_cache(&mut self, force: bool) {
        if !self.cache_enabled || !self.cache_dirty {
            return;
        }
        if !force && self.last_cache_flush_at.elapsed() < Duration::from_secs(10) {
            return;
        }
        if let Some(path) = self.cache_path.as_ref() {
            save_incremental_cache(path, &self.cache_store);
            self.cache_dirty = false;
            self.last_cache_flush_at = Instant::now();
        }
    }
}

async fn load_usage(common: &CommonArgs, timezone: &TimeZoneMode) -> Result<LoadedUsage> {
    let filter = parse_common_filter(common)?;
    let sources = build_sources(common).await?;
    if sources.is_empty() {
        bail!(
            "No valid source directories found. Please provide --claude-projects-dir/--codex-sessions-dir."
        );
    }

    let pricing = Arc::new(load_pricing(common.pricing_file.as_deref(), common.offline).await?);
    let ignore_rules = PathIgnoreRules::from_common(common);
    let files = discover_files(&sources, &ignore_rules, filter);
    let worker_count = worker_count_from_common(common);
    let pricing_key = pricing_cache_key(&pricing);
    let cache_enabled = !common.no_incremental_cache;
    let cache_path = incremental_cache_path();

    let mut cache_store = if cache_enabled {
        match cache_path.as_ref() {
            Some(path) => load_incremental_cache(path, &pricing_key),
            None => IncrementalCacheStore::new(pricing_key.clone()),
        }
    } else {
        IncrementalCacheStore::new(pricing_key.clone())
    };
    if common.rebuild_cache {
        cache_store = IncrementalCacheStore::new(pricing_key);
    }

    let parsed = parse_files_with_cache(
        &files,
        filter,
        timezone,
        pricing,
        worker_count,
        cache_enabled,
        &mut cache_store,
        true,
    );
    if cache_enabled
        && (parsed.cache_dirty || common.rebuild_cache)
        && let Some(path) = cache_path.as_ref()
    {
        save_incremental_cache(path, &cache_store);
    }

    Ok(parsed.loaded)
}

async fn build_sources(common: &CommonArgs) -> Result<Vec<SourceConfig>> {
    let home = dirs::home_dir().context("Failed to resolve home directory")?;

    let mut sources = Vec::new();

    if !common.no_claude {
        let roots = if common.claude_projects_dir.is_empty() {
            vec![
                home.join(".config").join("claude").join("projects"),
                home.join(".claude").join("projects"),
            ]
        } else {
            common
                .claude_projects_dir
                .iter()
                .map(|p| expand_user_path(p))
                .collect()
        };

        let existing = filter_existing_dirs(roots).await;
        if !existing.is_empty() {
            sources.push(SourceConfig {
                kind: SourceKind::Claude,
                roots: existing,
            });
        }
    }

    if !common.no_codex {
        let roots = if common.codex_sessions_dir.is_empty() {
            vec![
                home.join(".codex").join("sessions"),
                home.join(".codex").join("archived_sessions"),
                home.join(".config").join("codex").join("sessions"),
                home.join(".config").join("codex").join("archived_sessions"),
            ]
        } else {
            let mut out = Vec::new();
            for raw in &common.codex_sessions_dir {
                let path = expand_user_path(raw);
                out.push(path.clone());
                if path.file_name().and_then(|s| s.to_str()) == Some("sessions")
                    && let Some(parent) = path.parent()
                {
                    out.push(parent.join("archived_sessions"));
                }
            }
            out
        };

        let existing = filter_existing_dirs(roots).await;
        if !existing.is_empty() {
            sources.push(SourceConfig {
                kind: SourceKind::Codex,
                roots: existing,
            });
        }
    }

    Ok(sources)
}

async fn filter_existing_dirs(input: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for path in input {
        let normalized = normalize_path(&path);
        if !seen.insert(normalized.clone()) {
            continue;
        }

        if let Ok(meta) = fs::metadata(&normalized).await
            && meta.is_dir()
        {
            out.push(normalized);
        }
    }

    out
}

fn discover_files(
    sources: &[SourceConfig],
    ignore_rules: &PathIgnoreRules,
    filter: DateFilter,
) -> Vec<DiscoveredFile> {
    let mut files: Vec<DiscoveredFile> = sources
        .par_iter()
        .flat_map_iter(|source| {
            source.roots.iter().flat_map(move |root| {
                discover_files_in_root(source.kind, root, ignore_rules, filter)
            })
        })
        .collect();

    files.par_sort_unstable_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    files
}

fn discover_files_in_root(
    kind: SourceKind,
    root: &Path,
    ignore_rules: &PathIgnoreRules,
    _filter: DateFilter,
) -> Vec<DiscoveredFile> {
    // NOTE:
    // Do not short-circuit Codex discovery by directory date partition.
    // A Codex session file can continue receiving events on later days while
    // staying under its original directory (session start date), so partition
    // pruning can miss valid events and undercount filtered ranges.
    let mut out = Vec::new();
    let rules = ignore_rules.clone();
    let mut builder = WalkBuilder::new(root);
    builder
        .follow_links(false)
        // keep hidden entries visible because source roots often start with '.'
        .hidden(false)
        .filter_entry(move |entry| entry.depth() == 0 || !rules.should_skip_dir(entry.path()));

    for entry in builder.build().filter_map(Result::ok) {
        let path = entry.path();
        let is_file = entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false);
        if !is_file || ignore_rules.should_skip_path(path) {
            continue;
        }

        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !ext.eq_ignore_ascii_case("jsonl") {
            continue;
        }

        out.push(DiscoveredFile {
            source: kind,
            root: root.to_path_buf(),
            path: normalized_discovered_path(path),
        });
    }

    out
}

fn parse_files_concurrently(
    files: Vec<FileParseJob>,
    workers: usize,
    filter: DateFilter,
    timezone: TimeZoneMode,
    pricing: Arc<PricingTable>,
    stats: Arc<ParseStatsAtomic>,
) -> WorkerParseOutput {
    let (tx, rx) = bounded::<FileParseJob>(4096);

    let producer = {
        let tx = tx.clone();
        thread::spawn(move || {
            for file in files {
                if tx.send(file).is_err() {
                    break;
                }
            }
        })
    };
    drop(tx);

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let rx = rx.clone();
        let pricing = pricing.clone();
        let stats = stats.clone();
        let timezone = timezone.clone();

        let handle = thread::spawn(move || worker_loop(rx, filter, &timezone, &pricing, &stats));
        handles.push(handle);
    }

    let _ = producer.join();
    let mut out = WorkerParseOutput::default();
    for handle in handles {
        if let Ok(mut worker) = handle.join() {
            out.events.append(&mut worker.events);
            out.cache_updates.append(&mut worker.cache_updates);
        }
    }
    out
}

fn worker_loop(
    rx: Receiver<FileParseJob>,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: &PricingTable,
    stats: &ParseStatsAtomic,
) -> WorkerParseOutput {
    let mut output = WorkerParseOutput::default();
    while let Ok(job) = rx.recv() {
        let cache_key = job.cache_key.clone();
        if let Some(parsed) = parse_single_file(job, filter, timezone, pricing, stats) {
            output.events.extend(parsed.events);
            output.cache_updates.push((cache_key, parsed.cache_entry));
        }
    }
    output
}

fn parse_single_file(
    job: FileParseJob,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: &PricingTable,
    stats: &ParseStatsAtomic,
) -> Option<ParsedFileOutput> {
    let input = match File::open(&job.file.path) {
        Ok(f) => f,
        Err(_) => {
            stats.files_open_failed.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };

    let (session, project) = derive_session_meta(&job.file);
    let file_path = job.file.path.display().to_string();

    let mut reader = BufReader::new(input);
    let mut codex_state = CodexParseState::default();
    let mut claude_state = ClaudeDedupeState::default();
    let mut local_events = Vec::new();
    let mut cached_events = Vec::new();
    let mut line = String::new();
    let mut lines_total = 0usize;
    let mut lines_invalid_json = 0usize;
    let mut lines_missing_usage = 0usize;
    let mut lines_unknown_pricing = 0usize;
    let mut lines_filtered = 0usize;
    let mut lines_parsed = 0usize;
    let mut base_stats = CachedFileStats::default();
    let mut parsed_offset = 0u64;

    if let ParseStrategy::Incremental { base_cache } = job.strategy {
        let fallback_offset = base_cache.fingerprint.size;
        let seek_offset = base_cache
            .parsed_offset
            .max(fallback_offset)
            .min(job.fingerprint.size);
        if seek_offset > 0 && reader.seek(SeekFrom::Start(seek_offset)).is_ok() {
            parsed_offset = seek_offset;
            base_stats = base_cache.stats.clone();
            cached_events.extend(base_cache.events.iter().cloned());
            local_events.extend(hydrate_cached_events(
                &job.file,
                &base_cache,
                filter,
                timezone,
                stats,
            ));
            match job.file.source {
                SourceKind::Codex => {
                    codex_state.current_model = base_cache.codex_last_model.clone();
                    codex_state.previous_totals = base_cache.codex_last_totals;
                }
                SourceKind::Claude => {
                    claude_state = ClaudeDedupeState::with_seed(base_cache.claude_recent_keys);
                }
            }
        } else {
            let _ = reader.seek(SeekFrom::Start(0));
        }
    }

    loop {
        line.clear();
        let bytes = match reader.read_line(&mut line) {
            Ok(n) => n,
            Err(_) => break,
        };
        if bytes == 0 {
            break;
        }
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }

        lines_total += 1;

        if should_skip_parse_by_line_prefix(job.file.source, &line) {
            lines_missing_usage += 1;
            continue;
        }

        let parsed = match job.file.source {
            SourceKind::Claude => parse_claude_usage_line(&line, pricing, &mut claude_state),
            SourceKind::Codex => parse_codex_usage_line(&line, &mut codex_state, pricing),
        };

        let mut parsed = match parsed {
            ParseLineResult::Parsed(parsed) => parsed,
            ParseLineResult::InvalidJson => {
                lines_invalid_json += 1;
                continue;
            }
            ParseLineResult::MissingUsage => {
                lines_missing_usage += 1;
                continue;
            }
        };

        if parsed.used_unknown_pricing {
            lines_unknown_pricing += 1;
        }

        cached_events.push(CachedUsageEvent {
            timestamp: parsed.event.timestamp,
            model: parsed.event.model.clone(),
            usage: parsed.event.usage,
        });

        let day = local_date(parsed.event.timestamp, timezone);
        if !filter.allows(day) {
            lines_filtered += 1;
            continue;
        }

        parsed.event.session = session.clone();
        parsed.event.project = project.clone();
        parsed.event.file_path = file_path.clone();

        local_events.push(parsed.event);
        lines_parsed += 1;
    }

    stats.lines_total.fetch_add(lines_total, Ordering::Relaxed);
    stats
        .lines_invalid_json
        .fetch_add(lines_invalid_json, Ordering::Relaxed);
    stats
        .lines_missing_usage
        .fetch_add(lines_missing_usage, Ordering::Relaxed);
    stats
        .lines_unknown_pricing
        .fetch_add(lines_unknown_pricing, Ordering::Relaxed);
    stats
        .lines_filtered
        .fetch_add(lines_filtered, Ordering::Relaxed);
    stats
        .lines_parsed
        .fetch_add(lines_parsed, Ordering::Relaxed);

    let cache_entry = CachedFileEntry {
        fingerprint: job.fingerprint,
        stats: CachedFileStats {
            lines_total: base_stats.lines_total + lines_total,
            lines_invalid_json: base_stats.lines_invalid_json + lines_invalid_json,
            lines_missing_usage: base_stats.lines_missing_usage + lines_missing_usage,
            lines_unknown_pricing: base_stats.lines_unknown_pricing + lines_unknown_pricing,
        },
        events: cached_events,
        parsed_offset: job.fingerprint.size.max(parsed_offset),
        codex_last_model: codex_state.current_model,
        codex_last_totals: codex_state.previous_totals,
        claude_recent_keys: claude_state.snapshot(),
    };

    Some(ParsedFileOutput {
        events: local_events,
        cache_entry,
    })
}

fn should_skip_parse_by_line_prefix(source: SourceKind, line: &str) -> bool {
    match source {
        SourceKind::Codex => {
            !(line.contains("\"type\":\"event_msg\"")
                || line.contains("\"type\": \"event_msg\"")
                || line.contains("\"type\":\"turn_context\"")
                || line.contains("\"type\": \"turn_context\""))
        }
        SourceKind::Claude => {
            !(line.contains("\"type\":\"assistant\"") || line.contains("\"type\": \"assistant\""))
        }
    }
}

fn derive_session_meta(file: &DiscoveredFile) -> (String, Option<String>) {
    let relative = file
        .path
        .strip_prefix(&file.root)
        .unwrap_or(file.path.as_path())
        .to_path_buf();

    let session = relative
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/");

    let project = relative
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .filter(|s| !s.is_empty() && s != ".");

    (session, project)
}

fn parse_claude_usage_line(
    line: &str,
    pricing: &PricingTable,
    dedupe_state: &mut ClaudeDedupeState,
) -> ParseLineResult {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return ParseLineResult::InvalidJson,
    };

    if get_value(&value, "type")
        .and_then(Value::as_str)
        .is_some_and(|entry_type| entry_type != "assistant")
    {
        return ParseLineResult::MissingUsage;
    }

    let Some(timestamp) = extract_timestamp(&value) else {
        return ParseLineResult::MissingUsage;
    };
    let Some(model) = extract_model(&value, SourceKind::Claude) else {
        return ParseLineResult::MissingUsage;
    };

    let message_id = get_value(&value, "message.id")
        .and_then(Value::as_str)
        .or_else(|| get_value(&value, "messageId").and_then(Value::as_str));
    let request_id = get_value(&value, "requestId")
        .and_then(Value::as_str)
        .or_else(|| get_value(&value, "request_id").and_then(Value::as_str));
    if let (Some(message_id), Some(request_id)) = (message_id, request_id) {
        let key = format!("{message_id}:{request_id}");
        if !dedupe_state.insert(key) {
            return ParseLineResult::MissingUsage;
        }
    }

    let usage = UsageAccumulator {
        input_tokens: extract_u64(
            &value,
            &[
                "message.usage.input_tokens",
                "usage.input_tokens",
                "usage.inputTokens",
                "input_tokens",
                "inputTokens",
            ],
        )
        .unwrap_or(0),
        cache_creation_input_tokens: extract_u64(
            &value,
            &[
                "message.usage.cache_creation_input_tokens",
                "usage.cache_creation_input_tokens",
                "usage.cacheCreationInputTokens",
                "cache_creation_input_tokens",
                "cacheCreationInputTokens",
            ],
        )
        .unwrap_or(0),
        cache_read_input_tokens: extract_u64(
            &value,
            &[
                "message.usage.cache_read_input_tokens",
                "usage.cache_read_input_tokens",
                "usage.cached_input_tokens",
                "usage.cachedInputTokens",
                "cache_read_input_tokens",
                "cached_input_tokens",
                "cachedInputTokens",
            ],
        )
        .unwrap_or(0),
        output_tokens: extract_u64(
            &value,
            &[
                "message.usage.output_tokens",
                "usage.output_tokens",
                "usage.outputTokens",
                "output_tokens",
                "outputTokens",
            ],
        )
        .unwrap_or(0),
        reasoning_output_tokens: extract_u64(
            &value,
            &[
                "message.usage.reasoning_output_tokens",
                "usage.reasoning_output_tokens",
                "usage.reasoningOutputTokens",
                "usage.output_tokens_details.reasoning_tokens",
                "output_tokens_details.reasoning_tokens",
            ],
        )
        .unwrap_or(0),
        cost_usd: 0.0,
    };

    let total_tokens = usage.total_tokens();
    if total_tokens == 0 {
        return ParseLineResult::MissingUsage;
    }

    let (cost_usd, used_unknown_pricing) = match pricing.estimate_cost(&model, usage) {
        Some(v) => (v, false),
        None => (0.0, true),
    };

    ParseLineResult::Parsed(ParsedLine {
        event: UsageEvent {
            timestamp,
            source: SourceKind::Claude,
            model,
            session: String::new(),
            project: None,
            file_path: String::new(),
            usage: UsageAccumulator { cost_usd, ..usage },
        },
        used_unknown_pricing,
    })
}

fn parse_codex_usage_line(
    line: &str,
    state: &mut CodexParseState,
    pricing: &PricingTable,
) -> ParseLineResult {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return ParseLineResult::InvalidJson,
    };

    let Some(entry_type) = get_value(&value, "type").and_then(|v| v.as_str()) else {
        return ParseLineResult::MissingUsage;
    };

    if entry_type == "turn_context" {
        if let Some(payload) = get_value(&value, "payload")
            && let Some(model) = extract_codex_model(payload)
        {
            state.current_model = Some(model);
            state.current_model_is_fallback = false;
        }
        return ParseLineResult::MissingUsage;
    }

    if entry_type != "event_msg" {
        return ParseLineResult::MissingUsage;
    }

    let Some(payload_type) = get_value(&value, "payload.type").and_then(|v| v.as_str()) else {
        return ParseLineResult::MissingUsage;
    };
    if payload_type != "token_count" {
        return ParseLineResult::MissingUsage;
    }

    let Some(timestamp) = extract_timestamp(&value) else {
        return ParseLineResult::MissingUsage;
    };

    let info = get_value(&value, "payload.info");
    let last_usage = info.and_then(|v| get_value(v, "last_token_usage"));
    let total_usage = info.and_then(|v| get_value(v, "total_token_usage"));

    let parsed_last = last_usage.and_then(parse_codex_raw_usage);
    let parsed_total = total_usage.and_then(parse_codex_raw_usage);

    let raw_delta = if let Some(last) = parsed_last {
        Some(last)
    } else if let Some(total) = parsed_total {
        Some(subtract_codex_raw_usage(total, state.previous_totals))
    } else {
        None
    };

    if let Some(total) = parsed_total {
        state.previous_totals = Some(total);
    }

    let Some(raw_delta) = raw_delta else {
        return ParseLineResult::MissingUsage;
    };
    if raw_delta.is_zero() {
        return ParseLineResult::MissingUsage;
    }

    let extracted_model = get_value(&value, "payload").and_then(extract_codex_model);
    if let Some(model) = extracted_model.as_ref() {
        state.current_model = Some(model.clone());
        state.current_model_is_fallback = false;
    }

    let model = if let Some(model) = extracted_model {
        model
    } else if let Some(model) = state.current_model.clone() {
        model
    } else {
        state.current_model = Some(LEGACY_CODEX_FALLBACK_MODEL.to_string());
        state.current_model_is_fallback = true;
        LEGACY_CODEX_FALLBACK_MODEL.to_string()
    };

    let usage = codex_delta_to_usage(raw_delta);

    let (cost_usd, used_unknown_pricing) = match pricing.estimate_cost(&model, usage) {
        Some(v) => (v, false),
        None => (0.0, true),
    };

    ParseLineResult::Parsed(ParsedLine {
        event: UsageEvent {
            timestamp,
            source: SourceKind::Codex,
            model,
            session: String::new(),
            project: None,
            file_path: String::new(),
            usage: UsageAccumulator { cost_usd, ..usage },
        },
        used_unknown_pricing,
    })
}

fn extract_model(value: &Value, source: SourceKind) -> Option<String> {
    let candidate_paths = match source {
        SourceKind::Claude => &[
            "message.model",
            "usage.model",
            "model",
            "payload.model",
            "message.metadata.model",
        ][..],
        SourceKind::Codex => &[
            "payload.info.model",
            "payload.info.current_model",
            "payload.model",
            "model",
        ][..],
    };

    extract_string(value, candidate_paths)
}

fn extract_codex_model(value: &Value) -> Option<String> {
    extract_string(
        value,
        &[
            "model",
            "payload.model",
            "info.model",
            "info.current_model",
            "current_model",
        ],
    )
}

fn parse_codex_raw_usage(value: &Value) -> Option<CodexRawUsage> {
    let input_tokens = extract_u64(value, &["input_tokens"])?;
    let cached_input_tokens =
        extract_u64(value, &["cached_input_tokens", "cache_read_input_tokens"]).unwrap_or(0);
    let output_tokens = extract_u64(value, &["output_tokens"]).unwrap_or(0);
    let reasoning_output_tokens = extract_u64(value, &["reasoning_output_tokens"]).unwrap_or(0);
    let total_tokens =
        extract_u64(value, &["total_tokens"]).unwrap_or(input_tokens.saturating_add(output_tokens));

    Some(CodexRawUsage {
        input_tokens,
        cached_input_tokens,
        output_tokens,
        reasoning_output_tokens,
        total_tokens,
    })
}

fn subtract_codex_raw_usage(
    current: CodexRawUsage,
    previous: Option<CodexRawUsage>,
) -> CodexRawUsage {
    let prev = previous.unwrap_or_default();
    CodexRawUsage {
        input_tokens: current.input_tokens.saturating_sub(prev.input_tokens),
        cached_input_tokens: current
            .cached_input_tokens
            .saturating_sub(prev.cached_input_tokens),
        output_tokens: current.output_tokens.saturating_sub(prev.output_tokens),
        reasoning_output_tokens: current
            .reasoning_output_tokens
            .saturating_sub(prev.reasoning_output_tokens),
        total_tokens: current.total_tokens.saturating_sub(prev.total_tokens),
    }
}

fn codex_delta_to_usage(delta: CodexRawUsage) -> UsageAccumulator {
    UsageAccumulator {
        input_tokens: delta.input_tokens.saturating_sub(delta.cached_input_tokens),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: delta.cached_input_tokens,
        output_tokens: delta.output_tokens,
        reasoning_output_tokens: delta.reasoning_output_tokens,
        cost_usd: 0.0,
    }
}

fn extract_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    let candidate_paths = [
        "timestamp",
        "created_at",
        "createdAt",
        "message.timestamp",
        "message.created_at",
    ];

    for path in candidate_paths {
        if let Some(v) = get_value(value, path)
            && let Some(ts) = parse_timestamp_value(v)
        {
            return Some(ts);
        }
    }

    None
}

fn parse_timestamp_value(v: &Value) -> Option<DateTime<Utc>> {
    match v {
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc)),
        Value::Number(n) => {
            let raw = n.as_i64().or_else(|| n.as_u64().map(|u| u as i64))?;
            if raw > 10_000_000_000 {
                DateTime::from_timestamp_millis(raw)
            } else {
                DateTime::from_timestamp(raw, 0)
            }
        }
        _ => None,
    }
}

fn extract_u64(value: &Value, paths: &[&str]) -> Option<u64> {
    paths.iter().find_map(|path| {
        let v = get_value(value, path)?;
        match v {
            Value::Number(n) => n.as_u64().or_else(|| n.as_i64().map(|i| i.max(0) as u64)),
            Value::String(s) => u64::from_str(s).ok(),
            _ => None,
        }
    })
}

fn extract_string(value: &Value, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| {
        get_value(value, path)
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
    })
}

fn get_value<'a>(value: &'a Value, dotted_path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in dotted_path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

async fn load_pricing(path: Option<&str>, offline: bool) -> Result<PricingTable> {
    let mut table = PricingTable::default_table();

    if let Some(openrouter_exact) = load_openrouter_pricing_with_cache(!offline).await {
        table.merge_exact(openrouter_exact);
    }

    if let Some(path) = path {
        let file_path = expand_user_path(path);
        let body = fs::read_to_string(&file_path)
            .await
            .with_context(|| format!("Failed to read pricing file: {}", file_path.display()))?;

        let overrides: HashMap<String, PricingRate> = serde_json::from_str(&body)
            .context("Pricing file must be a JSON object of model -> rate")?;

        table.merge_exact(overrides);
    }

    Ok(table)
}

async fn load_openrouter_pricing_with_cache(
    allow_network_fetch: bool,
) -> Option<HashMap<String, PricingRate>> {
    let cache_path = openrouter_pricing_cache_path();
    let cached = cache_path
        .as_ref()
        .and_then(|path| load_openrouter_pricing_cache(path));
    let now = unix_now_secs();

    if let Some(cache) = cached.as_ref()
        && now.saturating_sub(cache.fetched_unix) < OPENROUTER_PRICING_CACHE_TTL_SECS
    {
        return Some(cache.exact.clone());
    }

    if !allow_network_fetch {
        return cached.map(|cache| cache.exact);
    }

    match fetch_openrouter_pricing().await {
        Ok(exact) => {
            if let Some(path) = cache_path.as_ref() {
                save_openrouter_pricing_cache(path, now, &exact);
            }
            Some(exact)
        }
        Err(_) => cached.map(|cache| cache.exact),
    }
}

async fn fetch_openrouter_pricing() -> Result<HashMap<String, PricingRate>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("tokenusage/0.1")
        .build()
        .context("Failed to initialize OpenRouter pricing client")?;

    let response = client
        .get(OPENROUTER_MODELS_URL)
        .send()
        .await
        .context("Failed to fetch OpenRouter model pricing")?
        .error_for_status()
        .context("OpenRouter model pricing request failed")?;

    let payload: OpenRouterModelsResponse = response
        .json()
        .await
        .context("Failed to decode OpenRouter model pricing response")?;

    let mut exact = HashMap::new();
    for model in payload.data {
        let Some(rate) = openrouter_rate(&model.pricing) else {
            continue;
        };

        for alias in openrouter_model_aliases(&model.id) {
            exact.insert(alias, rate.clone());
        }
    }

    Ok(exact)
}

fn openrouter_rate(pricing: &OpenRouterPricingEntry) -> Option<PricingRate> {
    let input_per_million = openrouter_token_price_per_million(pricing.prompt.as_ref())?;
    let output_per_million = openrouter_token_price_per_million(pricing.completion.as_ref())?;
    let cache_read_per_million =
        openrouter_token_price_per_million(pricing.input_cache_read.as_ref()).unwrap_or(0.0);
    let cache_creation_per_million =
        openrouter_token_price_per_million(pricing.input_cache_write.as_ref()).unwrap_or(0.0);

    Some(PricingRate {
        input_per_million,
        output_per_million,
        cache_creation_per_million,
        cache_read_per_million,
        // Reasoning tokens are already represented inside output tokens in our parser.
        reasoning_output_per_million: 0.0,
        ..PricingRate::default()
    })
}

fn openrouter_token_price_per_million(value: Option<&OpenRouterNumber>) -> Option<f64> {
    let per_token = match value? {
        OpenRouterNumber::Number(value) => *value,
        OpenRouterNumber::String(raw) => f64::from_str(raw).ok()?,
    };

    if !per_token.is_finite() || per_token < 0.0 {
        return None;
    }

    Some(per_token * 1_000_000.0)
}

fn openrouter_model_aliases(id: &str) -> Vec<String> {
    let normalized = canonical_model_name(id);
    if normalized.is_empty() {
        return Vec::new();
    }

    let mut aliases = HashSet::new();
    aliases.insert(normalized.clone());

    if let Some(tail) = normalized.rsplit('/').next() {
        aliases.insert(tail.to_string());
        if let Some(stripped) = strip_model_date_suffix(tail) {
            aliases.insert(stripped.to_string());
        }
        if tail.contains('.') {
            aliases.insert(tail.replace('.', "-"));
        }
    }

    aliases.into_iter().collect()
}

fn strip_model_date_suffix(model: &str) -> Option<&str> {
    let (head, tail) = model.rsplit_once('-')?;
    if tail.len() == 8 && tail.chars().all(|ch| ch.is_ascii_digit()) {
        Some(head)
    } else {
        None
    }
}

fn canonical_model_name(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

fn openrouter_pricing_cache_path() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("tokenusage").join("openrouter-pricing-v1.json"))
}

fn load_openrouter_pricing_cache(path: &Path) -> Option<OpenRouterPricingCacheStore> {
    let body = std::fs::read(path).ok()?;
    let cache: OpenRouterPricingCacheStore = serde_json::from_slice(&body).ok()?;
    (cache.version == OPENROUTER_PRICING_CACHE_VERSION).then_some(cache)
}

fn save_openrouter_pricing_cache(
    path: &Path,
    fetched_unix: u64,
    exact: &HashMap<String, PricingRate>,
) {
    let cache = OpenRouterPricingCacheStore {
        version: OPENROUTER_PRICING_CACHE_VERSION,
        fetched_unix,
        exact: exact.clone(),
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec(&cache) {
        let _ = std::fs::write(path, bytes);
    }
}

fn parse_date_filter(input: Option<&str>) -> Result<Option<NaiveDate>> {
    let Some(value) = input else {
        return Ok(None);
    };

    for fmt in ["%Y%m%d", "%Y-%m-%d"] {
        if let Ok(date) = NaiveDate::parse_from_str(value, fmt) {
            return Ok(Some(date));
        }
    }

    bail!("Invalid date format: {value}. Use YYYYMMDD or YYYY-MM-DD")
}

fn expand_user_path(input: &str) -> PathBuf {
    if input == "~"
        && let Some(home) = dirs::home_dir()
    {
        return home;
    }

    if let Some(stripped) = input.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(stripped);
    }

    PathBuf::from(input)
}

fn normalize_ignore_fragment(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.replace('\\', "/"))
}

fn incremental_cache_path() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("tokenusage").join("parse-cache-v2.json"))
}

fn load_incremental_cache(path: &Path, pricing_key: &str) -> IncrementalCacheStore {
    let body = match std::fs::read(path) {
        Ok(data) => data,
        Err(_) => return IncrementalCacheStore::new(pricing_key.to_string()),
    };

    let store: IncrementalCacheStore = match serde_json::from_slice(&body) {
        Ok(store) => store,
        Err(_) => return IncrementalCacheStore::new(pricing_key.to_string()),
    };

    if store.version != INCREMENTAL_CACHE_VERSION || store.pricing_key != pricing_key {
        return IncrementalCacheStore::new(pricing_key.to_string());
    }

    store
}

fn save_incremental_cache(path: &Path, store: &IncrementalCacheStore) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec(store) {
        let _ = std::fs::write(path, bytes);
    }
}

fn cache_file_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn read_file_fingerprint(path: &Path) -> Option<FileFingerprint> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let dur = modified.duration_since(UNIX_EPOCH).ok()?;
    Some(FileFingerprint {
        size: meta.len(),
        modified_unix_secs: dur.as_secs() as i64,
        modified_unix_nanos: dur.subsec_nanos(),
    })
}

fn can_incremental_parse(cached: &CachedFileEntry, current: FileFingerprint) -> bool {
    if current.size <= cached.fingerprint.size {
        return false;
    }
    let start_offset = cached.parsed_offset.max(cached.fingerprint.size);
    start_offset > 0 && start_offset <= current.size
}

fn hydrate_cached_events(
    file: &DiscoveredFile,
    cached: &CachedFileEntry,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    stats: &ParseStatsAtomic,
) -> Vec<UsageEvent> {
    stats
        .lines_total
        .fetch_add(cached.stats.lines_total, Ordering::Relaxed);
    stats
        .lines_invalid_json
        .fetch_add(cached.stats.lines_invalid_json, Ordering::Relaxed);
    stats
        .lines_missing_usage
        .fetch_add(cached.stats.lines_missing_usage, Ordering::Relaxed);
    stats
        .lines_unknown_pricing
        .fetch_add(cached.stats.lines_unknown_pricing, Ordering::Relaxed);

    let (session, project) = derive_session_meta(file);
    let file_path = file.path.display().to_string();
    let mut filtered = 0usize;
    let mut parsed = 0usize;
    let mut events = Vec::with_capacity(cached.events.len());

    for cached_event in &cached.events {
        let day = local_date(cached_event.timestamp, timezone);
        if !filter.allows(day) {
            filtered += 1;
            continue;
        }
        parsed += 1;
        events.push(UsageEvent {
            timestamp: cached_event.timestamp,
            source: file.source,
            model: cached_event.model.clone(),
            session: session.clone(),
            project: project.clone(),
            file_path: file_path.clone(),
            usage: cached_event.usage,
        });
    }

    stats.lines_filtered.fetch_add(filtered, Ordering::Relaxed);
    stats.lines_parsed.fetch_add(parsed, Ordering::Relaxed);

    events
}

fn pricing_cache_key(pricing: &PricingTable) -> String {
    let mut out = String::new();
    out.push_str("estimate-v2");
    out.push('|');

    let mut exact = pricing.exact.iter().collect::<Vec<_>>();
    exact.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (model, rate) in exact {
        out.push_str(model);
        out.push(':');
        out.push_str(&pricing_rate_key(rate));
        out.push('|');
    }

    out.push('#');
    for (prefix, rate) in &pricing.prefixes {
        out.push_str(prefix);
        out.push(':');
        out.push_str(&pricing_rate_key(rate));
        out.push('|');
    }

    out
}

fn pricing_rate_key(rate: &PricingRate) -> String {
    format!(
        "{:.8},{:.8},{:.8},{:.8},{:.8},{},{},{},{},{},{}",
        rate.input_per_million,
        rate.output_per_million,
        rate.cache_creation_per_million,
        rate.cache_read_per_million,
        rate.reasoning_output_per_million,
        rate.tier_threshold_tokens
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string()),
        rate.input_above_per_million
            .map(|v| format!("{v:.8}"))
            .unwrap_or_else(|| "-".to_string()),
        rate.output_above_per_million
            .map(|v| format!("{v:.8}"))
            .unwrap_or_else(|| "-".to_string()),
        rate.cache_creation_above_per_million
            .map(|v| format!("{v:.8}"))
            .unwrap_or_else(|| "-".to_string()),
        rate.cache_read_above_per_million
            .map(|v| format!("{v:.8}"))
            .unwrap_or_else(|| "-".to_string()),
        rate.reasoning_output_above_per_million
            .map(|v| format!("{v:.8}"))
            .unwrap_or_else(|| "-".to_string()),
    )
}

fn normalize_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }

    if path.is_absolute() {
        return path.to_path_buf();
    }

    match std::env::current_dir() {
        Ok(cwd) => cwd.join(path),
        Err(_) => path.to_path_buf(),
    }
}

fn normalized_discovered_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        normalize_path(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc_dt(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .unwrap()
    }

    fn test_event(ts: DateTime<Utc>, source: SourceKind, total_tokens: u64) -> UsageEvent {
        UsageEvent {
            timestamp: ts,
            source,
            model: "gpt-5.3-codex".to_string(),
            session: "s".to_string(),
            project: None,
            file_path: "/tmp/log.jsonl".to_string(),
            usage: UsageAccumulator {
                input_tokens: total_tokens,
                ..UsageAccumulator::default()
            },
        }
    }

    #[test]
    fn normalize_official_percent_treats_one_as_percent_not_ratio() {
        assert!((normalize_official_used_percent(0.82) - 82.0).abs() < f64::EPSILON);
        assert!((normalize_official_used_percent(82.0) - 82.0).abs() < f64::EPSILON);
        assert!((normalize_official_used_percent(1.0) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn resolve_live_bounds_aligns_to_official_reset_window() {
        let now = utc_dt(2026, 3, 3, 16, 7, 36);
        let official = OfficialCodexSnapshot {
            plan_type: Some("pro".to_string()),
            primary_used_percent: Some(99.0),
            secondary_used_percent: Some(82.0),
            primary_window_mins: Some(300),
            secondary_window_mins: Some(10080),
            primary_resets_at: Some(utc_dt(2026, 3, 3, 20, 59, 47).timestamp()),
            secondary_resets_at: Some(utc_dt(2026, 3, 10, 10, 59, 47).timestamp()),
        };

        let (start, end, window_secs) = resolve_live_block_bounds(
            now,
            5 * 3600,
            Some(SourceKind::Codex),
            Some(&official),
            None,
        );

        assert_eq!(window_secs, 5 * 3600);
        assert_eq!(start, utc_dt(2026, 3, 3, 15, 59, 47).timestamp());
        assert_eq!(end, utc_dt(2026, 3, 3, 20, 59, 47).timestamp());
        assert!(now.timestamp() >= start && now.timestamp() < end);
    }

    #[test]
    fn resolve_live_bounds_rolls_old_reset_forward_to_current_session() {
        let now = utc_dt(2026, 3, 3, 16, 7, 36);
        let stale = OfficialCodexSnapshot {
            plan_type: Some("pro".to_string()),
            primary_used_percent: Some(99.0),
            secondary_used_percent: Some(82.0),
            primary_window_mins: Some(300),
            secondary_window_mins: Some(10080),
            // previous session boundary; function should advance it.
            primary_resets_at: Some(utc_dt(2026, 3, 3, 10, 59, 47).timestamp()),
            secondary_resets_at: None,
        };

        let (start, end, window_secs) =
            resolve_live_block_bounds(now, 5 * 3600, Some(SourceKind::Codex), Some(&stale), None);

        assert_eq!(window_secs, 5 * 3600);
        assert_eq!(start, utc_dt(2026, 3, 3, 15, 59, 47).timestamp());
        assert_eq!(end, utc_dt(2026, 3, 3, 20, 59, 47).timestamp());
    }

    #[test]
    fn active_block_summary_for_bounds_only_counts_events_in_current_window() {
        let now = utc_dt(2026, 3, 3, 16, 30, 0);
        let block_start = utc_dt(2026, 3, 3, 15, 59, 47).timestamp();
        let block_end = utc_dt(2026, 3, 3, 20, 59, 47).timestamp();

        let events = vec![
            // previous window event, must be excluded
            test_event(utc_dt(2026, 3, 3, 15, 30, 0), SourceKind::Codex, 900),
            // current window events
            test_event(utc_dt(2026, 3, 3, 16, 0, 0), SourceKind::Codex, 100),
            test_event(utc_dt(2026, 3, 3, 16, 10, 0), SourceKind::Codex, 300),
        ];

        let summary = active_block_summary_for_bounds(&events, now, block_start, block_end)
            .expect("expected active summary for current window");

        assert_eq!(summary.totals.total_tokens, 400);
        assert_eq!(summary.dominant_source, Some(SourceKind::Codex));
        assert!(summary.remaining_minutes >= 0);
    }

    // ---- Antigravity tests ----

    #[test]
    fn parse_antigravity_user_status_response() {
        let json = serde_json::json!({
            "code": 0,
            "userStatus": {
                "email": "test@example.com",
                "planStatus": {
                    "planInfo": {
                        "planName": "pro",
                        "planDisplayName": "Pro Plan",
                        "displayName": "Pro",
                        "productName": "Antigravity Pro",
                        "planShortName": "pro"
                    }
                },
                "cascadeModelConfigData": {
                    "clientModelConfigs": [
                        {
                            "label": "Claude 4 Sonnet",
                            "modelOrAlias": { "model": "claude-4-sonnet" },
                            "quotaInfo": {
                                "remainingFraction": 0.75,
                                "resetTime": "1709500800"
                            }
                        },
                        {
                            "label": "Gemini Pro Low",
                            "modelOrAlias": { "model": "gemini-pro" },
                            "quotaInfo": {
                                "remainingFraction": 0.50,
                                "resetTime": "1709500800"
                            }
                        },
                        {
                            "label": "Gemini 2.5 Flash",
                            "modelOrAlias": { "model": "gemini-flash" },
                            "quotaInfo": {
                                "remainingFraction": 0.90
                            }
                        }
                    ]
                }
            }
        });

        let snapshot = parse_antigravity_user_status(&json).unwrap();
        assert_eq!(snapshot.plan_type.as_deref(), Some("Pro Plan"));
        assert_eq!(snapshot.account_email.as_deref(), Some("test@example.com"));
        assert_eq!(snapshot.models.len(), 3);

        // Claude is primary (25% used)
        assert!((snapshot.primary_used_percent.unwrap() - 25.0).abs() < 0.01);
        assert_eq!(snapshot.primary_label.as_deref(), Some("Claude 4 Sonnet"));

        // Gemini Pro Low is secondary (50% used)
        assert!((snapshot.secondary_used_percent.unwrap() - 50.0).abs() < 0.01);
        assert_eq!(snapshot.secondary_label.as_deref(), Some("Gemini Pro Low"));

        // Gemini Flash is tertiary (10% used)
        assert!((snapshot.tertiary_used_percent.unwrap() - 10.0).abs() < 0.01);
        assert_eq!(
            snapshot.tertiary_label.as_deref(),
            Some("Gemini 2.5 Flash")
        );
    }

    #[test]
    fn parse_antigravity_command_model_response() {
        let json = serde_json::json!({
            "code": "ok",
            "clientModelConfigs": [
                {
                    "label": "GPT-4o",
                    "modelOrAlias": { "model": "gpt-4o" },
                    "quotaInfo": {
                        "remainingFraction": 0.30
                    }
                }
            ]
        });

        let snapshot = parse_antigravity_command_model_configs(&json).unwrap();
        assert!(snapshot.plan_type.is_none());
        assert!(snapshot.account_email.is_none());
        assert_eq!(snapshot.models.len(), 1);
        // Fallback: single model becomes primary
        assert!((snapshot.primary_used_percent.unwrap() - 70.0).abs() < 0.01);
        assert_eq!(snapshot.primary_label.as_deref(), Some("GPT-4o"));
    }

    #[test]
    fn select_antigravity_models_prioritises_correctly() {
        let models = vec![
            AntigravityModelQuotaSnapshot {
                label: "Gemini 2.5 Flash".to_string(),
                model_id: "gemini-flash".to_string(),
                remaining_fraction: Some(0.9),
                reset_time: None,
            },
            AntigravityModelQuotaSnapshot {
                label: "Claude 4 Sonnet".to_string(),
                model_id: "claude-4-sonnet".to_string(),
                remaining_fraction: Some(0.5),
                reset_time: None,
            },
            AntigravityModelQuotaSnapshot {
                label: "Gemini Pro Low".to_string(),
                model_id: "gemini-pro-low".to_string(),
                remaining_fraction: Some(0.3),
                reset_time: None,
            },
        ];

        let ordered = select_antigravity_models(&models);
        assert_eq!(ordered.len(), 3);
        assert_eq!(ordered[0].label, "Claude 4 Sonnet");
        assert_eq!(ordered[1].label, "Gemini Pro Low");
        assert_eq!(ordered[2].label, "Gemini 2.5 Flash");
    }

    #[test]
    fn select_antigravity_models_fallback_sorts_by_remaining() {
        let models = vec![
            AntigravityModelQuotaSnapshot {
                label: "Model A".to_string(),
                model_id: "a".to_string(),
                remaining_fraction: Some(0.8),
                reset_time: None,
            },
            AntigravityModelQuotaSnapshot {
                label: "Model B".to_string(),
                model_id: "b".to_string(),
                remaining_fraction: Some(0.2),
                reset_time: None,
            },
        ];

        let ordered = select_antigravity_models(&models);
        assert_eq!(ordered[0].label, "Model B"); // 0.2 remaining = lowest first
        assert_eq!(ordered[1].label, "Model A");
    }

    #[test]
    fn extract_flag_value_works() {
        let cmd = "/path/to/language_server_macos --csrf_token=abc123 --extension_server_port 42150 --app_data_dir antigravity";
        assert_eq!(
            extract_flag_value("--csrf_token", cmd),
            Some("abc123".to_string())
        );
        assert_eq!(
            extract_flag_value("--extension_server_port", cmd),
            Some("42150".to_string())
        );
        assert_eq!(
            extract_flag_value("--app_data_dir", cmd),
            Some("antigravity".to_string())
        );
        assert_eq!(extract_flag_value("--nonexistent", cmd), None);
    }

    #[test]
    fn parse_antigravity_reset_time_iso8601() {
        let ts = parse_antigravity_reset_time("2024-03-04T12:00:00Z");
        assert!(ts.is_some());
        assert_eq!(ts.unwrap(), 1709553600);
    }

    #[test]
    fn parse_antigravity_reset_time_epoch() {
        let ts = parse_antigravity_reset_time("1709500800");
        assert_eq!(ts, Some(1709500800));
    }

    #[test]
    fn parse_antigravity_error_code_rejects_nonzero() {
        let json = serde_json::json!({
            "code": 7,
            "userStatus": null,
        });
        assert!(parse_antigravity_user_status(&json).is_err());
    }
}
