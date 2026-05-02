use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::Serialize;

use crate::activity::{ActivityDataset, activity_enabled, fetch_activity_dataset};
#[cfg(feature = "cli")]
use crate::cli::{
    ActivityArgs, AntigravityArgs, DailyArgs, MonthlyArgs, SessionArgs, TodayArgs, WeeklyArgs,
};
use crate::cli::{CommonArgs, SortOrder};
#[cfg(feature = "cli")]
use crate::output::{print_report_table_with_options, run_report_tui};
use crate::types::{ActivitySummary, DailyReport, DailyRow, ParseStats, TokenCounts, UsageEvent};

#[cfg(feature = "cli")]
use super::activity_report::*;
#[cfg(feature = "cli")]
use super::official::{fetch_antigravity_official_limits, select_antigravity_models};
use super::parsing::{build_sources, discover_files, load_usage};
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
    sources: Vec<DoctorSourceReport>,
    opencode_debug: Option<DoctorOpencodeDebugReport>,
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

    let report = DoctorReport {
        timezone: format!("{tz:?}"),
        selected_sources: args
            .common
            .selected_sources()
            .into_iter()
            .map(|source| source.as_str().to_string())
            .collect(),
        sources,
        opencode_debug,
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
        Ok(())
    }
}

fn collect_source_totals(events: &[UsageEvent], source: SourceKind) -> TokenCounts {
    events
        .iter()
        .filter(|event| event.source == source)
        .fold(TokenCounts::default(), |mut acc, event| {
            acc.add_assign(event.usage.to_counts());
            acc
        })
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
    db_only_args.ignore_path.push("storage\\message".to_string());

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

    let rows = build_group_rows(&events, &args.common.order, |event| {
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

    let report = build_report_from_rows(rows, activity_totals, loaded.stats);

    if use_json {
        emit_json(&report, args.common.jq.as_deref())
    } else if args.tui {
        run_report_tui(&report)
    } else {
        print_report_table_with_options(&report, args.common.compact, args.common.breakdown);
        print_debug(&report.stats, &args.common);
        Ok(())
    }
}

#[cfg(feature = "cli")]
pub(crate) async fn run_monthly(args: MonthlyArgs) -> Result<()> {
    let use_json = should_emit_json(&args.common);
    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let loaded = load_usage(&args.common, &tz).await?;

    let rows = build_group_rows(&loaded.events, &args.common.order, |event| {
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

    let report = build_report_from_rows(rows, activity_totals, loaded.stats);

    if use_json {
        #[derive(Serialize)]
        struct MonthlyOut {
            monthly: Vec<DailyRow>,
            totals: TokenCounts,
            activity_totals: Option<ActivitySummary>,
            stats: ParseStats,
        }
        let out = MonthlyOut {
            monthly: report.daily,
            totals: report.totals,
            activity_totals: report.activity_totals,
            stats: report.stats,
        };
        emit_json(&out, args.common.jq.as_deref())
    } else {
        let show = DailyReport {
            daily: report.daily,
            totals: report.totals,
            activity_totals: report.activity_totals,
            stats: report.stats,
        };
        print_report_table_with_options(&show, args.common.compact, args.common.breakdown);
        print_debug(&show.stats, &args.common);
        Ok(())
    }
}

#[cfg(feature = "cli")]
pub(crate) async fn run_weekly(args: WeeklyArgs) -> Result<()> {
    let use_json = should_emit_json(&args.common);
    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let loaded = load_usage(&args.common, &tz).await?;

    let rows = build_group_rows(&loaded.events, &args.common.order, |event| {
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

    let report = build_report_from_rows(rows, activity_totals, loaded.stats);

    if use_json {
        #[derive(Serialize)]
        struct WeeklyOut {
            weekly: Vec<DailyRow>,
            totals: TokenCounts,
            activity_totals: Option<ActivitySummary>,
            stats: ParseStats,
        }
        let out = WeeklyOut {
            weekly: report.daily,
            totals: report.totals,
            activity_totals: report.activity_totals,
            stats: report.stats,
        };
        emit_json(&out, args.common.jq.as_deref())
    } else {
        let show = DailyReport {
            daily: report.daily,
            totals: report.totals,
            activity_totals: report.activity_totals,
            stats: report.stats,
        };
        print_report_table_with_options(&show, args.common.compact, args.common.breakdown);
        print_debug(&show.stats, &args.common);
        Ok(())
    }
}

#[cfg(feature = "cli")]
pub(crate) async fn run_today(mut args: TodayArgs) -> Result<()> {
    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let today = tz.now_date();
    args.common.with_activity = true;
    args.common.since = Some(today.to_string());
    args.common.until = Some(today.to_string());

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
    let activity = dataset.summary_for_day(today);
    let hourly_activity = dataset.hourly_buckets_for_day(today, None);
    let hourly_tokens = aggregate_hourly_token_counts(&filtered_events, today, &tz);
    let hourly_rows = join_hourly_rows(&hourly_activity, &hourly_tokens);
    let overview = build_activity_overview(
        today,
        today,
        u32::from(day_totals.total_tokens > 0 || activity.is_some()),
        &filtered_events,
        activity.as_ref(),
    );
    let breakdowns = TodayProjectBreakdowns {
        projects: enrich_activity_breakdowns_with_tokens(
            dataset.project_breakdowns(today, today, 5),
            &aggregate_usage_totals_by_project(&filtered_events),
        ),
        languages: enrich_activity_breakdowns_with_tokens(
            dataset.language_breakdowns(today, today, 5),
            &aggregate_usage_totals_by_language(&filtered_events),
        ),
        sources: enrich_activity_breakdowns_with_tokens(
            dataset.source_breakdowns(today, today, 5),
            &aggregate_usage_totals_by_source(&filtered_events),
        ),
        models,
    };

    if use_json {
        let out = TodayOut {
            date: today.to_string(),
            overview,
            hourly: hourly_rows,
            breakdowns,
            stats,
        };
        emit_json(&out, args.common.jq.as_deref())
    } else {
        print_today_view(
            &today.to_string(),
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
        print_report_table_with_options(&report, args.common.compact, false);
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

            for model in ordered.iter().chain(rest.into_iter()) {
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
        grouped
            .entry(event.session.clone())
            .or_default()
            .add_event(&event);
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
                activity: None,
            })
            .collect::<Vec<_>>();

        let show = DailyReport {
            daily: rows,
            totals: json_report.totals,
            activity_totals: None,
            stats: json_report.stats,
        };
        print_report_table_with_options(&show, args.common.compact, args.common.breakdown);
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

            build_group_rows(&events, &common.order, |event| {
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
        ReportPeriod::Monthly => build_group_rows(&loaded.events, &common.order, |event| {
            let day = local_date(event.timestamp, &tz);
            format!("{:04}-{:02}", day.year(), day.month())
        }),
        ReportPeriod::Weekly => build_group_rows(&loaded.events, &common.order, |event| {
            let day = local_date(event.timestamp, &tz);
            let start = week_start(day, start_of_week);
            start.format("%Y-%m-%d").to_string()
        }),
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

    Ok(build_report_from_rows(rows, activity_totals, loaded.stats))
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
