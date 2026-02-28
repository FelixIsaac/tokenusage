use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Local, NaiveDate, Utc};
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
    BlocksArgs, CommonArgs, CostSource, DailyArgs, MonthlyArgs, SessionArgs, SortOrder,
    StatuslineArgs, VisualBurnRate, WeekStart, WeeklyArgs,
};
use crate::output::{print_report_table_with_options, run_report_tui};
use crate::types::{
    CodexParseState, CodexRawUsage, DailyReport, DailyRow, DateFilter, DiscoveredFile,
    LEGACY_CODEX_FALLBACK_MODEL, ParseLineResult, ParseStats, ParseStatsAtomic, ParsedLine,
    PricingRate, PricingTable, SourceConfig, SourceKind, TokenCounts, UsageAccumulator, UsageEvent,
};

#[derive(Debug, Clone)]
enum TimeZoneMode {
    Local,
    Utc,
    Named(Tz),
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
        let merged_model = format!("{}:{}", event.source.as_str(), event.model);
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
    now: DateTime<Utc>,
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
}

#[derive(Debug, Clone, Copy)]
struct LimitDisplayContext<'a> {
    token_limit: Option<u64>,
    token_limit_source: TokenLimitSource,
    membership_estimate: Option<&'a MembershipEstimate>,
}

#[derive(Debug, Clone, Copy)]
struct ProjectionTotals {
    current_tokens: u64,
    projected_tokens: u64,
    current_cost: f64,
    projected_cost: f64,
}

#[derive(Debug)]
struct LiveFrameContext<'a> {
    refresh_every: u64,
    window_secs: i64,
    elapsed_secs: i64,
    now_text: String,
    block_start_text: String,
    block_end_text: String,
    limit: LimitDisplayContext<'a>,
    active: Option<&'a ActiveBlockSummary>,
    stats: &'a ParseStats,
}

impl<'a> LiveFrameContext<'a> {
    fn new(
        now: DateTime<Utc>,
        tz: &TimeZoneMode,
        window_secs: i64,
        refresh_every: u64,
        limit: LimitDisplayContext<'a>,
        active: Option<&'a ActiveBlockSummary>,
        stats: &'a ParseStats,
    ) -> Self {
        let now_unix = now.timestamp();
        let block_start_unix = now_unix - now_unix.rem_euclid(window_secs);
        let block_start = DateTime::from_timestamp(block_start_unix, 0).unwrap_or(now);
        let block_end = block_start + chrono::TimeDelta::seconds(window_secs);

        Self {
            refresh_every,
            window_secs,
            elapsed_secs: (now_unix - block_start_unix).clamp(0, window_secs.max(1)),
            now_text: format_display_datetime(now, tz),
            block_start_text: format_display_datetime(block_start, tz),
            block_end_text: format_display_datetime(block_end, tz),
            limit,
            active,
            stats,
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
const INCREMENTAL_CACHE_VERSION: u32 = 1;
const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const OPENROUTER_PRICING_CACHE_VERSION: u32 = 1;
const OPENROUTER_PRICING_CACHE_TTL_SECS: u64 = 6 * 60 * 60;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    if args.live {
        return run_blocks_live(&args, &tz, window_secs, token_limit_mode).await;
    }

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
        );
        print_debug(&show.stats, &args.common);
        Ok(())
    }
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
    }
}

