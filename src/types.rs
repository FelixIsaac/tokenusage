use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Which AI coding assistant produced a log entry.
///
/// Each usage event is tagged with a source so reports can break down
/// token consumption by tool.
///
/// # Serialisation
///
/// Serialises to / deserialises from `"Claude"` or `"Codex"` (PascalCase).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub enum SourceKind {
    /// Anthropic Claude Code (`~/.claude/projects/*/`).
    Claude,
    /// OpenAI Codex CLI (`~/.codex/sessions/`).
    Codex,
}

impl SourceKind {
    /// Lowercase string label — `"claude"` or `"codex"`.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Claude => "claude",
            SourceKind::Codex => "codex",
        }
    }
}

/// A configured log source: kind + root directories to scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceConfig {
    pub kind: SourceKind,
    pub roots: Vec<PathBuf>,
}

/// A single JSONL file discovered during source scanning.
#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub source: SourceKind,
    pub root: PathBuf,
    pub path: PathBuf,
}

/// Running accumulator for token counts and estimated cost.
///
/// This is the mutable workhorse used during parsing.  Call
/// [`to_counts`](UsageAccumulator::to_counts) to freeze it into a
/// serialisable [`TokenCounts`] snapshot.
///
/// # Token categories
///
/// | Field | Meaning |
/// |-------|---------|
/// | `input_tokens` | Fresh (non-cached) input tokens |
/// | `cache_creation_input_tokens` | Tokens written to the prompt cache |
/// | `cache_read_input_tokens` | Tokens served from the prompt cache |
/// | `output_tokens` | Model output tokens (includes reasoning unless split) |
/// | `reasoning_output_tokens` | Reasoning/chain-of-thought tokens (subset of output when billed separately) |
/// | `cost_usd` | Estimated USD cost based on model pricing |
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct UsageAccumulator {
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub cost_usd: f64,
}

impl UsageAccumulator {
    /// Add another accumulator's values to this one.
    pub fn add(&mut self, other: UsageAccumulator) {
        self.input_tokens += other.input_tokens;
        self.cache_creation_input_tokens += other.cache_creation_input_tokens;
        self.cache_read_input_tokens += other.cache_read_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
        self.cost_usd += other.cost_usd;
    }

    /// Sum of input + cache-creation + cache-read + output tokens.
    ///
    /// This counts every token that passed through the model, regardless of
    /// billing tier.  Reasoning tokens are already included in `output_tokens`.
    pub fn total_tokens(self) -> u64 {
        self.input_tokens
            + self.cache_creation_input_tokens
            + self.cache_read_input_tokens
            + self.output_tokens
    }

    /// Freeze this accumulator into a serialisable [`TokenCounts`] snapshot.
    pub fn to_counts(self) -> TokenCounts {
        TokenCounts {
            input_tokens: self.input_tokens,
            cache_creation_input_tokens: self.cache_creation_input_tokens,
            cache_read_input_tokens: self.cache_read_input_tokens,
            output_tokens: self.output_tokens,
            reasoning_output_tokens: self.reasoning_output_tokens,
            total_tokens: self.total_tokens(),
            cost_usd: self.cost_usd,
        }
    }
}

/// Immutable token count snapshot with pre-computed total.
///
/// This is the serialisable form of [`UsageAccumulator`], used in report
/// output (JSON, table rows, etc.).  The `total_tokens` field is eagerly
/// computed so consumers do not need to sum the components themselves.
///
/// Implements `Serialize` + `Deserialize` for JSON round-tripping.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenCounts {
    /// Fresh (non-cached) prompt tokens.
    pub input_tokens: u64,
    /// Tokens written to the prompt cache.
    pub cache_creation_input_tokens: u64,
    /// Tokens served from the prompt cache.
    pub cache_read_input_tokens: u64,
    /// Model output tokens (includes reasoning tokens unless billed separately).
    pub output_tokens: u64,
    /// Reasoning/chain-of-thought tokens (subset of output).
    pub reasoning_output_tokens: u64,
    /// Pre-computed `input + cache_creation + cache_read + output`.
    pub total_tokens: u64,
    /// Estimated cost in US dollars.
    pub cost_usd: f64,
}

