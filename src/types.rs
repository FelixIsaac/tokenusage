use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::ReportInsights;

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
    /// Google Gemini CLI (`~/.gemini/tmp/**/chats/*.jsonl`).
    Gemini,
    /// OpenCode (`~/.local/share/opencode/storage/message/**/*.json`).
    OpenCode,
    /// Grok Build CLI (`~/.grok/sessions/**/updates.jsonl` or `~/.grok/logs/unified.jsonl`).
    Grok,
    /// Pi coding agent (`~/.pi/agent/sessions/**/*.jsonl`).
    Pi,
    /// Muse Code (`$MUSE_HOME/sessions/**/session.jsonl`).
    Muse,
}

impl SourceKind {
    pub const fn all() -> [SourceKind; 7] {
        [
            SourceKind::Claude,
            SourceKind::Codex,
            SourceKind::Gemini,
            SourceKind::OpenCode,
            SourceKind::Grok,
            SourceKind::Pi,
            SourceKind::Muse,
        ]
    }

    /// Lowercase string label — `"claude"` or `"codex"`.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Claude => "claude",
            SourceKind::Codex => "codex",
            SourceKind::Gemini => "gemini",
            SourceKind::OpenCode => "opencode",
            SourceKind::Grok => "grok",
            SourceKind::Pi => "pi",
            SourceKind::Muse => "muse",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            SourceKind::Claude => "Claude",
            SourceKind::Codex => "Codex",
            SourceKind::Gemini => "Antigravity",
            SourceKind::OpenCode => "OpenCode",
            SourceKind::Grok => "Grok",
            SourceKind::Pi => "Pi",
            SourceKind::Muse => "Muse",
        }
    }

    pub const fn supports_official_limits(self) -> bool {
        matches!(
            self,
            SourceKind::Claude | SourceKind::Codex | SourceKind::Gemini
        )
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
/// Token metrics collected for a single usage record or aggregated bucket.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
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

    /// Add another accumulator's token counts and cost into this one.
    pub fn merge(&mut self, other: UsageAccumulator) {
        self.add(other);
    }

    /// Sum of input + cache-creation + cache-read + output tokens.
    ///
    /// Output tokens already encompass generated reasoning/thinking tokens
    /// across model APIs (Anthropic, OpenAI, xAI, Gemini).
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
    /// Model output tokens (excluding reasoning tokens when tracked separately).
    pub output_tokens: u64,
    /// Reasoning/chain-of-thought tokens (may be billed separately depending on model).
    pub reasoning_output_tokens: u64,
    /// Pre-computed `input + cache_creation + cache_read + output + reasoning`.
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
#[derive(Debug, Clone, Copy, Default)]
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
#[derive(Debug, Clone, Serialize, Default)]
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

/// Service speed tier for API requests (standard, priority/fast, ultrafast).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceSpeed {
    #[default]
    Standard,
    Fast,
    Ultrafast,
}

/// Token rates in USD per million tokens for a single speed/context tier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenRates {
    pub input_per_million: f64,
    pub output_per_million: f64,
    #[serde(default)]
    pub cache_creation_per_million: f64,
    #[serde(default)]
    pub cache_creation_1h_per_million: f64,
    #[serde(default)]
    pub cache_read_per_million: f64,
    #[serde(default)]
    pub reasoning_output_per_million: f64,
}

/// Whole-request context-window pricing tier (activated when per-call prompt tokens > `threshold_tokens`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextTierRate {
    pub threshold_tokens: u64,
    pub standard: TokenRates,
    #[serde(default)]
    pub fast: Option<TokenRates>,
    #[serde(default)]
    pub ultrafast: Option<TokenRates>,
}

/// Extra per-request billing metadata needed for exact pricing beyond base token counts.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PricingContext {
    /// Subset of `cache_creation_input_tokens` written to the 1-hour ephemeral cache (2.0x input).
    pub cache_creation_1h_tokens: u64,
    /// Service speed tier (`standard`, `fast`/`priority`, `ultrafast`).
    pub speed: ServiceSpeed,
    /// Number of underlying API calls aggregated into this event (`>= 1`).
    /// Used to avoid falsely triggering single-call context-window cliffs on multi-call summaries.
    pub model_calls: u32,
    /// Provider-reported authoritative cost in USD (e.g. Grok `costUsdTicks` or Claude `costUSD`).
    pub reported_cost_usd: Option<f64>,
}