async fn run_blocks_live(
    args: &BlocksArgs,
    tz: &TimeZoneMode,
    window_secs: i64,
    token_limit_mode: Option<TokenLimitMode>,
) -> Result<()> {
    if !std::io::stdout().is_terminal() {
        bail!("--live requires an interactive terminal");
    }

    let refresh_every = args.refresh_interval.max(1);
    let mut session = BlocksLiveSession::enter()?;

    loop {
        let now = Utc::now();
        let loaded = load_usage(&args.common, tz).await?;
        let membership_estimate = estimate_membership_from_logs(&loaded.events, now, window_secs);
        let inferred_limit = membership_estimate
            .as_ref()
            .map(|estimate| estimate.estimated_window_tokens);
        let resolved_from_mode =
            resolve_token_limit(token_limit_mode, &loaded.events, now, window_secs);
        let token_limit = resolved_from_mode.or(inferred_limit);
        let token_limit_source =
            resolve_token_limit_source(token_limit_mode, resolved_from_mode, inferred_limit);
        let active = active_block_summary(&loaded.events, now, window_secs);
        let frame_context = LiveFrameContext::new(
            now,
            tz,
            window_secs,
            refresh_every,
            LimitDisplayContext {
                token_limit,
                token_limit_source,
                membership_estimate: membership_estimate.as_ref(),
            },
            active.as_ref(),
            &loaded.stats,
        );

        render_blocks_live_frame(&mut session, &frame_context)?;

        if wait_for_blocks_live_exit(Duration::from_secs(refresh_every))? {
            break;
        }
    }

    Ok(())
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
    let header_height = if root.width >= 112 { 2 } else { 4 };
    let [header_area, progress_area, body_area, stats_area] = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Length(4),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .margin(1)
    .areas(root);

    let header_lines = if root.width >= 112 {
        vec![
            Line::from(vec![
                Span::styled(
                    "tu live",
                    Style::default()
                        .fg(TuiColor::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    "  refresh {}s  |  {}  |  {}h window",
                    context.refresh_every,
                    context.now_text,
                    context.window_secs / 3600
                )),
            ]),
            Line::from(format!(
                "block {} -> {}  |  q / Esc / Ctrl+C exit",
                context.block_start_text, context.block_end_text
            )),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled(
                    "tu live",
                    Style::default()
                        .fg(TuiColor::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    "  refresh {}s  |  {}",
                    context.refresh_every, context.now_text
                )),
            ]),
            Line::from(format!("{}h window", context.window_secs / 3600)),
            Line::from(format!(
                "block {} -> {}",
                context.block_start_text, context.block_end_text
            )),
            Line::from("q / Esc / Ctrl+C exit"),
        ]
    };

    let header = Paragraph::new(header_lines).wrap(Wrap { trim: true });
    frame.render_widget(header, header_area);

    render_live_progress_bars(frame, progress_area, context);

    let mut body_lines = Vec::new();
    body_lines.push(Line::from(vec![Span::styled(
        "Current",
        Style::default()
            .fg(TuiColor::Cyan)
            .add_modifier(Modifier::BOLD),
    )]));
    body_lines.extend(live_usage_lines(context.active));
    body_lines.push(Line::from(""));
    body_lines.push(Line::from(vec![Span::styled(
        "Projection / Limit",
        Style::default()
            .fg(TuiColor::Cyan)
            .add_modifier(Modifier::BOLD),
    )]));
    body_lines.extend(live_projection_lines(
        context.active,
        context.limit.token_limit,
        context.limit.token_limit_source,
        context.limit.membership_estimate,
    ));
    let body_widget = Paragraph::new(body_lines).wrap(Wrap { trim: true });
    frame.render_widget(body_widget, body_area);

    let stats_line = format!(
        "files={} parsed={} filtered={} invalid={} missing={} unknown_pricing={}",
        context.stats.files_discovered,
        context.stats.lines_parsed,
        context.stats.lines_filtered,
        context.stats.lines_invalid_json,
        context.stats.lines_missing_usage,
        context.stats.lines_unknown_pricing
    );
    let stats_widget = Paragraph::new(Line::from(stats_line)).wrap(Wrap { trim: true });
    frame.render_widget(stats_widget, stats_area);
}