impl TokenCounts {
    /// Accumulate another `TokenCounts` into this one (mutable addition).
    pub fn add_assign(&mut self, other: TokenCounts) {
        self.input_tokens += other.input_tokens;
        self.cache_creation_input_tokens += other.cache_creation_input_tokens;
        self.cache_read_input_tokens += other.cache_read_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
        self.total_tokens += other.total_tokens;
        self.cost_usd += other.cost_usd;
    }
}

/// Coding activity summary derived from heartbeat data.
///
/// When [`Config::with_activity`](crate::Config::with_activity) is enabled,
/// each [`DailyRow`] may include an `ActivitySummary` showing how much
/// coding time was tracked alongside the token usage.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActivitySummary {
    /// Total coding seconds in the period.
    pub total_seconds: u64,
    /// Human-readable duration string (e.g. `"2h 15m"`).
    pub text: String,
    /// Most active project name, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_project: Option<String>,
    /// Most used programming language, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_language: Option<String>,
    /// Most frequent heartbeat source, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_source: Option<String>,
}

/// A single parsed token usage event from a log file.
///
/// Each event represents one API request/response pair recorded by an AI
/// coding assistant.  Events carry the timestamp, model name, session
/// identifier, and per-category token counts.
///
/// # Example
///
/// ```no_run
/// use tokenusage::Config;
///
/// # async fn example() -> anyhow::Result<()> {
/// let events = tokenusage::load_events(Config::default()).await?;
/// for e in &events {
///     println!("[{}] {}: {} in + {} out = {} total (${:.6})",
///         e.timestamp.format("%Y-%m-%d %H:%M"),
///         e.model,
///         e.usage.input_tokens,
///         e.usage.output_tokens,
///         e.usage.total_tokens(),
///         e.usage.cost_usd,
///     );
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct UsageEvent {
    /// When the API call was made (UTC).
    pub timestamp: DateTime<Utc>,
    /// Which tool produced this event.
    pub source: SourceKind,
    /// Model identifier (e.g. `"claude-sonnet-4-20250514"`).
    pub model: String,
    /// Session or conversation identifier.
    pub session: String,
    /// Project name, if detectable from the log path.
    pub project: Option<String>,
    /// Absolute path to the source log file.
    pub file_path: String,
    /// Token counts and estimated cost for this single event.
    pub usage: UsageAccumulator,
}

/// Inclusive date range filter.
///
/// Both bounds are optional.  When a bound is `None`, that side is unbounded.
///
/// Used internally during event filtering, and exposed for library consumers
/// who want to build custom filters.
#[derive(Debug, Clone, Copy)]
pub struct DateFilter {
    /// Earliest date to include (inclusive).
    pub since: Option<NaiveDate>,
    /// Latest date to include (inclusive).
    pub until: Option<NaiveDate>,
}

impl DateFilter {
    /// Check whether a given date falls within this filter's range.
    pub fn allows(self, day: NaiveDate) -> bool {
        if let Some(since) = self.since
            && day < since
        {
            return false;
        }
        if let Some(until) = self.until
            && day > until
        {
            return false;
        }
        true
    }
}

/// Thread-safe atomic counters for parse progress tracking.
///
/// Used internally during parallel parsing.  Call
/// [`snapshot`](ParseStatsAtomic::snapshot) to freeze into a [`ParseStats`].
#[derive(Debug, Default)]
pub struct ParseStatsAtomic {
    pub files_discovered: AtomicUsize,
    pub files_open_failed: AtomicUsize,
    pub lines_total: AtomicUsize,
    pub lines_parsed: AtomicUsize,
    pub lines_filtered: AtomicUsize,
    pub lines_invalid_json: AtomicUsize,
    pub lines_missing_usage: AtomicUsize,
    pub lines_unknown_pricing: AtomicUsize,
}

