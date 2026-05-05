use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::types::{DailyRow, SourceKind, TokenCounts};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReportInsights {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_share_pct: Option<f64>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeakPeriod {
    pub date: String,
    pub total_tokens: u64,
    pub cost_usd: f64,
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
}

pub fn compute_report_insights(rows: &[DailyRow], totals: &TokenCounts) -> ReportInsights {
    let total_tokens = totals.total_tokens;
    let cache_tokens = totals.cache_creation_input_tokens + totals.cache_read_input_tokens;
    let output_tokens = totals.output_tokens + totals.reasoning_output_tokens;

    let cache_share_pct = percent(cache_tokens, total_tokens);
    let output_share_pct = percent(output_tokens, total_tokens);
    let cost_per_mtoken = if total_tokens > 0 && totals.cost_usd > 0.0 {
        Some(totals.cost_usd / (total_tokens as f64 / 1_000_000.0))
    } else {
        None
    };
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

    let spikes = token_spikes(rows, 3);
    let anomalies = token_anomalies(rows, 3);
    let (mix_tokens_pct, mix_cost_pct) = provider_mix(rows);

    ReportInsights {
        cache_share_pct,
        output_share_pct,
        cost_per_mtoken,
        tokens_per_usd,
        top_source,
        top_model,
        top_source_share_pct,
        top_model_share_pct,
        peak_period,
        longest_streak_days,
        current_streak_days,
        avg_tokens_per_active_day,
        avg_cost_per_active_day,
        spikes,
        mix_tokens_pct,
        mix_cost_pct,
        anomalies,
    }
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
        .and_then(|(last, _)| {
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
            Some(run)
        })
        .unwrap_or(0);

    (
        Some(longest).filter(|v| *v > 0),
        Some(current).filter(|v| *v > 0),
        active_days,
    )
}

fn token_spikes(rows: &[DailyRow], limit: usize) -> Vec<SpikePeriod> {
    let mut values = rows
        .iter()
        .filter_map(|row| Some((row.date.clone(), row.totals.total_tokens)))
        .collect::<Vec<_>>();
    if values.len() < 7 {
        return Vec::new();
    }
    let mut token_samples = values.iter().map(|(_, t)| *t).collect::<Vec<_>>();
    token_samples.sort_unstable();
    let median = token_samples[token_samples.len() / 2];
    if median == 0 {
        return Vec::new();
    }

    values.sort_by(|a, b| b.1.cmp(&a.1));
    values
        .into_iter()
        .filter(|(_, tokens)| *tokens > median.saturating_mul(3))
        .take(limit)
        .map(|(date, total_tokens)| {
            let (top_source, top_model) = row_attribution(rows, &date);
            SpikePeriod {
                date,
                total_tokens,
                baseline_median: median,
                top_source,
                top_model,
            }
        })
        .collect()
}

fn token_anomalies(rows: &[DailyRow], limit: usize) -> Vec<AnomalyPeriod> {
    let mut values = rows
        .iter()
        .map(|row| (row.date.clone(), row.totals.total_tokens))
        .collect::<Vec<_>>();
    if values.len() < 7 {
        return Vec::new();
    }
    let mut samples = values.iter().map(|(_, t)| *t).collect::<Vec<_>>();
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    if median == 0 {
        return Vec::new();
    }
    let mut deviations = samples
        .iter()
        .map(|t| t.abs_diff(median))
        .collect::<Vec<_>>();
    deviations.sort_unstable();
    let mad = deviations[deviations.len() / 2];
    if mad == 0 {
        return Vec::new();
    }

    values.sort_by(|a, b| b.1.cmp(&a.1));
    values
        .into_iter()
        .filter_map(|(date, total_tokens)| {
            if total_tokens <= median {
                return None;
            }
            let robust_z = 0.6745 * (total_tokens.abs_diff(median) as f64 / mad.max(1) as f64);
            if robust_z < 3.5 {
                return None;
            }
            let (top_source, top_model) = row_attribution(rows, &date);
            Some(AnomalyPeriod {
                date,
                total_tokens,
                median,
                mad,
                robust_z,
                top_source,
                top_model,
            })
        })
        .take(limit)
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
        let insights = compute_report_insights(&rows, &totals);
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

        let spikes = token_spikes(&rows, 3);
        assert_eq!(spikes.len(), 1);
        assert_eq!(spikes[0].top_source.as_deref(), Some("codex"));
        assert_eq!(spikes[0].top_model.as_deref(), Some("gpt-5.2"));
    }
}