fn render_live_progress_bars(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    let [time_label_area, time_area, limit_label_area, limit_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

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
        .gauge_style(
            Style::default()
                .fg(TuiColor::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(time_ratio)
        .label(time_label);
    frame.render_widget(time_gauge, time_area);

    let (current_tokens, projected_tokens) = context
        .active
        .map(|active_block| {
            let (projected_tokens, _) = projected_end(active_block);
            (active_block.totals.total_tokens, projected_tokens)
        })
        .unwrap_or((0, 0));

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
        TokenLimitSource::EstimatedFromLogs if promoted => "Limit (estimated + tiered)",
        TokenLimitSource::EstimatedFromLogs => "Limit (estimated from logs)",
        TokenLimitSource::HistoricalMax => "Limit (historical max)",
        TokenLimitSource::Explicit => "Limit (explicit)",
        TokenLimitSource::Unset => "Limit",
    };
    let limit_title = Paragraph::new(Line::from(vec![Span::styled(
        limit_title_text,
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    frame.render_widget(limit_title, limit_label_area);

    let limit_gauge = Gauge::default()
        .gauge_style(
            Style::default()
                .fg(limit_color)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(limit_ratio)
        .label(limit_label);
    frame.render_widget(limit_gauge, limit_area);
}

fn live_usage_lines(active: Option<&ActiveBlockSummary>) -> Vec<Line<'static>> {
    let Some(active_block) = active else {
        return vec![
            Line::from("No active block usage in current window."),
            Line::from("Waiting for token events..."),
        ];
    };

    vec![
        Line::from(format!(
            "Current: {} tokens | {}",
            format_u64(active_block.totals.total_tokens),
            format_usd(active_block.totals.cost_usd)
        )),
        Line::from(format!(
            "Input {} | Output {}",
            format_u64(active_block.totals.input_tokens),
            format_u64(active_block.totals.output_tokens)
        )),
        Line::from(format!(
            "Cache Create {} | Cache Read {}",
            format_u64(active_block.totals.cache_creation_input_tokens),
            format_u64(active_block.totals.cache_read_input_tokens),
        )),
        Line::from(format!(
            "Remaining: {}",
            format_remaining_minutes(active_block.remaining_minutes)
        )),
    ]
}

fn live_projection_lines(
    active: Option<&ActiveBlockSummary>,
    token_limit: Option<u64>,
    token_limit_source: TokenLimitSource,
    membership_estimate: Option<&MembershipEstimate>,
) -> Vec<Line<'static>> {
    let Some(active_block) = active else {
        return match token_limit {
            Some(limit) if limit > 0 => {
                let label = if token_limit_source == TokenLimitSource::EstimatedFromLogs {
                    "Estimated token limit"
                } else {
                    "Token limit"
                };
                vec![Line::from(format!("{label}: {}", format_u64(limit)))]
            }
            Some(_) => vec![Line::from("Token limit: 0 (disabled)")],
            None => {
                if let Some(estimate) = membership_estimate {
                    vec![
                        Line::from(format!(
                            "Estimated plan: {} ({:.0}% confidence)",
                            display_plan_label(&estimate.estimated_plan),
                            estimate.confidence * 100.0
                        )),
                        Line::from(format!(
                            "Estimated window limit: {} tokens",
                            format_u64(estimate.estimated_window_tokens)
                        )),
                    ]
                } else {
                    vec![Line::from("Token limit: not set")]
                }
            }
        };
    };

    let current_tokens = active_block.totals.total_tokens;
    let current_cost = active_block.totals.cost_usd;
    let mut lines = Vec::new();

    if let Some(burn) = active_block.burn.as_ref() {
        let status_text = burn_status_text(burn.status);
        lines.push(Line::from(format!(
            "Burn: {} tokens/min | {}/hr",
            format_u64(burn.tokens_per_minute.round().max(0.0) as u64),
            format_usd(burn.cost_per_hour)
        )));
        lines.push(Line::from(vec![
            Span::raw("Burn status: "),
            Span::styled(
                status_text,
                Style::default()
                    .fg(burn_status_color(burn.status))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        let (projected_tokens, projected_cost) = projected_end(active_block);
        lines.push(Line::from(format!(
            "Projected end: {} | {}",
            format_u64(projected_tokens),
            format_usd(projected_cost)
        )));

        append_limit_lines(
            &mut lines,
            LimitDisplayContext {
                token_limit,
                token_limit_source,
                membership_estimate,
            },
            ProjectionTotals {
                current_tokens,
                projected_tokens,
                current_cost,
                projected_cost,
            },
        );
    } else {
        lines.push(Line::from("Burn: waiting for more activity..."));
        append_limit_lines(
            &mut lines,
            LimitDisplayContext {
                token_limit,
                token_limit_source,
                membership_estimate,
            },
            ProjectionTotals {
                current_tokens,
                projected_tokens: current_tokens,
                current_cost,
                projected_cost: current_cost,
            },
        );
    }

    if let Some(estimate) = membership_estimate {
        lines.push(Line::from(format!(
            "Estimated plan: {} ({:.0}% confidence)",
            display_plan_label(&estimate.estimated_plan),
            estimate.confidence * 100.0
        )));
    }

    lines
}

fn append_limit_lines(
    lines: &mut Vec<Line<'static>>,
    limit_ctx: LimitDisplayContext<'_>,
    projection: ProjectionTotals,
) {
    let ProjectionTotals {
        current_tokens,
        projected_tokens,
        current_cost,
        projected_cost,
    } = projection;

    match limit_ctx.token_limit {
        Some(0) => lines.push(Line::from("Token limit: 0 (disabled)")),
        Some(limit) => {
            let (effective_limit, promotions) = resolve_display_limit(
                limit,
                projected_tokens,
                limit_ctx.token_limit_source,
                limit_ctx.membership_estimate,
            );
            let current_pct = (current_tokens as f64 / effective_limit as f64) * 100.0;
            let projected_pct = (projected_tokens as f64 / effective_limit as f64) * 100.0;
            let (status, status_color) = limit_status(projected_pct);
            let remaining_tokens = effective_limit.saturating_sub(current_tokens);
            let remaining_cost = (projected_cost - current_cost).max(0.0);
            let label = if limit_ctx.token_limit_source == TokenLimitSource::EstimatedFromLogs {
                "Estimated limit"
            } else {
                "Limit"
            };

            if promotions.is_empty() {
                lines.push(Line::from(format!(
                    "{label}: {} | current {:.1}% | projected {:.1}%",
                    format_u64(effective_limit),
                    current_pct,
                    projected_pct
                )));
            } else {
                lines.push(Line::from(format!(
                    "{label}: {} (auto-upgraded from {}) | current {:.1}% | projected {:.1}%",
                    format_u64(effective_limit),
                    format_u64(limit),
                    current_pct,
                    projected_pct
                )));
                lines.push(Line::from(format!(
                    "Auto tier path: {}",
                    promotions.join(" -> ")
                )));
            }
            lines.push(Line::from(vec![
                Span::raw("Status: "),
                Span::styled(
                    status,
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    " | remaining {} tokens | +{}",
                    format_u64(remaining_tokens),
                    format_usd(remaining_cost)
                )),
            ]));
        }
        None => lines.push(Line::from("Token limit: not set (--token-limit <n|max>)")),
    }
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

fn wait_for_blocks_live_exit(timeout: Duration) -> Result<bool> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let poll_for = remaining.min(Duration::from_millis(200));
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
            KeyCode::Esc | KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(true);
            }
            _ => {}
        }
    }

    Ok(false)
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
    let line = build_statusline_line(
        &args,
        hook.as_ref(),
        session_totals.as_ref(),
        &today_totals,
        block_summary.as_ref(),
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
    })
}

fn build_statusline_line(
    args: &StatuslineArgs,
    hook: Option<&StatuslineHookInput>,
    session_totals: Option<&TokenCounts>,
    today_totals: &TokenCounts,
    block: Option<&ActiveBlockSummary>,
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

    parts.join(" | ")
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

fn print_membership_estimate(
    estimate: &Option<MembershipEstimate>,
    token_limit: Option<u64>,
    token_limit_source: TokenLimitSource,
) {
    let Some(estimate) = estimate else {
        return;
    };

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
    match tz {
        TimeZoneMode::Local => ts.with_timezone(&Local).date_naive(),
        TimeZoneMode::Utc => ts.date_naive(),
        TimeZoneMode::Named(tz) => ts.with_timezone(tz).date_naive(),
    }
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

async fn load_usage(common: &CommonArgs, timezone: &TimeZoneMode) -> Result<LoadedUsage> {
    let filter = DateFilter {
        since: parse_date_filter(common.since.as_deref())?,
        until: parse_date_filter(common.until.as_deref())?,
    };

    if let (Some(since), Some(until)) = (filter.since, filter.until)
        && since > until
    {
        bail!("--since must be earlier than or equal to --until");
    }

    let sources = build_sources(common).await?;
    if sources.is_empty() {
        bail!(
            "No valid source directories found. Please provide --claude-projects-dir/--codex-sessions-dir."
        );
    }

    let pricing = Arc::new(load_pricing(common.pricing_file.as_deref(), common.offline).await?);
    let ignore_rules = PathIgnoreRules::from_common(common);

    let files = discover_files(&sources, &ignore_rules);
    let worker_count = common.workers.unwrap_or_else(|| {
        thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    });

    let stats = Arc::new(ParseStatsAtomic::default());
    stats.files_discovered.store(files.len(), Ordering::Relaxed);

    let pricing_key = pricing_cache_key(&pricing);
    let cache_path = incremental_cache_path();
    let cache_enabled = !common.no_incremental_cache;

    let mut cache_store = if cache_enabled {
        match cache_path.as_ref() {
            Some(path) => load_incremental_cache(path, &pricing_key),
            None => IncrementalCacheStore::new(pricing_key.clone()),
        }
    } else {
        IncrementalCacheStore::new(pricing_key.clone())
    };

    if common.rebuild_cache {
        cache_store = IncrementalCacheStore::new(pricing_key.clone());
    }

    let mut cache_dirty = common.rebuild_cache;
    let mut seen_cache_keys = HashSet::with_capacity(files.len());
    let mut parse_jobs = Vec::new();
    let mut events = Vec::new();

    for file in files {
        let key = cache_file_key(&file.path);
        seen_cache_keys.insert(key.clone());

        let Some(fingerprint) = read_file_fingerprint(&file.path) else {
            parse_jobs.push(FileParseJob {
                file,
                cache_key: key,
                fingerprint: FileFingerprint {
                    size: 0,
                    modified_unix_secs: 0,
                    modified_unix_nanos: 0,
                },
            });
            continue;
        };

        if cache_enabled
            && let Some(cached) = cache_store.files.get(&key)
            && cached.fingerprint == fingerprint
        {
            events.extend(hydrate_cached_events(
                &file, cached, filter, timezone, &stats,
            ));
            continue;
        }

        parse_jobs.push(FileParseJob {
            file,
            cache_key: key,
            fingerprint,
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

        if cache_dirty && let Some(path) = cache_path.as_ref() {
            save_incremental_cache(path, &cache_store);
        }
    }

    events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    Ok(LoadedUsage {
        events,
        stats: stats.snapshot(),
    })
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
                home.join(".config").join("codex").join("sessions"),
            ]
        } else {
            common
                .codex_sessions_dir
                .iter()
                .map(|p| expand_user_path(p))
                .collect()
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

fn discover_files(sources: &[SourceConfig], ignore_rules: &PathIgnoreRules) -> Vec<DiscoveredFile> {
    let mut files: Vec<DiscoveredFile> = sources
        .par_iter()
        .flat_map_iter(|source| {
            source
                .roots
                .iter()
                .flat_map(move |root| discover_files_in_root(source.kind, root, ignore_rules))
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
) -> Vec<DiscoveredFile> {
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

    let reader = BufReader::new(input);
    let mut codex_state = CodexParseState::default();
    let mut local_events = Vec::new();
    let mut cached_events = Vec::new();
    let mut reader = reader;
    let mut line = String::new();
    let mut lines_total = 0usize;
    let mut lines_invalid_json = 0usize;
    let mut lines_missing_usage = 0usize;
    let mut lines_unknown_pricing = 0usize;
    let mut lines_filtered = 0usize;
    let mut lines_parsed = 0usize;

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

        let parsed = match job.file.source {
            SourceKind::Claude => parse_claude_usage_line(&line, pricing),
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
            lines_total,
            lines_invalid_json,
            lines_missing_usage,
            lines_unknown_pricing,
        },
        events: cached_events,
    };

    Some(ParsedFileOutput {
        events: local_events,
        cache_entry,
    })
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

fn parse_claude_usage_line(line: &str, pricing: &PricingTable) -> ParseLineResult {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return ParseLineResult::InvalidJson,
    };

    let Some(timestamp) = extract_timestamp(&value) else {
        return ParseLineResult::MissingUsage;
    };
    let Some(model) = extract_model(&value, SourceKind::Claude) else {
        return ParseLineResult::MissingUsage;
    };

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
    Some(base.join("tokenusage").join("parse-cache-v1.json"))
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