/// Parsing diagnostics — how many files/lines were processed and why some
/// were skipped.
///
/// Included in every [`DailyReport`] and [`UsageSnapshot`](crate::UsageSnapshot)
/// so library consumers can detect data quality issues.
///
/// # Fields
///
/// | Counter | Meaning |
/// |---------|---------|
/// | `files_discovered` | Total JSONL files found during source scanning |
/// | `files_open_failed` | Files that could not be opened (permissions, etc.) |
/// | `lines_total` | Total lines read across all files |
/// | `lines_parsed` | Lines successfully parsed into [`UsageEvent`]s |
/// | `lines_filtered` | Lines excluded by date or source filters |
/// | `lines_invalid_json` | Lines that were not valid JSON |
/// | `lines_missing_usage` | JSON lines without token usage fields |
/// | `lines_unknown_pricing` | Events where the model had no pricing data |
#[derive(Debug, Clone, Serialize)]
pub struct ParseStats {
    pub files_discovered: usize,
    pub files_open_failed: usize,
    pub lines_total: usize,
    pub lines_parsed: usize,
    pub lines_filtered: usize,
    pub lines_invalid_json: usize,
    pub lines_missing_usage: usize,
    pub lines_unknown_pricing: usize,
}

impl ParseStatsAtomic {
    /// Freeze atomic counters into a plain [`ParseStats`] snapshot.
    pub fn snapshot(&self) -> ParseStats {
        ParseStats {
            files_discovered: self.files_discovered.load(Ordering::Relaxed),
            files_open_failed: self.files_open_failed.load(Ordering::Relaxed),
            lines_total: self.lines_total.load(Ordering::Relaxed),
            lines_parsed: self.lines_parsed.load(Ordering::Relaxed),
            lines_filtered: self.lines_filtered.load(Ordering::Relaxed),
            lines_invalid_json: self.lines_invalid_json.load(Ordering::Relaxed),
            lines_missing_usage: self.lines_missing_usage.load(Ordering::Relaxed),
            lines_unknown_pricing: self.lines_unknown_pricing.load(Ordering::Relaxed),
        }
    }
}

/// Per-model pricing rates in USD per million tokens.
///
/// Supports tiered pricing where costs differ above a token threshold
/// (e.g. Anthropic's 200K-token tier).
///
/// # Tiered pricing
///
/// If any `*_above_per_million` field is set, the
/// `tier_threshold_tokens` value (default 200,000) determines the cutoff.
/// Tokens below the threshold use the base rate; tokens above use the
/// `*_above_per_million` rate.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PricingRate {
    /// Base cost per million input tokens.
    pub input_per_million: f64,
    /// Base cost per million output tokens.
    pub output_per_million: f64,
    /// Base cost per million cache-creation input tokens.
    #[serde(default)]
    pub cache_creation_per_million: f64,
    /// Base cost per million cache-read input tokens.
    #[serde(default)]
    pub cache_read_per_million: f64,
    /// Base cost per million reasoning output tokens.
    #[serde(default)]
    pub reasoning_output_per_million: f64,
    /// Token count threshold for tiered pricing.
    #[serde(default)]
    pub tier_threshold_tokens: Option<u64>,
    /// Above-threshold cost per million input tokens.
    #[serde(default)]
    pub input_above_per_million: Option<f64>,
    /// Above-threshold cost per million output tokens.
    #[serde(default)]
    pub output_above_per_million: Option<f64>,
    /// Above-threshold cost per million cache-creation tokens.
    #[serde(default)]
    pub cache_creation_above_per_million: Option<f64>,
    /// Above-threshold cost per million cache-read tokens.
    #[serde(default)]
    pub cache_read_above_per_million: Option<f64>,
    /// Above-threshold cost per million reasoning output tokens.
    #[serde(default)]
    pub reasoning_output_above_per_million: Option<f64>,
}

/// Model pricing lookup table with exact-match and prefix-match entries.
///
/// The table first checks for an exact model name match, then falls back
/// to the longest-prefix match.  This allows e.g. `"claude-sonnet"` to
/// cover all Sonnet variants while still supporting per-version overrides.
///
/// # Built-in defaults
///
/// [`PricingTable::default_table`] returns a table with approximate
/// pricing for common Claude and Codex models, suitable for offline
/// estimation.
#[derive(Debug, Clone, Default)]
pub struct PricingTable {
    /// Exact model name → pricing rate.
    pub exact: HashMap<String, PricingRate>,
    /// Prefix → pricing rate, sorted longest-first for greedy matching.
    pub prefixes: Vec<(String, PricingRate)>,
}

