use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::types::{DailyRow, PricingTable, SourceKind, TokenCounts};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReportInsights {
    /// Usage, efficiency, and cost changes between adjacent completed calendar periods.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trend: Option<ReportTrend>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_share_pct: Option<f64>,
    /// Cached reads as a percentage of prompt tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_share_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_per_mtoken: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_per_usd: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_source_share_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_model_share_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_period: Option<PeakPeriod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_spend: Option<PeakPeriod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longest_streak_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_streak_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_tokens_per_active_day: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_cost_per_active_day: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spikes: Vec<SpikePeriod>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mix_tokens_pct: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mix_cost_pct: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anomalies: Vec<AnomalyPeriod>,
    /// Estimated net $ saved by prompt caching vs. paying full input price for
    /// every cached read (minus the cache-write premium). Counterfactual using
    /// canonical public rates. Periods with cached tokens require sufficient
    /// priced-token coverage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_savings_usd: Option<f64>,
    /// cache_read / cache_creation — how many times each cached token is reused.
    /// <1 means you write more cache than you read back (churn).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_reuse_ratio: Option<f64>,
    /// The source whose share of cost most exceeds its share of tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_concentration: Option<CostConcentration>,
    /// Human scale concise summary (e.g. ~4.0x English Wikipedia · 1,250 books written)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_summary: Option<String>,
    /// Full dual-lens information scale details
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<crate::scale::InformationScale>,
}

impl ReportInsights {
    pub(crate) fn update_period_attribution(
        &mut self,
        attribution: &HashMap<&str, PeriodAttribution>,
    ) {
        for spike in &mut self.spikes {
            if let Some(found) = attribution.get(spike.date.as_str()) {
                spike.top_source.clone_from(&found.top_source);
                spike.top_model.clone_from(&found.top_model);
                spike.top_project.clone_from(&found.top_project);
                spike.top_session.clone_from(&found.top_session);
            }
        }
        for anomaly in &mut self.anomalies {
            if let Some(found) = attribution.get(anomaly.date.as_str()) {
                anomaly.top_source.clone_from(&found.top_source);
                anomaly.top_model.clone_from(&found.top_model);
                anomaly.top_project.clone_from(&found.top_project);
                anomaly.top_session.clone_from(&found.top_session);
            }
        }
    }
}

