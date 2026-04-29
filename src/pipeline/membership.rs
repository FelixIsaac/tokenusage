use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::types::{SourceKind, UsageEvent};

use super::statusline::{
    format_reset_timestamp, format_time_until_reset_short, official_window_details,
};
use super::*;

pub(super) fn parse_token_limit_mode(raw: Option<&str>) -> Result<Option<TokenLimitMode>> {
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

pub(super) fn resolve_token_limit(
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

pub(super) fn resolve_token_limit_source(
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

pub(super) fn estimate_membership_from_logs(
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
    for source in [
        SourceKind::Claude,
        SourceKind::Codex,
        SourceKind::Gemini,
        SourceKind::OpenCode,
    ] {
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

pub(super) fn build_membership_source_estimate(
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

pub(super) fn percentile_nearest_rank(sorted_values: &[u64], percentile: f64) -> u64 {
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

pub(super) fn estimate_membership_confidence(
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

pub(super) fn classify_estimated_plan(
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
        Some(SourceKind::Gemini) | Some(SourceKind::OpenCode) => "unknown",
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

pub(super) fn display_plan_label(raw: &str) -> &'static str {
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

pub(super) fn resolve_display_limit(
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

pub(super) fn inferred_plan_key(estimate: Option<&MembershipEstimate>) -> Option<&str> {
    let estimate = estimate?;
    if estimate.source_breakdown.len() == 1 {
        return estimate
            .source_breakdown
            .first()
            .map(|entry| entry.estimated_plan.as_str());
    }

    Some(estimate.estimated_plan.as_str())
}

pub(super) fn next_plan_transition(plan: &str) -> Option<(&'static str, f64)> {
    match plan {
        "claude_pro" => Some(("claude_max_5x", 5.0)),
        "claude_max_5x" => Some(("claude_max_20x", 4.0)),
        "codex_plus_or_business" => Some(("codex_pro", 6.0)),
        "mixed_standard" => Some(("mixed_high", 4.0)),
        "mixed_high" => Some(("mixed_very_high", 3.0)),
        _ => None,
    }
}

pub(super) fn max_completed_block_tokens(
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

pub(super) fn print_membership_estimate(
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
            (
                "primary",
                official.primary_label.as_deref(),
                official.primary_used_percent,
                official.primary_resets_at,
            ),
            (
                "secondary",
                official.secondary_label.as_deref(),
                official.secondary_used_percent,
                official.secondary_resets_at,
            ),
            (
                "tertiary",
                official.tertiary_label.as_deref(),
                official.tertiary_used_percent,
                official.tertiary_resets_at,
            ),
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

pub(super) fn token_limit_source_label(source: TokenLimitSource) -> &'static str {
    match source {
        TokenLimitSource::Explicit => "explicit",
        TokenLimitSource::HistoricalMax => "historical_max",
        TokenLimitSource::EstimatedFromLogs => "estimated_from_logs",
        TokenLimitSource::Unset => "unset",
    }
}
