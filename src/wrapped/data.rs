use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{Datelike, NaiveDate, TimeDelta};
use serde::{Deserialize, Serialize};

use crate::carbon::{
    EnvironmentalEquivalences, EnvironmentalMetrics, GridRegion, format_carbon_human,
    format_commas_u64, format_water_human,
};
use crate::cli::WeekStart;
use crate::insights::{
    compute_report_insights_with_pricing, sanitize_project_label, sanitize_session_label,
};
use crate::pipeline::{
    LocalDayCache, ReportPeriod, TimeZoneMode, apply_group_history, build_group_rows, week_start,
};
use crate::scale::InformationScale;
use crate::tui_theme::{format_compact_tokens, format_compact_usd};
use crate::types::{DailyRow, PricingTable, SourceKind, TokenCounts, UsageEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedReport {
    pub period_label: String,
    pub year: Option<i32>,
    pub all_time: bool,
    pub include_projects: bool,
    pub timezone: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub active_days: u32,
    pub span_days: u32,
    pub total_events: u64,
    pub total_sessions: u64,
    pub longest_streak_days: u32,
    pub current_streak_days: u32,
    pub totals: TokenCounts,
    pub cost_per_mtoken_usd: Option<f64>,
    pub archetype: WrappedArchetype,
    pub chronotype: WrappedChronotype,
    pub heatmap: WrappedHeatmap,
    pub hourly_tokens: [u64; 24],
    pub weekday_tokens: [u64; 7],
    pub weekend_share_pct: f64,
    pub monthly: Vec<WrappedMonthStat>,
    pub quarterly_eras: Vec<WrappedQuarterEra>,
    pub top_providers: Vec<WrappedProviderStat>,
    pub top_models: Vec<WrappedModelStat>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_projects: Vec<WrappedProjectStat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_day: Option<WrappedDayHighlight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_spend_day: Option<WrappedDayHighlight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_session: Option<WrappedSessionHighlight>,
    pub cache: WrappedCacheStats,
    pub scale: InformationScale,
    pub scale_headline: String,
    pub pop_culture_scale: String,
    pub carbon: WrappedCarbonStats,
    pub badges: Vec<WrappedBadge>,
    pub share_blurb: String,
    pub agent_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedArchetype {
    pub id: String,
    pub tier: String,
    pub title: String,
    pub tagline: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedChronotype {
    pub label: String,
    pub window: String,
    pub peak_hour: u32,
    pub night_pct: f64,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedHeatmapDay {
    pub date: String,
    pub tokens: u64,
    pub cost_usd: f64,
    pub level: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedHeatmap {
    pub start_date: String,
    pub end_date: String,
    pub weeks: usize,
    pub max_day_tokens: u64,
    /// 7 rows (Mon..Sun) x `weeks` columns with intensity 0..=4.
    pub grid: Vec<Vec<u8>>,
    pub week_dates: Vec<String>,
    pub days: Vec<WrappedHeatmapDay>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedMonthStat {
    pub month: String,
    pub label: String,
    pub tokens: u64,
    pub cost_usd: f64,
    pub active_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedQuarterEra {
    pub quarter: String,
    pub title: String,
    pub top_model: String,
    pub top_provider: String,
    pub tokens: u64,
    pub cost_usd: f64,
    pub share_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedProviderStat {
    pub name: String,
    pub tokens: u64,
    pub cost_usd: f64,
    pub share_pct: f64,
    pub active_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedModelStat {
    pub name: String,
    pub raw_name: String,
    pub tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
    pub share_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedProjectStat {
    pub name: String,
    pub tokens: u64,
    pub cost_usd: f64,
    pub share_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedDayHighlight {
    pub date: String,
    pub tokens: u64,
    pub cost_usd: f64,
    pub top_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedSessionHighlight {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub date: String,
    pub model: String,
    pub tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedCacheStats {
    pub hit_pct: f64,
    pub share_pct: f64,
    pub reuse_ratio: f64,
    pub savings_usd: f64,
    pub uncached_cost_usd: f64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedCarbonStats {
    pub energy_kwh: f64,
    pub carbon_gco2e: f64,
    pub water_ml: f64,
    pub smartphone_charges: f64,
    pub tree_months: f64,
    pub cups_boiled: f64,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedBadge {
    pub id: String,
    pub glyph: String,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub(crate) struct WrappedBundle {
    pub(crate) report: WrappedReport,
    pub(crate) revealed_projects: Vec<WrappedProjectStat>,
    pub(crate) revealed_session_label: Option<String>,
}

impl WrappedBundle {
    pub(crate) fn set_include_projects(&mut self, include: bool) {
        self.report.include_projects = include;
        if include {
            self.report.top_projects = self.revealed_projects.clone();
            if let Some(session) = self.report.peak_session.as_mut() {
                session.label = self.revealed_session_label.clone();
            }
        } else {
            self.report.top_projects.clear();
            if let Some(session) = self.report.peak_session.as_mut() {
                session.label = None;
            }
        }
        self.report.share_blurb = build_share_blurb(&self.report);
        self.report.agent_prompt = build_agent_prompt(&self.report);
    }
}

pub(crate) struct WrappedBuildOptions<'a> {
    pub(crate) events: &'a [UsageEvent],
    pub(crate) tz: &'a TimeZoneMode,
    pub(crate) timezone_label: String,
    pub(crate) pricing: &'a PricingTable,
    pub(crate) target_year: Option<i32>,
    pub(crate) all_time: bool,
    pub(crate) include_projects: bool,
    pub(crate) common: Option<&'a crate::cli::CommonArgs>,
}

#[derive(Default)]
struct DayAgg {
    counts: TokenCounts,
    by_model: HashMap<String, u64>,
}

#[derive(Default)]
struct ProviderAgg {
    tokens: u64,
    cost_usd: f64,
    days: BTreeSet<NaiveDate>,
}

#[derive(Default)]
struct ModelAgg {
    tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cost_usd: f64,
}

#[derive(Default)]
struct SessionAgg {
    tokens: u64,
    cost_usd: f64,
    first_date: Option<NaiveDate>,
    by_model: HashMap<String, u64>,
}

pub(crate) fn build_wrapped_bundle(opts: WrappedBuildOptions<'_>) -> WrappedBundle {
    let mut day_cache = LocalDayCache::new(opts.tz);
    let now_day = opts.tz.now_date();

    let mut tagged: Vec<(&UsageEvent, NaiveDate, u32)> = Vec::with_capacity(opts.events.len());
    let mut min_year = i32::MAX;
    let mut max_year = i32::MIN;
    let mut has_current_year = false;

    for event in opts.events {
        let (day, hour) = day_cache.date_and_hour(event.timestamp);
        let y = day.year();
        min_year = min_year.min(y);
        max_year = max_year.max(y);
        if y == now_day.year() {
            has_current_year = true;
        }
        tagged.push((event, day, hour));
    }

    let resolved_year = if opts.all_time {
        None
    } else if let Some(y) = opts.target_year {
        Some(y)
    } else if opts
        .common
        .is_some_and(|c| c.since.is_some() || c.until.is_some())
    {
        if min_year != i32::MAX && min_year == max_year {
            Some(max_year)
        } else {
            None
        }
    } else if has_current_year {
        Some(now_day.year())
    } else if max_year != i32::MIN {
        Some(max_year)
    } else {
        Some(now_day.year())
    };

    let mut filtered_events: Vec<&UsageEvent> = Vec::with_capacity(tagged.len());
    let mut hourly_tokens = [0u64; 24];
    let mut weekday_tokens = [0u64; 7];
    let mut weekend_tokens = 0u64;

    let mut by_day: BTreeMap<NaiveDate, DayAgg> = BTreeMap::new();
    let mut by_provider: HashMap<SourceKind, ProviderAgg> = HashMap::new();
    let mut by_model: HashMap<String, ModelAgg> = HashMap::new();
    let mut by_project: HashMap<String, (u64, f64)> = HashMap::new();
    let mut by_session: HashMap<String, SessionAgg> = HashMap::new();
    let mut by_quarter: [HashMap<String, (u64, f64)>; 4] = Default::default();
    let mut by_quarter_prov: [HashMap<SourceKind, u64>; 4] = Default::default();
    let mut by_quarter_totals: [(u64, f64); 4] = [(0, 0.0); 4];

    for (event, day, hour) in tagged {
        if let Some(y) = resolved_year
            && day.year() != y
        {
            continue;
        }
        filtered_events.push(event);
        let counts = event.usage.to_counts();
        let tok = counts.total_tokens;
        let cost = counts.cost_usd;

        hourly_tokens[(hour as usize).min(23)] += tok;
        let wd = day.weekday().num_days_from_monday() as usize; // 0=Mon..6=Sun
        weekday_tokens[wd] += tok;
        if wd >= 5 {
            weekend_tokens += tok;
        }

        let day_agg = by_day.entry(day).or_default();
        day_agg.counts.add_assign(counts.clone());
        *day_agg.by_model.entry(event.model.clone()).or_insert(0) += tok;

        let prov = by_provider.entry(event.source).or_default();
        prov.tokens += tok;
        prov.cost_usd += cost;
        prov.days.insert(day);

        let mdl = by_model.entry(event.model.clone()).or_default();
        mdl.tokens += tok;
        mdl.output_tokens += counts.output_tokens + counts.reasoning_output_tokens;
        mdl.cache_read_tokens += counts.cache_read_input_tokens;
        mdl.cost_usd += cost;

        if let Some(proj) = event.project.as_deref()
            && !proj.trim().is_empty()
            && proj.trim() != "-"
        {
            let clean = sanitize_project_label(proj);
            let entry = by_project.entry(clean).or_insert((0, 0.0));
            entry.0 += tok;
            entry.1 += cost;
        }

        if !event.session.trim().is_empty() && event.session.trim() != "-" {
            let sess = by_session.entry(event.session.clone()).or_default();
            sess.tokens += tok;
            sess.cost_usd += cost;
            sess.first_date = Some(sess.first_date.map_or(day, |prev| prev.min(day)));
            *sess.by_model.entry(event.model.clone()).or_insert(0) += tok;
        }

        let q_idx = ((day.month() - 1) / 3) as usize;
        by_quarter_totals[q_idx].0 += tok;
        by_quarter_totals[q_idx].1 += cost;
        let qm = by_quarter[q_idx]
            .entry(event.model.clone())
            .or_insert((0, 0.0));
        qm.0 += tok;
        qm.1 += cost;
        *by_quarter_prov[q_idx].entry(event.source).or_insert(0) += tok;
    }

    // Build DailyRow list for insights & optional history DB merge
    let cloned_events: Vec<UsageEvent> = filtered_events.iter().map(|e| (*e).clone()).collect();
    let mut daily_day_cache = LocalDayCache::new(opts.tz);
    let mut daily_rows: Vec<DailyRow> = build_group_rows(
        &cloned_events,
        |event| daily_day_cache.date_of(event.timestamp),
        |day| day.format("%Y-%m-%d").to_string(),
    );

    if let Some(common) = opts.common {
        let _ = common.order;
        if opts.all_time {
            apply_group_history(
                &mut daily_rows,
                ReportPeriod::Daily,
                common,
                WeekStart::Monday,
            );
        }
    }
    daily_rows.sort_by(|a, b| a.date.cmp(&b.date));

    let mut totals = TokenCounts::default();
    for row in &daily_rows {
        totals.add_assign(row.totals.clone());
    }

    let insights = compute_report_insights_with_pricing(&daily_rows, &totals, None, opts.pricing);

    let start_date = daily_rows.first().map(|r| r.date.clone());
    let end_date = daily_rows.last().map(|r| r.date.clone());
    let active_days = daily_rows
        .iter()
        .filter(|r| r.totals.total_tokens > 0)
        .count() as u32;
    let span_days = match (
        start_date
            .as_deref()
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()),
        end_date
            .as_deref()
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()),
    ) {
        (Some(s), Some(e)) if e >= s => ((e - s).num_days() + 1) as u32,
        _ => 0,
    };

    let (longest_streak_days, current_streak_days) = compute_streaks(&by_day);

    let period_label = match (resolved_year, opts.all_time) {
        (Some(y), false) => y.to_string(),
        _ => match (start_date.as_deref(), end_date.as_deref()) {
            (Some(s), Some(e)) if s.len() >= 4 && e.len() >= 4 => {
                if s[..4] == e[..4] {
                    s[..4].to_string()
                } else {
                    format!("{}–{}", &s[..4], &e[..4])
                }
            }
            _ => "All-Time".to_string(),
        },
    };

    // Build monthly breakdown (all 12 months of resolved_year, or observed months for all-time)
    let monthly = build_monthly_stats(&by_day, resolved_year);

    // Quarterly eras
    let quarterly_eras = build_quarterly_eras(
        totals.total_tokens,
        &by_quarter_totals,
        &by_quarter,
        &by_quarter_prov,
    );

    // Provider leaderboard
    let mut top_providers: Vec<WrappedProviderStat> = by_provider
        .into_iter()
        .map(|(source, agg)| WrappedProviderStat {
            name: provider_display_name(source).to_string(),
            tokens: agg.tokens,
            cost_usd: agg.cost_usd,
            share_pct: pct_of(agg.tokens, totals.total_tokens),
            active_days: agg.days.len() as u32,
        })
        .collect();
    top_providers.sort_by_key(|a| std::cmp::Reverse(a.tokens));

    // Model leaderboard (normalized display name while keeping raw)
    let mut top_models: Vec<WrappedModelStat> = by_model
        .into_iter()
        .map(|(raw_name, agg)| WrappedModelStat {
            name: sanitize_model_display(&raw_name),
            raw_name,
            tokens: agg.tokens,
            output_tokens: agg.output_tokens,
            cache_read_tokens: agg.cache_read_tokens,
            cost_usd: agg.cost_usd,
            share_pct: pct_of(agg.tokens, totals.total_tokens),
        })
        .collect();
    top_models.sort_by_key(|a| std::cmp::Reverse(a.tokens));
    top_models.truncate(10);

    // Projects (prepared for toggle, omitted from report unless include_projects = true)
    let mut revealed_projects: Vec<WrappedProjectStat> = by_project
        .into_iter()
        .map(|(name, (tokens, cost_usd))| WrappedProjectStat {
            name,
            tokens,
            cost_usd,
            share_pct: pct_of(tokens, totals.total_tokens),
        })
        .collect();
    revealed_projects.sort_by_key(|a| std::cmp::Reverse(a.tokens));
    revealed_projects.truncate(8);

    // Peak days & peak session
    let peak_day = by_day
        .iter()
        .max_by_key(|(_, agg)| agg.counts.total_tokens)
        .filter(|(_, agg)| agg.counts.total_tokens > 0)
        .map(|(day, agg)| WrappedDayHighlight {
            date: day.format("%Y-%m-%d").to_string(),
            tokens: agg.counts.total_tokens,
            cost_usd: agg.counts.cost_usd,
            top_model: top_key_u64(&agg.by_model)
                .map(|m| sanitize_model_display(&m))
                .unwrap_or_else(|| "-".to_string()),
        });

    let peak_spend_day = by_day
        .iter()
        .max_by(|(_, a), (_, b)| a.counts.cost_usd.total_cmp(&b.counts.cost_usd))
        .filter(|(_, agg)| agg.counts.cost_usd > 0.0)
        .map(|(day, agg)| WrappedDayHighlight {
            date: day.format("%Y-%m-%d").to_string(),
            tokens: agg.counts.total_tokens,
            cost_usd: agg.counts.cost_usd,
            top_model: top_key_u64(&agg.by_model)
                .map(|m| sanitize_model_display(&m))
                .unwrap_or_else(|| "-".to_string()),
        });

    let total_sessions = by_session.len() as u64;
    let peak_session_pair = by_session
        .into_iter()
        .max_by_key(|(_, agg)| agg.tokens)
        .filter(|(_, agg)| agg.tokens > 0);
    let revealed_session_label = peak_session_pair
        .as_ref()
        .map(|(id, _)| sanitize_session_label(id));
    let peak_session = peak_session_pair.map(|(_, agg)| WrappedSessionHighlight {
        label: opts
            .include_projects
            .then(|| revealed_session_label.clone())
            .flatten(),
        date: agg
            .first_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "-".to_string()),
        model: top_key_u64(&agg.by_model)
            .map(|m| sanitize_model_display(&m))
            .unwrap_or_else(|| "-".to_string()),
        tokens: agg.tokens,
        cost_usd: agg.cost_usd,
    });

    // Heatmap
    let heatmap = build_heatmap(&by_day, resolved_year, now_day);

    // Chronotype & Archetype
    let weekend_share_pct = pct_of(weekend_tokens, totals.total_tokens);
    let chronotype = classify_chronotype(&hourly_tokens);

    // Cache stats
    let prompt_tokens =
        totals.input_tokens + totals.cache_creation_input_tokens + totals.cache_read_input_tokens;
    let cache_hit_pct = pct_of(totals.cache_read_input_tokens, prompt_tokens);
    let cache_share_pct = pct_of(
        totals.cache_read_input_tokens + totals.cache_creation_input_tokens,
        totals.total_tokens,
    );
    let reuse_ratio = insights.cache_reuse_ratio.unwrap_or_else(|| {
        if totals.cache_creation_input_tokens > 0 {
            totals.cache_read_input_tokens as f64 / totals.cache_creation_input_tokens as f64
        } else {
            0.0
        }
    });
    let savings_usd = insights
        .cache_savings_usd
        .unwrap_or_else(|| estimate_fallback_cache_savings(&totals));
    let cache = WrappedCacheStats {
        hit_pct: cache_hit_pct,
        share_pct: cache_share_pct,
        reuse_ratio,
        savings_usd: savings_usd.max(0.0),
        uncached_cost_usd: totals.cost_usd + savings_usd.max(0.0),
        cache_read_tokens: totals.cache_read_input_tokens,
        cache_write_tokens: totals.cache_creation_input_tokens,
    };

    // Scale & Carbon
    let scale = InformationScale::from_counts(&totals);
    let scale_headline = scale.digestion_headline();
    let pop_culture_scale = compute_pop_culture_scale(&scale);
    let carbon_metrics =
        EnvironmentalMetrics::calculate_events(&cloned_events, GridRegion::UsEast, 1.2, 0.5);
    let eq = EnvironmentalEquivalences::from_metrics(&carbon_metrics);
    let carbon = WrappedCarbonStats {
        energy_kwh: carbon_metrics.energy_kwh,
        carbon_gco2e: carbon_metrics.carbon_gco2e,
        water_ml: carbon_metrics.water_ml,
        smartphone_charges: eq.smartphone_charges,
        tree_months: eq.tree_months,
        cups_boiled: eq.cups_boiled,
        summary: format!(
            "{:.1} kWh · {} · {} water (~{} phone charges)",
            carbon_metrics.energy_kwh,
            format_carbon_human(carbon_metrics.carbon_gco2e),
            format_water_human(carbon_metrics.water_ml),
            format_commas_u64(eq.smartphone_charges.round() as u64),
        ),
    };

    let top_provider_share_pct = top_providers.first().map_or(0.0, |p| p.share_pct);
    let archetype = classify_archetype(
        &totals,
        &chronotype,
        &cache,
        top_providers.len(),
        top_provider_share_pct,
        weekend_share_pct,
        longest_streak_days,
        active_days,
    );

    let badges = compute_badges(
        &totals,
        &cache,
        &chronotype,
        &scale,
        &top_providers,
        longest_streak_days,
        weekend_share_pct,
    );

    let cost_per_mtoken_usd = if totals.total_tokens > 0 && totals.cost_usd > 0.0 {
        Some(totals.cost_usd / (totals.total_tokens as f64 / 1_000_000.0))
    } else {
        None
    };

    let mut report = WrappedReport {
        period_label,
        year: resolved_year,
        all_time: opts.all_time,
        include_projects: opts.include_projects,
        timezone: opts.timezone_label,
        start_date,
        end_date,
        active_days,
        span_days,
        total_events: filtered_events.len() as u64,
        total_sessions,
        longest_streak_days,
        current_streak_days,
        totals,
        cost_per_mtoken_usd,
        archetype,
        chronotype,
        heatmap,
        hourly_tokens,
        weekday_tokens,
        weekend_share_pct,
        monthly,
        quarterly_eras,
        top_providers,
        top_models,
        top_projects: if opts.include_projects {
            revealed_projects.clone()
        } else {
            Vec::new()
        },
        peak_day,
        peak_spend_day,
        peak_session,
        cache,
        scale,
        scale_headline,
        pop_culture_scale,
        carbon,
        badges,
        share_blurb: String::new(),
        agent_prompt: String::new(),
    };
    report.share_blurb = build_share_blurb(&report);
    report.agent_prompt = build_agent_prompt(&report);

    WrappedBundle {
        report,
        revealed_projects,
        revealed_session_label,
    }
}

fn pct_of(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (part as f64 / total as f64) * 100.0
    }
}

fn top_key_u64(map: &HashMap<String, u64>) -> Option<String> {
    map.iter().max_by_key(|(_, v)| *v).map(|(k, _)| k.clone())
}

fn provider_display_name(source: SourceKind) -> &'static str {
    match source {
        SourceKind::Claude => "Claude Code",
        SourceKind::Codex => "Codex CLI",
        SourceKind::Gemini => "Gemini CLI",
        SourceKind::OpenCode => "OpenCode",
        SourceKind::Grok => "Grok CLI",
        SourceKind::Pi => "Pi",
        SourceKind::Muse => "Muse",
    }
}

pub(crate) fn sanitize_model_display(raw: &str) -> String {
    let mut model = raw.trim();
    for prefix in ["claude:", "codex:", "openrouter/", "anthropic/", "openai/"] {
        if let Some(rest) = model.strip_prefix(prefix) {
            model = rest;
        }
    }
    let mut out = model.to_string();
    if let Some((base, suffix)) = out.rsplit_once('-')
        && suffix.len() == 8
        && suffix.chars().all(|c| c.is_ascii_digit())
    {
        out = base.to_string();
    }
    if let Some(rest) = out.strip_prefix("claude-") {
        out = rest.to_string();
    }
    out
}

fn estimate_fallback_cache_savings(totals: &TokenCounts) -> f64 {
    // Conservative fallback when model pricing lookup is incomplete: ~$2.70 saved per 1M cache reads.
    (totals.cache_read_input_tokens as f64 / 1_000_000.0) * 2.7
}

fn compute_streaks(by_day: &BTreeMap<NaiveDate, DayAgg>) -> (u32, u32) {
    let active_dates: Vec<NaiveDate> = by_day
        .iter()
        .filter(|(_, agg)| agg.counts.total_tokens > 0)
        .map(|(d, _)| *d)
        .collect();
    if active_dates.is_empty() {
        return (0, 0);
    }

    let mut longest = 1u32;
    let mut current = 1u32;
    for pair in active_dates.windows(2) {
        if (pair[1] - pair[0]).num_days() == 1 {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 1;
        }
    }
    (longest, current)
}

fn build_monthly_stats(
    by_day: &BTreeMap<NaiveDate, DayAgg>,
    resolved_year: Option<i32>,
) -> Vec<WrappedMonthStat> {
    const NAMES: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut by_ym: BTreeMap<(i32, u32), (u64, f64, u32)> = BTreeMap::new();
    for (day, agg) in by_day {
        let entry = by_ym
            .entry((day.year(), day.month()))
            .or_insert((0, 0.0, 0));
        entry.0 += agg.counts.total_tokens;
        entry.1 += agg.counts.cost_usd;
        if agg.counts.total_tokens > 0 {
            entry.2 += 1;
        }
    }

    if let Some(year) = resolved_year {
        (1..=12)
            .map(|m| {
                let (tokens, cost_usd, active_days) =
                    by_ym.get(&(year, m)).copied().unwrap_or((0, 0.0, 0));
                WrappedMonthStat {
                    month: format!("{year:04}-{m:02}"),
                    label: NAMES[(m - 1) as usize].to_string(),
                    tokens,
                    cost_usd,
                    active_days,
                }
            })
            .collect()
    } else {
        by_ym
            .into_iter()
            .rev()
            .take(12)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(
                |((y, m), (tokens, cost_usd, active_days))| WrappedMonthStat {
                    month: format!("{y:04}-{m:02}"),
                    label: format!("{} '{:02}", NAMES[(m - 1) as usize], y.rem_euclid(100)),
                    tokens,
                    cost_usd,
                    active_days,
                },
            )
            .collect()
    }
}

fn build_quarterly_eras(
    total_tokens: u64,
    totals: &[(u64, f64); 4],
    models: &[HashMap<String, (u64, f64)>; 4],
    providers: &[HashMap<SourceKind, u64>; 4],
) -> Vec<WrappedQuarterEra> {
    let mut eras = Vec::with_capacity(4);
    for idx in 0..4 {
        let (q_tok, q_cost) = totals[idx];
        let top_model = models[idx]
            .iter()
            .max_by_key(|(_, (t, _))| *t)
            .map(|(m, _)| sanitize_model_display(m))
            .unwrap_or_else(|| "Quiet".to_string());
        let top_provider = providers[idx]
            .iter()
            .max_by_key(|(_, t)| *t)
            .map(|(s, _)| provider_display_name(*s).to_string())
            .unwrap_or_else(|| "-".to_string());
        let title = if q_tok == 0 {
            "Resting Orbit".to_string()
        } else {
            format!("{} Era", short_era_token(&top_model))
        };
        eras.push(WrappedQuarterEra {
            quarter: format!("Q{}", idx + 1),
            title,
            top_model,
            top_provider,
            tokens: q_tok,
            cost_usd: q_cost,
            share_pct: pct_of(q_tok, total_tokens),
        });
    }
    eras
}

fn short_era_token(model: &str) -> String {
    let lower = model.to_ascii_lowercase();
    if lower.contains("opus") {
        "Opus".to_string()
    } else if lower.contains("sonnet") {
        "Sonnet".to_string()
    } else if lower.contains("haiku") {
        "Haiku".to_string()
    } else if lower.contains("codex") {
        "Codex".to_string()
    } else if lower.contains("gpt-5") {
        "GPT-5".to_string()
    } else if lower.contains("gemini") {
        "Gemini".to_string()
    } else if lower.contains("grok") {
        "Grok".to_string()
    } else {
        model
            .split(['-', ' '])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("Builder")
            .to_string()
    }
}

fn build_heatmap(
    by_day: &BTreeMap<NaiveDate, DayAgg>,
    resolved_year: Option<i32>,
    fallback_day: NaiveDate,
) -> WrappedHeatmap {
    let (raw_start, raw_end) = if let Some(year) = resolved_year {
        let s = NaiveDate::from_ymd_opt(year, 1, 1).expect("valid Jan 1");
        let dec31 = NaiveDate::from_ymd_opt(year, 12, 31).expect("valid Dec 31");
        let last_event_day = by_day.keys().next_back().copied().unwrap_or(dec31);
        let e = if year == fallback_day.year() {
            fallback_day.max(last_event_day).min(dec31)
        } else {
            dec31
        };
        (s, e)
    } else {
        let end = by_day.keys().next_back().copied().unwrap_or(fallback_day);
        let start = end - TimeDelta::days(364);
        (start, end)
    };

    let grid_start = week_start(raw_start, WeekStart::Monday);
    let end_week_start = week_start(raw_end, WeekStart::Monday);
    let total_weeks = (((end_week_start - grid_start).num_days() / 7) + 1).clamp(1, 54) as usize;

    let mut non_zero_tokens: Vec<u64> = by_day
        .iter()
        .filter(|(d, agg)| **d >= raw_start && **d <= raw_end && agg.counts.total_tokens > 0)
        .map(|(_, agg)| agg.counts.total_tokens)
        .collect();
    non_zero_tokens.sort_unstable();

    let quantile = |q: f64| -> u64 {
        if non_zero_tokens.is_empty() {
            return 0;
        }
        let idx = ((non_zero_tokens.len() - 1) as f64 * q).round() as usize;
        non_zero_tokens[idx]
    };
    let q1 = quantile(0.25);
    let q2 = quantile(0.50);
    let q3 = quantile(0.75);
    let max_day_tokens = non_zero_tokens.last().copied().unwrap_or(0);

    let level_for = |tokens: u64| -> u8 {
        if tokens == 0 {
            0
        } else if tokens <= q1 {
            1
        } else if tokens <= q2 {
            2
        } else if tokens <= q3 {
            3
        } else {
            4
        }
    };

    let mut grid = vec![vec![0u8; total_weeks]; 7];
    let week_dates: Vec<String> = (0..total_weeks)
        .map(|week| {
            (grid_start + TimeDelta::days((week * 7) as i64))
                .format("%Y-%m-%d")
                .to_string()
        })
        .collect();
    let mut days = Vec::with_capacity(total_weeks * 7);

    for (row, grid_row) in grid.iter_mut().enumerate() {
        for (week, cell) in grid_row.iter_mut().enumerate() {
            let day = grid_start + TimeDelta::days((week * 7 + row) as i64);
            let in_window = day >= raw_start && day <= raw_end;
            let agg = if in_window { by_day.get(&day) } else { None };
            let tokens = agg.map_or(0, |a| a.counts.total_tokens);
            let cost_usd = agg.map_or(0.0, |a| a.counts.cost_usd);
            let lvl = level_for(tokens);
            *cell = lvl;
            if in_window {
                days.push(WrappedHeatmapDay {
                    date: day.format("%Y-%m-%d").to_string(),
                    tokens,
                    cost_usd,
                    level: lvl,
                });
            }
        }
    }
    days.sort_by(|a, b| a.date.cmp(&b.date));

    WrappedHeatmap {
        start_date: raw_start.format("%Y-%m-%d").to_string(),
        end_date: raw_end.format("%Y-%m-%d").to_string(),
        weeks: total_weeks,
        max_day_tokens,
        grid,
        week_dates,
        days,
    }
}

pub(crate) fn classify_chronotype(hourly: &[u64; 24]) -> WrappedChronotype {
    let total: u64 = hourly.iter().sum();
    let peak_hour = hourly
        .iter()
        .enumerate()
        .max_by_key(|(_, v)| *v)
        .map_or(14, |(h, _)| h as u32);

    let early: u64 = hourly[5..11].iter().sum();
    let daylight: u64 = hourly[11..18].iter().sum();
    let late_night: u64 = hourly[22..24].iter().sum::<u64>() + hourly[0..5].iter().sum::<u64>();
    let evening: u64 = hourly[18..22].iter().sum();

    let night_pct = pct_of(late_night, total);
    let early_pct = pct_of(early, total);
    let active_hours = hourly
        .iter()
        .filter(|&&v| total > 0 && (v as f64 / total as f64) >= 0.015)
        .count();

    let (label, window, detail) = if active_hours >= 18 && night_pct >= 12.0 {
        (
            "Cyberpunk Netrunner",
            "24/7 Neural Link".to_string(),
            format!(
                "Prompt flow across {active_hours}/24 hours; peak telemetry at {:02}:00 ({night_pct:.0}% late-night).",
                peak_hour
            ),
        )
    } else if night_pct >= 28.0 || !(4..22).contains(&peak_hour) {
        (
            "Gotham Night Shift",
            "22:00 – 04:00".to_string(),
            format!(
                "{night_pct:.0}% of tokens burned after 22:00; peak bat-signal at {:02}:00.",
                peak_hour
            ),
        )
    } else if early_pct >= 32.0 || (5..=10).contains(&peak_hour) {
        (
            "Rocky 5AM Montage",
            "05:00 – 11:00".to_string(),
            format!(
                "{early_pct:.0}% of volume shipped before 11:00; sunrise peak at {:02}:00.",
                peak_hour
            ),
        )
    } else if evening > daylight {
        (
            "After-Hours Boss Fight",
            "18:00 – 22:00".to_string(),
            format!(
                "After-hours momentum surged between 18:00 and 22:00; peak at {:02}:00.",
                peak_hour
            ),
        )
    } else {
        (
            "Lumon Macrodata Refiner",
            "09:00 – 18:00".to_string(),
            format!(
                "Focused daytime macrodata refinement anchored around {:02}:00.",
                peak_hour
            ),
        )
    };

    WrappedChronotype {
        label: label.to_string(),
        window,
        peak_hour,
        night_pct,
        detail,
    }
}

pub(crate) fn compute_usage_tier(totals: &TokenCounts, active_days: u32) -> String {
    let avg_cost = totals.cost_usd / (active_days.max(1) as f64);
    if avg_cost >= 150.0 || totals.total_tokens >= 50_000_000_000 {
        "Est. Top 0.1% · Hyperscale Tier".to_string()
    } else if avg_cost >= 60.0 || totals.total_tokens >= 10_000_000_000 {
        "Est. Top 1% · Power-Law Tier".to_string()
    } else if avg_cost >= 30.0 || totals.total_tokens >= 2_000_000_000 {
        "Est. Top 5% · Heavy Agentic Tier".to_string()
    } else if avg_cost >= 13.0 || totals.total_tokens >= 500_000_000 {
        "Est. Top 20% · Daily Shipper".to_string()
    } else {
        "Precision Builder Tier".to_string()
    }
}

pub(crate) fn compute_pop_culture_scale(scale: &InformationScale) -> String {
    const BEE_MOVIE: f64 = 9_165.0;
    const THE_HOBBIT: f64 = 95_022.0;
    const DUNE: f64 = 188_000.0;
    const MOBY_DICK: f64 = 209_117.0;
    const LOTR_TRILOGY: f64 = 481_103.0;
    const HARRY_POTTER: f64 = 1_084_170.0;
    const ASOIAF: f64 = 1_770_000.0;
    const WHEEL_OF_TIME: f64 = 4_410_036.0;

    let w = scale.output_words as f64;
    if w < 50_000.0 {
        format!(
            "~{:.1}× Bee Movie scripts written",
            (w / BEE_MOVIE).max(0.1)
        )
    } else if w < 300_000.0 {
        format!(
            "~{:.1}× The Hobbit (or ~{:.0}× Bee Movie scripts)",
            w / THE_HOBBIT,
            w / BEE_MOVIE
        )
    } else if w < 1_000_000.0 {
        format!(
            "~{:.1}× Dune novels (or ~{:.1}× Moby-Dick)",
            w / DUNE,
            w / MOBY_DICK
        )
    } else if w < 10_000_000.0 {
        format!(
            "~{:.1}× Lord of the Rings trilogies (~{:.1}× Dune)",
            w / LOTR_TRILOGY,
            w / DUNE
        )
    } else if w < 100_000_000.0 {
        format!(
            "~{:.1}× Harry Potter series (~{:.1}× Wheel of Time)",
            w / HARRY_POTTER,
            w / WHEEL_OF_TIME
        )
    } else {
        let hp = format_commas_u64((w / HARRY_POTTER).round() as u64);
        let lotr = format_commas_u64((w / LOTR_TRILOGY).round() as u64);
        let _asoiaf = ASOIAF;
        format!("~{hp}× Harry Potter series (or ~{lotr}× LOTR trilogies)")
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn classify_archetype(
    totals: &TokenCounts,
    chronotype: &WrappedChronotype,
    cache: &WrappedCacheStats,
    provider_count: usize,
    top_provider_share_pct: f64,
    weekend_share_pct: f64,
    longest_streak_days: u32,
    active_days: u32,
) -> WrappedArchetype {
    let tier = compute_usage_tier(totals, active_days);

    if totals.total_tokens >= 50_000_000_000 || totals.cost_usd >= 25_000.0 {
        return WrappedArchetype {
            id: "tony-stark-jarvis".to_string(),
            tier,
            title: "Tony Stark & J.A.R.V.I.S.".to_string(),
            tagline: "Full lab HUD online — Iron Legion scale autonomous workflows.".to_string(),
            reason: format!(
                "{} tokens across {} active days ({} API value; {:.1}% cache hit).",
                format_compact_tokens(totals.total_tokens),
                active_days,
                format_compact_usd(totals.cost_usd),
                cache.hit_pct
            ),
        };
    }
    if totals.total_tokens >= 10_000_000_000 || longest_streak_days >= 100 {
        return WrappedArchetype {
            id: "neo-in-the-matrix".to_string(),
            tier,
            title: "Neo in the Matrix".to_string(),
            tagline: "Stopped seeing syntax trees — sees pure token streams in green rain."
                .to_string(),
            reason: format!(
                "{} tokens processed with a {}-day streak ({} saved vs uncached).",
                format_compact_tokens(totals.total_tokens),
                longest_streak_days,
                format_compact_usd(cache.savings_usd)
            ),
        };
    }
    // Coding agents routinely achieve ~91-96.6% aggregate hit rates; reserve cache persona for top-percentile 98%+ / 40x reuse.
    if cache.hit_pct >= 98.0 && cache.reuse_ratio >= 40.0 && cache.cache_read_tokens >= 500_000_000
    {
        return WrappedArchetype {
            id: "paul-atreides-prescience".to_string(),
            tier,
            title: "Paul Atreides (Cache Prescience)".to_string(),
            tagline: "98%+ KV-cache prescience — cold prompt misses basically extinct.".to_string(),
            reason: format!(
                "{:.1}% hit rate with {:.1}× reuse ratio outperformed typical coding-agent benchmarks.",
                cache.hit_pct, cache.reuse_ratio
            ),
        };
    }
    if provider_count >= 3 && top_provider_share_pct < 80.0 {
        return WrappedArchetype {
            id: "rick-sanchez-multiverse".to_string(),
            tier,
            title: "Rick Sanchez (Multiverse)".to_string(),
            tagline: "Portal-gunning workflows fluidly across multiple AI harnesses.".to_string(),
            reason: format!(
                "Split across {provider_count} providers (top share {:.0}%).",
                top_provider_share_pct
            ),
        };
    }
    if chronotype.night_pct >= 28.0 {
        return WrappedArchetype {
            id: "batman-gotham-shift".to_string(),
            tier,
            title: "Batman · Gotham Night Shift".to_string(),
            tagline: "Merges cleanly at 3 AM while the rest of the timezone sleeps.".to_string(),
            reason: format!(
                "{:.0}% of total tokens flowed between 22:00 and 05:00.",
                chronotype.night_pct
            ),
        };
    }
    if weekend_share_pct >= 30.0 {
        return WrappedArchetype {
            id: "frieren-weekend-quest".to_string(),
            tier,
            title: "Frieren · Weekend Quest".to_string(),
            tagline: "Unbothered Saturday–Sunday deep quests while everyone else logged off."
                .to_string(),
            reason: format!("{weekend_share_pct:.1}% of volume arrived on weekends."),
        };
    }
    if longest_streak_days >= 30 {
        return WrappedArchetype {
            id: "carmy-yes-chef".to_string(),
            tier,
            title: "Carmy · \"Yes, Chef!\"".to_string(),
            tagline: "Every ticket fires on cadence — relentless daily shipping service."
                .to_string(),
            reason: format!("Logged {longest_streak_days} consecutive days of AI engineering."),
        };
    }
    WrappedArchetype {
        id: "john-wick-surgical".to_string(),
        tier,
        title: "John Wick · Surgical Prompt".to_string(),
        tagline: "Low-waste, crisp instructions with zero extraneous tokens.".to_string(),
        reason: format!(
            "Generated {} tokens across tight, focused engineering runs.",
            format_compact_tokens(totals.output_tokens + totals.reasoning_output_tokens)
        ),
    }
}

fn compute_badges(
    totals: &TokenCounts,
    cache: &WrappedCacheStats,
    chronotype: &WrappedChronotype,
    scale: &InformationScale,
    providers: &[WrappedProviderStat],
    longest_streak_days: u32,
    weekend_share_pct: f64,
) -> Vec<WrappedBadge> {
    let mut badges = Vec::new();
    // Since typical coding agents hit ~91-96.6%, only award the cache badge for top-tier 97%+ hit AND 30x+ reuse.
    if cache.hit_pct >= 97.0 && cache.reuse_ratio >= 30.0 {
        badges.push(WrappedBadge {
            id: "lisan-al-gaib-cache".to_string(),
            glyph: "[🏜]".to_string(),
            title: "Lisan al-Gaib (Cache)".to_string(),
            detail: format!(
                "{:.1}% hit · {:.1}× reuse (above ~91–96.6% norm)",
                cache.hit_pct, cache.reuse_ratio
            ),
        });
    }
    if cache.savings_usd >= 10_000.0 {
        badges.push(WrappedBadge {
            id: "big-short-arbitrage".to_string(),
            glyph: "[📉]".to_string(),
            title: "Big Short Arbitrage".to_string(),
            detail: format!(
                "~{} counterfactual spend avoided via cache reads",
                format_compact_usd(cache.savings_usd)
            ),
        });
    }
    if totals.total_tokens >= 1_000_000_000 {
        badges.push(WrappedBadge {
            id: "tres-commas".to_string(),
            glyph: "[💸]".to_string(),
            title: "Tres Commas Club".to_string(),
            detail: format!(
                "{} tokens · entered Silicon Valley billionaire club",
                format_compact_tokens(totals.total_tokens)
            ),
        });
    }
    if longest_streak_days >= 30 {
        badges.push(WrappedBadge {
            id: "streak-dojo".to_string(),
            glyph: "[🔥]".to_string(),
            title: if longest_streak_days >= 100 {
                "Hyperbolic Time Chamber".to_string()
            } else {
                "Yes, Chef! Streak".to_string()
            },
            detail: format!("{longest_streak_days} consecutive days shipped"),
        });
    }
    if scale.digestion_public_libraries >= 1.0 {
        badges.push(WrappedBadge {
            id: "library-of-alexandria".to_string(),
            glyph: "[📚]".to_string(),
            title: "Library of Alexandria".to_string(),
            detail: format!(
                "{:.1}× 40k-book public libraries digested",
                scale.digestion_public_libraries
            ),
        });
    }
    if providers.len() >= 3 {
        badges.push(WrappedBadge {
            id: "spider-verse".to_string(),
            glyph: "[🕸]".to_string(),
            title: "Into the Spider-Verse".to_string(),
            detail: format!("Active across {} distinct AI harnesses", providers.len()),
        });
    }
    if chronotype.night_pct >= 20.0 {
        badges.push(WrappedBadge {
            id: "night-city-netrunner".to_string(),
            glyph: "[🌃]".to_string(),
            title: "Night City Netrunner".to_string(),
            detail: format!(
                "{:.0}% late-night flow (peak {:02}:00)",
                chronotype.night_pct, chronotype.peak_hour
            ),
        });
    } else if weekend_share_pct >= 25.0 {
        badges.push(WrappedBadge {
            id: "zero-grass-touched".to_string(),
            glyph: "[🕹]".to_string(),
            title: "Zero Grass Touched".to_string(),
            detail: format!("{weekend_share_pct:.0}% of tokens burned on Sat/Sun"),
        });
    }
    badges.truncate(6);
    badges
}

fn build_share_blurb(report: &WrappedReport) -> String {
    let top_model = report
        .top_models
        .first()
        .map_or("Claude/Codex", |m| m.name.as_str());
    format!(
        "My {} AI Year in Review ({}):\n• Persona: {} [{}]\n• {} tokens · {} ({:.1}× reuse / ~{} saved)\n• {}-day streak · Peak day: {}\n• Main squeeze: {}\n\nbrew install FelixIsaac/tap/tokenusage && tu wrapped",
        report.period_label,
        report.chronotype.label,
        report.archetype.title,
        report.archetype.tier,
        format_compact_tokens(report.totals.total_tokens),
        report.pop_culture_scale,
        report.cache.reuse_ratio,
        format_compact_usd(report.cache.savings_usd),
        report.longest_streak_days,
        report
            .peak_day
            .as_ref()
            .map_or("-".to_string(), |d| format!(
                "{} ({})",
                d.date,
                format_compact_tokens(d.tokens)
            )),
        top_model,
    )
}

fn build_agent_prompt(report: &WrappedReport) -> String {
    let models = report
        .top_models
        .iter()
        .take(4)
        .map(|m| {
            format!(
                "{} ({:.1}%, {})",
                m.name,
                m.share_pct,
                format_compact_tokens(m.tokens)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let providers = report
        .top_providers
        .iter()
        .take(4)
        .map(|p| format!("{} ({:.1}%)", p.name, p.share_pct))
        .collect::<Vec<_>>()
        .join(", ");
    let peak = report.peak_day.as_ref().map_or("-".to_string(), |d| {
        format!(
            "{} ({} tokens, {})",
            d.date,
            format_compact_tokens(d.tokens),
            format_compact_usd(d.cost_usd)
        )
    });

    format!(
        "You are my witty creative director making a viral, sharp-edged {} AI Year in Review roast & hype thread from my anonymized local usage metrics (zero repo names or private paths included):\n\
- Archetype: {} [{}] ({})\n\
- Chronotype: {} (peak hour {:02}:00, {:.0}% late-night, {:.0}% weekends)\n\
- Total Tokens: {} ({} output words · {})\n\
- API Value: {} ({} saved by {:.1}% prompt cache hit / {:.1}× reuse; typical agent norm ~91–96.6%)\n\
- Cadence: {} active days · {}-day longest streak · Peak day: {}\n\
- Top Providers: {}\n\
- Top Models: {}\n\
- Footprint: {}\n\n\
Write:\n\
1. A punchy 3-line X/Twitter flex post leaning into the pop-culture references\n\
2. A playful 5-bullet 'AI Wrapped Roast' interpreting my habits\n\
3. Three concrete 2027 workflow upgrades based on these numbers\n\
4. (Optional) A standalone interactive Three.js + GSAP cyber/carbon HTML showcase displaying these anonymized stats.",
        report.period_label,
        report.archetype.title,
        report.archetype.tier,
        report.archetype.tagline,
        report.chronotype.label,
        report.chronotype.peak_hour,
        report.chronotype.night_pct,
        report.weekend_share_pct,
        format_compact_tokens(report.totals.total_tokens),
        format_commas_u64(report.scale.output_words),
        report.pop_culture_scale,
        format_compact_usd(report.totals.cost_usd),
        format_compact_usd(report.cache.savings_usd),
        report.cache.hit_pct,
        report.cache.reuse_ratio,
        report.active_days,
        report.longest_streak_days,
        peak,
        providers,
        models,
        report.carbon.summary,
    )
}

pub(crate) fn print_wrapped_static(report: &WrappedReport) {
    println!();
    println!(
        "  TOKENUSAGE WRAPPED {}  ·  {}  [{}]",
        report.period_label, report.archetype.title, report.archetype.tier
    );
    println!("  {}", report.archetype.tagline);
    println!("  {}", "─".repeat(72));
    println!(
        "  Total Volume   : {} tokens  ({} output · {} sessions)",
        format_compact_tokens(report.totals.total_tokens),
        format_compact_tokens(report.totals.output_tokens + report.totals.reasoning_output_tokens),
        format_commas_u64(report.total_sessions)
    );
    println!(
        "  API Spend      : {}  (Cache saved ~{} @ {:.1}% hit / {:.1}× reuse)",
        format_compact_usd(report.totals.cost_usd),
        format_compact_usd(report.cache.savings_usd),
        report.cache.hit_pct,
        report.cache.reuse_ratio
    );
    println!(
        "  Cadence        : {} active days · {}d longest streak · {} ({:02}:00 peak)",
        report.active_days,
        report.longest_streak_days,
        report.chronotype.label,
        report.chronotype.peak_hour
    );
    if let Some(peak) = &report.peak_day {
        println!(
            "  Peak Day       : {} · {} tokens · {} ({})",
            peak.date,
            format_compact_tokens(peak.tokens),
            format_compact_usd(peak.cost_usd),
            peak.top_model
        );
    }
    println!("  Pop Scale      : {}", report.pop_culture_scale);
    println!("  Human Scale    : {}", report.scale_headline);
    println!("  Carbon         : {}", report.carbon.summary);

    if !report.top_models.is_empty() {
        println!();
        println!("  Top Models:");
        for (idx, m) in report.top_models.iter().take(5).enumerate() {
            println!(
                "    #{:<2} {:<30} {:>8} ({:>5.1}%)  {}",
                idx + 1,
                m.name,
                format_compact_tokens(m.tokens),
                m.share_pct,
                format_compact_usd(m.cost_usd)
            );
        }
    }
    if report.include_projects && !report.top_projects.is_empty() {
        println!();
        println!("  Top Projects (--include-projects):");
        for (idx, p) in report.top_projects.iter().take(5).enumerate() {
            println!(
                "    #{:<2} {:<30} {:>8} ({:>5.1}%)",
                idx + 1,
                p.name,
                format_compact_tokens(p.tokens),
                p.share_pct
            );
        }
    }
    println!();
    println!("  Tip: export --html wrapped.html, --png wrapped.png, or --prompt for an AI roast.");
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::UsageAccumulator;

    #[allow(clippy::too_many_arguments)]
    fn sample_event(
        ts_str: &str,
        model: &str,
        source: SourceKind,
        project: &str,
        session: &str,
        input: u64,
        output: u64,
        cache_read: u64,
        cost: f64,
    ) -> UsageEvent {
        let ts = chrono::DateTime::parse_from_rfc3339(ts_str)
            .unwrap()
            .with_timezone(&chrono::Utc);
        UsageEvent {
            timestamp: ts,
            model: model.to_string(),
            source,
            project: Some(project.to_string()),
            session: session.to_string(),
            file_path: String::new(),
            usage: UsageAccumulator {
                input_tokens: input,
                output_tokens: output,
                cache_read_input_tokens: cache_read,
                cache_creation_input_tokens: 1_000,
                reasoning_output_tokens: 0,
                cost_usd: cost,
            },
        }
    }

    #[test]
    fn chronotype_identifies_gotham_night_shift_and_lumon_refiner() {
        let mut night = [0u64; 24];
        night[23] = 500_000;
        night[1] = 400_000;
        night[14] = 100_000;
        assert_eq!(classify_chronotype(&night).label, "Gotham Night Shift");

        let mut day = [0u64; 24];
        day[11] = 300_000;
        day[14] = 600_000;
        day[16] = 200_000;
        assert_eq!(classify_chronotype(&day).label, "Lumon Macrodata Refiner");
    }

    #[test]
    fn typical_ninety_percent_cache_hit_does_not_grant_cache_persona_or_badge() {
        let events = vec![sample_event(
            "2026-03-10T14:00:00Z",
            "claude-opus-4-6-20260101",
            SourceKind::Claude,
            "/Users/secret/Projects/repo",
            "sess-1",
            100_000_000,
            20_000_000,
            900_000_000,
            250.0,
        )];
        let tz = TimeZoneMode::Utc;
        let pricing = PricingTable::default_table();
        let bundle = build_wrapped_bundle(WrappedBuildOptions {
            events: &events,
            tz: &tz,
            timezone_label: "UTC".to_string(),
            pricing: &pricing,
            target_year: Some(2026),
            all_time: false,
            include_projects: false,
            common: None,
        });
        assert_ne!(
            bundle.report.archetype.title,
            "Paul Atreides (Cache Prescience)"
        );
        assert!(
            !bundle
                .report
                .badges
                .iter()
                .any(|b| b.title == "Lisan al-Gaib (Cache)")
        );
    }

    #[test]
    fn pop_culture_scale_handles_small_medium_and_massive_outputs() {
        let small = InformationScale::calculate(20_000, 100_000);
        assert!(compute_pop_culture_scale(&small).contains("Bee Movie"));

        let medium = InformationScale::calculate(600_000, 3_000_000);
        assert!(compute_pop_culture_scale(&medium).contains("Dune"));

        let massive = InformationScale::calculate(600_000_000, 100_000_000_000);
        let label = compute_pop_culture_scale(&massive);
        assert!(label.contains("Harry Potter") && label.contains("LOTR"));
    }

    #[test]
    fn wrapped_report_redacts_projects_and_sessions_by_default() {
        let events = vec![
            sample_event(
                "2026-03-10T14:00:00Z",
                "claude-opus-4-6-20260101",
                SourceKind::Claude,
                "/Users/secret/Projects/super-secret-repo",
                "secret-session-uuid-123456",
                100_000,
                20_000,
                900_000,
                2.50,
            ),
            sample_event(
                "2026-03-11T15:00:00Z",
                "gpt-5.3-codex",
                SourceKind::Codex,
                "/Users/secret/Projects/other-secret",
                "secret-session-uuid-999999",
                50_000,
                10_000,
                400_000,
                1.25,
            ),
        ];

        let tz = TimeZoneMode::Utc;
        let pricing = PricingTable::default_table();
        let mut bundle = build_wrapped_bundle(WrappedBuildOptions {
            events: &events,
            tz: &tz,
            timezone_label: "UTC".to_string(),
            pricing: &pricing,
            target_year: Some(2026),
            all_time: false,
            include_projects: false,
            common: None,
        });

        let json = serde_json::to_string(&bundle.report).unwrap();
        assert!(!json.contains("super-secret-repo"));
        assert!(!json.contains("secret-session-uuid"));
        assert!(bundle.report.top_projects.is_empty());
        assert!(bundle.report.peak_session.as_ref().unwrap().label.is_none());
        assert_eq!(bundle.report.longest_streak_days, 2);

        bundle.set_include_projects(true);
        assert!(!bundle.report.top_projects.is_empty());
        assert!(bundle.report.peak_session.as_ref().unwrap().label.is_some());
    }
}