/// Where money concentrates relative to volume: a source whose cost share
/// outruns its token share (the expensive-per-token provider).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostConcentration {
    pub label: String,
    pub token_pct: f64,
    pub cost_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PeriodAttribution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_session: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeakPeriod {
    pub date: String,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportTrend {
    pub previous: TrendPeriod,
    pub latest: TrendPeriod,
    /// Percentage change relative to the previous period, when its volume was positive.
    pub token_change_pct: Option<f64>,
    /// Percentage change relative to the previous period, when its cost was positive.
    pub cost_change_pct: Option<f64>,
    /// Difference in cache hit rate, in percentage points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_change_pp: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendPeriod {
    /// Period label matching the report table. Weekly labels are week start dates.
    pub date: String,
    pub total_tokens: u64,
    pub cost_usd: f64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub reasoning_output_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_per_mtoken: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_savings_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_reuse_ratio: Option<f64>,
    /// Per-provider usage available in the logs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sources: BTreeMap<String, TokenCounts>,
    /// Per-model usage available in the logs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, TokenCounts>,
}

impl TrendPeriod {
    pub(crate) fn from_rows(date: String, rows: &[&DailyRow]) -> Self {
        let mut totals = TokenCounts::default();
        let mut sources = BTreeMap::<String, TokenCounts>::new();
        let mut models = BTreeMap::<String, TokenCounts>::new();
        for row in rows {
            totals.add_assign(row.totals.clone());
            for (breakdown, aggregate) in [(&row.sources, &mut sources), (&row.models, &mut models)]
            {
                for (label, counts) in breakdown {
                    aggregate
                        .entry(label.clone())
                        .or_default()
                        .add_assign(counts.clone());
                }
            }
        }
        sources.retain(|label, _| parse_source_kind(label).is_some());
        models.retain(|_, counts| counts.total_tokens > 0 || counts.cost_usd > 0.0);
        Self {
            date,
            total_tokens: totals.total_tokens,
            cost_usd: totals.cost_usd,
            input_tokens: totals.input_tokens,
            output_tokens: totals.output_tokens,
            reasoning_output_tokens: totals.reasoning_output_tokens,
            cache_read_input_tokens: totals.cache_read_input_tokens,
            cache_creation_input_tokens: totals.cache_creation_input_tokens,
            cache_hit_pct: cache_hit_pct(&totals),
            cost_per_mtoken: cost_per_mtoken(&totals),
            cache_savings_usd: cache_savings(&models, &totals),
            cache_reuse_ratio: cache_reuse_ratio(&totals),
            sources,
            models,
        }
    }
}

impl ReportTrend {
    pub(crate) fn new(previous: TrendPeriod, latest: TrendPeriod) -> Self {
        Self {
            token_change_pct: token_change_pct(previous.total_tokens, latest.total_tokens),
            cost_change_pct: relative_change_pct(
                previous.cost_usd,
                latest.cost_usd - previous.cost_usd,
            ),
            cache_hit_change_pp: previous
                .cache_hit_pct
                .zip(latest.cache_hit_pct)
                .map(|(before, after)| after - before),
            previous,
            latest,
        }
    }
}

pub(crate) fn relative_change_pct(previous: f64, difference: f64) -> Option<f64> {
    (previous > 0.0).then(|| (difference / previous) * 100.0)
}

pub(crate) fn token_change_pct(previous: u64, latest: u64) -> Option<f64> {
    let difference = (i128::from(latest) - i128::from(previous)) as f64;
    relative_change_pct(previous as f64, difference)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpikePeriod {
    pub date: String,
    pub total_tokens: u64,
    pub baseline_median: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_session: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyPeriod {
    pub date: String,
    pub total_tokens: u64,
    pub median: u64,
    pub mad: u64,
    /// Robust z-score using median absolute deviation.
    pub robust_z: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_session: Option<String>,
}

pub fn compute_report_insights(
    rows: &[DailyRow],
    totals: &TokenCounts,
    period_attribution: Option<&BTreeMap<String, PeriodAttribution>>,
) -> ReportInsights {
    let total_tokens = totals.total_tokens;
    let cache_tokens = totals.cache_creation_input_tokens + totals.cache_read_input_tokens;
    let output_tokens = totals.output_tokens + totals.reasoning_output_tokens;

    let cache_share_pct = percent(cache_tokens, total_tokens);
    let output_share_pct = percent(output_tokens, total_tokens);
    let cost_per_mtoken = cost_per_mtoken(totals);
    let tokens_per_usd = if totals.cost_usd > f64::EPSILON {
        Some((total_tokens as f64 / totals.cost_usd).round().max(0.0) as u64)
    } else {
        None
    };

    let (top_source, top_source_share_pct) = top_source_label(rows, total_tokens);
    let (top_model, top_model_share_pct) = top_model_label(rows, total_tokens);
    let peak_period = rows
        .iter()
        .max_by_key(|row| row.totals.total_tokens)
        .map(|row| PeakPeriod {
            date: row.date.clone(),
            total_tokens: row.totals.total_tokens,
            cost_usd: row.totals.cost_usd,
        })
        .filter(|peak| peak.total_tokens > 0);

    let peak_spend = rows
        .iter()
        .max_by(|a, b| a.totals.cost_usd.total_cmp(&b.totals.cost_usd))
        .map(|row| PeakPeriod {
            date: row.date.clone(),
            total_tokens: row.totals.total_tokens,
            cost_usd: row.totals.cost_usd,
        })
        .filter(|peak| peak.cost_usd > 0.0);

    let scale = if total_tokens > 0 {
        Some(crate::scale::InformationScale::from_counts(totals))
    } else {
        None
    };
    let scale_summary = scale.as_ref().map(|s| s.compact_summary());

    let (longest_streak_days, current_streak_days, active_days) = streaks_if_daily(rows);
    let avg_tokens_per_active_day = if active_days > 0 && total_tokens > 0 {
        Some((total_tokens as f64 / active_days as f64).round().max(0.0) as u64)
    } else {
        None
    };
    let avg_cost_per_active_day = if active_days > 0 && totals.cost_usd > 0.0 {
        Some(totals.cost_usd / active_days as f64)
    } else {
        None
    };

    let spikes = token_spikes(rows, 3, period_attribution);
    let anomalies = token_anomalies(rows, 3, period_attribution);
    let (mix_tokens_pct, mix_cost_pct) = provider_mix(rows);
    let cache_savings_usd = cache_savings(rows.iter().flat_map(|row| &row.models), totals);
    let cache_reuse_ratio = cache_reuse_ratio(totals);
    let cost_concentration = cost_concentration(&mix_tokens_pct, &mix_cost_pct);

    ReportInsights {
        trend: None,
        cache_share_pct,
        cache_hit_pct: cache_hit_pct(totals),
        output_share_pct,
        cost_per_mtoken,
        tokens_per_usd,
        top_source,
        top_model,
        top_source_share_pct,
        top_model_share_pct,
        peak_period,
        peak_spend,
        longest_streak_days,
        current_streak_days,
        avg_tokens_per_active_day,
        avg_cost_per_active_day,
        spikes,
        mix_tokens_pct,
        mix_cost_pct,
        anomalies,
        cache_savings_usd,
        cache_reuse_ratio,
        cost_concentration,
        scale_summary,
        scale,
    }
}

/// Minimum fraction of cacheable tokens that must have known cache pricing
/// before we trust (and emit) the savings estimate.
const CACHE_PRICING_COVERAGE_MIN: f64 = 60.0;

/// Counterfactual prompt-cache economics using canonical public rates.
///
/// Net savings are the read discount less the write premium, when enough
/// of the period's cached tokens have known model pricing.
fn cache_savings<'a>(
    models: impl IntoIterator<Item = (&'a String, &'a TokenCounts)>,
    totals: &TokenCounts,
) -> Option<f64> {
    let reported_cache_tokens = totals.cache_read_input_tokens + totals.cache_creation_input_tokens;
    let pricing = PricingTable::default_table();
    let mut net_savings = 0.0;
    let mut modeled_cache_tokens = 0u64;
    let mut priced_tokens = 0u64;

    for (model, counts) in models {
        let read = counts.cache_read_input_tokens;
        let create = counts.cache_creation_input_tokens;
        modeled_cache_tokens += read + create;

        if read + create > 0
            && let Some(rate) = pricing.find_rate(model)
            && (rate.cache_read_per_million > 0.0 || rate.cache_creation_per_million > 0.0)
        {
            priced_tokens += read + create;
            net_savings += (read as f64 / 1_000_000.0)
                * (rate.input_per_million - rate.cache_read_per_million);
            net_savings -= (create as f64 / 1_000_000.0)
                * (rate.cache_creation_per_million - rate.input_per_million);
        }
    }

    let cache_tokens = reported_cache_tokens.max(modeled_cache_tokens);
    if cache_tokens == 0 && totals.input_tokens > 0 {
        return Some(0.0);
    }
    let coverage = percent(priced_tokens, cache_tokens)?;
    (coverage >= CACHE_PRICING_COVERAGE_MIN).then_some(net_savings)
}

fn cache_hit_pct(totals: &TokenCounts) -> Option<f64> {
    let prompt_tokens =
        totals.input_tokens + totals.cache_creation_input_tokens + totals.cache_read_input_tokens;
    percent(totals.cache_read_input_tokens, prompt_tokens)
}

fn cache_reuse_ratio(totals: &TokenCounts) -> Option<f64> {
    let writes = totals.cache_creation_input_tokens;
    (writes > 0).then(|| totals.cache_read_input_tokens as f64 / writes as f64)
}

fn cost_per_mtoken(totals: &TokenCounts) -> Option<f64> {
    (totals.total_tokens > 0 && totals.cost_usd > 0.0)
        .then(|| totals.cost_usd / (totals.total_tokens as f64 / 1_000_000.0))
}

/// Source whose cost share most exceeds its token share. The render layer
/// decides whether the divergence is large enough to surface.
fn cost_concentration(
    mix_tokens_pct: &BTreeMap<String, f64>,
    mix_cost_pct: &BTreeMap<String, f64>,
) -> Option<CostConcentration> {
    let mut best: Option<CostConcentration> = None;
    for (label, cost_pct) in mix_cost_pct {
        let token_pct = mix_tokens_pct.get(label).copied().unwrap_or(0.0);
        let divergence = cost_pct - token_pct;
        let is_better = match &best {
            None => true,
            Some(b) => divergence > (b.cost_pct - b.token_pct),
        };
        if is_better {
            best = Some(CostConcentration {
                label: label.clone(),
                token_pct,
                cost_pct: *cost_pct,
            });
        }
    }
    best
}

fn percent(part: u64, total: u64) -> Option<f64> {
    if total == 0 {
        return None;
    }
    Some((part as f64 / total as f64) * 100.0)
}

fn top_source_label(rows: &[DailyRow], grand_total: u64) -> (Option<String>, Option<f64>) {
    let mut totals = BTreeMap::<SourceKind, u64>::new();
    for row in rows {
        for (source, counts) in &row.sources {
            if let Some(kind) = parse_source_kind(source) {
                *totals.entry(kind).or_insert(0) += counts.total_tokens;
            }
        }
    }
    let top = totals.into_iter().max_by_key(|(_, total)| *total);
    let Some((kind, total)) = top else {
        return (None, None);
    };
    (
        Some(kind.display_name().to_string()),
        percent(total, grand_total),
    )
}

fn parse_source_kind(raw: &str) -> Option<SourceKind> {
    let value = raw.trim().to_ascii_lowercase();
    match value.as_str() {
        "claude" => Some(SourceKind::Claude),
        "codex" => Some(SourceKind::Codex),
        "gemini" => Some(SourceKind::Gemini),
        "opencode" => Some(SourceKind::OpenCode),
        "grok" => Some(SourceKind::Grok),
        _ => None,
    }
}

fn top_model_label(rows: &[DailyRow], grand_total: u64) -> (Option<String>, Option<f64>) {
    let mut totals = BTreeMap::<String, u64>::new();
    for row in rows {
        for (model, counts) in &row.models {
            *totals.entry(model.clone()).or_insert(0) += counts.total_tokens;
        }
    }
    let top = totals.into_iter().max_by_key(|(_, total)| *total);
    let Some((model, total)) = top else {
        return (None, None);
    };
    (Some(model), percent(total, grand_total))
}

fn streaks_if_daily(rows: &[DailyRow]) -> (Option<u32>, Option<u32>, u32) {
    // Only makes sense for daily rows. We detect this by parsing the date.
    let mut days = rows
        .iter()
        .filter_map(|row| {
            let day = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d").ok()?;
            let active = row.totals.total_tokens > 0;
            Some((day, active))
        })
        .collect::<Vec<_>>();
    if days.len() != rows.len() || days.is_empty() {
        return (None, None, 0);
    }
    days.sort_by_key(|(day, _)| *day);

    let mut active_days = 0u32;
    let mut longest = 0u32;
    let mut current_run = 0u32;
    let mut last_day = None::<NaiveDate>;
    for (day, active) in &days {
        if *active {
            active_days += 1;
        }
        if let Some(prev) = last_day {
            if *day != prev.succ_opt().unwrap_or(prev) {
                current_run = 0;
            }
        }
        if *active {
            current_run += 1;
            longest = longest.max(current_run);
        } else {
            current_run = 0;
        }
        last_day = Some(*day);
    }

    let current = days
        .last()
        .map(|(last, _)| {
            let mut run = 0u32;
            let mut cursor = *last;
            for (day, active) in days.iter().rev() {
                if *day != cursor {
                    break;
                }
                if *active {
                    run += 1;
                } else {
                    break;
                }
                cursor = cursor.pred_opt().unwrap_or(cursor);
            }
            run
        })
        .unwrap_or(0);

    (
        Some(longest).filter(|v| *v > 0),
        Some(current).filter(|v| *v > 0),
        active_days,
    )
}

fn token_spikes(
    rows: &[DailyRow],
    limit: usize,
    period_attribution: Option<&BTreeMap<String, PeriodAttribution>>,
) -> Vec<SpikePeriod> {
    if rows.len() < 7 {
        return Vec::new();
    }
    let mut token_samples = rows
        .iter()
        .map(|row| row.totals.total_tokens)
        .collect::<Vec<_>>();
    let midpoint = token_samples.len() / 2;
    let (_, &mut median, _) = token_samples.select_nth_unstable(midpoint);
    if median == 0 {
        return Vec::new();
    }

    let mut values = rows
        .iter()
        .filter(|row| row.totals.total_tokens > median.saturating_mul(3))
        .map(|row| (row.date.as_str(), row.totals.total_tokens))
        .collect::<Vec<_>>();
    values.sort_by_key(|v| std::cmp::Reverse(v.1));
    values
        .into_iter()
        .take(limit)
        .map(|(date, total_tokens)| {
            let attribution = attribution_for_period(rows, date, period_attribution);
            SpikePeriod {
                date: date.to_owned(),
                total_tokens,
                baseline_median: median,
                top_source: attribution.top_source,
                top_model: attribution.top_model,
                top_project: attribution.top_project,
                top_session: attribution.top_session,
            }
        })
        .collect()
}

fn token_anomalies(
    rows: &[DailyRow],
    limit: usize,
    period_attribution: Option<&BTreeMap<String, PeriodAttribution>>,
) -> Vec<AnomalyPeriod> {
    if rows.len() < 7 {
        return Vec::new();
    }
    let mut samples = rows
        .iter()
        .map(|row| row.totals.total_tokens)
        .collect::<Vec<_>>();
    let midpoint = samples.len() / 2;
    let (_, &mut median, _) = samples.select_nth_unstable(midpoint);
    if median == 0 {
        return Vec::new();
    }
    for sample in &mut samples {
        *sample = sample.abs_diff(median);
    }
    let (_, &mut mad, _) = samples.select_nth_unstable(midpoint);
    if mad == 0 {
        return Vec::new();
    }

    let mut values = rows
        .iter()
        .filter_map(|row| {
            let total_tokens = row.totals.total_tokens;
            if total_tokens <= median {
                return None;
            }
            let robust_z = 0.6745 * (total_tokens.abs_diff(median) as f64 / mad.max(1) as f64);
            if robust_z < 3.5 {
                return None;
            }
            Some((row.date.as_str(), total_tokens, robust_z))
        })
        .collect::<Vec<_>>();
    values.sort_by_key(|v| std::cmp::Reverse(v.1));
    values
        .into_iter()
        .take(limit)
        .map(|(date, total_tokens, robust_z)| {
            let attribution = attribution_for_period(rows, date, period_attribution);
            AnomalyPeriod {
                date: date.to_owned(),
                total_tokens,
                median,
                mad,
                robust_z,
                top_source: attribution.top_source,
                top_model: attribution.top_model,
                top_project: attribution.top_project,
                top_session: attribution.top_session,
            }
        })
        .collect()
}

fn row_attribution(rows: &[DailyRow], date: &str) -> (Option<String>, Option<String>) {
    let Some(row) = rows.iter().find(|row| row.date == date) else {
        return (None, None);
    };
    let top_source = row
        .sources
        .iter()
        .max_by_key(|(_, counts)| counts.total_tokens)
        .map(|(source, _)| source.clone());
    let top_model = row
        .models
        .iter()
        .max_by_key(|(_, counts)| counts.total_tokens)
        .map(|(model, _)| model.clone());
    (top_source, top_model)
}

fn attribution_for_period(
    rows: &[DailyRow],
    period: &str,
    period_attribution: Option<&BTreeMap<String, PeriodAttribution>>,
) -> PeriodAttribution {
    if let Some(found) = period_attribution.and_then(|map| map.get(period)) {
        return found.clone();
    }
    let (top_source, top_model) = row_attribution(rows, period);
    PeriodAttribution {
        top_source,
        top_model,
        top_project: None,
        top_session: None,
    }
}

fn provider_mix(rows: &[DailyRow]) -> (BTreeMap<String, f64>, BTreeMap<String, f64>) {
    let mut token_totals = BTreeMap::<String, u64>::new();
    let mut cost_totals = BTreeMap::<String, f64>::new();
    let mut grand_tokens = 0u64;
    let mut grand_cost = 0.0;
    for row in rows {
        for (source, counts) in &row.sources {
            let label = source.trim().to_string();
            grand_tokens += counts.total_tokens;
            grand_cost += counts.cost_usd;
            *token_totals.entry(label.clone()).or_insert(0) += counts.total_tokens;
            *cost_totals.entry(label).or_insert(0.0) += counts.cost_usd;
        }
    }

    let mut mix_tokens = BTreeMap::new();
    if grand_tokens > 0 {
        for (label, total) in token_totals {
            if total > 0 {
                mix_tokens.insert(label, (total as f64 / grand_tokens as f64) * 100.0);
            }
        }
    }
    let mut mix_cost = BTreeMap::new();
    if grand_cost > f64::EPSILON {
        for (label, total) in cost_totals {
            if total > f64::EPSILON {
                mix_cost.insert(label, (total / grand_cost) * 100.0);
            }
        }
    }
    (mix_tokens, mix_cost)
}

/// Clean and sanitize messy project paths, directory slugs, or hash encodings
/// into human-readable project titles (e.g. "-Users-felix-Projects-crypto-trading" -> "crypto-trading").
pub fn sanitize_project_label(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() || s == "-" {
        return "-".to_string();
    }
    let normalized = s.replace('\\', "/");
    let mut parts: Vec<&str> = if normalized.starts_with('-') && !normalized.contains('/') {
        normalized.split('-').filter(|p| !p.is_empty()).collect()
    } else {
        normalized.split('/').filter(|p| !p.is_empty()).collect()
    };

    while parts.len() > 2 {
        let first = parts[0].to_ascii_lowercase();
        if first == "users" || first == "home" || first == "root" || first == "c:" || first == "d:"
        {
            parts.remove(0);
            if !parts.is_empty() {
                parts.remove(0); // username
            }
            continue;
        }
        if first == "projects"
            || first == "repos"
            || first == "dev"
            || first == "work"
            || first == "code"
        {
            parts.remove(0);
            continue;
        }
        break;
    }

    if parts.is_empty() {
        s.to_string()
    } else if parts.len() >= 2 {
        format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
    } else {
        parts[0].to_string()
    }
}

/// Sanitize session identifiers or file paths to short, clean labels.
pub fn sanitize_session_label(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() || s == "-" {
        return "-".to_string();
    }
    let name = s.rsplit('/').next().unwrap_or(s);
    let name = name.strip_suffix(".jsonl").unwrap_or(name);
    let name = name.strip_suffix(".json").unwrap_or(name);
    if name.len() >= 32 && name.contains('-') {
        let prefix = &name[..8.min(name.len())];
        format!("{prefix}..")
    } else if name.len() > 16 {
        format!("{}..", &name[..14])
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::types::{DailyRow, TokenCounts};

    fn row(date: &str, tokens: u64, cost: f64) -> DailyRow {
        DailyRow {
            date: date.to_string(),
            totals: TokenCounts {
                total_tokens: tokens,
                cost_usd: cost,
                ..TokenCounts::default()
            },
            models: Default::default(),
            sources: Default::default(),
            models_by_source: Default::default(),
            activity: None,
        }
    }

    #[test]
    fn insights_peak_period_selects_max_tokens_row() {
        let rows = vec![row("2026-05-01", 10, 0.1), row("2026-05-02", 50, 0.2)];
        let totals = TokenCounts {
            total_tokens: 60,
            cost_usd: 0.3,
            ..TokenCounts::default()
        };
        let insights = compute_report_insights(&rows, &totals, None);
        assert_eq!(insights.peak_period.unwrap().date, "2026-05-02");
    }

    #[test]
    fn percent_handles_zero_total() {
        assert!(percent(1, 0).is_none());
        assert!((percent(1, 4).unwrap() - 25.0).abs() < 1e-6);
    }

    #[test]
    fn streaks_return_none_for_non_daily_rows() {
        let rows = vec![row("2026-05", 10, 0.1)];
        let (longest, current, active) = streaks_if_daily(&rows);
        assert!(longest.is_none());
        assert!(current.is_none());
        assert_eq!(active, 0);
    }

    #[test]
    fn streaks_compute_for_daily_rows() {
        let rows = vec![
            row("2026-05-01", 1, 0.0),
            row("2026-05-02", 1, 0.0),
            row("2026-05-03", 0, 0.0),
            row("2026-05-04", 1, 0.0),
        ];
        let (longest, current, active_days) = streaks_if_daily(&rows);
        assert_eq!(longest, Some(2));
        assert_eq!(current, Some(1));
        assert_eq!(active_days, 3);
    }

    #[test]
    fn provider_mix_returns_empty_when_no_sources() {
        let rows = vec![row("2026-05-01", 10, 0.1)];
        let (mix_tokens, mix_cost) = provider_mix(&rows);
        assert!(mix_tokens.is_empty());
        assert!(mix_cost.is_empty());
    }

    fn row_with_model(date: &str, model: &str, counts: TokenCounts) -> DailyRow {
        let mut models = BTreeMap::new();
        models.insert(model.to_string(), counts.clone());
        DailyRow {
            date: date.to_string(),
            totals: counts,
            models,
            sources: Default::default(),
            models_by_source: Default::default(),
            activity: None,
        }
    }

    #[test]
    fn cache_economics_computes_net_savings_and_reuse() {
        // claude-sonnet default rates: input 3.0, cache_read 0.3, cache_creation 3.75 per Mtok.
        let counts = TokenCounts {
            cache_read_input_tokens: 10_000_000,    // reads
            cache_creation_input_tokens: 1_000_000, // writes
            total_tokens: 11_000_000,
            ..TokenCounts::default()
        };
        let row = row_with_model("2026-05-01", "claude-sonnet-4-6", counts);
        let savings = cache_savings(&row.models, &row.totals);
        // reads saved: 10M * (3.0 - 0.3)/1e6 = 27.0 ; write premium: 1M * (3.75 - 3.0)/1e6 = 0.75
        let expected = 27.0 - 0.75;
        assert!(
            (savings.unwrap() - expected).abs() < 1e-6,
            "savings={savings:?}"
        );
        // reuse = 10M reads / 1M writes = 10x
        assert_eq!(cache_reuse_ratio(&row.totals), Some(10.0));

        // A history override can add cache volume without any model breakdown.
        let mut totals = row.totals;
        totals.cache_read_input_tokens *= 2;
        assert!(cache_savings(&row.models, &totals).is_none());
    }

    #[test]
    fn cache_economics_suppresses_savings_for_unpriced_models() {
        // Unknown model -> no cache pricing -> coverage 0 -> savings None, reuse still raw.
        let counts = TokenCounts {
            cache_read_input_tokens: 5_000_000,
            cache_creation_input_tokens: 1_000_000,
            total_tokens: 6_000_000,
            ..TokenCounts::default()
        };
        let row = row_with_model("2026-05-01", "some-unlisted-model", counts);
        assert!(cache_savings(&row.models, &row.totals).is_none());
        assert_eq!(cache_reuse_ratio(&row.totals), Some(5.0));
    }

    #[test]
    fn cache_economics_reuse_none_without_writes() {
        let counts = TokenCounts {
            cache_read_input_tokens: 5_000_000,
            total_tokens: 5_000_000,
            ..TokenCounts::default()
        };
        assert!(cache_reuse_ratio(&counts).is_none());
    }

    #[test]
    fn cost_concentration_picks_largest_cost_over_token_divergence() {
        let mut tokens = BTreeMap::new();
        tokens.insert("claude".to_string(), 80.0);
        tokens.insert("codex".to_string(), 20.0);
        let mut cost = BTreeMap::new();
        cost.insert("claude".to_string(), 60.0); // cost share BELOW token share
        cost.insert("codex".to_string(), 40.0); // cost share ABOVE token share (+20pp)
        let conc = cost_concentration(&tokens, &cost).unwrap();
        assert_eq!(conc.label, "codex");
        assert!((conc.token_pct - 20.0).abs() < 1e-6);
        assert!((conc.cost_pct - 40.0).abs() < 1e-6);
    }

    #[test]
    fn cost_concentration_none_when_no_cost() {
        let conc = cost_concentration(&BTreeMap::new(), &BTreeMap::new());
        assert!(conc.is_none());
    }

    #[test]
    fn spikes_include_top_source_and_model() {
        let mut sources = BTreeMap::new();
        sources.insert(
            "codex".to_string(),
            TokenCounts {
                total_tokens: 10_000,
                ..TokenCounts::default()
            },
        );
        let mut models = BTreeMap::new();
        models.insert(
            "gpt-5.2".to_string(),
            TokenCounts {
                total_tokens: 10_000,
                ..TokenCounts::default()
            },
        );
        let mut rows = vec![
            row("2026-05-01", 10, 0.0),
            row("2026-05-02", 10, 0.0),
            row("2026-05-03", 10, 0.0),
            row("2026-05-04", 10, 0.0),
            row("2026-05-05", 10, 0.0),
            row("2026-05-06", 10, 0.0),
            row("2026-05-07", 10_000, 0.0),
        ];
        rows[6].sources = sources;
        rows[6].models = models;

        let spikes = token_spikes(&rows, 3, None);
        assert_eq!(spikes.len(), 1);
        assert_eq!(spikes[0].top_source.as_deref(), Some("codex"));
        assert_eq!(spikes[0].top_model.as_deref(), Some("gpt-5.2"));
    }

    #[test]
    fn spikes_use_period_attribution_when_available() {
        let rows = vec![
            row("2026-05-01", 10, 0.0),
            row("2026-05-02", 10, 0.0),
            row("2026-05-03", 10, 0.0),
            row("2026-05-04", 10, 0.0),
            row("2026-05-05", 10, 0.0),
            row("2026-05-06", 10, 0.0),
            row("2026-05-07", 10_000, 0.0),
        ];
        let mut period_attribution = BTreeMap::new();
        period_attribution.insert(
            "2026-05-07".to_string(),
            PeriodAttribution {
                top_source: Some("opencode".to_string()),
                top_model: Some("gpt-5.2".to_string()),
                top_project: Some("Life/OS".to_string()),
                top_session: Some("ses_abc123".to_string()),
            },
        );

        let spikes = token_spikes(&rows, 3, Some(&period_attribution));
        assert_eq!(spikes.len(), 1);
        assert_eq!(spikes[0].top_source.as_deref(), Some("opencode"));
        assert_eq!(spikes[0].top_model.as_deref(), Some("gpt-5.2"));
        assert_eq!(spikes[0].top_project.as_deref(), Some("Life/OS"));
        assert_eq!(spikes[0].top_session.as_deref(), Some("ses_abc123"));
    }
}
