use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub(crate) enum SourceKind {
    Claude,
    Codex,
}

impl SourceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            SourceKind::Claude => "claude",
            SourceKind::Codex => "codex",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceConfig {
    pub(crate) kind: SourceKind,
    pub(crate) roots: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredFile {
    pub(crate) source: SourceKind,
    pub(crate) root: PathBuf,
    pub(crate) path: PathBuf,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub(crate) struct UsageAccumulator {
    pub(crate) input_tokens: u64,
    pub(crate) cache_creation_input_tokens: u64,
    pub(crate) cache_read_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_output_tokens: u64,
    pub(crate) cost_usd: f64,
}

impl UsageAccumulator {
    pub(crate) fn add(&mut self, other: UsageAccumulator) {
        self.input_tokens += other.input_tokens;
        self.cache_creation_input_tokens += other.cache_creation_input_tokens;
        self.cache_read_input_tokens += other.cache_read_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
        self.cost_usd += other.cost_usd;
    }

    pub(crate) fn total_tokens(self) -> u64 {
        self.input_tokens
            + self.cache_creation_input_tokens
            + self.cache_read_input_tokens
            + self.output_tokens
    }

    pub(crate) fn to_counts(self) -> TokenCounts {
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

#[derive(Debug, Clone, Serialize, Default)]
pub(crate) struct TokenCounts {
    pub(crate) input_tokens: u64,
    pub(crate) cache_creation_input_tokens: u64,
    pub(crate) cache_read_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_output_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cost_usd: f64,
}

impl TokenCounts {
    pub(crate) fn add_assign(&mut self, other: TokenCounts) {
        self.input_tokens += other.input_tokens;
        self.cache_creation_input_tokens += other.cache_creation_input_tokens;
        self.cache_read_input_tokens += other.cache_read_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
        self.total_tokens += other.total_tokens;
        self.cost_usd += other.cost_usd;
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UsageEvent {
    pub(crate) timestamp: DateTime<Utc>,
    pub(crate) source: SourceKind,
    pub(crate) model: String,
    pub(crate) session: String,
    pub(crate) project: Option<String>,
    pub(crate) file_path: String,
    pub(crate) usage: UsageAccumulator,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DateFilter {
    pub(crate) since: Option<NaiveDate>,
    pub(crate) until: Option<NaiveDate>,
}

impl DateFilter {
    pub(crate) fn allows(self, day: NaiveDate) -> bool {
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

#[derive(Debug, Default)]
pub(crate) struct ParseStatsAtomic {
    pub(crate) files_discovered: AtomicUsize,
    pub(crate) files_open_failed: AtomicUsize,
    pub(crate) lines_total: AtomicUsize,
    pub(crate) lines_parsed: AtomicUsize,
    pub(crate) lines_filtered: AtomicUsize,
    pub(crate) lines_invalid_json: AtomicUsize,
    pub(crate) lines_missing_usage: AtomicUsize,
    pub(crate) lines_unknown_pricing: AtomicUsize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ParseStats {
    pub(crate) files_discovered: usize,
    pub(crate) files_open_failed: usize,
    pub(crate) lines_total: usize,
    pub(crate) lines_parsed: usize,
    pub(crate) lines_filtered: usize,
    pub(crate) lines_invalid_json: usize,
    pub(crate) lines_missing_usage: usize,
    pub(crate) lines_unknown_pricing: usize,
}

impl ParseStatsAtomic {
    pub(crate) fn snapshot(&self) -> ParseStats {
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct PricingRate {
    pub(crate) input_per_million: f64,
    pub(crate) output_per_million: f64,
    #[serde(default)]
    pub(crate) cache_creation_per_million: f64,
    #[serde(default)]
    pub(crate) cache_read_per_million: f64,
    #[serde(default)]
    pub(crate) reasoning_output_per_million: f64,
    #[serde(default)]
    pub(crate) tier_threshold_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) input_above_per_million: Option<f64>,
    #[serde(default)]
    pub(crate) output_above_per_million: Option<f64>,
    #[serde(default)]
    pub(crate) cache_creation_above_per_million: Option<f64>,
    #[serde(default)]
    pub(crate) cache_read_above_per_million: Option<f64>,
    #[serde(default)]
    pub(crate) reasoning_output_above_per_million: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PricingTable {
    pub(crate) exact: HashMap<String, PricingRate>,
    pub(crate) prefixes: Vec<(String, PricingRate)>,
}

impl PricingTable {
    pub(crate) fn default_table() -> Self {
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

    pub(crate) fn merge_exact(&mut self, overrides: HashMap<String, PricingRate>) {
        for (model, rate) in overrides {
            self.exact.insert(canonical_model_key(&model), rate);
        }
    }

    pub(crate) fn find_rate(&self, model: &str) -> Option<&PricingRate> {
        let model_key = canonical_model_key(model);
        if let Some(rate) = self.exact.get(&model_key) {
            return Some(rate);
        }
        self.prefixes
            .iter()
            .find_map(|(prefix, rate)| model_key.starts_with(prefix).then_some(rate))
    }

    pub(crate) fn estimate_cost(&self, model: &str, usage: UsageAccumulator) -> Option<f64> {
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
pub(crate) struct ParsedLine {
    pub(crate) event: UsageEvent,
    pub(crate) used_unknown_pricing: bool,
}

#[derive(Debug)]
pub(crate) enum ParseLineResult {
    Parsed(ParsedLine),
    InvalidJson,
    MissingUsage,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub(crate) struct CodexRawUsage {
    pub(crate) input_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_output_tokens: u64,
    pub(crate) total_tokens: u64,
}

impl CodexRawUsage {
    pub(crate) fn is_zero(self) -> bool {
        self.input_tokens == 0
            && self.cached_input_tokens == 0
            && self.output_tokens == 0
            && self.reasoning_output_tokens == 0
    }
}

#[derive(Debug, Default)]
pub(crate) struct CodexParseState {
    pub(crate) current_model: Option<String>,
    pub(crate) current_model_is_fallback: bool,
    pub(crate) previous_totals: Option<CodexRawUsage>,
}

pub(crate) const LEGACY_CODEX_FALLBACK_MODEL: &str = "gpt-5";

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DailyRow {
    pub(crate) date: String,
    pub(crate) totals: TokenCounts,
    pub(crate) models: BTreeMap<String, TokenCounts>,
    pub(crate) sources: BTreeMap<String, TokenCounts>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DailyReport {
    pub(crate) daily: Vec<DailyRow>,
    pub(crate) totals: TokenCounts,
    pub(crate) stats: ParseStats,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum TableLayout {
    Compact,
    Standard,
    Full,
}

impl TableLayout {
    pub(crate) fn from_terminal_width(width: usize) -> Self {
        if width <= 110 {
            Self::Compact
        } else if width <= 160 {
            Self::Standard
        } else {
            Self::Full
        }
    }

    pub(crate) fn model_line_limit(self) -> usize {
        match self {
            Self::Compact => 2,
            Self::Standard => 4,
            Self::Full => 6,
        }
    }

    pub(crate) fn model_char_limit(self) -> usize {
        match self {
            Self::Compact => 24,
            Self::Standard => 38,
            Self::Full => 56,
        }
    }
}
