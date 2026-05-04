use std::collections::BTreeMap;

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
    pub peak_period: Option<PeakPeriod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeakPeriod {
    pub date: String,
    pub total_tokens: u64,
    pub cost_usd: f64,
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

    let top_source = top_source_label(rows);
    let peak_period = rows
        .iter()
        .max_by_key(|row| row.totals.total_tokens)
        .map(|row| PeakPeriod {
            date: row.date.clone(),
            total_tokens: row.totals.total_tokens,
            cost_usd: row.totals.cost_usd,
        })
        .filter(|peak| peak.total_tokens > 0);

    ReportInsights {
        cache_share_pct,
        output_share_pct,
        cost_per_mtoken,
        tokens_per_usd,
        top_source,
        peak_period,
    }
}

fn percent(part: u64, total: u64) -> Option<f64> {
    if total == 0 {
        return None;
    }
    Some((part as f64 / total as f64) * 100.0)
}

fn top_source_label(rows: &[DailyRow]) -> Option<String> {
    let mut totals = BTreeMap::<SourceKind, u64>::new();
    for row in rows {
        for (source, counts) in &row.sources {
            if let Some(kind) = parse_source_kind(source) {
                *totals.entry(kind).or_insert(0) += counts.total_tokens;
            }
        }
    }
    totals
        .into_iter()
        .max_by_key(|(_, total)| *total)
        .map(|(kind, _)| kind.display_name().to_string())
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
}