impl PricingTable {
    /// Create a table with built-in approximate pricing for common models.
    ///
    /// Includes fallback rates for `claude-opus`, `claude-sonnet`,
    /// `claude-haiku`, `gpt-5-codex`, and `gpt-5`.
    pub fn default_table() -> Self {
        // Approximate defaults for offline estimation.
        let mut table = PricingTable::default();

        table.prefixes.push((
            "claude-opus".to_string(),
            PricingRate {
                input_per_million: 15.0,
                output_per_million: 75.0,
                cache_creation_per_million: 18.75,
                cache_read_per_million: 1.5,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "claude-sonnet".to_string(),
            PricingRate {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_creation_per_million: 3.75,
                cache_read_per_million: 0.3,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "claude-haiku".to_string(),
            PricingRate {
                input_per_million: 0.8,
                output_per_million: 4.0,
                cache_creation_per_million: 1.0,
                cache_read_per_million: 0.08,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "gpt-5-codex".to_string(),
            PricingRate {
                input_per_million: 1.5,
                output_per_million: 6.0,
                cache_creation_per_million: 0.0,
                cache_read_per_million: 0.0,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "gpt-5".to_string(),
            PricingRate {
                input_per_million: 1.25,
                output_per_million: 10.0,
                cache_creation_per_million: 0.0,
                cache_read_per_million: 0.0,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));

        table
            .prefixes
            .sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()));

        table
    }

    /// Merge exact-match pricing overrides into this table.
    pub fn merge_exact(&mut self, overrides: HashMap<String, PricingRate>) {
        for (model, rate) in overrides {
            self.exact.insert(canonical_model_key(&model), rate);
        }
    }

    /// Look up the pricing rate for a model name.
    ///
    /// Tries exact match first, then longest prefix match.
    /// Returns `None` if no pricing is available for this model.
    pub fn find_rate(&self, model: &str) -> Option<&PricingRate> {
        let model_key = canonical_model_key(model);
        if let Some(rate) = self.exact.get(&model_key) {
            return Some(rate);
        }
        self.prefixes
            .iter()
            .find_map(|(prefix, rate)| model_key.starts_with(prefix).then_some(rate))
    }

    /// Estimate the USD cost of a set of token counts for a given model.
    ///
    /// Returns `None` if no pricing data is available for the model.
    pub fn estimate_cost(&self, model: &str, usage: UsageAccumulator) -> Option<f64> {
        let rate = self.find_rate(model)?;
        let threshold = tier_threshold(rate);
        let separate_reasoning_pricing = has_separate_reasoning_pricing(rate);
        let reasoning_tokens = if separate_reasoning_pricing {
            usage.reasoning_output_tokens.min(usage.output_tokens)
        } else {
            0
        };
        let output_tokens = if separate_reasoning_pricing {
            usage.output_tokens.saturating_sub(reasoning_tokens)
        } else {
            usage.output_tokens
        };

        Some(
            component_cost(
                usage.input_tokens,
                rate.input_per_million,
                rate.input_above_per_million,
                threshold,
            ) + component_cost(
                usage.cache_creation_input_tokens,
                rate.cache_creation_per_million,
                rate.cache_creation_above_per_million,
                threshold,
            ) + component_cost(
                usage.cache_read_input_tokens,
                rate.cache_read_per_million,
                rate.cache_read_above_per_million,
                threshold,
            ) + component_cost(
                output_tokens,
                rate.output_per_million,
                rate.output_above_per_million,
                threshold,
            ) + component_cost(
                reasoning_tokens,
                rate.reasoning_output_per_million,
                rate.reasoning_output_above_per_million,
                threshold,
            ),
        )
    }
}

fn tier_threshold(rate: &PricingRate) -> Option<u64> {
    let has_above_tiers = rate.input_above_per_million.is_some()
        || rate.output_above_per_million.is_some()
        || rate.cache_creation_above_per_million.is_some()
        || rate.cache_read_above_per_million.is_some()
        || rate.reasoning_output_above_per_million.is_some();

    if has_above_tiers {
        Some(rate.tier_threshold_tokens.unwrap_or(200_000))
    } else {
        None
    }
}

fn has_separate_reasoning_pricing(rate: &PricingRate) -> bool {
    rate.reasoning_output_per_million > 0.0
        || rate
            .reasoning_output_above_per_million
            .is_some_and(|v| v > 0.0)
}

fn component_cost(
    tokens: u64,
    base_per_million: f64,
    above_per_million: Option<f64>,
    threshold: Option<u64>,
) -> f64 {
    if tokens == 0 {
        return 0.0;
    }

    let million = 1_000_000.0;
    let Some(above_per_million) = above_per_million else {
        return (tokens as f64 / million) * base_per_million;
    };

    let threshold = threshold.unwrap_or(200_000);
    let base_tokens = tokens.min(threshold);
    let above_tokens = tokens.saturating_sub(threshold);

    (base_tokens as f64 / million) * base_per_million
        + (above_tokens as f64 / million) * above_per_million
}

fn canonical_model_key(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

#[derive(Debug)]
pub struct ParsedLine {
    pub event: UsageEvent,
    pub used_unknown_pricing: bool,
}

#[derive(Debug)]
pub enum ParseLineResult {
    Parsed(ParsedLine),
    InvalidJson,
    MissingUsage,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct CodexRawUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
}

impl CodexRawUsage {
    pub fn is_zero(self) -> bool {
        self.input_tokens == 0
            && self.cached_input_tokens == 0
            && self.output_tokens == 0
            && self.reasoning_output_tokens == 0
    }
}

#[derive(Debug, Default)]
pub struct CodexParseState {
    pub current_model: Option<String>,
    pub current_model_is_fallback: bool,
    pub previous_totals: Option<CodexRawUsage>,
}

pub const LEGACY_CODEX_FALLBACK_MODEL: &str = "gpt-5";

/// A single row in a daily/weekly/monthly usage report.
///
/// Each row aggregates all events for one time period.  When `instances`
/// is requested, there is one row per session instead.
#[derive(Debug, Clone, Serialize)]
pub struct DailyRow {
    /// Date label — `"2025-06-15"` for daily, `"2025-06"` for monthly, etc.
    pub date: String,
    /// Aggregate token counts and cost for this period.
    pub totals: TokenCounts,
    /// Per-model breakdown: model name → token counts.
    pub models: BTreeMap<String, TokenCounts>,
    /// Per-source breakdown: `"claude"` / `"codex"` → token counts.
    pub sources: BTreeMap<String, TokenCounts>,
    /// Coding activity summary (only present when `with_activity` is enabled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<ActivitySummary>,
}

/// Complete usage report with per-period rows and grand totals.
///
/// Returned by [`daily_report`](crate::daily_report) and
/// [`daily_report_with_week_start`](crate::daily_report_with_week_start).
///
/// # JSON output
///
/// This struct implements `Serialize`, so you can convert it directly to
/// JSON with `serde_json::to_string(&report)?`.
#[derive(Debug, Clone, Serialize)]
pub struct DailyReport {
    /// Per-period rows (daily, weekly, or monthly depending on the request).
    pub daily: Vec<DailyRow>,
    /// Grand total across all periods.
    pub totals: TokenCounts,
    /// Grand total coding activity (if `with_activity` was enabled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_totals: Option<ActivitySummary>,
    /// Parsing diagnostics for this report.
    pub stats: ParseStats,
}

#[derive(Debug, Clone, Copy)]
pub enum TableLayout {
    Compact,
    Standard,
    Full,
}

impl TableLayout {
    pub fn from_terminal_width(width: usize) -> Self {
        if width <= 110 {
            Self::Compact
        } else if width <= 160 {
            Self::Standard
        } else {
            Self::Full
        }
    }

    pub fn model_line_limit(self) -> usize {
        match self {
            Self::Compact => 2,
            Self::Standard => 4,
            Self::Full => 6,
        }
    }

    pub fn model_char_limit(self) -> usize {
        match self {
            Self::Compact => 24,
            Self::Standard => 38,
            Self::Full => 56,
        }
    }
}
