use std::collections::HashMap;

use anyhow::Result;
use chrono::NaiveDate;
use comfy_table::{Color as TableColor, Row as TableRow};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color as TuiColor, Style};
use ratatui::symbols;
use ratatui::text::Span;
use ratatui::widgets::{Axis, Block, Chart, Dataset, GraphType};
use serde::Serialize;

use crate::activity::{
    ActivityBreakdownStat, ActivityDataset, ActivityHourlyBucket, format_activity_duration,
};
use crate::cli::CommonArgs;
use crate::types::{ActivitySummary, SourceKind, TokenCounts, UsageEvent};

use super::display::*;
use super::*;

#[derive(Debug, Clone, Serialize)]
pub(super) struct ActivityOverview {
    pub(super) start: String,
    pub(super) end: String,
    pub(super) days_in_range: u32,
    pub(super) active_days: u32,
    pub(super) totals: TokenCounts,
    pub(super) activity: Option<ActivitySummary>,
    pub(super) avg_coding_per_day: Option<String>,
    pub(super) tokens_per_hour: Option<u64>,
    pub(super) cost_per_hour: Option<f64>,
    pub(super) top_model: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct TodayHourlyRow {
    hour: u8,
    label: String,
    coding_seconds: u64,
    coding_text: String,
    tokens: TokenCounts,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct TodayProjectBreakdowns {
    pub(super) projects: Vec<ActivityBreakdownStat>,
    pub(super) languages: Vec<ActivityBreakdownStat>,
    pub(super) sources: Vec<ActivityBreakdownStat>,
    pub(super) models: Vec<TokenBreakdownStat>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ActivityRangeBreakdowns {
    pub(super) projects: Vec<ActivityBreakdownStat>,
    pub(super) languages: Vec<ActivityBreakdownStat>,
    pub(super) sources: Vec<ActivityBreakdownStat>,
    pub(super) models: Vec<TokenBreakdownStat>,
}

#[derive(Debug, Clone)]
pub(super) struct ActivityReportBuildOptions<'a> {
    pub(super) tz: &'a TimeZoneMode,
    pub(super) order: &'a SortOrder,
    pub(super) start: NaiveDate,
    pub(super) end: NaiveDate,
    pub(super) stats: ParseStats,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct TokenBreakdownStat {
    name: String,
    total_tokens: u64,
    cost_usd: f64,
    percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct TodayOut {
    pub(super) date: String,
    pub(super) overview: ActivityOverview,
    pub(super) hourly: Vec<TodayHourlyRow>,
    pub(super) breakdowns: TodayProjectBreakdowns,
    pub(super) stats: ParseStats,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ActivityOut {
    pub(super) overview: ActivityOverview,
    pub(super) daily: Vec<DailyRow>,
    pub(super) breakdowns: ActivityRangeBreakdowns,
    pub(super) stats: ParseStats,
}

pub(super) fn apply_default_activity_range(
    common: &mut CommonArgs,
    tz: &TimeZoneMode,
    days: u32,
) -> Result<()> {
    let fallback_days = days.max(1);
    let today = tz.now_date();

    match (
        parse_date_filter(common.since.as_deref())?,
        parse_date_filter(common.until.as_deref())?,
    ) {
        (None, None) => {
            let start = today
                .checked_sub_signed(chrono::TimeDelta::days(i64::from(fallback_days - 1)))
                .unwrap_or(today);
            common.since = Some(start.to_string());
            common.until = Some(today.to_string());
        }
        (None, Some(until)) => {
            let start = until
                .checked_sub_signed(chrono::TimeDelta::days(i64::from(fallback_days - 1)))
                .unwrap_or(until);
            common.since = Some(start.to_string());
        }
        (Some(_), None) => {
            common.until = Some(today.to_string());
        }
        (Some(_), Some(_)) => {}
    }

    Ok(())
}

pub(super) fn filter_events_by_project(
    events: Vec<UsageEvent>,
    project_filter: Option<&str>,
) -> Vec<UsageEvent> {
    events
        .into_iter()
        .filter(|event| event_matches_project(event, project_filter))
        .collect()
}

pub(super) fn event_matches_project(event: &UsageEvent, project_filter: Option<&str>) -> bool {
    match project_filter {
        Some(project_filter) => event
            .project
            .as_deref()
            .is_some_and(|project| project.contains(project_filter)),
        None => true,
    }
}

pub(super) fn aggregate_token_counts(events: &[UsageEvent]) -> TokenCounts {
    events
        .iter()
        .fold(TokenCounts::default(), |mut acc, event| {
            acc.add_assign(event.usage.to_counts());
            acc
        })
}

pub(super) fn aggregate_hourly_token_counts(
    events: &[UsageEvent],
    day: NaiveDate,
    tz: &TimeZoneMode,
) -> Vec<TokenCounts> {
    let mut buckets = std::iter::repeat_with(TokenCounts::default)
        .take(24)
        .collect::<Vec<_>>();

    for event in events {
        if local_date(event.timestamp, tz) != day {
            continue;
        }
        let hour = tz.hour_of(event.timestamp) as usize;
        if let Some(bucket) = buckets.get_mut(hour) {
            bucket.add_assign(event.usage.to_counts());
        }
    }

    buckets
}

pub(super) fn join_hourly_rows(
    activity_rows: &[ActivityHourlyBucket],
    token_rows: &[TokenCounts],
) -> Vec<TodayHourlyRow> {
    (0usize..24)
        .map(|hour| {
            let activity = activity_rows
                .get(hour)
                .cloned()
                .unwrap_or(ActivityHourlyBucket {
                    hour: hour as u8,
                    total_seconds: 0,
                    text: "0s".to_string(),
                });
            let totals = token_rows.get(hour).cloned().unwrap_or_default();
            TodayHourlyRow {
                hour: hour as u8,
                label: format!("{hour:02}:00-{:02}:00", (hour + 1) % 24),
                coding_seconds: activity.total_seconds,
                coding_text: activity.text,
                tokens: totals,
            }
        })
        .collect()
}

pub(super) fn build_activity_daily_report(
    events: &[UsageEvent],
    dataset: &ActivityDataset,
    options: ActivityReportBuildOptions<'_>,
) -> DailyReport {
    let mut grouped: BTreeMap<NaiveDate, GroupAggregate> = BTreeMap::new();
    for event in events {
        let day = local_date(event.timestamp, options.tz);
        grouped.entry(day).or_default().add_event(event);
    }

    let mut rows = Vec::new();
    let mut cursor = options.start;
    while cursor <= options.end {
        let token_group = grouped.remove(&cursor);
        let activity = dataset.summary_for_day(cursor);
        if token_group.is_none() && activity.is_none() {
            cursor = cursor
                .checked_add_signed(chrono::TimeDelta::days(1))
                .unwrap_or(options.end.succ_opt().unwrap_or(options.end));
            continue;
        }

        let (totals, models, sources) = if let Some(group) = token_group {
            (
                group.totals.to_counts(),
                group
                    .by_model
                    .into_iter()
                    .map(|(model, totals)| (model, totals.to_counts()))
                    .collect::<BTreeMap<_, _>>(),
                group
                    .by_source
                    .into_iter()
                    .map(|(source, totals)| (source.as_str().to_string(), totals.to_counts()))
                    .collect::<BTreeMap<_, _>>(),
            )
        } else {
            (TokenCounts::default(), BTreeMap::new(), BTreeMap::new())
        };

        rows.push(DailyRow {
            date: cursor.format("%Y-%m-%d").to_string(),
            totals,
            models,
            sources,
            activity,
        });

        cursor = cursor
            .checked_add_signed(chrono::TimeDelta::days(1))
            .unwrap_or(options.end.succ_opt().unwrap_or(options.end));
    }

    if *options.order == SortOrder::Desc {
        rows.reverse();
    }

    let activity_totals = dataset.summary_for_range(options.start, options.end);

    build_report_from_rows(rows, activity_totals, options.stats)
}

pub(super) fn build_activity_overview(
    start: NaiveDate,
    end: NaiveDate,
    active_days: u32,
    events: &[UsageEvent],
    activity: Option<&ActivitySummary>,
) -> ActivityOverview {
    let totals = aggregate_token_counts(events);
    let days_in_range = (end - start).num_days().max(0) as u32 + 1;
    let top_model = token_breakdowns_by_model(events, 1)
        .into_iter()
        .next()
        .map(|row| row.name);
    let (tokens_per_hour, cost_per_hour) = activity_rate(activity, &totals).unwrap_or((0, 0.0));

    ActivityOverview {
        start: start.to_string(),
        end: end.to_string(),
        days_in_range,
        active_days,
        totals,
        activity: activity.cloned(),
        avg_coding_per_day: activity.map(|summary| {
            format_activity_duration(summary.total_seconds / u64::from(days_in_range.max(1)))
        }),
        tokens_per_hour: activity
            .filter(|summary| summary.total_seconds > 0)
            .map(|_| tokens_per_hour),
        cost_per_hour: activity
            .filter(|summary| summary.total_seconds > 0)
            .map(|_| cost_per_hour),
        top_model,
    }
}

pub(super) fn activity_rate(
    activity: Option<&ActivitySummary>,
    totals: &TokenCounts,
) -> Option<(u64, f64)> {
    let summary = activity?;
    if summary.total_seconds == 0 {
        return None;
    }
    let hours = summary.total_seconds as f64 / 3600.0;
    if hours <= f64::EPSILON {
        return None;
    }
    Some((
        (totals.total_tokens as f64 / hours).round().max(0.0) as u64,
        totals.cost_usd / hours,
    ))
}

pub(super) fn token_breakdowns_by_model(
    events: &[UsageEvent],
    limit: usize,
) -> Vec<TokenBreakdownStat> {
    let mut totals = HashMap::<String, TokenCounts>::new();
    let mut grand_total = 0u64;

    for event in events {
        let counts = event.usage.to_counts();
        grand_total = grand_total.saturating_add(counts.total_tokens);
        totals
            .entry(event.model.clone())
            .or_default()
            .add_assign(counts);
    }

    let mut rows = totals.into_iter().collect::<Vec<_>>();
    rows.sort_by(|(name_a, counts_a), (name_b, counts_b)| {
        counts_b
            .total_tokens
            .cmp(&counts_a.total_tokens)
            .then_with(|| name_a.cmp(name_b))
    });
    rows.truncate(limit.max(1));

    rows.into_iter()
        .map(|(name, counts)| TokenBreakdownStat {
            name,
            total_tokens: counts.total_tokens,
            cost_usd: counts.cost_usd,
            percent: if grand_total == 0 {
                0.0
            } else {
                (counts.total_tokens as f64 / grand_total as f64) * 100.0
            },
        })
        .collect()
}

pub(super) fn aggregate_usage_totals_by_project(
    events: &[UsageEvent],
) -> HashMap<String, TokenCounts> {
    aggregate_usage_totals_by(events, |event| Some(project_label_for_activity(event)))
}

pub(super) fn aggregate_usage_totals_by_language(
    _events: &[UsageEvent],
) -> HashMap<String, TokenCounts> {
    HashMap::new()
}

pub(super) fn aggregate_usage_totals_by_source(
    events: &[UsageEvent],
) -> HashMap<String, TokenCounts> {
    aggregate_usage_totals_by(events, |event| {
        Some(activity_source_label(event.source).to_string())
    })
}

pub(super) fn aggregate_usage_totals_by<F>(
    events: &[UsageEvent],
    mut label_fn: F,
) -> HashMap<String, TokenCounts>
where
    F: FnMut(&UsageEvent) -> Option<String>,
{
    let mut totals = HashMap::<String, TokenCounts>::new();
    for event in events {
        let Some(label) = label_fn(event) else {
            continue;
        };
        let counts = event.usage.to_counts();
        totals
            .entry(label)
            .and_modify(|value| value.add_assign(counts.clone()))
            .or_insert(counts);
    }
    totals
}

pub(super) fn enrich_activity_breakdowns_with_tokens(
    mut rows: Vec<ActivityBreakdownStat>,
    usage_totals: &HashMap<String, TokenCounts>,
) -> Vec<ActivityBreakdownStat> {
    for row in &mut rows {
        if let Some(counts) = usage_totals.get(&row.name) {
            row.total_tokens = counts.total_tokens;
            row.cost_usd = counts.cost_usd;
        }
    }
    rows
}

pub(super) fn project_label_for_activity(event: &UsageEvent) -> String {
    if let Some(project) = event.project.as_deref().map(str::trim)
        && !project.is_empty()
    {
        return project.to_string();
    }

    let path = std::path::Path::new(&event.file_path);
    if let Some(parent) = path.parent().and_then(|value| value.file_name()) {
        let name = parent.to_string_lossy().trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }

    "unknown".to_string()
}

pub(super) fn activity_source_label(source: SourceKind) -> &'static str {
    match source {
        SourceKind::Claude => "Claude",
        SourceKind::Codex => "Codex",
        SourceKind::Gemini => "Gemini",
        SourceKind::OpenCode => "OpenCode",
    }
}

pub(super) fn print_activity_overview(title: &str, overview: &ActivityOverview) {
    if overview.start == overview.end {
        println!("{title} {}", overview.start);
    } else {
        println!("{title} {} -> {}", overview.start, overview.end);
    }
    println!(
        "{:<12} {} {:>5.1}%  {}/{}",
        "Active days",
        render_progress_meter(
            overview.active_days as f64 / overview.days_in_range.max(1) as f64,
            18
        ),
        overview.active_days as f64 / overview.days_in_range.max(1) as f64 * 100.0,
        overview.active_days,
        overview.days_in_range
    );
    if let Some(activity) = overview.activity.as_ref() {
        println!(
            "{:<12} {} {:>5.1}%  {}",
            "Day cover",
            render_progress_meter(activity.total_seconds as f64 / 86_400.0, 18),
            activity.total_seconds as f64 / 86_400.0 * 100.0,
            activity.text
        );
    }

    let inline_metrics = should_render_activity_metrics_inline(overview);
    let mut table = create_text_table();
    table.add_row(TableRow::from(vec![
        metric_value_cell(
            "Coding",
            overview
                .activity
                .as_ref()
                .map(|summary| summary.text.as_str())
                .unwrap_or("-"),
            Some(TableColor::Green),
            inline_metrics,
        ),
        metric_value_cell(
            "Avg / day",
            overview.avg_coding_per_day.as_deref().unwrap_or("-"),
            Some(TableColor::Green),
            inline_metrics,
        ),
        metric_value_cell(
            "Tokens",
            &format_u64(overview.totals.total_tokens),
            Some(TableColor::Cyan),
            inline_metrics,
        ),
        metric_value_cell(
            "Cost",
            &format_usd(overview.totals.cost_usd),
            Some(TableColor::Green),
            inline_metrics,
        ),
    ]));
    table.add_row(TableRow::from(vec![
        metric_value_cell(
            "Tok / hr",
            &overview
                .tokens_per_hour
                .map(format_u64)
                .unwrap_or_else(|| "-".to_string()),
            Some(TableColor::Yellow),
            inline_metrics,
        ),
        metric_value_cell(
            "Cost / hr",
            &overview
                .cost_per_hour
                .map(format_usd)
                .unwrap_or_else(|| "-".to_string()),
            Some(TableColor::Yellow),
            inline_metrics,
        ),
        metric_value_cell(
            "Top model",
            overview.top_model.as_deref().unwrap_or("-"),
            Some(TableColor::White),
            inline_metrics,
        ),
        metric_value_cell(
            "Top project",
            overview
                .activity
                .as_ref()
                .and_then(|activity| activity.top_project.as_deref())
                .unwrap_or("-"),
            Some(TableColor::White),
            inline_metrics,
        ),
    ]));
    table.add_row(TableRow::from(vec![
        metric_value_cell(
            "Top lang",
            overview
                .activity
                .as_ref()
                .and_then(|activity| activity.top_language.as_deref())
                .unwrap_or("-"),
            Some(TableColor::White),
            inline_metrics,
        ),
        metric_value_cell(
            "Top source",
            overview
                .activity
                .as_ref()
                .and_then(|activity| activity.top_source.as_deref())
                .unwrap_or("-"),
            Some(TableColor::White),
            inline_metrics,
        ),
        metric_value_cell(
            "Range",
            &if overview.start == overview.end {
                overview.start.clone()
            } else {
                format!("{} -> {}", overview.start, overview.end)
            },
            Some(TableColor::DarkGrey),
            inline_metrics,
        ),
        value_cell("", None),
    ]));
    println!("{table}");
}

pub(super) fn print_today_view(
    date: &str,
    overview: &ActivityOverview,
    hourly: &[TodayHourlyRow],
    breakdowns: &TodayProjectBreakdowns,
    total_tokens: u64,
) {
    print_activity_overview("Today", overview);

    let render_chart = should_render_today_hourly_chart(hourly);
    if !render_chart
        && let Some(peak) = hourly
            .iter()
            .max_by(|a, b| {
                a.coding_seconds
                    .cmp(&b.coding_seconds)
                    .then_with(|| a.tokens.total_tokens.cmp(&b.tokens.total_tokens))
            })
            .filter(|row| row.coding_seconds > 0 || row.tokens.total_tokens > 0)
    {
        let mut peak_table = create_text_table();
        peak_table.set_header(vec![
            header_cell("Peak window"),
            header_cell("Coding"),
            header_cell("Tokens"),
            header_cell("Cost"),
        ]);
        peak_table.add_row(TableRow::from(vec![
            value_cell(&peak.label, Some(TableColor::White)),
            value_cell(
                if peak.coding_seconds > 0 {
                    &peak.coding_text
                } else {
                    "-"
                },
                Some(TableColor::Green),
            ),
            value_cell(
                &format_u64(peak.tokens.total_tokens),
                Some(TableColor::Cyan),
            ),
            value_cell(&format_usd(peak.tokens.cost_usd), Some(TableColor::Green)),
        ]));
        println!("{peak_table}");
    } else if !render_chart && total_tokens == 0 {
        println!("{:<14} -", "Peak hour");
    }

    println!();
    if render_chart {
        print_today_hourly_chart(hourly);
    } else {
        print_today_hourly_table(hourly);
    }

    print_activity_breakdown_section("Projects", &breakdowns.projects);
    print_activity_breakdown_section("Languages", &breakdowns.languages);
    print_source_model_breakdown_section(&breakdowns.sources, &breakdowns.models);
    println!();
    println!("Date          {date}");
}

pub(super) fn print_activity_breakdown_section(title: &str, rows: &[ActivityBreakdownStat]) {
    if rows.is_empty() {
        return;
    }

    println!();
    let mut table = create_text_table();
    let show_tokens = rows.iter().any(|row| row.total_tokens > 0);
    let show_cost = rows.iter().any(|row| row.cost_usd > 0.0);
    if show_tokens && show_cost {
        table.set_header(vec![
            header_cell(title),
            header_cell("Coding"),
            header_cell("Tokens"),
            header_cell("Cost"),
            header_cell("Share"),
        ]);
        for row in rows {
            table.add_row(TableRow::from(vec![
                value_cell(
                    &truncate_display_text(&row.name, 28),
                    Some(TableColor::White),
                ),
                value_cell(&row.text, Some(TableColor::Green)),
                value_cell(&format_u64(row.total_tokens), Some(TableColor::Cyan)),
                value_cell(&format_usd(row.cost_usd), Some(TableColor::Green)),
                value_cell(
                    &format!(
                        "{} {:>5.1}%",
                        render_progress_meter(row.percent / 100.0, 10),
                        row.percent
                    ),
                    Some(TableColor::Yellow),
                ),
            ]));
        }
    } else if show_tokens {
        table.set_header(vec![
            header_cell(title),
            header_cell("Coding"),
            header_cell("Tokens"),
            header_cell("Share"),
        ]);
        for row in rows {
            table.add_row(TableRow::from(vec![
                value_cell(
                    &truncate_display_text(&row.name, 28),
                    Some(TableColor::White),
                ),
                value_cell(&row.text, Some(TableColor::Green)),
                value_cell(&format_u64(row.total_tokens), Some(TableColor::Cyan)),
                value_cell(
                    &format!(
                        "{} {:>5.1}%",
                        render_progress_meter(row.percent / 100.0, 10),
                        row.percent
                    ),
                    Some(TableColor::Yellow),
                ),
            ]));
        }
    } else {
        table.set_header(vec![
            header_cell(title),
            header_cell("Coding"),
            header_cell("Share"),
        ]);
        for row in rows {
            table.add_row(TableRow::from(vec![
                value_cell(
                    &truncate_display_text(&row.name, 28),
                    Some(TableColor::White),
                ),
                value_cell(&row.text, Some(TableColor::Green)),
                value_cell(
                    &format!(
                        "{} {:>5.1}%",
                        render_progress_meter(row.percent / 100.0, 10),
                        row.percent
                    ),
                    Some(TableColor::Yellow),
                ),
            ]));
        }
    }
    println!("{table}");
}

pub(super) fn should_render_today_hourly_chart(hourly: &[TodayHourlyRow]) -> bool {
    detect_terminal_width() >= 96
        && hourly
            .iter()
            .filter(|row| row.coding_seconds > 0 || row.tokens.total_tokens > 0)
            .count()
            >= 2
}

pub(super) fn print_today_hourly_chart(hourly: &[TodayHourlyRow]) {
    let max_tokens = hourly
        .iter()
        .map(|row| row.tokens.total_tokens)
        .max()
        .unwrap_or(0);
    if max_tokens == 0 {
        print_today_hourly_table(hourly);
        return;
    }

    let peak = hourly
        .iter()
        .max_by_key(|row| row.tokens.total_tokens)
        .expect("non-empty hourly chart points");
    let chart_width = detect_terminal_width().saturating_sub(2).clamp(88, 140) as u16;
    let chart_height = 13u16;
    let y_max = pretty_axis_upper_bound(max_tokens);
    let y_mid = y_max / 2;
    let points = hourly
        .iter()
        .map(|row| (f64::from(row.hour), row.tokens.total_tokens as f64))
        .collect::<Vec<_>>();
    let backend = TestBackend::new(chart_width, chart_height);
    let mut terminal = match Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(_) => {
            print_today_hourly_table(hourly);
            return;
        }
    };

    let x_labels = vec![
        Span::raw("00"),
        Span::raw("06"),
        Span::raw("12"),
        Span::raw("18"),
        Span::raw("23"),
    ];
    let y_labels = vec![
        Span::raw("0"),
        Span::raw(format_token_compact(y_mid)),
        Span::raw(format_token_compact(y_max)),
    ];
    let title = format!(
        "Hourly tokens · peak {} @ {}",
        format_token_compact(peak.tokens.total_tokens),
        peak.label
    );

    if terminal
        .draw(|frame| {
            let dataset = Dataset::default()
                .marker(symbols::Marker::HalfBlock)
                .style(Style::default().fg(TuiColor::Cyan))
                .graph_type(GraphType::Bar)
                .data(&points);
            let chart = Chart::new(vec![dataset])
                .block(Block::bordered().title(title.as_str()))
                .x_axis(
                    Axis::default()
                        .bounds([0.0, 23.0])
                        .labels(x_labels.clone())
                        .style(Style::default().fg(TuiColor::DarkGray)),
                )
                .y_axis(
                    Axis::default()
                        .bounds([0.0, y_max as f64])
                        .labels(y_labels.clone())
                        .style(Style::default().fg(TuiColor::DarkGray)),
                );
            frame.render_widget(chart, frame.area());
        })
        .is_err()
    {
        print_today_hourly_table(hourly);
        return;
    }

    for line in ratatui_buffer_lines(terminal.backend().buffer()) {
        println!("{line}");
    }
}

pub(super) fn print_today_hourly_table(hourly: &[TodayHourlyRow]) {
    let mut hourly_table = create_text_table();
    hourly_table.set_header(vec![
        header_cell("Window"),
        header_cell("Coding"),
        header_cell("Tokens"),
        header_cell("Cost"),
    ]);
    let mut visible_rows = 0usize;
    for row in hourly {
        if row.coding_seconds == 0 && row.tokens.total_tokens == 0 {
            continue;
        }
        visible_rows += 1;
        hourly_table.add_row(TableRow::from(vec![
            value_cell(&row.label, Some(TableColor::White)),
            value_cell(&row.coding_text, Some(TableColor::Green)),
            value_cell(&format_u64(row.tokens.total_tokens), Some(TableColor::Cyan)),
            value_cell(&format_usd(row.tokens.cost_usd), Some(TableColor::Green)),
        ]));
    }
    if visible_rows == 0 {
        hourly_table.add_row(TableRow::from(vec![
            value_cell("No active hours", Some(TableColor::DarkGrey)),
            value_cell("-", Some(TableColor::DarkGrey)),
            value_cell("0", Some(TableColor::DarkGrey)),
            value_cell("$0.00", Some(TableColor::DarkGrey)),
        ]));
    }
    println!("{hourly_table}");
}

pub(super) fn print_token_breakdown_section(title: &str, rows: &[TokenBreakdownStat]) {
    if rows.is_empty() {
        return;
    }

    println!();
    let mut table = create_text_table();
    table.set_header(vec![
        header_cell(title),
        header_cell("Tokens"),
        header_cell("Cost"),
        header_cell("Share"),
    ]);
    for row in rows {
        table.add_row(TableRow::from(vec![
            value_cell(
                &truncate_display_text(&row.name, 32),
                Some(TableColor::White),
            ),
            value_cell(&format_u64(row.total_tokens), Some(TableColor::Cyan)),
            value_cell(&format_usd(row.cost_usd), Some(TableColor::Green)),
            value_cell(
                &format!(
                    "{} {:>5.1}%",
                    render_progress_meter(row.percent / 100.0, 10),
                    row.percent
                ),
                Some(TableColor::Yellow),
            ),
        ]));
    }
    println!("{table}");
}

pub(super) fn print_source_model_breakdown_section(
    sources: &[ActivityBreakdownStat],
    models: &[TokenBreakdownStat],
) {
    if let (Some(source), Some(model)) = (sources.first(), models.first())
        && sources.len() == 1
        && models.len() == 1
    {
        println!();
        let mut table = create_text_table();
        table.set_header(vec![
            header_cell("Source"),
            header_cell("Model"),
            header_cell("Coding"),
            header_cell("Tokens"),
            header_cell("Cost"),
        ]);
        table.add_row(TableRow::from(vec![
            value_cell(
                &truncate_display_text(&source.name, 18),
                Some(TableColor::White),
            ),
            value_cell(
                &truncate_display_text(&model.name, 28),
                Some(TableColor::White),
            ),
            value_cell(&source.text, Some(TableColor::Green)),
            value_cell(&format_u64(model.total_tokens), Some(TableColor::Cyan)),
            value_cell(&format_usd(model.cost_usd), Some(TableColor::Green)),
        ]));
        println!("{table}");
        return;
    }

    print_activity_breakdown_section("Sources", sources);
    print_token_breakdown_section("Models", models);
}
