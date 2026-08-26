use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::Serialize;

use crate::activity::{ActivityDataset, activity_enabled, fetch_activity_dataset};
use crate::carbon::{
    EnvironmentalEquivalences, EnvironmentalMetrics, GridRegion, eco_rating, format_carbon_human,
    format_commas_f64, format_commas_u64, format_currency, format_water_human,
};
#[cfg(feature = "cli")]
use crate::cli::{
    ActivityArgs, AnthropicApiArgs, AntigravityArgs, CarbonArgs, CarbonPeriodArg, DailyArgs,
    DeepseekArgs, GrokArgs, KimiArgs, MonthlyArgs, OpenrouterArgs, SessionArgs, TodayArgs,
    WeeklyArgs,
};
use crate::cli::{CommonArgs, SortOrder};
#[cfg(feature = "cli")]
use crate::output::{print_report_table_with_options, run_report_tui};
use crate::types::{ActivitySummary, DailyReport, DailyRow, ParseStats, TokenCounts, UsageEvent};

#[cfg(feature = "cli")]
use super::activity_report::*;
#[cfg(feature = "cli")]
use super::official::{
    fetch_anthropic_api_limits, fetch_antigravity_official_limits, fetch_deepseek_official_limits,
    fetch_grok_official_limits, fetch_kimi_official_limits, fetch_openrouter_account_limits,
    select_antigravity_models,
};
use super::parsing::{
    build_sources, discover_files, incremental_cache_stats, load_pricing, load_usage,
};
#[cfg(feature = "cli")]
use super::statusline::{format_reset_timestamp, format_time_until_reset_short};
use super::*;

