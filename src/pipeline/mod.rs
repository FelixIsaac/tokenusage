#[cfg(feature = "cli")]
mod activity_report;
mod block_report;
mod commands;
#[cfg(feature = "cli")]
mod display;
#[cfg(feature = "cli")]
mod live;
#[cfg(feature = "cli")]
mod membership;
mod official;
mod parsing;
mod pricing;
#[cfg(feature = "cli")]
mod statusline;
#[cfg(feature = "cli")]
mod top;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

// Re-exports: public API (library consumers)
pub use commands::{collect_report, collect_usage_snapshot};

// Re-exports: crate-internal (CLI commands)
#[cfg(feature = "cli")]
pub(crate) use commands::{
    run_activity, run_antigravity, run_daily, run_monthly, run_session, run_today, run_weekly,
};
#[cfg(feature = "cli")]
pub(crate) use live::run_blocks;
#[cfg(feature = "cli")]
pub(crate) use statusline::run_statusline;
#[cfg(feature = "cli")]
pub(crate) use top::run_top;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
#[cfg(feature = "cli")]
use std::io::IsTerminal;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::cli::{
    CommonArgs, SortOrder, WeekStart,
};
#[cfg(feature = "cli")]
use crate::output::print_report_table_with_options;
use crate::types::{
    ActivitySummary, CodexRawUsage, DailyReport, DailyRow, DateFilter,
    DiscoveredFile, ParseStats, PricingRate, PricingTable, SourceConfig, SourceKind, TokenCounts, UsageAccumulator,
    UsageEvent,
};

#[derive(Debug, Clone)]
pub enum TimeZoneMode {
    Local,
    Utc,
    Named(Tz),
}

impl TimeZoneMode {
    pub fn date_of(&self, ts: DateTime<Utc>) -> NaiveDate {
        match self {
            TimeZoneMode::Local => ts.with_timezone(&Local).date_naive(),
            TimeZoneMode::Utc => ts.date_naive(),
            TimeZoneMode::Named(tz) => ts.with_timezone(tz).date_naive(),
        }
    }

    pub fn hour_of(&self, ts: DateTime<Utc>) -> u32 {
        match self {
            TimeZoneMode::Local => ts.with_timezone(&Local).hour(),
            TimeZoneMode::Utc => ts.hour(),
            TimeZoneMode::Named(tz) => ts.with_timezone(tz).hour(),
        }
    }

    pub fn now_date(&self) -> NaiveDate {
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
    /// Claude files deferred until the second frame so Codex data renders instantly.
    deferred_claude_files: Option<Vec<DiscoveredFile>>,
    last_discovery_at: Instant,
    discovery_interval: Duration,
    last_sources_refresh_at: Instant,
    sources_refresh_interval: Duration,
    last_cache_flush_at: Instant,
}

pub struct UsageSnapshot {
    pub events: Vec<UsageEvent>,
    pub stats: ParseStats,
    pub timezone: TimeZoneMode,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OfficialCodexSnapshot {
    plan_type: Option<String>,
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
    primary_window_mins: Option<i64>,
    secondary_window_mins: Option<i64>,
    primary_resets_at: Option<i64>,
    secondary_resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OfficialClaudeSnapshot {
    plan_type: Option<String>,
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
    primary_window_mins: Option<i64>,
    secondary_window_mins: Option<i64>,
    primary_resets_at: Option<i64>,
    secondary_resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AntigravityModelQuotaSnapshot {
    label: String,
    model_id: String,
    remaining_fraction: Option<f64>,
    reset_time: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    today_activity: Option<ActivitySummary>,
    last_30d_activity: Option<ActivitySummary>,
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
        today_activity: Option<ActivitySummary>,
        last_30d_activity: Option<ActivitySummary>,
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
            today_activity,
            last_30d_activity,
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
pub enum ReportPeriod {
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

fn build_report_from_rows(
    rows: Vec<DailyRow>,
    activity_totals: Option<ActivitySummary>,
    stats: ParseStats,
) -> DailyReport {
    let totals = rows.iter().fold(TokenCounts::default(), |mut acc, row| {
        acc.add_assign(row.totals.clone());
        acc
    });

    DailyReport {
        daily: rows,
        totals,
        activity_totals,
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
            activity: None,
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
            .map(|n: std::num::NonZero<usize>| n.get())
            .unwrap_or(4)
    })
}


pub(super) fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
