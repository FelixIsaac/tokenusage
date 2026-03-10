use std::io::{IsTerminal, Read as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};

use crate::cli::{CostSource, StatuslineArgs, VisualBurnRate};
use crate::types::{SourceKind, TokenCounts, UsageEvent};

use super::*;
use super::live::fetch_selected_official_limits;
use super::parsing::load_usage;

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

pub(super) fn read_statusline_hook_input() -> Result<Option<StatuslineHookInput>> {
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

pub(super) fn statusline_cache_path(session_id: Option<&str>) -> PathBuf {
    let suffix = session_id
        .map(sanitize_cache_key)
        .unwrap_or_else(|| "global".to_string());
    std::env::temp_dir().join(format!("tu_statusline_cache_{suffix}.json"))
}

pub(super) fn sanitize_cache_key(input: &str) -> String {
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

pub(super) fn read_statusline_cache(
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

pub(super) fn write_statusline_cache(cache_path: &Path, line: &str, transcript_path: Option<&str>) {
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

pub(super) fn file_mtime_unix(path: &str) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

pub(super) fn aggregate_session_totals(events: &[UsageEvent], session_id: &str) -> Option<TokenCounts> {
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

pub(super) fn session_id_matches(candidate: &str, query: &str) -> bool {
    candidate == query || candidate.ends_with(query) || candidate.contains(query)
}

pub(super) fn active_block_summary(
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

pub(super) fn active_block_summary_for_bounds(
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
pub(super) fn build_statusline_line(
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

pub(super) fn build_statusline_official_codex_segment(
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

pub(super) fn build_statusline_official_claude_segment(
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

pub(super) fn build_statusline_official_antigravity_segment(
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

pub(super) fn format_remaining_minutes(minutes: i64) -> String {
    format!("{} left", format_hours_minutes(minutes))
}

pub(super) fn format_hours_minutes(minutes: i64) -> String {
    let safe = minutes.max(0);
    let hrs = safe / 60;
    let mins = safe % 60;
    if hrs > 0 {
        format!("{hrs}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

pub(super) fn official_window_details(
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

pub(super) fn format_reset_timestamp(unix_secs: i64, tz: &TimeZoneMode) -> String {
    DateTime::from_timestamp(unix_secs, 0)
        .map(|ts| format_display_datetime(ts, tz))
        .unwrap_or_else(|| format!("unix:{unix_secs}"))
}

pub(super) fn format_time_until_reset_short(resets_at: i64, now: DateTime<Utc>) -> String {
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