#[derive(Debug, Serialize)]
struct DoctorSourceReport {
    source: String,
    roots: Vec<String>,
    discovered_files: usize,
    retained_events: usize,
    mode: String,
    opencode_db_files: Option<usize>,
    opencode_legacy_files: Option<usize>,
    opencode_db_events: Option<usize>,
    opencode_legacy_events: Option<usize>,
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    timezone: String,
    selected_sources: Vec<String>,
    pricing: DoctorPricingReport,
    cache: DoctorCacheReport,
    sources: Vec<DoctorSourceReport>,
    opencode_debug: Option<DoctorOpencodeDebugReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DoctorCacheReport {
    path: Option<String>,
    exists: bool,
    size_bytes: u64,
    size_human: String,
    entries: Option<usize>,
}

#[derive(Debug, Serialize)]
struct DoctorPricingReport {
    offline: bool,
    openrouter_cache_path: Option<String>,
    openrouter_cache_fetched_unix: Option<u64>,
    openrouter_cache_age_secs: Option<u64>,
    openrouter_cache_ttl_secs: u64,
    openrouter_cached_models: Option<usize>,
}

#[derive(Debug, Serialize)]
struct DoctorOpencodeDebugReport {
    merged: TokenCounts,
    db_only: TokenCounts,
    legacy_only: TokenCounts,
    overlap_estimate: TokenCounts,
}

#[cfg(feature = "cli")]
pub(crate) async fn run_doctor(args: DailyArgs) -> Result<()> {
    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let filter = DateFilter {
        since: None,
        until: None,
    };
    let ignore_rules = PathIgnoreRules::from_common(&args.common);
    let source_configs = build_sources(&args.common).await?;
    let discovered = discover_files(&source_configs, &ignore_rules, filter);
    let loaded = load_usage(&args.common, &tz).await?;
    let opencode_debug = build_opencode_debug_report(&args.common, &tz, &loaded).await?;

    let pricing = load_pricing(args.common.pricing_file.as_deref(), args.common.offline).await?;
    let mut missing_pricing_models: BTreeMap<String, usize> = BTreeMap::new();
    if args.common.pricing_debug {
        for event in &loaded.events {
            if event.usage.total_tokens() == 0 {
                continue;
            }
            if pricing.find_rate(&event.model).is_none() {
                *missing_pricing_models
                    .entry(event.model.clone())
                    .or_insert(0) += 1;
            }
        }
    }

    let now_unix = u64::try_from(Utc::now().timestamp()).unwrap_or(0);
    let (openrouter_cache_path, openrouter_cache_fetched_unix, openrouter_cached_models) =
        super::pricing::openrouter_pricing_cache_path()
            .and_then(|path| {
                let store = super::pricing::load_openrouter_pricing_cache(&path)?;
                Some((
                    Some(path.to_string_lossy().to_string()),
                    Some(store.fetched_unix),
                    Some(store.exact.len()),
                ))
            })
            .unwrap_or((None, None, None));
    let openrouter_cache_age_secs =
        openrouter_cache_fetched_unix.map(|fetched| now_unix.saturating_sub(fetched));

    let mut by_source: HashMap<SourceKind, usize> = HashMap::new();
    for file in &discovered {
        *by_source.entry(file.source).or_insert(0) += 1;
    }
    let mut retained_by_source: HashMap<SourceKind, usize> = HashMap::new();
    let mut opencode_db_events = 0usize;
    let mut opencode_legacy_events = 0usize;
    for event in &loaded.events {
        *retained_by_source.entry(event.source).or_insert(0) += 1;
        if event.source == SourceKind::OpenCode {
            let path = event.file_path.replace('\\', "/");
            if path.ends_with("/opencode.db") {
                opencode_db_events += 1;
            } else if path.contains("/storage/message/") {
                opencode_legacy_events += 1;
            }
        }
    }

    let mut sources = Vec::new();
    for source in &source_configs {
        let (opencode_db_files, opencode_legacy_files) = if source.kind == SourceKind::OpenCode {
            let db = discovered
                .iter()
                .filter(|file| {
                    file.source == SourceKind::OpenCode
                        && file.path.file_name().and_then(|name| name.to_str())
                            == Some("opencode.db")
                })
                .count();
            let legacy = discovered
                .iter()
                .filter(|file| {
                    file.source == SourceKind::OpenCode
                        && file
                            .path
                            .to_string_lossy()
                            .replace('\\', "/")
                            .contains("/storage/message/")
                })
                .count();
            (Some(db), Some(legacy))
        } else {
            (None, None)
        };
        let (opencode_db_events_field, opencode_legacy_events_field) =
            if source.kind == SourceKind::OpenCode {
                (Some(opencode_db_events), Some(opencode_legacy_events))
            } else {
                (None, None)
            };
        let mode = if source.kind == SourceKind::OpenCode {
            let has_db = source
                .roots
                .iter()
                .any(|root| root.join("opencode.db").is_file());
            let has_legacy = source
                .roots
                .iter()
                .any(|root| root.join("storage").join("message").is_dir());
            match (has_db, has_legacy) {
                (true, true) => "db+legacy".to_string(),
                (true, false) => "db".to_string(),
                (false, true) => "legacy".to_string(),
                (false, false) => "none".to_string(),
            }
        } else {
            "jsonl".to_string()
        };

        sources.push(DoctorSourceReport {
            source: source.kind.as_str().to_string(),
            roots: source
                .roots
                .iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect(),
            discovered_files: *by_source.get(&source.kind).unwrap_or(&0),
            retained_events: *retained_by_source.get(&source.kind).unwrap_or(&0),
            mode,
            opencode_db_files,
            opencode_legacy_files,
            opencode_db_events: opencode_db_events_field,
            opencode_legacy_events: opencode_legacy_events_field,
        });
    }

    let cache_stats = incremental_cache_stats();
    let cache = DoctorCacheReport {
        path: cache_stats
            .as_ref()
            .map(|c| c.path.to_string_lossy().to_string()),
        exists: cache_stats.as_ref().is_some_and(|c| c.exists),
        size_bytes: cache_stats.as_ref().map(|c| c.size_bytes).unwrap_or(0),
        size_human: human_bytes(cache_stats.as_ref().map(|c| c.size_bytes).unwrap_or(0)),
        entries: cache_stats.as_ref().and_then(|c| c.entries),
    };

    // Surface actionable health problems, not just raw numbers.
    let mut warnings = Vec::new();
    if cache.size_bytes > 100 * 1024 * 1024 {
        warnings.push(format!(
            "parse cache is {} ({} entries) — large; run `tu --rebuild-cache` to compact",
            cache.size_human,
            cache.entries.unwrap_or(0)
        ));
    }
    if let Some(age) = openrouter_cache_age_secs
        && age > OPENROUTER_PRICING_CACHE_TTL_SECS
    {
        warnings.push(format!(
            "openrouter pricing cache is stale (age {age}s > ttl {OPENROUTER_PRICING_CACHE_TTL_SECS}s); refreshes on next online run"
        ));
    }
    for source in &sources {
        if source.discovered_files > 0 && source.retained_events == 0 {
            warnings.push(format!(
                "source {} discovered {} files but retained 0 events (parse/format issue?)",
                source.source, source.discovered_files
            ));
        }
    }

    let report = DoctorReport {
        timezone: format!("{tz:?}"),
        selected_sources: args
            .common
            .selected_sources()
            .into_iter()
            .map(|source| source.as_str().to_string())
            .collect(),
        pricing: DoctorPricingReport {
            offline: args.common.offline,
            openrouter_cache_path,
            openrouter_cache_fetched_unix,
            openrouter_cache_age_secs,
            openrouter_cache_ttl_secs: OPENROUTER_PRICING_CACHE_TTL_SECS,
            openrouter_cached_models,
        },
        cache,
        sources,
        opencode_debug,
        warnings,
    };

    if should_emit_json(&args.common) {
        emit_json(&report, args.common.jq.as_deref())
    } else {
        println!("timezone: {}", report.timezone);
        if report.selected_sources.is_empty() {
            println!("selected-sources: all");
        } else {
            println!("selected-sources: {}", report.selected_sources.join(", "));
        }
        println!(
            "pricing: openrouter cache ttl={}s offline={}",
            report.pricing.openrouter_cache_ttl_secs, report.pricing.offline
        );
        if let Some(path) = &report.pricing.openrouter_cache_path {
            println!("  openrouter-cache: {path}");
        }
        if let Some(fetched) = report.pricing.openrouter_cache_fetched_unix {
            println!("  openrouter-cache-fetched-unix: {fetched}");
        }
        if let Some(age) = report.pricing.openrouter_cache_age_secs {
            println!("  openrouter-cache-age-secs: {age}");
        }
        if let Some(models) = report.pricing.openrouter_cached_models {
            println!("  openrouter-cache-models: {models}");
        }
        println!();
        println!("parse-cache:");
        if let Some(path) = &report.cache.path {
            println!("  path: {path}");
        }
        println!("  exists: {}", report.cache.exists);
        println!(
            "  size: {} ({} bytes)",
            report.cache.size_human, report.cache.size_bytes
        );
        if let Some(entries) = report.cache.entries {
            println!("  entries: {entries}");
        }
        if args.common.pricing_debug {
            println!(
                "pricing-debug: unknown-models={}",
                missing_pricing_models.len()
            );
            for (model, count) in missing_pricing_models.iter().take(20) {
                println!("  missing: {model} (events={count})");
            }
            if missing_pricing_models.len() > 20 {
                println!(
                    "  ... +{} more (rerun with --json for full list)",
                    missing_pricing_models.len() - 20
                );
            }
        }
        println!();
        for source in &report.sources {
            println!("source: {}", source.source);
            println!("  mode: {}", source.mode);
            println!("  discovered-files: {}", source.discovered_files);
            println!("  retained-events: {}", source.retained_events);
            if let Some(db_files) = source.opencode_db_files {
                println!("  opencode-db-files: {db_files}");
            }
            if let Some(legacy_files) = source.opencode_legacy_files {
                println!("  opencode-legacy-files: {legacy_files}");
            }
            if let Some(db_events) = source.opencode_db_events {
                println!("  opencode-db-events: {db_events}");
            }
            if let Some(legacy_events) = source.opencode_legacy_events {
                println!("  opencode-legacy-events: {legacy_events}");
            }
            for root in &source.roots {
                println!("  root: {}", root);
            }
            println!();
        }
        if let Some(debug) = &report.opencode_debug {
            println!("opencode-debug:");
            println!(
                "  merged: input={} output={} cache-create={} cache-read={} total={} cost=${:.2}",
                debug.merged.input_tokens,
                debug.merged.output_tokens,
                debug.merged.cache_creation_input_tokens,
                debug.merged.cache_read_input_tokens,
                debug.merged.total_tokens,
                debug.merged.cost_usd
            );
            println!(
                "  db-only: input={} output={} cache-create={} cache-read={} total={} cost=${:.2}",
                debug.db_only.input_tokens,
                debug.db_only.output_tokens,
                debug.db_only.cache_creation_input_tokens,
                debug.db_only.cache_read_input_tokens,
                debug.db_only.total_tokens,
                debug.db_only.cost_usd
            );
            println!(
                "  legacy-only: input={} output={} cache-create={} cache-read={} total={} cost=${:.2}",
                debug.legacy_only.input_tokens,
                debug.legacy_only.output_tokens,
                debug.legacy_only.cache_creation_input_tokens,
                debug.legacy_only.cache_read_input_tokens,
                debug.legacy_only.total_tokens,
                debug.legacy_only.cost_usd
            );
            println!(
                "  overlap-estimate: input={} output={} cache-create={} cache-read={} total={} cost=${:.2}",
                debug.overlap_estimate.input_tokens,
                debug.overlap_estimate.output_tokens,
                debug.overlap_estimate.cache_creation_input_tokens,
                debug.overlap_estimate.cache_read_input_tokens,
                debug.overlap_estimate.total_tokens,
                debug.overlap_estimate.cost_usd
            );
            println!();
        }
        if !report.warnings.is_empty() {
            println!("warnings:");
            for warning in &report.warnings {
                println!("  ! {warning}");
            }
        }
        Ok(())
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn collect_source_totals(events: &[UsageEvent], source: SourceKind) -> TokenCounts {
    events.iter().filter(|event| event.source == source).fold(
        TokenCounts::default(),
        |mut acc, event| {
            acc.add_assign(event.usage.to_counts());
            acc
        },
    )
}

fn overlap_estimate_counts(
    merged: &TokenCounts,
    db_only: &TokenCounts,
    legacy_only: &TokenCounts,
) -> TokenCounts {
    TokenCounts {
        input_tokens: db_only
            .input_tokens
            .saturating_add(legacy_only.input_tokens)
            .saturating_sub(merged.input_tokens),
        cache_creation_input_tokens: db_only
            .cache_creation_input_tokens
            .saturating_add(legacy_only.cache_creation_input_tokens)
            .saturating_sub(merged.cache_creation_input_tokens),
        cache_read_input_tokens: db_only
            .cache_read_input_tokens
            .saturating_add(legacy_only.cache_read_input_tokens)
            .saturating_sub(merged.cache_read_input_tokens),
        output_tokens: db_only
            .output_tokens
            .saturating_add(legacy_only.output_tokens)
            .saturating_sub(merged.output_tokens),
        reasoning_output_tokens: db_only
            .reasoning_output_tokens
            .saturating_add(legacy_only.reasoning_output_tokens)
            .saturating_sub(merged.reasoning_output_tokens),
        total_tokens: db_only
            .total_tokens
            .saturating_add(legacy_only.total_tokens)
            .saturating_sub(merged.total_tokens),
        cost_usd: (db_only.cost_usd + legacy_only.cost_usd - merged.cost_usd).max(0.0),
    }
}

async fn build_opencode_debug_report(
    common: &CommonArgs,
    tz: &TimeZoneMode,
    merged_loaded: &LoadedUsage,
) -> Result<Option<DoctorOpencodeDebugReport>> {
    let merged = collect_source_totals(&merged_loaded.events, SourceKind::OpenCode);
    if merged.total_tokens == 0 {
        return Ok(None);
    }

    let mut db_only_args = common.clone();
    db_only_args.no_claude = true;
    db_only_args.no_codex = true;
    db_only_args.no_gemini = true;
    db_only_args.no_opencode = false;
    db_only_args.ignore_path.push("storage/message".to_string());
    db_only_args
        .ignore_path
        .push("storage\\message".to_string());

    let mut legacy_only_args = common.clone();
    legacy_only_args.no_claude = true;
    legacy_only_args.no_codex = true;
    legacy_only_args.no_gemini = true;
    legacy_only_args.no_opencode = false;
    legacy_only_args.ignore_path.push("opencode.db".to_string());

    let db_only_loaded = load_usage(&db_only_args, tz).await?;
    let legacy_only_loaded = load_usage(&legacy_only_args, tz).await?;

    let db_only = collect_source_totals(&db_only_loaded.events, SourceKind::OpenCode);
    let legacy_only = collect_source_totals(&legacy_only_loaded.events, SourceKind::OpenCode);
    let overlap_estimate = overlap_estimate_counts(&merged, &db_only, &legacy_only);

    Ok(Some(DoctorOpencodeDebugReport {
        merged,
        db_only,
        legacy_only,
        overlap_estimate,
    }))
}

pub(super) async fn enrich_rows_with_activity(
    common: &CommonArgs,
    period: ReportPeriod,
    instances: bool,
    tz: &TimeZoneMode,
    project_filter: Option<&str>,
    events: &[UsageEvent],
    mut rows: Vec<DailyRow>,
) -> Result<(Vec<DailyRow>, Option<ActivitySummary>)> {
    if !activity_enabled(common) || rows.is_empty() {
        return Ok((rows, None));
    }

    let Some(dataset) = fetch_activity_dataset(common, tz, events, project_filter).await? else {
        return Ok((rows, None));
    };

    for row in &mut rows {
        row.activity = activity_summary_for_row(&dataset, period, instances, &row.date);
    }

    let totals = report_date_bounds(events, tz)
        .and_then(|(range_start, range_end)| dataset.summary_for_range(range_start, range_end));

    Ok((rows, totals))
}

pub(super) fn report_date_bounds(
    events: &[UsageEvent],
    tz: &TimeZoneMode,
) -> Option<(NaiveDate, NaiveDate)> {
    let mut min_day: Option<NaiveDate> = None;
    let mut max_day: Option<NaiveDate> = None;

    for event in events {
        let day = local_date(event.timestamp, tz);
        min_day = Some(match min_day {
            Some(current) => current.min(day),
            None => day,
        });
        max_day = Some(match max_day {
            Some(current) => current.max(day),
            None => day,
        });
    }

    Some((min_day?, max_day?))
}

pub(super) fn activity_summary_for_row(
    dataset: &ActivityDataset,
    period: ReportPeriod,
    instances: bool,
    key: &str,
) -> Option<ActivitySummary> {
    if instances && period == ReportPeriod::Daily {
        let (project_name, day) = parse_instance_day_key(key)?;
        return dataset.project_summary_for_day(day, project_name);
    }

    match period {
        ReportPeriod::Daily => {
            let day = NaiveDate::parse_from_str(key, "%Y-%m-%d").ok()?;
            dataset.summary_for_day(day)
        }
        ReportPeriod::Weekly => {
            let start = NaiveDate::parse_from_str(key, "%Y-%m-%d").ok()?;
            let end = start
                .checked_add_signed(chrono::TimeDelta::days(6))
                .unwrap_or(start);
            dataset.summary_for_range(start, end)
        }
        ReportPeriod::Monthly => {
            let (start, end) = parse_month_bounds(key)?;
            dataset.summary_for_range(start, end)
        }
    }
}

pub(super) fn parse_instance_day_key(key: &str) -> Option<(&str, NaiveDate)> {
    let (project_name, day_text) = key.rsplit_once(" | ")?;
    let day = NaiveDate::parse_from_str(day_text, "%Y-%m-%d").ok()?;
    Some((project_name, day))
}

pub(super) fn parse_month_bounds(key: &str) -> Option<(NaiveDate, NaiveDate)> {
    let (year_text, month_text) = key.split_once('-')?;
    let year = year_text.parse::<i32>().ok()?;
    let month = month_text.parse::<u32>().ok()?;
    let start = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };
    let end = next
        .checked_sub_signed(chrono::TimeDelta::days(1))
        .unwrap_or(start);
    Some((start, end))
}

#[cfg(feature = "cli")]
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

    let rows = build_group_rows(&events, ReportPeriod::Daily, &args.common, |event| {
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
    let period_attribution = build_period_attribution(&events, |event| {
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

    let (rows, activity_totals) = enrich_rows_with_activity(
        &args.common,
        ReportPeriod::Daily,
        args.instances,
        &tz,
        args.project.as_deref(),
        &events,
        rows,
    )
    .await?;

    let report = build_report_from_rows(
        rows,
        activity_totals,
        loaded.stats,
        Some(period_attribution),
    );

    if use_json {
        emit_json(&report, args.common.jq.as_deref())
    } else if args.tui {
        run_report_tui(&report)
    } else {
        print_report_table_with_options(
            &report,
            args.common.compact,
            args.common.breakdown,
            args.common.brief,
        );
        print_debug(&report.stats, &args.common);
        Ok(())
    }
}

#[cfg(feature = "cli")]
pub(crate) async fn run_monthly(args: MonthlyArgs) -> Result<()> {
    let use_json = should_emit_json(&args.common);
    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let loaded = load_usage(&args.common, &tz).await?;

    let rows = build_group_rows(
        &loaded.events,
        ReportPeriod::Monthly,
        &args.common,
        |event| {
            let day = local_date(event.timestamp, &tz);
            format!("{:04}-{:02}", day.year(), day.month())
        },
    );
    let period_attribution = build_period_attribution(&loaded.events, |event| {
        let day = local_date(event.timestamp, &tz);
        format!("{:04}-{:02}", day.year(), day.month())
    });

    let (rows, activity_totals) = enrich_rows_with_activity(
        &args.common,
        ReportPeriod::Monthly,
        false,
        &tz,
        None,
        &loaded.events,
        rows,
    )
    .await?;

    let report = build_report_from_rows(
        rows,
        activity_totals,
        loaded.stats,
        Some(period_attribution),
    );

    if use_json {
        #[derive(Serialize)]
        struct MonthlyOut {
            monthly: Vec<DailyRow>,
            totals: TokenCounts,
            activity_totals: Option<ActivitySummary>,
            stats: ParseStats,
            insights: Option<crate::ReportInsights>,
        }
        let out = MonthlyOut {
            monthly: report.daily,
            totals: report.totals,
            activity_totals: report.activity_totals,
            stats: report.stats,
            insights: report.insights,
        };
        emit_json(&out, args.common.jq.as_deref())
    } else {
        let show = DailyReport {
            daily: report.daily,
            totals: report.totals,
            activity_totals: report.activity_totals,
            stats: report.stats,
            insights: report.insights,
        };
        print_report_table_with_options(
            &show,
            args.common.compact,
            args.common.breakdown,
            args.common.brief,
        );
        print_debug(&show.stats, &args.common);
        Ok(())
    }
}

#[cfg(feature = "cli")]
pub(crate) async fn run_weekly(args: WeeklyArgs) -> Result<()> {
    let use_json = should_emit_json(&args.common);
    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let loaded = load_usage(&args.common, &tz).await?;

    let rows = build_group_rows(
        &loaded.events,
        ReportPeriod::Weekly,
        &args.common,
        |event| {
            let day = local_date(event.timestamp, &tz);
            let start = week_start(day, args.start_of_week);
            format!("{}", start.format("%Y-%m-%d"))
        },
    );
    let period_attribution = build_period_attribution(&loaded.events, |event| {
        let day = local_date(event.timestamp, &tz);
        let start = week_start(day, args.start_of_week);
        format!("{}", start.format("%Y-%m-%d"))
    });

    let (rows, activity_totals) = enrich_rows_with_activity(
        &args.common,
        ReportPeriod::Weekly,
        false,
        &tz,
        None,
        &loaded.events,
        rows,
    )
    .await?;

    let report = build_report_from_rows(
        rows,
        activity_totals,
        loaded.stats,
        Some(period_attribution),
    );

    if use_json {
        #[derive(Serialize)]
        struct WeeklyOut {
            weekly: Vec<DailyRow>,
            totals: TokenCounts,
            activity_totals: Option<ActivitySummary>,
            stats: ParseStats,
            insights: Option<crate::ReportInsights>,
        }
        let out = WeeklyOut {
            weekly: report.daily,
            totals: report.totals,
            activity_totals: report.activity_totals,
            stats: report.stats,
            insights: report.insights,
        };
        emit_json(&out, args.common.jq.as_deref())
    } else {
        let show = DailyReport {
            daily: report.daily,
            totals: report.totals,
            activity_totals: report.activity_totals,
            stats: report.stats,
            insights: report.insights,
        };
        print_report_table_with_options(
            &show,
            args.common.compact,
            args.common.breakdown,
            args.common.brief,
        );
        print_debug(&show.stats, &args.common);
        Ok(())
    }
}

#[cfg(feature = "cli")]
pub(crate) async fn run_today(mut args: TodayArgs) -> Result<()> {
    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    args.common.with_activity = true;

    let today = tz.now_date();
    let start = parse_date_filter(args.common.since.as_deref())?.unwrap_or(today);
    let end = parse_date_filter(args.common.until.as_deref())?.unwrap_or(today);
    if end < start {
        anyhow::bail!("--until must be on/after --since");
    }

    args.common.since = Some(start.to_string());
    args.common.until = Some(end.to_string());

    let use_json = should_emit_json(&args.common);
    let loaded = load_usage(&args.common, &tz).await?;
    let LoadedUsage { events, stats } = loaded;
    let filtered_events = filter_events_by_project(events, args.project.as_deref());
    let day_totals = aggregate_token_counts(&filtered_events);
    let models = token_breakdowns_by_model(&filtered_events, 5);

    let dataset =
        fetch_activity_dataset(&args.common, &tz, &filtered_events, args.project.as_deref())
            .await?
            .unwrap_or_default();
    let activity = dataset.summary_for_range(start, end);
    let active_days = dataset.active_days_in_range(start, end);
    let hourly_activity = dataset.hourly_buckets_for_day(end, None);
    let hourly_tokens = aggregate_hourly_token_counts(&filtered_events, end, &tz);
    let hourly_rows = join_hourly_rows(&hourly_activity, &hourly_tokens);
    let overview =
        build_activity_overview(start, end, active_days, &filtered_events, activity.as_ref());
    let breakdowns = TodayProjectBreakdowns {
        projects: enrich_activity_breakdowns_with_tokens(
            dataset.project_breakdowns(start, end, 5),
            &aggregate_usage_totals_by_project(&filtered_events),
        ),
        languages: enrich_activity_breakdowns_with_tokens(
            dataset.language_breakdowns(start, end, 5),
            &aggregate_usage_totals_by_language(&filtered_events),
        ),
        sources: enrich_activity_breakdowns_with_tokens(
            dataset.source_breakdowns(start, end, 5),
            &aggregate_usage_totals_by_source(&filtered_events),
        ),
        models,
    };

    if use_json {
        let out = TodayOut {
            date: end.to_string(),
            overview,
            hourly: hourly_rows,
            breakdowns,
            stats,
        };
        emit_json(&out, args.common.jq.as_deref())
    } else if args.common.brief {
        let top_model = breakdowns.models.first().map(|m| m.name()).unwrap_or("-");
        println!(
            "{}  {} tok  {}  top {top_model}",
            end,
            crate::output::format_u64(day_totals.total_tokens),
            crate::output::format_usd(day_totals.cost_usd),
        );
        Ok(())
    } else {
        print_today_view(
            &end.to_string(),
            &overview,
            &hourly_rows,
            &breakdowns,
            day_totals.total_tokens,
        );
        print_debug(&stats, &args.common);
        Ok(())
    }
}

#[cfg(feature = "cli")]
pub(crate) async fn run_activity(mut args: ActivityArgs) -> Result<()> {
    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    args.common.with_activity = true;
    apply_default_activity_range(&mut args.common, &tz, args.days)?;

    let start = parse_date_filter(args.common.since.as_deref())?.unwrap_or_else(|| {
        tz.now_date()
            .checked_sub_signed(chrono::TimeDelta::days(6))
            .unwrap_or_else(|| tz.now_date())
    });
    let end = parse_date_filter(args.common.until.as_deref())?.unwrap_or_else(|| tz.now_date());
    let use_json = should_emit_json(&args.common);

    let loaded = load_usage(&args.common, &tz).await?;
    let LoadedUsage { events, stats } = loaded;
    let filtered_events = filter_events_by_project(events, args.project.as_deref());
    let dataset =
        fetch_activity_dataset(&args.common, &tz, &filtered_events, args.project.as_deref())
            .await?
            .unwrap_or_default();
    let report = build_activity_daily_report(
        &filtered_events,
        &dataset,
        ActivityReportBuildOptions {
            tz: &tz,
            order: &args.common.order,
            start,
            end,
            stats: stats.clone(),
        },
    );
    let overview = build_activity_overview(
        start,
        end,
        report.daily.len() as u32,
        &filtered_events,
        report.activity_totals.as_ref(),
    );
    let breakdowns = ActivityRangeBreakdowns {
        projects: enrich_activity_breakdowns_with_tokens(
            dataset.project_breakdowns(start, end, args.limit.max(1)),
            &aggregate_usage_totals_by_project(&filtered_events),
        ),
        languages: enrich_activity_breakdowns_with_tokens(
            dataset.language_breakdowns(start, end, args.limit.max(1)),
            &aggregate_usage_totals_by_language(&filtered_events),
        ),
        sources: enrich_activity_breakdowns_with_tokens(
            dataset.source_breakdowns(start, end, args.limit.max(1)),
            &aggregate_usage_totals_by_source(&filtered_events),
        ),
        models: token_breakdowns_by_model(&filtered_events, args.limit.max(1)),
    };

    if use_json {
        let out = ActivityOut {
            overview,
            daily: report.daily,
            breakdowns,
            stats: report.stats,
        };
        emit_json(&out, args.common.jq.as_deref())
    } else {
        print_activity_overview("Activity", &overview);
        println!();
        print_report_table_with_options(&report, args.common.compact, false, args.common.brief);
        print_activity_breakdown_section("Projects", &breakdowns.projects);
        print_activity_breakdown_section("Languages", &breakdowns.languages);
        print_source_model_breakdown_section(&breakdowns.sources, &breakdowns.models);
        print_debug(&report.stats, &args.common);
        Ok(())
    }
}

#[cfg(feature = "cli")]
pub(crate) async fn run_antigravity(args: AntigravityArgs) -> Result<()> {
    let tz = parse_timezone_mode(args.timezone.as_deref())?;
    let snapshot = fetch_antigravity_official_limits()
        .await
        .context("Failed to probe Antigravity language server")?;

    if args.json {
        emit_json(&snapshot, None)?;
    } else {
        let plan = snapshot.plan_type.as_deref().unwrap_or("unknown");
        if let Some(email) = snapshot.account_email.as_deref() {
            println!("Antigravity  plan={plan}  email={email}");
        } else {
            println!("Antigravity  plan={plan}");
        }
        println!();

        if snapshot.models.is_empty() {
            println!("  (no model quotas available)");
        } else {
            // Show all models: those with quota data first, then unknowns
            let ordered = select_antigravity_models(&snapshot.models);
            let now = Utc::now();

            // Collect models already shown via priority selection
            let shown_labels: HashSet<&str> = ordered.iter().map(|m| m.label.as_str()).collect();

            // Remaining models not in the priority list
            let rest: Vec<_> = snapshot
                .models
                .iter()
                .filter(|m| !shown_labels.contains(m.label.as_str()))
                .collect();

            for model in ordered.iter().chain(rest) {
                if let Some(frac) = model.remaining_fraction {
                    let remaining_pct = frac * 100.0;
                    let used_pct = 100.0 - remaining_pct;
                    let bar = quota_bar(remaining_pct);
                    let mut line = format!(
                        "  {:<32} {bar}  {remaining_pct:5.1}% remaining  ({used_pct:.1}% used)",
                        model.label
                    );
                    if let Some(reset_ts) = model.reset_time {
                        let reset_text = format_reset_timestamp(reset_ts, &tz);
                        let eta_text = format_time_until_reset_short(reset_ts, now);
                        line.push_str(&format!("  resets {reset_text} (in {eta_text})"));
                    }
                    println!("{line}");
                } else {
                    let bar = "\x1b[90m[░░░░░░░░░░░░░░░░░░░░]\x1b[0m";
                    let mut line = format!("  {:<32} {bar}  quota not reported", model.label);
                    if let Some(reset_ts) = model.reset_time {
                        let reset_text = format_reset_timestamp(reset_ts, &tz);
                        let eta_text = format_time_until_reset_short(reset_ts, now);
                        line.push_str(&format!("  resets {reset_text} (in {eta_text})"));
                    }
                    println!("{line}");
                }
            }
        }
    }

    Ok(())
}

#[cfg(feature = "cli")]
pub(super) fn quota_bar(remaining_pct: f64) -> String {
    let width: usize = 20;
    let filled = ((remaining_pct / 100.0) * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    let color = if remaining_pct >= 40.0 {
        "\x1b[32m" // green
    } else if remaining_pct >= 15.0 {
        "\x1b[33m" // yellow
    } else {
        "\x1b[31m" // red
    };
    format!(
        "{color}[{}{}]\x1b[0m",
        "█".repeat(filled),
        "░".repeat(empty)
    )
}

#[cfg(feature = "cli")]
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
        let session_key = if event.session.is_empty() {
            event
                .project
                .clone()
                .unwrap_or_else(|| "unknown".to_string())
        } else {
            event.session.clone()
        };
        grouped.entry(session_key).or_default().add_event(&event);
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
            let models_by_source = agg
                .models_by_source
                .into_iter()
                .map(|(source, models)| (source.as_str().to_string(), models))
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
                models_by_source,
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
                models_by_source: row.models_by_source.clone(),
                activity: None,
            })
            .collect::<Vec<_>>();

        let totals = json_report.totals.clone();
        let insights = Some(crate::insights::compute_report_insights(
            &rows, &totals, None,
        ));

        let show = DailyReport {
            daily: rows,
            totals,
            activity_totals: None,
            stats: json_report.stats,
            insights,
        };
        print_report_table_with_options(
            &show,
            args.common.compact,
            args.common.breakdown,
            args.common.brief,
        );
        print_debug(&show.stats, &args.common);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// tu top — real-time per-session token process viewer (htop for tokens)
// ---------------------------------------------------------------------------

pub async fn collect_report(
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
                .iter()
                .filter(|event| match project.as_deref() {
                    Some(project_filter) => event
                        .project
                        .as_deref()
                        .is_some_and(|p| p.contains(project_filter)),
                    None => true,
                })
                .cloned()
                .collect::<Vec<_>>();

            build_group_rows(&events, ReportPeriod::Daily, &common, |event| {
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
        ReportPeriod::Monthly => {
            build_group_rows(&loaded.events, ReportPeriod::Monthly, &common, |event| {
                let day = local_date(event.timestamp, &tz);
                format!("{:04}-{:02}", day.year(), day.month())
            })
        }
        ReportPeriod::Weekly => {
            build_group_rows(&loaded.events, ReportPeriod::Weekly, &common, |event| {
                let day = local_date(event.timestamp, &tz);
                let start = week_start(day, start_of_week);
                start.format("%Y-%m-%d").to_string()
            })
        }
    };

    let (rows, activity_totals) = enrich_rows_with_activity(
        &common,
        period,
        instances,
        &tz,
        project.as_deref(),
        &loaded.events,
        rows,
    )
    .await?;

    let period_attribution = build_period_attribution(&loaded.events, |event| match period {
        ReportPeriod::Daily => {
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
        }
        ReportPeriod::Monthly => {
            let day = local_date(event.timestamp, &tz);
            format!("{:04}-{:02}", day.year(), day.month())
        }
        ReportPeriod::Weekly => {
            let day = local_date(event.timestamp, &tz);
            let start = week_start(day, start_of_week);
            start.format("%Y-%m-%d").to_string()
        }
    });

    Ok(build_report_from_rows(
        rows,
        activity_totals,
        loaded.stats,
        Some(period_attribution),
    ))
}

pub async fn collect_usage_snapshot(common: CommonArgs) -> Result<UsageSnapshot> {
    let timezone = parse_timezone_mode(common.timezone.as_deref())?;
    let loaded = load_usage(&common, &timezone).await?;
    Ok(UsageSnapshot {
        events: loaded.events,
        stats: loaded.stats,
        timezone,
    })
}

#[cfg(feature = "cli")]
pub(crate) async fn run_deepseek(args: DeepseekArgs) -> Result<()> {
    let snapshot = fetch_deepseek_official_limits()
        .await
        .context("Failed to query DeepSeek balance API")?;
    if args.json {
        emit_json(&snapshot, None)?;
        return Ok(());
    }
    if !snapshot.is_available {
        println!("DeepSeek  balance=unavailable");
        return Ok(());
    }
    let currency = snapshot.currency.as_deref().unwrap_or("USD");
    let total = snapshot.total_balance.unwrap_or(0.0);
    let granted = snapshot.granted_balance.unwrap_or(0.0);
    let topped_up = snapshot.topped_up_balance.unwrap_or(0.0);
    println!("DeepSeek");
    println!();
    println!("  balance        {total:.4} {currency}");
    println!("  granted        {granted:.4} {currency}");
    println!("  topped-up      {topped_up:.4} {currency}");
    Ok(())
}

#[cfg(feature = "cli")]
pub(crate) async fn run_openrouter(args: OpenrouterArgs) -> Result<()> {
    let snapshot = fetch_openrouter_account_limits()
        .await
        .context("Failed to query OpenRouter account API")?;
    if args.json {
        emit_json(&snapshot, None)?;
        return Ok(());
    }
    let label = snapshot.label.as_deref().unwrap_or("(no label)");
    println!("OpenRouter  key={label}");
    println!();
    let tier = if snapshot.is_free_tier {
        "free"
    } else {
        "paid"
    };
    println!("  tier           {tier}");
    if let Some(used) = snapshot.credits_used {
        println!("  credits used   ${used:.4}");
    }
    if let Some(limit) = snapshot.credits_limit {
        println!("  credits limit  ${limit:.4}");
    }
    if let Some(pct) = snapshot.used_percent {
        let bar = quota_bar(100.0 - pct);
        println!("  {bar}  {pct:.1}% used");
    }
    Ok(())
}

#[cfg(feature = "cli")]
pub(crate) async fn run_grok(args: GrokArgs) -> Result<()> {
    let snapshot = fetch_grok_official_limits()
        .await
        .context("Failed to query Grok billing API")?;
    if args.json {
        emit_json(&snapshot, None)?;
        return Ok(());
    }
    let currency = snapshot.currency.as_deref().unwrap_or("USD");
    println!("Grok (xAI)");
    println!();
    if let Some(granted) = snapshot.total_granted {
        println!("  total granted  {granted:.4} {currency}");
    }
    if let Some(used) = snapshot.total_used {
        println!("  total used     {used:.4} {currency}");
    }
    if let Some(remaining) = snapshot.total_remaining {
        println!("  remaining      {remaining:.4} {currency}");
    }
    if let Some(pct) = snapshot.used_percent {
        let bar = quota_bar(100.0 - pct);
        println!("  {bar}  {pct:.1}% used");
    }
    Ok(())
}

#[cfg(feature = "cli")]
pub(crate) async fn run_kimi(args: KimiArgs) -> Result<()> {
    let snapshot = fetch_kimi_official_limits()
        .await
        .context("Failed to query Kimi balance API")?;
    if args.json {
        emit_json(&snapshot, None)?;
        return Ok(());
    }
    let currency = snapshot.currency.as_deref().unwrap_or("CNY");
    println!("Kimi (Moonshot AI)");
    println!();
    if let Some(avail) = snapshot.available_balance {
        println!("  available      {avail:.4} {currency}");
    }
    if let Some(voucher) = snapshot.voucher_balance {
        println!("  voucher        {voucher:.4} {currency}");
    }
    if let Some(cash) = snapshot.cash_balance {
        println!("  cash           {cash:.4} {currency}");
    }
    Ok(())
}

#[cfg(feature = "cli")]
pub(crate) async fn run_anthropic_api(args: AnthropicApiArgs) -> Result<()> {
    let snapshot = fetch_anthropic_api_limits()
        .await
        .context("Failed to query Anthropic API usage")?;
    if args.json {
        emit_json(&snapshot, None)?;
        return Ok(());
    }
    println!("Anthropic API  (today)");
    println!();
    let key_label = if std::env::var("ANTHROPIC_ADMIN_KEY").is_ok() {
        "admin key"
    } else {
        "standard key"
    };
    println!("  auth           {key_label}");
    if let Some(input) = snapshot.input_tokens_today {
        println!("  input tokens   {input}");
    }
    if let Some(output) = snapshot.output_tokens_today {
        println!("  output tokens  {output}");
    }
    if let Some(cache) = snapshot.cache_read_tokens_today {
        println!("  cache read     {cache}");
    }
    if let Some(cost) = snapshot.cost_usd_today {
        println!("  cost today     ${cost:.4} USD");
    }
    Ok(())
}

#[cfg(feature = "cli")]
pub(crate) async fn run_carbon(args: CarbonArgs) -> Result<()> {
    use std::str::FromStr;

    let use_json = should_emit_json(&args.common);

    if args.period == CarbonPeriodArg::About {
        return print_carbon_about(use_json, args.common.jq.as_deref());
    }

    let grid = GridRegion::from_str(&args.region).unwrap_or(GridRegion::UsEast);
    let pue = 1.15;
    let wue = 4.30;

    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let loaded = load_usage(&args.common, &tz).await?;

    use chrono::Datelike;

    let today_date = local_date(Utc::now(), &tz);
    let monday_date = week_start(today_date, crate::cli::WeekStart::Monday);
    let month_start_date =
        chrono::NaiveDate::from_ymd_opt(today_date.year(), today_date.month(), 1)
            .unwrap_or(today_date);
    let seven_days_ago = today_date - chrono::TimeDelta::days(7);

    let events = loaded.events;

    // Filter events based on period (today, daily, weekly, monthly) and since/until
    let events = events
        .into_iter()
        .filter(|e| {
            let event_date = local_date(e.timestamp, &tz);
            match args.period {
                CarbonPeriodArg::Today => {
                    if event_date != today_date {
                        return false;
                    }
                }
                CarbonPeriodArg::Daily => {
                    if event_date < seven_days_ago {
                        return false;
                    }
                }
                CarbonPeriodArg::Weekly => {
                    if event_date < monday_date {
                        return false;
                    }
                }
                CarbonPeriodArg::Monthly => {
                    if event_date < month_start_date {
                        return false;
                    }
                }
                CarbonPeriodArg::All | CarbonPeriodArg::About => {}
            }
            if let Some(since) = &args.common.since {
                if let Ok(dt) = chrono::NaiveDate::parse_from_str(since, "%Y-%m-%d") {
                    if event_date < dt {
                        return false;
                    }
                }
            }
            if let Some(until) = &args.common.until {
                if let Ok(dt) = chrono::NaiveDate::parse_from_str(until, "%Y-%m-%d") {
                    if event_date > dt {
                        return false;
                    }
                }
            }
            true
        })
        .collect::<Vec<_>>();

    // Calculate overall metrics
    let total_metrics = EnvironmentalMetrics::calculate_events(&events, grid, pue, wue);
    let total_tokens: u64 = events.iter().map(|e| e.usage.total_tokens()).sum();
    let total_cost: f64 = events.iter().map(|e| e.usage.cost_usd).sum();

    // Group events by model for per-model breakdown
    let mut model_events: BTreeMap<String, Vec<&UsageEvent>> = BTreeMap::new();
    for event in &events {
        model_events
            .entry(event.model.clone())
            .or_default()
            .push(event);
    }

    #[derive(Serialize)]
    struct CarbonModelRow {
        model: String,
        events: usize,
        tokens: u64,
        cost_usd: f64,
        energy_kwh: f64,
        carbon_gco2e: f64,
        water_ml: f64,
    }

    let mut model_rows: Vec<CarbonModelRow> = Vec::new();
    for (model, evs) in &model_events {
        let evs_owned: Vec<UsageEvent> = evs.iter().map(|&e| e.clone()).collect();
        let m = EnvironmentalMetrics::calculate_events(&evs_owned, grid, pue, wue);
        let tok: u64 = evs.iter().map(|e| e.usage.total_tokens()).sum();
        let cost: f64 = evs.iter().map(|e| e.usage.cost_usd).sum();
        model_rows.push(CarbonModelRow {
            model: model.clone(),
            events: evs.len(),
            tokens: tok,
            cost_usd: cost,
            energy_kwh: m.energy_kwh,
            carbon_gco2e: m.carbon_gco2e,
            water_ml: m.water_ml,
        });
    }
    model_rows.sort_by(|a, b| b.carbon_gco2e.partial_cmp(&a.carbon_gco2e).unwrap());

    let equiv = EnvironmentalEquivalences::from_metrics(&total_metrics);
    let gco2_per_1k = if total_tokens > 0 {
        (total_metrics.carbon_gco2e / total_tokens as f64) * 1000.0
    } else {
        0.0
    };
    let (rating_grade, rating_desc) = eco_rating(gco2_per_1k);

    if use_json {
        #[derive(Serialize)]
        struct CarbonReportJson {
            period: String,
            grid_region: &'static str,
            pue: f64,
            wue_l_kwh: f64,
            total_tokens: u64,
            total_cost_usd: f64,
            environmental_metrics: EnvironmentalMetrics,
            equivalences: EnvironmentalEquivalences,
            eco_rating: String,
            models: Vec<CarbonModelRow>,
        }
        let out = CarbonReportJson {
            period: format!("{:?}", args.period),
            grid_region: grid.display_name(),
            pue,
            wue_l_kwh: wue,
            total_tokens,
            total_cost_usd: total_cost,
            environmental_metrics: total_metrics,
            equivalences: equiv,
            eco_rating: format!("Grade {rating_grade} ({rating_desc})"),
            models: model_rows,
        };
        emit_json(&out, args.common.jq.as_deref())
    } else {
        println!();
        println!(
            "+--------------------------------------------------------------------------------------+"
        );
        println!(
            "|                    TOKENUSAGE (tu) ENVIRONMENTAL IMPACT REPORT                      |"
        );
        println!(
            "|                                Period: {:<45} |",
            format!("{:?}", args.period)
        );
        println!(
            "+--------------------------------------------------------------------------------------+"
        );
        println!();
        println!(" SUMMARY METRICS:");
        println!(
            "  Total Tokens Processed : {}",
            format_commas_u64(total_tokens)
        );
        println!("  Estimated USD Cost     : {}", format_currency(total_cost));
        println!(
            "----------------------------------------------------------------------------------------"
        );
        println!(" ENERGY & POWER:");
        println!(
            "  Total IT + DC Energy   : {} kWh ({} Wh)",
            format_commas_f64(total_metrics.energy_kwh, 3),
            format_commas_f64(total_metrics.energy_kwh * 1000.0, 1)
        );
        println!(
            "  Datacenter PUE Factor  : {:.2}x (Hyperscaler Average)",
            pue
        );
        println!();
        println!(" ENVIRONMENTAL FOOTPRINT:");
        println!(
            "  Carbon Footprint (CO2e): {}",
            format_carbon_human(total_metrics.carbon_gco2e)
        );
        println!("  Grid Carbon Intensity  : {}", grid.display_name());
        println!(
            "  Water Consumption      : {}",
            format_water_human(total_metrics.water_ml)
        );
        println!();
        println!(" ENVIRONMENTAL EQUIVALENCES:");
        for line in equiv.wow_factor_summary_lines(&total_metrics) {
            println!("{}", line);
        }
        println!();
        println!(" MODEL BREAKDOWN:");
        println!(
            "{:<30} {:>16} {:>14} {:>14} {:>12}",
            "Model", "Tokens", "Energy (kWh)", "Carbon (kg)", "Water (L)"
        );
        println!("{}", "-".repeat(88));
        for row in &model_rows {
            println!(
                "{:<30} {:>16} {:>14.4} {:>14.2} {:>12.2}",
                truncate_str_len(&row.model, 30),
                format_commas_u64(row.tokens),
                row.energy_kwh,
                row.carbon_gco2e / 1000.0,
                row.water_ml / 1000.0
            );
        }
        println!("{}", "-".repeat(88));
        println!(
            "{:<30} {:>16} {:>14.4} {:>14.2} {:>12.2}",
            "TOTAL",
            format_commas_u64(total_tokens),
            total_metrics.energy_kwh,
            total_metrics.carbon_gco2e / 1000.0,
            total_metrics.water_ml / 1000.0
        );
        println!();
        println!(
            " ECO-EFFICIENCY RATING:  [ Grade {} ] - {}",
            rating_grade, rating_desc
        );
        println!(
            "+--------------------------------------------------------------------------------------+"
        );
        println!(" DATA PROVENANCE & METHODOLOGY NOTE:");
        println!(
            "  • Anthropic, OpenAI & Google treat per-inference energy & cluster hardware as trade secrets."
        );
        println!(
            "  • Estimates use peer-reviewed GPU physics models (EcoLogits, ML.ENERGY, Luccioni et al. 2023)."
        );
        println!(
            "  • Assumes tiered H100/A100 compute profiles, PUE {:.2}x, & {} grid.",
            pue,
            grid.display_name()
        );
        println!(
            "  • 💡 Tip: Run 'tu carbon about' for full methodology, physics assumptions & term definitions."
        );
        println!(
            "+--------------------------------------------------------------------------------------+"
        );
        println!();
        Ok(())
    }
}

fn print_carbon_about(use_json: bool, jq: Option<&str>) -> Result<()> {
    if use_json {
        #[derive(Serialize)]
        struct AboutOutput {
            title: &'static str,
            purpose: &'static str,
            physics_assumptions: serde_json::Value,
            jargon_glossary: serde_json::Value,
            equivalences: serde_json::Value,
            data_confidence: serde_json::Value,
        }
        let out = AboutOutput {
            title: "TOKENUSAGE (tu) - ENVIRONMENTAL IMPACT METRICS & METHODOLOGY GUIDE",
            purpose: "Calculates electrical energy (kWh), carbon emissions (gCO2e), and cooling water draw (mL) associated with LLM token workloads.",
            physics_assumptions: serde_json::json!({
                "prefill_input": "Compute-bound matrix multiplication (0.03 - 0.15 kWh per 1M tokens)",
                "autoregressive_decoding_output": "Memory-bandwidth bound token generation (0.18 - 3.80 kWh per 1M tokens; 4x-6x higher energy per token)",
                "prompt_cache_read": "KV cache reuse skipping matrix multiplication (0.02 - 0.40 kWh per 1M tokens)",
                "reasoning_thinking_tokens": "Chain-of-thought tokens produced during reasoning steps (billed at output decoding energy rates)",
            }),
            jargon_glossary: serde_json::json!({
                "pue": "Power Usage Effectiveness: Ratio of total datacenter facility energy to IT hardware energy (Default: 1.15x)",
                "wue": "Water Usage Effectiveness: Direct evaporative cooling + power plant cooling water draw (Default: 4.30 L/kWh)",
                "carbon_intensity": "Grams of CO2e per kWh based on regional grid mix (us-east: 310, us-west: 120, eu-west: 55, nordic: 15, google-cfe: 100, global: 475)",
                "embodied_carbon": "Emissions from GPU silicon manufacturing & datacenter construction (~0.003 gCO2e per 1k tokens)",
            }),
            equivalences: serde_json::json!({
                "smartphone_charge": "12 Watt-hours (Wh) per full charge",
                "kettle_cup_boiled": "15 Watt-hours (Wh) to boil 1 cup (250ml) water",
                "ev_km_driven": "150 Wh per kilometer driven in an Electric Vehicle",
                "petrol_car_km_driven": "690 Wh energy equivalent / 240 gCO2e per km",
                "water_bottle_evaporated": "400 mL drinking water bottle evaporated for datacenter cooling",
            }),
            data_confidence: serde_json::json!({
                "grade_a": "High Precision: Local M-Series Apple Silicon (MLX / Ollama direct SoC profiler)",
                "grade_b_plus": "Medium-High: Google Gemini (Google annual PUE 1.10 & 24/7 CFE reports)",
                "grade_b": "Scientific Estimation: Anthropic Claude & OpenAI GPT (Peer-reviewed models: EcoLogits, CodeCarbon, ML.ENERGY, Luccioni et al. 2023)",
            }),
        };
        emit_json(&out, jq)
    } else {
        println!();
        println!(
            "========================================================================================"
        );
        println!("             TOKENUSAGE (tu) - ENVIRONMENTAL IMPACT METRICS & METHODOLOGY GUIDE");
        println!(
            "========================================================================================"
        );
        println!();
        println!(" 🌿 1. PURPOSE & OVERVIEW");
        println!(
            "    As developer AI tooling (Claude Code, Antigravity, Codex, Grok, etc.) consumes millions"
        );
        println!(
            "    of tokens daily, tokenusage calculates the physical electrical energy (kWh), carbon"
        );
        println!(
            "    emissions (gCO2e), and cooling water draw (mL) associated with your LLM workloads."
        );
        println!();
        println!(" ⚡ 2. COMPUTATIONAL & ENERGY PHYSICS ASSUMPTIONS");
        println!("    LLM inference operates in distinct computational phases:");
        println!();
        println!("    • PREFILL (Input Tokens):");
        println!(
            "      Compute-bound matrix multiplication (2 * P * L FLOPs). Processed in parallel"
        );
        println!("      batches. Energy rate: ~0.03 - 0.15 kWh per 1 Million tokens.");
        println!();
        println!("    • AUTOREGRESSIVE DECODING (Output & Reasoning Tokens):");
        println!(
            "      Memory-bandwidth bound. Every generated token requires loading the full model weight"
        );
        println!("      matrices (2 * P bytes) from GPU High-Bandwidth Memory (HBM).");
        println!(
            "      --> Energy rate: ~0.18 - 3.80 kWh per 1 Million tokens (4x - 6x MORE energy than input!)."
        );
        println!();
        println!("    • PROMPT CACHE READS:");
        println!("      Reuses existing KV attention state, skipping model weight calculations.");
        println!(
            "      --> Energy rate: ~0.02 - 0.40 kWh per 1 Million tokens (~80% energy savings)."
        );
        println!();
        println!("    • REASONING / THINKING TOKENS:");
        println!(
            "      Chain-of-thought tokens produced during reasoning steps (e.g. Claude Opus 4.5/5, o1/o3)."
        );
        println!(
            "      Billed at output decoding energy rates due to step-by-step HBM memory passes."
        );
        println!();
        println!(" 📊 3. TERMINOLOGY & JARGON GLOSSARY");
        println!("    • PUE (Power Usage Effectiveness):");
        println!("      Ratio of total datacenter facility energy to IT hardware energy.");
        println!(
            "      --> Default: 1.15x (Hyperscaler average: 1.0 kWh GPU compute + 0.15 kWh cooling/power)."
        );
        println!();
        println!("    • WUE (Water Usage Effectiveness):");
        println!(
            "      Total water consumed per kWh generated (direct cooling evaporation + indirect power plant cooling)."
        );
        println!(
            "      --> Default: 4.30 Liters / kWh (UC Riverside 'Making AI Less Thirsty' study)."
        );
        println!();
        println!("    • GRID CARBON INTENSITY (gCO2e / kWh):");
        println!(
            "      Carbon emissions produced per kilowatt-hour based on the regional electrical grid mix:"
        );
        println!(
            "        - us-east    : 310 gCO2e/kWh (Virginia / AWS us-east-1 fossil + nuclear)"
        );
        println!("        - us-west    : 120 gCO2e/kWh (Oregon Hydroelectric)");
        println!("        - us-avg     : 368 gCO2e/kWh (US National Grid Average)");
        println!("        - eu-west    :  55 gCO2e/kWh (France Low-Carbon Nuclear)");
        println!("        - nordic     :  15 gCO2e/kWh (Iceland/Norway 100% Hydro/Geothermal)");
        println!("        - google-cfe : 100 gCO2e/kWh (Google 24/7 Clean Energy Matched Average)");
        println!("        - global     : 475 gCO2e/kWh (Global Grid Average)");
        println!();
        println!("    • EMBODIED CARBON:");
        println!(
            "      Emissions from GPU silicon manufacturing, server chassis, and datacenter construction,"
        );
        println!("      amortized across lifetime token volume (~0.003 gCO2e per 1,000 tokens).");
        println!();
        println!(" ☕ 4. HUMAN-SCALE EQUIVALENCES & BENCHMARKS");
        println!("    • 📱 Smartphone Charge  : ~12 Watt-hours (Wh) full charge.");
        println!(
            "    • ☕ Cup of Tea / Coffee : ~15 Watt-hours (Wh) to boil 250ml water in an electric kettle."
        );
        println!("    • 🚗 EV Driving        : ~150 Wh per kilometer driven.");
        println!("    • 🚙 Gas Car Driving    : ~690 Wh energy equivalent (or ~240 gCO2 per km).");
        println!(
            "    • 💧 Bottled Water       : 400 mL drinking water bottle evaporated for datacenter cooling."
        );
        println!();
        println!(" 🔍 5. PROVIDER TRANSPARENCY & DATA CONFIDENCE GRADES");
        println!(
            "    Anthropic, OpenAI, and Google treat specific accelerator hardware allocations, active parameter"
        );
        println!(
            "    counts, and cluster PUEs as trade secrets. tokenusage assigns confidence grades:"
        );
        println!();
        println!(
            "    🟢 Grade A (High Precision): Local M-Series Apple Silicon (MLX / Ollama direct SoC profiler)."
        );
        println!(
            "    🟡 Grade B+ (Medium-High): Google Gemini (Google publishes annual 1.10 PUE & 24/7 CFE metrics)."
        );
        println!(
            "    🟠 Grade B (Scientific Estimation): Anthropic Claude & OpenAI GPT (Peer-reviewed GPU models:"
        );
        println!(
            "                                         EcoLogits, CodeCarbon, ML.ENERGY, Luccioni et al. 2023)."
        );
        println!();
        println!(
            "========================================================================================"
        );
        println!();
        Ok(())
    }
}

fn truncate_str_len(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count > max_len {
        let keep = max_len.saturating_sub(3);
        let prefix: String = s.chars().take(keep).collect();
        format!("{prefix}...")
    } else {
        s.to_string()
    }
}