/// Per-model pricing rates in USD per million tokens.
///
/// Supports tiered pricing where costs differ above a prompt-token threshold
/// (e.g. 200K or 272K context-window cliffs), 1-hour cache writes, and
/// priority/fast service tiers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PricingRate {
    /// Base cost per million input tokens.
    pub input_per_million: f64,
    /// Base cost per million output tokens.
    pub output_per_million: f64,
    /// Base cost per million cache-creation input tokens (5-minute TTL).
    #[serde(default)]
    pub cache_creation_per_million: f64,
    /// Base cost per million 1-hour cache-creation input tokens.
    #[serde(default)]
    pub cache_creation_1h_per_million: Option<f64>,
    /// Base cost per million cache-read input tokens.
    #[serde(default)]
    pub cache_read_per_million: f64,
    /// Base cost per million reasoning output tokens.
    #[serde(default)]
    pub reasoning_output_per_million: f64,
    /// Token count threshold for legacy tiered pricing.
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
    /// Fast / priority speed-tier rates.
    #[serde(default)]
    pub fast: Option<Box<TokenRates>>,
    /// Ultrafast speed-tier rates.
    #[serde(default)]
    pub ultrafast: Option<Box<TokenRates>>,
    /// Whole-request context-window cliff tiers, sorted descending by `threshold_tokens`.
    #[serde(default)]
    pub context_tiers: Vec<ContextTierRate>,
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
                cache_creation_1h_per_million: Some(30.0),
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
                cache_creation_1h_per_million: Some(6.0),
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
                cache_creation_1h_per_million: Some(1.6),
                cache_read_per_million: 0.08,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "claude-3-7-sonnet".to_string(),
            PricingRate {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_creation_per_million: 3.75,
                cache_creation_1h_per_million: Some(6.0),
                cache_read_per_million: 0.3,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "claude-3-5-sonnet".to_string(),
            PricingRate {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_creation_per_million: 3.75,
                cache_creation_1h_per_million: Some(6.0),
                cache_read_per_million: 0.3,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "claude-3-5-haiku".to_string(),
            PricingRate {
                input_per_million: 0.8,
                output_per_million: 4.0,
                cache_creation_per_million: 1.0,
                cache_creation_1h_per_million: Some(1.6),
                cache_read_per_million: 0.08,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "claude-3-haiku".to_string(),
            PricingRate {
                input_per_million: 0.25,
                output_per_million: 1.25,
                cache_creation_per_million: 0.30,
                cache_creation_1h_per_million: Some(0.50),
                cache_read_per_million: 0.03,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "claude-3-opus".to_string(),
            PricingRate {
                input_per_million: 15.0,
                output_per_million: 75.0,
                cache_creation_per_million: 18.75,
                cache_creation_1h_per_million: Some(30.0),
                cache_read_per_million: 1.5,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "gemini-3.1-pro".to_string(),
            PricingRate {
                input_per_million: 2.0,
                output_per_million: 12.0,
                cache_creation_per_million: 0.0,
                cache_read_per_million: 0.2,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "gemini-3-pro".to_string(),
            PricingRate {
                input_per_million: 1.25,
                output_per_million: 5.0,
                cache_creation_per_million: 0.0,
                cache_read_per_million: 0.3,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "gemini-3.6-flash".to_string(),
            PricingRate {
                input_per_million: 0.1,
                output_per_million: 0.4,
                cache_creation_per_million: 0.0,
                cache_read_per_million: 0.025,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "gemini-2.5-flash".to_string(),
            PricingRate {
                input_per_million: 0.1,
                output_per_million: 0.4,
                cache_creation_per_million: 0.0,
                cache_read_per_million: 0.025,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "gemini-flash".to_string(),
            PricingRate {
                input_per_million: 0.1,
                output_per_million: 0.4,
                cache_creation_per_million: 0.0,
                cache_read_per_million: 0.025,
                reasoning_output_per_million: 0.0,
                ..PricingRate::default()
            },
        ));
        table.prefixes.push((
            "gemini-pro".to_string(),
            PricingRate {
                input_per_million: 1.25,
                output_per_million: 5.0,
                cache_creation_per_million: 0.0,
                cache_read_per_million: 0.3,
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

        // xAI Grok 4.5: $2/M input, $0.30/M cached input, $6/M output under
        // 200K prompt tokens; doubles above that threshold.
        table.prefixes.push((
            "grok-4.5".to_string(),
            PricingRate {
                input_per_million: 2.0,
                output_per_million: 6.0,
                cache_creation_per_million: 0.0,
                cache_read_per_million: 0.3,
                reasoning_output_per_million: 0.0,
                tier_threshold_tokens: Some(200_000),
                input_above_per_million: Some(4.0),
                output_above_per_million: Some(12.0),
                cache_creation_above_per_million: None,
                cache_read_above_per_million: Some(0.6),
                reasoning_output_above_per_million: None,
                ..PricingRate::default()
            },
        ));

        // xAI Grok 4.6: $2/M input, $0.50/M cached input, $6/M output under
        // 200K prompt tokens; doubles above threshold.
        table.prefixes.push((
            "grok-4.6".to_string(),
            PricingRate {
                input_per_million: 2.0,
                output_per_million: 6.0,
                cache_creation_per_million: 0.0,
                cache_read_per_million: 0.5,
                reasoning_output_per_million: 0.0,
                tier_threshold_tokens: Some(200_000),
                input_above_per_million: Some(4.0),
                output_above_per_million: Some(12.0),
                cache_creation_above_per_million: None,
                cache_read_above_per_million: Some(1.0),
                reasoning_output_above_per_million: None,
                ..PricingRate::default()
            },
        ));

        table.prefixes.push((
            "claude-opus-5-5".to_string(),
            PricingRate {
                input_per_million: 4.0,
                output_per_million: 20.0,
                cache_creation_per_million: 5.0,
                cache_creation_1h_per_million: Some(8.0),
                cache_read_per_million: 0.2,
                reasoning_output_per_million: 0.0,
                fast: Some(Box::new(TokenRates {
                    input_per_million: 8.0,
                    output_per_million: 40.0,
                    cache_creation_per_million: 10.0,
                    cache_creation_1h_per_million: 16.0,
                    cache_read_per_million: 0.4,
                    reasoning_output_per_million: 0.0,
                })),
                ..PricingRate::default()
            },
        ));

        table.prefixes.push((
            "gpt-5.4".to_string(),
            PricingRate {
                input_per_million: 2.5,
                output_per_million: 15.0,
                cache_creation_per_million: 0.0,
                cache_read_per_million: 0.25,
                reasoning_output_per_million: 0.0,
                fast: Some(Box::new(TokenRates {
                    input_per_million: 5.0,
                    output_per_million: 30.0,
                    cache_creation_per_million: 0.0,
                    cache_creation_1h_per_million: 0.0,
                    cache_read_per_million: 0.5,
                    reasoning_output_per_million: 0.0,
                })),
                context_tiers: vec![ContextTierRate {
                    threshold_tokens: 272_000,
                    standard: TokenRates {
                        input_per_million: 5.0,
                        output_per_million: 22.5,
                        cache_creation_per_million: 0.0,
                        cache_creation_1h_per_million: 0.0,
                        cache_read_per_million: 0.5,
                        reasoning_output_per_million: 0.0,
                    },
                    fast: Some(TokenRates {
                        input_per_million: 10.0,
                        output_per_million: 45.0,
                        cache_creation_per_million: 0.0,
                        cache_creation_1h_per_million: 0.0,
                        cache_read_per_million: 1.0,
                        reasoning_output_per_million: 0.0,
                    }),
                    ultrafast: None,
                }],
                ..PricingRate::default()
            },
        ));

        table.prefixes.push((
            "gpt-5.3-codex".to_string(),
            PricingRate {
                input_per_million: 1.75,
                output_per_million: 14.0,
                cache_creation_per_million: 0.0,
                cache_read_per_million: 0.175,
                reasoning_output_per_million: 0.0,
                fast: Some(Box::new(TokenRates {
                    input_per_million: 3.5,
                    output_per_million: 28.0,
                    cache_creation_per_million: 0.0,
                    cache_creation_1h_per_million: 0.0,
                    cache_read_per_million: 0.35,
                    reasoning_output_per_million: 0.0,
                })),
                ..PricingRate::default()
            },
        ));

        table
            .prefixes
            .sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.len()));

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
    /// Checks exact & aliased-exact matches before falling back to longest-prefix
    /// matches so reasoning-effort variants (e.g. `claude-opus-5-5-medium`) hit
    /// the exact model rate rather than an outdated family prefix.
    /// Returns `None` if no pricing is available or the model is an unpriceable placeholder.
    pub fn find_rate(&self, model: &str) -> Option<&PricingRate> {
        let model_key = canonical_model_key_fast(model);
        let bare = model_key
            .rsplit_once('/')
            .map(|(_, tail)| tail)
            .unwrap_or(model_key.as_ref());
        if bare.is_empty() || is_unpriceable_model(bare) {
            return None;
        }

        // 1. Exact match (including YYYYMMDD date-suffix strip).
        if let Some(rate) = lookup_exact_or_date(&self.exact, model_key.as_ref()) {
            return Some(rate);
        }

        // 2. Aliased exact match (strips effort suffixes / -build, maps preview names).
        let aliased = model_pricing_alias(model_key.as_ref());
        if aliased.as_ref() != model_key.as_ref()
            && let Some(rate) = lookup_exact_or_date(&self.exact, aliased.as_ref())
        {
            return Some(rate);
        }

        // 3. Bare model name exact match (when input carried a provider prefix).
        if bare != model_key.as_ref() {
            if let Some(rate) = lookup_exact_or_date(&self.exact, bare) {
                return Some(rate);
            }
            let bare_aliased = model_pricing_alias(bare);
            if bare_aliased.as_ref() != bare
                && let Some(rate) = lookup_exact_or_date(&self.exact, bare_aliased.as_ref())
            {
                return Some(rate);
            }
        }

        // 4. Prefix fallbacks (raw, aliased, then bare).
        if let Some(rate) = self
            .prefixes
            .iter()
            .find_map(|(prefix, rate)| model_key.starts_with(prefix.as_str()).then_some(rate))
        {
            return Some(rate);
        }
        if aliased.as_ref() != model_key.as_ref()
            && let Some(rate) = self
                .prefixes
                .iter()
                .find_map(|(prefix, rate)| aliased.starts_with(prefix.as_str()).then_some(rate))
        {
            return Some(rate);
        }
        if bare != model_key.as_ref() {
            if let Some(rate) = self
                .prefixes
                .iter()
                .find_map(|(prefix, rate)| bare.starts_with(prefix.as_str()).then_some(rate))
            {
                return Some(rate);
            }
            let bare_aliased = model_pricing_alias(bare);
            if bare_aliased.as_ref() != bare {
                return self.prefixes.iter().find_map(|(prefix, rate)| {
                    bare_aliased.starts_with(prefix.as_str()).then_some(rate)
                });
            }
        }
        None
    }

    /// Estimate the USD cost of a set of token counts for a given model.
    ///
    /// Returns `None` if no pricing data is available for the model.
    pub fn estimate_cost(&self, model: &str, usage: UsageAccumulator) -> Option<f64> {
        self.estimate_cost_with_context(model, usage, PricingContext::default())
    }

    /// Estimate USD cost using per-request context (1h cache writes, speed tier,
    /// multi-call aggregation guard, and provider-reported cost).
    pub fn estimate_cost_with_context(
        &self,
        model: &str,
        usage: UsageAccumulator,
        ctx: PricingContext,
    ) -> Option<f64> {
        if let Some(reported) = ctx.reported_cost_usd
            && reported.is_finite()
            && reported >= 0.0
        {
            return Some(reported);
        }

        let rate = self.find_rate(model)?;
        self.estimate_cost_with_rate_and_context(Some(rate), usage, ctx)
    }

    /// Estimate USD cost using an already-resolved optional [`PricingRate`] and
    /// per-request context, avoiding repeated model lookups in hot loops.
    pub fn estimate_cost_with_rate_and_context(
        &self,
        rate: Option<&PricingRate>,
        usage: UsageAccumulator,
        ctx: PricingContext,
    ) -> Option<f64> {
        if let Some(reported) = ctx.reported_cost_usd
            && reported.is_finite()
            && reported >= 0.0
        {
            return Some(reported);
        }

        let rate = rate?;
        let active = active_token_rates(rate, usage, ctx);
        let separate_reasoning_pricing = active.reasoning_output_per_million > 0.0;
        let (output_tokens, reasoning_tokens) = if separate_reasoning_pricing {
            (
                usage
                    .output_tokens
                    .saturating_sub(usage.reasoning_output_tokens),
                usage.reasoning_output_tokens,
            )
        } else {
            (usage.output_tokens, 0)
        };

        let one_hour_cache = ctx
            .cache_creation_1h_tokens
            .min(usage.cache_creation_input_tokens);
        let five_min_cache = usage
            .cache_creation_input_tokens
            .saturating_sub(one_hour_cache);

        let million = 1_000_000.0;
        Some(
            (usage.input_tokens as f64 / million) * active.input_per_million
                + (five_min_cache as f64 / million) * active.cache_creation_per_million
                + (one_hour_cache as f64 / million) * active.cache_creation_1h_per_million
                + (usage.cache_read_input_tokens as f64 / million) * active.cache_read_per_million
                + (output_tokens as f64 / million) * active.output_per_million
                + (reasoning_tokens as f64 / million) * active.reasoning_output_per_million,
        )
    }
}

fn base_token_rates(rate: &PricingRate) -> TokenRates {
    TokenRates {
        input_per_million: rate.input_per_million,
        output_per_million: rate.output_per_million,
        cache_creation_per_million: rate.cache_creation_per_million,
        cache_creation_1h_per_million: rate
            .cache_creation_1h_per_million
            .unwrap_or(rate.cache_creation_per_million),
        cache_read_per_million: rate.cache_read_per_million,
        reasoning_output_per_million: rate.reasoning_output_per_million,
    }
}

fn active_token_rates(
    rate: &PricingRate,
    usage: UsageAccumulator,
    ctx: PricingContext,
) -> TokenRates {
    let prompt_tokens =
        usage.input_tokens + usage.cache_creation_input_tokens + usage.cache_read_input_tokens;
    let calls = u64::from(ctx.model_calls.max(1));
    let effective_prompt = prompt_tokens / calls;
    let base = base_token_rates(rate);

    if !rate.context_tiers.is_empty() {
        if let Some(tier) = rate
            .context_tiers
            .iter()
            .find(|tier| effective_prompt > tier.threshold_tokens)
        {
            return match ctx.speed {
                ServiceSpeed::Standard => tier.standard,
                ServiceSpeed::Fast => tier.fast.unwrap_or(tier.standard),
                ServiceSpeed::Ultrafast => tier.ultrafast.unwrap_or(tier.standard),
            };
        }
        return match ctx.speed {
            ServiceSpeed::Standard => base,
            ServiceSpeed::Fast => rate.fast.as_deref().copied().unwrap_or(base),
            ServiceSpeed::Ultrafast => rate.ultrafast.as_deref().copied().unwrap_or(base),
        };
    }

    if let Some(threshold) = tier_threshold(rate)
        && effective_prompt > threshold
    {
        let tiered = TokenRates {
            input_per_million: rate
                .input_above_per_million
                .unwrap_or(base.input_per_million),
            output_per_million: rate
                .output_above_per_million
                .unwrap_or(base.output_per_million),
            cache_creation_per_million: rate
                .cache_creation_above_per_million
                .unwrap_or(base.cache_creation_per_million),
            cache_creation_1h_per_million: rate
                .cache_creation_above_per_million
                .unwrap_or(base.cache_creation_1h_per_million),
            cache_read_per_million: rate
                .cache_read_above_per_million
                .unwrap_or(base.cache_read_per_million),
            reasoning_output_per_million: rate
                .reasoning_output_above_per_million
                .unwrap_or(base.reasoning_output_per_million),
        };
        return tiered;
    }

    match ctx.speed {
        ServiceSpeed::Standard => base,
        ServiceSpeed::Fast => rate.fast.as_deref().copied().unwrap_or(base),
        ServiceSpeed::Ultrafast => rate.ultrafast.as_deref().copied().unwrap_or(base),
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

fn is_unpriceable_model(bare_key: &str) -> bool {
    matches!(
        bare_key,
        "<synthetic>" | "synthetic" | "opus" | "sonnet" | "haiku" | "fable"
    )
}

fn lookup_exact_or_date<'a>(
    exact: &'a HashMap<String, PricingRate>,
    key: &str,
) -> Option<&'a PricingRate> {
    if let Some(rate) = exact.get(key) {
        return Some(rate);
    }
    if let Some(stripped) = strip_trailing_date_suffix(key)
        && let Some(rate) = exact.get(stripped)
    {
        return Some(rate);
    }
    None
}

fn strip_trailing_date_suffix(model: &str) -> Option<&str> {
    let (head, tail) = model.rsplit_once('-')?;
    if tail.len() == 8 && tail.chars().all(|ch| ch.is_ascii_digit()) {
        Some(head)
    } else {
        None
    }
}

fn canonical_model_key(model: &str) -> String {
    canonical_model_key_fast(model).into_owned()
}

fn canonical_model_key_fast(model: &str) -> std::borrow::Cow<'_, str> {
    let mut trimmed = model.trim();
    for prefix in &["f16x/", "svr/", "nov/", "f16/"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            trimmed = rest;
            break;
        }
    }
    if let Some(bracket) = trimmed.find('[') {
        trimmed = &trimmed[..bracket];
    }
    if trimmed.bytes().any(|b| b.is_ascii_uppercase()) {
        std::borrow::Cow::Owned(trimmed.to_ascii_lowercase())
    } else {
        std::borrow::Cow::Borrowed(trimmed)
    }
}

fn model_pricing_alias<'a>(model_key: &'a str) -> Cow<'a, str> {
    let base = [
        "-extra-low",
        "-xhigh",
        "-high",
        "-medium",
        "-low",
        "-none",
        "-thinking",
        "-tiered",
        "-control",
        "-orchestrator",
        "-trial",
        "-n",
        "-r",
    ]
    .iter()
    .find_map(|suffix| model_key.strip_suffix(suffix))
    .unwrap_or(model_key);

    match base {
        "gemini-3.1-pro" => Cow::Borrowed("gemini-3.1-pro-preview"),
        "gemini-3-pro" => Cow::Borrowed("gemini-3-pro-preview"),
        "gemini-3-flash" => Cow::Borrowed("gemini-3-flash-preview"),
        "grok-proxy" | "grok-build" => Cow::Borrowed(GROK_DEFAULT_MODEL_FALLBACK),
        other if other.ends_with("-build") => {
            Cow::Borrowed(other.strip_suffix("-build").unwrap_or(other))
        }
        _ => Cow::Borrowed(base),
    }
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
    pub service_tier: ServiceSpeed,
    pub fork_anchor_ts: Option<DateTime<Utc>>,
    pub request_ledger_active: bool,
    pub last_usage_signature: Option<(u64, u64, u64, u64, u64)>,
}

pub const LEGACY_CODEX_FALLBACK_MODEL: &str = "gpt-5";

/// Model label used for every Grok usage event.
///
/// `~/.grok/logs/unified.jsonl` records per-turn token counts but never the
/// model that served the turn, so all Grok events are attributed to this
/// fallback (xAI's current flagship default, matching `~/.grok/config.toml`'s
/// `[models].default`) rather than a value read from the log.
pub const GROK_DEFAULT_MODEL_FALLBACK: &str = "grok-4.5";

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
    /// Per-source unique model names detected for this period.
    ///
    /// This is used for accurate per-provider model counts (e.g. in TOTAL rows),
    /// even when the same model name can appear across multiple providers
    /// (for example `gpt-*` models used by both Codex and OpenCode).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models_by_source: BTreeMap<String, BTreeSet<String>>,
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
#[derive(Debug, Clone, Serialize, Default)]
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

    /// Extra derived analytics for the report (cache ratio, peak period, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insights: Option<ReportInsights>,
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

#[cfg(test)]
mod pricing_tests {
    use super::*;

    fn rate(input: f64, output: f64, cache_read: f64) -> PricingRate {
        PricingRate {
            input_per_million: input,
            output_per_million: output,
            cache_creation_per_million: 0.0,
            cache_read_per_million: cache_read,
            reasoning_output_per_million: 0.0,
            ..PricingRate::default()
        }
    }

    #[test]
    fn find_rate_matches_exact_before_alias() {
        let mut table = PricingTable::default();
        table
            .exact
            .insert("gemini-3.1-pro-low".to_string(), rate(1.0, 1.0, 1.0));
        table
            .exact
            .insert("gemini-3.1-pro-preview".to_string(), rate(2.0, 2.0, 2.0));

        let found = table.find_rate("gemini-3.1-pro-low").unwrap();
        assert_eq!(found.input_per_million, 1.0);
    }

    #[test]
    fn find_rate_strips_effort_suffix_and_aliases_to_preview() {
        let mut table = PricingTable::default();
        table
            .exact
            .insert("gemini-3.1-pro-preview".to_string(), rate(2.0, 12.0, 0.2));

        for suffixed in [
            "gemini-3.1-pro-low",
            "gemini-3.1-pro-medium",
            "gemini-3.1-pro-high",
            "gemini-3.1-pro-xhigh",
            "gemini-3.1-pro-none",
        ] {
            let found = table
                .find_rate(suffixed)
                .unwrap_or_else(|| panic!("expected a rate for {suffixed}"));
            assert_eq!(found.input_per_million, 2.0);
            assert_eq!(found.output_per_million, 12.0);
        }
    }

    #[test]
    fn find_rate_aliases_other_known_gemini_bases() {
        let mut table = PricingTable::default();
        table
            .exact
            .insert("gemini-3-pro-preview".to_string(), rate(1.25, 5.0, 0.3));
        table
            .exact
            .insert("gemini-3-flash-preview".to_string(), rate(0.1, 0.4, 0.025));

        assert_eq!(
            table
                .find_rate("gemini-3-pro-low")
                .unwrap()
                .input_per_million,
            1.25
        );
        assert_eq!(
            table
                .find_rate("gemini-3-flash-high")
                .unwrap()
                .input_per_million,
            0.1
        );
    }

    #[test]
    fn find_rate_returns_none_for_unmapped_model() {
        let table = PricingTable::default();
        assert!(table.find_rate("gemini-3.1-pro-low").is_none());
        assert!(table.find_rate("totally-unknown-model").is_none());
    }

    #[test]
    fn estimate_cost_uses_alias_fallback_end_to_end() {
        let mut table = PricingTable::default();
        table
            .exact
            .insert("gemini-3.1-pro-preview".to_string(), rate(2.0, 12.0, 0.2));

        let usage = UsageAccumulator {
            input_tokens: 1_000_000,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 1_000_000,
            output_tokens: 0,
            reasoning_output_tokens: 0,
            cost_usd: 0.0,
        };
        let cost = table
            .estimate_cost("gemini-3.1-pro-low", usage)
            .expect("alias fallback should price this model");
        assert!((cost - 2.2).abs() < 1e-9);
    }

    #[test]
    fn find_rate_checks_aliased_exact_before_prefix() {
        let mut table = PricingTable::default_table();
        table
            .exact
            .insert("claude-opus-5-5".to_string(), rate(4.0, 20.0, 0.2));

        let found = table.find_rate("claude-opus-5-5-medium").unwrap();
        assert_eq!(found.input_per_million, 4.0);
        assert_eq!(found.output_per_million, 20.0);
    }

    #[test]
    fn find_rate_strips_variant_brackets_and_rejects_synthetic() {
        let mut table = PricingTable::default_table();
        table
            .exact
            .insert("claude-opus-5-5".to_string(), rate(4.0, 20.0, 0.2));

        assert_eq!(
            table
                .find_rate("claude-opus-5-5[1m]")
                .unwrap()
                .input_per_million,
            4.0
        );
        assert!(table.find_rate("<synthetic>").is_none());
        assert!(table.find_rate("synthetic").is_none());
        assert!(table.find_rate("opus").is_none());
    }

    #[test]
    fn estimate_cost_with_context_handles_1h_cache_tiers_speed_and_multicall() {
        let mut table = PricingTable::default();
        let std_base = TokenRates {
            input_per_million: 2.0,
            output_per_million: 10.0,
            cache_creation_per_million: 2.5,
            cache_creation_1h_per_million: 4.0,
            cache_read_per_million: 0.1,
            reasoning_output_per_million: 0.0,
        };
        let tier_272 = ContextTierRate {
            threshold_tokens: 272_000,
            standard: TokenRates {
                input_per_million: 4.0,
                output_per_million: 15.0,
                cache_creation_per_million: 5.0,
                cache_creation_1h_per_million: 8.0,
                cache_read_per_million: 0.2,
                reasoning_output_per_million: 0.0,
            },
            fast: Some(TokenRates {
                input_per_million: 8.0,
                output_per_million: 30.0,
                cache_creation_per_million: 10.0,
                cache_creation_1h_per_million: 16.0,
                cache_read_per_million: 0.4,
                reasoning_output_per_million: 0.0,
            }),
            ultrafast: None,
        };
        table.exact.insert(
            "gpt-6.1-sol".to_string(),
            PricingRate {
                input_per_million: std_base.input_per_million,
                output_per_million: std_base.output_per_million,
                cache_creation_per_million: std_base.cache_creation_per_million,
                cache_creation_1h_per_million: Some(std_base.cache_creation_1h_per_million),
                cache_read_per_million: std_base.cache_read_per_million,
                fast: Some(Box::new(TokenRates {
                    input_per_million: 4.0,
                    output_per_million: 20.0,
                    cache_creation_per_million: 5.0,
                    cache_creation_1h_per_million: 8.0,
                    cache_read_per_million: 0.2,
                    reasoning_output_per_million: 0.0,
                })),
                context_tiers: vec![tier_272],
                ..PricingRate::default()
            },
        );

        // 1) 1h cache split: 1M 5m ($2.50) + 1M 1h ($4.00) = $6.50 when prompt <= 272k?
        // Use 100k 5m + 100k 1h = 200k prompt (<= 272k): 0.1*2.5 + 0.1*4.0 = $0.65
        let usage_1h = UsageAccumulator {
            cache_creation_input_tokens: 200_000,
            ..UsageAccumulator::default()
        };
        let cost_1h = table
            .estimate_cost_with_context(
                "gpt-6.1-sol",
                usage_1h,
                PricingContext {
                    cache_creation_1h_tokens: 100_000,
                    ..PricingContext::default()
                },
            )
            .unwrap();
        assert!((cost_1h - 0.65).abs() < 1e-9, "got {cost_1h}");

        // 2) Single-call > 272k triggers cliff ($4/M input * 0.3M = $1.20)
        let usage_cliff = UsageAccumulator {
            input_tokens: 300_000,
            ..UsageAccumulator::default()
        };
        let cost_single = table
            .estimate_cost_with_context("gpt-6.1-sol", usage_cliff, PricingContext::default())
            .unwrap();
        assert!((cost_single - 1.20).abs() < 1e-9, "got {cost_single}");

        // 3) Multi-call aggregate (6 calls averaging 50k each) stays on base tier ($2/M * 0.3M = $0.60)
        let cost_multi = table
            .estimate_cost_with_context(
                "gpt-6.1-sol",
                usage_cliff,
                PricingContext {
                    model_calls: 6,
                    ..PricingContext::default()
                },
            )
            .unwrap();
        assert!((cost_multi - 0.60).abs() < 1e-9, "got {cost_multi}");

        // 4) Fast speed tier on >272k request uses tier.fast ($8/M * 0.3M = $2.40)
        let cost_fast = table
            .estimate_cost_with_context(
                "gpt-6.1-sol",
                usage_cliff,
                PricingContext {
                    speed: ServiceSpeed::Fast,
                    ..PricingContext::default()
                },
            )
            .unwrap();
        assert!((cost_fast - 2.40).abs() < 1e-9, "got {cost_fast}");

        // 5) Reported cost takes precedence
        let cost_rep = table
            .estimate_cost_with_context(
                "gpt-6.1-sol",
                usage_cliff,
                PricingContext {
                    reported_cost_usd: Some(0.42),
                    ..PricingContext::default()
                },
            )
            .unwrap();
        assert!((cost_rep - 0.42).abs() < 1e-9);
    }
}
