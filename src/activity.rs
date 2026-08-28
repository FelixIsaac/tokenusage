use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Local, NaiveDate, TimeDelta, Timelike, Utc};
use serde::Serialize;

use crate::cli::CommonArgs;
use crate::heartbeat::{HeartbeatLoadOptions, HeartbeatRecord, load_heartbeat_records};
use crate::pipeline::TimeZoneMode;
use crate::types::{ActivitySummary, SourceKind, UsageEvent};

const MAX_ACTIVE_GAP_SECS: i64 = 20 * 60;
const ISOLATED_TAIL_SECS: i64 = 3 * 60;
const HEARTBEAT_OVERRIDE_THRESHOLD: f64 = 0.60;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ActivityBreakdownStat {
    pub(crate) name: String,
    pub(crate) total_seconds: u64,
    pub(crate) text: String,
    pub(crate) total_tokens: u64,
    pub(crate) cost_usd: f64,
    pub(crate) percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ActivityHourlyBucket {
    pub(crate) hour: u8,
    pub(crate) total_seconds: u64,
    pub(crate) text: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ActivityDataset {
    days: BTreeMap<NaiveDate, ActivityDayDetail>,
}

#[derive(Debug, Clone, Default)]
struct ActivityDayDetail {
    total_seconds: u64,
    hourly_seconds: [u64; 24],
    projects: HashMap<String, u64>,
    project_hourly: HashMap<String, [u64; 24]>,
    languages: HashMap<String, u64>,
    sources: HashMap<String, u64>,
}

pub(crate) fn activity_enabled(common: &CommonArgs) -> bool {
    common.with_activity
}

pub(crate) async fn fetch_activity_dataset(
    common: &CommonArgs,
    tz: &TimeZoneMode,
    events: &[UsageEvent],
    project_filter: Option<&str>,
) -> Result<Option<ActivityDataset>> {
    if !activity_enabled(common) {
        return Ok(None);
    }

    let token_dataset = if events.is_empty() {
        ActivityDataset::default()
    } else {
        infer_activity_dataset(tz, events)
    };

    let heartbeat_dataset = match requested_activity_range(common, tz, events)? {
        Some((start, end)) => {
            let records = load_heartbeat_records(HeartbeatLoadOptions {
                start,
                end,
                tz,
                project_filter,
            })
            .await?;
            if records.is_empty() {
                ActivityDataset::default()
            } else {
                dataset_from_heartbeats(tz, &records)
            }
        }
        None => ActivityDataset::default(),
    };

    let merged = merge_activity_datasets(token_dataset, heartbeat_dataset);
    if merged.is_empty() {
        Ok(None)
    } else {
        Ok(Some(merged))
    }
}

pub(crate) fn format_activity_duration(total_seconds: u64) -> String {
    if total_seconds == 0 {
        return "0s".to_string();
    }

    let days = total_seconds / 86_400;
    let hours = (total_seconds % 86_400) / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if days > 0 {
        if hours > 0 {
            format!("{days}d {hours}h")
        } else {
            format!("{days}d")
        }
    } else if hours > 0 {
        if minutes > 0 {
            format!("{hours}h {minutes}m")
        } else {
            format!("{hours}h")
        }
    } else if minutes > 0 {
        if seconds > 0 && minutes < 10 {
            format!("{minutes}m {seconds}s")
        } else {
            format!("{minutes}m")
        }
    } else {
        format!("{seconds}s")
    }
}

impl ActivityDataset {
    pub(crate) fn is_empty(&self) -> bool {
        self.days.is_empty()
    }

    pub(crate) fn active_days_in_range(&self, start: NaiveDate, end: NaiveDate) -> u32 {
        self.days
            .range(start..=end)
            .filter(|(_, detail)| detail.total_seconds > 0)
            .count() as u32
    }

    pub(crate) fn summary_for_day(&self, day: NaiveDate) -> Option<ActivitySummary> {
        let detail = self.days.get(&day)?;
        Some(summary_from_detail(detail))
    }

    pub(crate) fn summary_for_range(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Option<ActivitySummary> {
        let total_seconds = self
            .days
            .range(start..=end)
            .map(|(_, detail)| detail.total_seconds)
            .sum::<u64>();
        if total_seconds == 0 {
            return None;
        }

        let projects = aggregate_range_map(self.days.range(start..=end).map(|(_, d)| &d.projects));
        let languages =
            aggregate_range_map(self.days.range(start..=end).map(|(_, d)| &d.languages));
        let sources = aggregate_range_map(self.days.range(start..=end).map(|(_, d)| &d.sources));

        Some(ActivitySummary {
            total_seconds,
            text: format_activity_duration(total_seconds),
            top_project: top_name(&projects),
            top_language: top_name(&languages),
            top_source: top_name(&sources),
        })
    }

    pub(crate) fn project_summary_for_day(
        &self,
        day: NaiveDate,
        project_name: &str,
    ) -> Option<ActivitySummary> {
        let detail = self.days.get(&day)?;
        let total_seconds = detail.projects.get(project_name).copied().unwrap_or(0);
        summary_for_project(project_name, total_seconds)
    }

    pub(crate) fn project_breakdowns(
        &self,
        start: NaiveDate,
        end: NaiveDate,
        limit: usize,
    ) -> Vec<ActivityBreakdownStat> {
        let totals = aggregate_range_map(self.days.range(start..=end).map(|(_, d)| &d.projects));
        breakdown_rows(&totals, limit)
    }

    pub(crate) fn language_breakdowns(
        &self,
        start: NaiveDate,
        end: NaiveDate,
        limit: usize,
    ) -> Vec<ActivityBreakdownStat> {
        let totals = aggregate_range_map(self.days.range(start..=end).map(|(_, d)| &d.languages));
        breakdown_rows(&totals, limit)
    }

    pub(crate) fn source_breakdowns(
        &self,
        start: NaiveDate,
        end: NaiveDate,
        limit: usize,
    ) -> Vec<ActivityBreakdownStat> {
        let totals = aggregate_range_map(self.days.range(start..=end).map(|(_, d)| &d.sources));
        breakdown_rows(&totals, limit)
    }

    pub(crate) fn hourly_buckets_for_day(
        &self,
        day: NaiveDate,
        project_name: Option<&str>,
    ) -> Vec<ActivityHourlyBucket> {
        let mut hourly = [0u64; 24];
        if let Some(detail) = self.days.get(&day) {
            if let Some(project_name) = project_name {
                if let Some(project_hourly) = detail.project_hourly.get(project_name) {
                    hourly = *project_hourly;
                }
            } else {
                hourly = detail.hourly_seconds;
            }
        }

        (0u8..24)
            .map(|hour| ActivityHourlyBucket {
                hour,
                total_seconds: hourly[usize::from(hour)],
                text: format_activity_duration(hourly[usize::from(hour)]),
            })
            .collect()
    }
}

fn infer_activity_dataset(tz: &TimeZoneMode, events: &[UsageEvent]) -> ActivityDataset {
    let mut ordered = events.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|event| event.timestamp);

    let mut days = BTreeMap::<NaiveDate, ActivityDayDetail>::new();

    for (index, event) in ordered.iter().enumerate() {
        let start = event.timestamp;
        let end = inferred_interval_end(start, ordered.get(index + 1).map(|next| next.timestamp));
        if end <= start {
            continue;
        }

        let project = infer_project_label(event);
        let language = infer_language(&event.file_path);
        let source = display_source_label(event.source);
        accumulate_interval(
            &mut days,
            tz,
            start,
            end,
            &project,
            language.as_deref(),
            Some(source),
        );
    }

    ActivityDataset { days }
}

pub(crate) fn dataset_from_heartbeats(
    tz: &TimeZoneMode,
    records: &[HeartbeatRecord],
) -> ActivityDataset {
    let mut ordered = records.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|record| record.timestamp);

    let mut days = BTreeMap::<NaiveDate, ActivityDayDetail>::new();
    for (index, record) in ordered.iter().enumerate() {
        let start = record.timestamp;
        let end = inferred_heartbeat_interval_end(start, record, ordered.get(index + 1).copied());
        if end <= start {
            continue;
        }
        let project = heartbeat_project_label(record);
        accumulate_interval(
            &mut days,
            tz,
            start,
            end,
            &project,
            record.language.as_deref(),
            None,
        );
    }

    ActivityDataset { days }
}

fn inferred_interval_end(start: DateTime<Utc>, next: Option<DateTime<Utc>>) -> DateTime<Utc> {
    match next {
        Some(next) => {
            let gap = (next - start).num_seconds();
            if gap > 0 && gap <= MAX_ACTIVE_GAP_SECS {
                next
            } else if gap > 0 {
                start + TimeDelta::seconds(ISOLATED_TAIL_SECS)
            } else {
                start
            }
        }
        None => start + TimeDelta::seconds(ISOLATED_TAIL_SECS),
    }
}

fn inferred_heartbeat_interval_end(
    start: DateTime<Utc>,
    record: &HeartbeatRecord,
    next: Option<&HeartbeatRecord>,
) -> DateTime<Utc> {
    let timeout = i64::from(record.timeout_seconds.max(1));
    let pulse = i64::from(record.pulse_seconds.max(1));
    match next {
        Some(next) => {
            let gap = (next.timestamp - start).num_seconds();
            if gap > 0 && gap <= timeout {
                next.timestamp
            } else if gap > 0 {
                start + TimeDelta::seconds(pulse)
            } else {
                start
            }
        }
        None => start + TimeDelta::seconds(pulse),
    }
}

fn accumulate_interval(
    days: &mut BTreeMap<NaiveDate, ActivityDayDetail>,
    tz: &TimeZoneMode,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    project: &str,
    language: Option<&str>,
    source: Option<&str>,
) {
    let mut cursor = start;
    while cursor < end {
        let day = tz.date_of(cursor);
        let hour = tz.hour_of(cursor) as usize;
        let mut boundary = next_hour_boundary_utc(cursor, tz);
        if boundary <= cursor {
            boundary = cursor + TimeDelta::hours(1);
        }
        let segment_end = end.min(boundary);
        let seconds = (segment_end - cursor).num_seconds().max(0) as u64;
        if seconds == 0 {
            break;
        }

        let detail = days.entry(day).or_default();
        detail.total_seconds = detail.total_seconds.saturating_add(seconds);
        detail.hourly_seconds[hour] = detail.hourly_seconds[hour].saturating_add(seconds);
        let project_total = detail.projects.entry(project.to_string()).or_default();
        *project_total = project_total.saturating_add(seconds);
        let project_hourly = detail
            .project_hourly
            .entry(project.to_string())
            .or_insert([0u64; 24]);
        project_hourly[hour] = project_hourly[hour].saturating_add(seconds);
        if let Some(language) = language {
            let language_total = detail.languages.entry(language.to_string()).or_default();
            *language_total = language_total.saturating_add(seconds);
        }
        if let Some(source) = source {
            let source_total = detail.sources.entry(source.to_string()).or_default();
            *source_total = source_total.saturating_add(seconds);
        }

        cursor = segment_end;
    }
}

fn next_hour_boundary_utc(ts: DateTime<Utc>, tz: &TimeZoneMode) -> DateTime<Utc> {
    match tz {
        TimeZoneMode::Local => {
            let local = ts.with_timezone(&Local);
            let truncated = local
                .with_minute(0)
                .and_then(|value| value.with_second(0))
                .and_then(|value| value.with_nanosecond(0))
                .unwrap_or(local);
            (truncated + TimeDelta::hours(1)).with_timezone(&Utc)
        }
        TimeZoneMode::Utc => {
            let truncated = ts
                .with_minute(0)
                .and_then(|value| value.with_second(0))
                .and_then(|value| value.with_nanosecond(0))
                .unwrap_or(ts);
            truncated + TimeDelta::hours(1)
        }
        TimeZoneMode::Named(tz) => {
            let local = ts.with_timezone(tz);
            let truncated = local
                .with_minute(0)
                .and_then(|value| value.with_second(0))
                .and_then(|value| value.with_nanosecond(0))
                .unwrap_or(local);
            (truncated + TimeDelta::hours(1)).with_timezone(&Utc)
        }
    }
}

fn infer_project_label(event: &UsageEvent) -> String {
    if let Some(project) = event.project.as_deref().map(str::trim)
        && !project.is_empty()
    {
        return project.to_string();
    }

    let path = Path::new(&event.file_path);
    if let Some(parent) = path.parent().and_then(|value| value.file_name()) {
        let name = parent.to_string_lossy().trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }

    format!(
        "{} {}",
        display_source_label(event.source),
        short_session_tail(&event.session)
    )
}

fn short_session_tail(session: &str) -> String {
    session
        .chars()
        .rev()
        .take(8)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>()
}

fn infer_language(_file_path: &str) -> Option<String> {
    // Usage logs do not reliably carry the active code file, so inferring a
    // language from the transcript path would be misleading. Keep this empty
    // until we have a source-backed language signal.
    None
}

fn heartbeat_project_label(record: &HeartbeatRecord) -> String {
    record
        .project
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            Path::new(&record.entity)
                .parent()
                .and_then(|value| value.file_name())
                .map(|value| value.to_string_lossy().trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "heartbeat".to_string())
        })
}

fn display_source_label(source: SourceKind) -> &'static str {
    match source {
        SourceKind::Claude => "Claude",
        SourceKind::Codex => "Codex",
        SourceKind::Gemini => "Antigravity",
        SourceKind::OpenCode => "OpenCode",
        SourceKind::Grok => "Grok",
    }
}

fn summary_from_detail(detail: &ActivityDayDetail) -> ActivitySummary {
    ActivitySummary {
        total_seconds: detail.total_seconds,
        text: format_activity_duration(detail.total_seconds),
        top_project: top_name(&detail.projects),
        top_language: top_name(&detail.languages),
        top_source: top_name(&detail.sources),
    }
}

fn summary_for_project(project_name: &str, total_seconds: u64) -> Option<ActivitySummary> {
    if total_seconds == 0 {
        return None;
    }

    Some(ActivitySummary {
        total_seconds,
        text: format_activity_duration(total_seconds),
        top_project: Some(project_name.to_string()),
        top_language: None,
        top_source: None,
    })
}

fn aggregate_range_map<'a>(
    maps: impl Iterator<Item = &'a HashMap<String, u64>>,
) -> HashMap<String, u64> {
    let mut totals = HashMap::<String, u64>::new();
    for map in maps {
        for (name, total_seconds) in map {
            totals
                .entry(name.clone())
                .and_modify(|value| *value = value.saturating_add(*total_seconds))
                .or_insert(*total_seconds);
        }
    }
    totals
}

fn top_name(map: &HashMap<String, u64>) -> Option<String> {
    map.iter()
        .max_by(|(name_a, total_a), (name_b, total_b)| {
            total_a.cmp(total_b).then_with(|| name_b.cmp(name_a))
        })
        .map(|(name, _)| name.clone())
}

fn breakdown_rows(totals: &HashMap<String, u64>, limit: usize) -> Vec<ActivityBreakdownStat> {
    if totals.is_empty() || limit == 0 {
        return Vec::new();
    }

    let grand_total = totals.values().copied().sum::<u64>();
    let mut rows = totals.iter().collect::<Vec<_>>();
    rows.sort_by(|(name_a, total_a), (name_b, total_b)| {
        total_b.cmp(total_a).then_with(|| name_a.cmp(name_b))
    });
    rows.truncate(limit);

    rows.into_iter()
        .map(|(name, total_seconds)| ActivityBreakdownStat {
            name: name.clone(),
            total_seconds: *total_seconds,
            text: format_activity_duration(*total_seconds),
            total_tokens: 0,
            cost_usd: 0.0,
            percent: if grand_total == 0 {
                0.0
            } else {
                (*total_seconds as f64 / grand_total as f64) * 100.0
            },
        })
        .collect()
}

fn requested_activity_range(
    common: &CommonArgs,
    tz: &TimeZoneMode,
    events: &[UsageEvent],
) -> Result<Option<(NaiveDate, NaiveDate)>> {
    let explicit_since = parse_activity_date(common.since.as_deref())?;
    let explicit_until = parse_activity_date(common.until.as_deref())?;
    let inferred = event_date_bounds(events, tz);

    let since = explicit_since.or(inferred.map(|(start, _)| start));
    let until = explicit_until.or(inferred.map(|(_, end)| end));

    match (since, until) {
        (Some(since), Some(until)) => Ok(Some((since.min(until), since.max(until)))),
        (Some(since), None) => Ok(Some((since, since))),
        (None, Some(until)) => Ok(Some((until, until))),
        (None, None) => Ok(None),
    }
}

fn parse_activity_date(input: Option<&str>) -> Result<Option<NaiveDate>> {
    crate::pipeline::parse_date_filter(input)
}

fn event_date_bounds(events: &[UsageEvent], tz: &TimeZoneMode) -> Option<(NaiveDate, NaiveDate)> {
    let mut min_day: Option<NaiveDate> = None;
    let mut max_day: Option<NaiveDate> = None;
    for event in events {
        let day = tz.date_of(event.timestamp);
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

fn merge_activity_datasets(
    token_dataset: ActivityDataset,
    heartbeat_dataset: ActivityDataset,
) -> ActivityDataset {
    if heartbeat_dataset.is_empty() {
        return token_dataset;
    }
    if token_dataset.is_empty() {
        return heartbeat_dataset;
    }

    let mut merged_days = token_dataset.days.clone();
    for (day, heartbeat_detail) in heartbeat_dataset.days {
        match merged_days.get_mut(&day) {
            Some(token_detail) => {
                *token_detail = merge_day_detail(token_detail, heartbeat_detail);
            }
            None => {
                merged_days.insert(day, heartbeat_detail);
            }
        }
    }

    ActivityDataset { days: merged_days }
}

fn merge_day_detail(
    token_detail: &ActivityDayDetail,
    heartbeat_detail: ActivityDayDetail,
) -> ActivityDayDetail {
    if heartbeat_detail.total_seconds == 0 {
        return token_detail.clone();
    }
    if token_detail.total_seconds == 0 {
        return heartbeat_detail;
    }

    let heartbeat_ratio = heartbeat_detail.total_seconds as f64 / token_detail.total_seconds as f64;
    if heartbeat_ratio >= HEARTBEAT_OVERRIDE_THRESHOLD {
        let mut merged = heartbeat_detail;
        if !token_detail.sources.is_empty() {
            merged.sources = scale_map_to_total(&token_detail.sources, merged.total_seconds);
        }
        return merged;
    }

    let mut merged = token_detail.clone();
    if merged.languages.is_empty() && !heartbeat_detail.languages.is_empty() {
        merged.languages = scale_map_to_total(&heartbeat_detail.languages, merged.total_seconds);
    }
    if merged.projects.is_empty() && !heartbeat_detail.projects.is_empty() {
        merged.projects = scale_map_to_total(&heartbeat_detail.projects, merged.total_seconds);
        merged.project_hourly = heartbeat_detail.project_hourly;
    }
    merged
}

fn scale_map_to_total(values: &HashMap<String, u64>, target_total: u64) -> HashMap<String, u64> {
    let source_total = values.values().copied().sum::<u64>();
    if source_total == 0 || target_total == 0 {
        return HashMap::new();
    }

    let mut entries = values
        .iter()
        .map(|(name, value)| {
            let scaled = ((*value as f64 / source_total as f64) * target_total as f64).round();
            (name.clone(), scaled.max(0.0) as u64)
        })
        .collect::<Vec<_>>();

    let current_total = entries.iter().map(|(_, value)| *value).sum::<u64>();
    if let Some((_, value)) = entries.first_mut() {
        if current_total < target_total {
            *value = value.saturating_add(target_total - current_total);
        } else if current_total > target_total {
            *value = value.saturating_sub(current_total - target_total);
        }
    }

    entries.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, TimeZone, Utc};

    use super::{
        ActivityDataset, format_activity_duration, infer_activity_dataset, merge_activity_datasets,
    };
    use crate::heartbeat::{HeartbeatEntityKind, HeartbeatRecord};
    use crate::pipeline::TimeZoneMode;
    use crate::types::{SourceKind, UsageAccumulator, UsageEvent};

    #[test]
    fn active_days_in_range_counts_only_nonzero_days() {
        let mut dataset = ActivityDataset::default();
        dataset.days.insert(
            chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
            super::ActivityDayDetail {
                total_seconds: 0,
                ..super::ActivityDayDetail::default()
            },
        );
        dataset.days.insert(
            chrono::NaiveDate::from_ymd_opt(2026, 5, 2).unwrap(),
            super::ActivityDayDetail {
                total_seconds: 60,
                ..super::ActivityDayDetail::default()
            },
        );
        dataset.days.insert(
            chrono::NaiveDate::from_ymd_opt(2026, 5, 3).unwrap(),
            super::ActivityDayDetail {
                total_seconds: 120,
                ..super::ActivityDayDetail::default()
            },
        );

        assert_eq!(
            dataset.active_days_in_range(
                chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
                chrono::NaiveDate::from_ymd_opt(2026, 5, 3).unwrap()
            ),
            2
        );
    }

    #[test]
    fn formats_activity_duration_compactly() {
        assert_eq!(format_activity_duration(0), "0s");
        assert_eq!(format_activity_duration(59), "59s");
        assert_eq!(format_activity_duration(65), "1m 5s");
        assert_eq!(format_activity_duration(3_720), "1h 2m");
        assert_eq!(format_activity_duration(90_000), "1d 1h");
    }

    #[test]
    fn infers_activity_without_overlapping_projects() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let dataset = dataset_from_events(vec![
            event(day, 10, 0, SourceKind::Codex, "alpha", "main.rs"),
            event(day, 10, 10, SourceKind::Codex, "alpha", "main.rs"),
            event(day, 10, 50, SourceKind::Claude, "beta", "app.py"),
        ]);

        let summary = dataset.summary_for_day(day).unwrap();
        assert_eq!(summary.total_seconds, 960);
        assert_eq!(summary.top_project.as_deref(), Some("alpha"));
        assert_eq!(summary.top_language.as_deref(), None);
        assert_eq!(summary.top_source.as_deref(), Some("Codex"));

        let projects = dataset.project_breakdowns(day, day, 5);
        assert_eq!(projects[0].name, "alpha");
        assert_eq!(projects[0].total_seconds, 780);
        assert_eq!(projects[1].name, "beta");
        assert_eq!(projects[1].total_seconds, 180);
    }

    #[test]
    fn splits_activity_across_hour_boundaries() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let dataset = dataset_from_events(vec![
            event(day, 10, 55, SourceKind::Codex, "alpha", "main.rs"),
            event(day, 11, 5, SourceKind::Codex, "alpha", "main.rs"),
        ]);

        let hourly = dataset.hourly_buckets_for_day(day, None);
        assert_eq!(hourly[10].total_seconds, 300);
        assert_eq!(hourly[11].total_seconds, 480);
    }

    #[test]
    fn heartbeat_days_override_token_inference() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let token_dataset = dataset_from_events(vec![
            event(day, 10, 0, SourceKind::Codex, "alpha", "main.rs"),
            event(day, 10, 10, SourceKind::Codex, "alpha", "main.rs"),
        ]);
        let heartbeat_dataset = super::dataset_from_heartbeats(
            &TimeZoneMode::Utc,
            &[
                HeartbeatRecord {
                    timestamp: Utc
                        .with_ymd_and_hms(day.year(), day.month(), day.day(), 10, 0, 0)
                        .single()
                        .unwrap(),
                    entity: "/tmp/main.rs".to_string(),
                    kind: HeartbeatEntityKind::File,
                    project: Some("alpha".to_string()),
                    language: Some("Rust".to_string()),
                    branch: None,
                    origin: "manual".to_string(),
                    is_write: true,
                    pulse_seconds: 120,
                    timeout_seconds: 900,
                },
                HeartbeatRecord {
                    timestamp: Utc
                        .with_ymd_and_hms(day.year(), day.month(), day.day(), 10, 8, 0)
                        .single()
                        .unwrap(),
                    entity: "/tmp/main.rs".to_string(),
                    kind: HeartbeatEntityKind::File,
                    project: Some("alpha".to_string()),
                    language: Some("Rust".to_string()),
                    branch: None,
                    origin: "manual".to_string(),
                    is_write: true,
                    pulse_seconds: 120,
                    timeout_seconds: 900,
                },
            ],
        );

        let merged = merge_activity_datasets(token_dataset, heartbeat_dataset);
        let summary = merged.summary_for_day(day).unwrap();
        assert_eq!(summary.total_seconds, 600);
        assert_eq!(summary.top_project.as_deref(), Some("alpha"));
        assert_eq!(summary.top_language.as_deref(), Some("Rust"));
        assert_eq!(summary.top_source.as_deref(), Some("Codex"));
    }

    #[test]
    fn sparse_heartbeat_does_not_underestimate_busy_day() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let token_dataset = dataset_from_events(vec![
            event(day, 10, 0, SourceKind::Codex, "alpha", "main.rs"),
            event(day, 10, 10, SourceKind::Codex, "alpha", "main.rs"),
        ]);
        let heartbeat_dataset = super::dataset_from_heartbeats(
            &TimeZoneMode::Utc,
            &[HeartbeatRecord {
                timestamp: Utc
                    .with_ymd_and_hms(day.year(), day.month(), day.day(), 10, 0, 0)
                    .single()
                    .unwrap(),
                entity: "/tmp/main.rs".to_string(),
                kind: HeartbeatEntityKind::File,
                project: Some("alpha".to_string()),
                language: Some("Rust".to_string()),
                branch: None,
                origin: "manual".to_string(),
                is_write: true,
                pulse_seconds: 120,
                timeout_seconds: 900,
            }],
        );

        let merged = merge_activity_datasets(token_dataset, heartbeat_dataset);
        let summary = merged.summary_for_day(day).unwrap();
        assert_eq!(summary.total_seconds, 780);
        assert_eq!(summary.top_project.as_deref(), Some("alpha"));
        assert_eq!(summary.top_language.as_deref(), Some("Rust"));
    }

    fn dataset_from_events(events: Vec<UsageEvent>) -> ActivityDataset {
        infer_activity_dataset(&TimeZoneMode::Utc, &events)
    }

    fn event(
        day: chrono::NaiveDate,
        hour: u32,
        minute: u32,
        source: SourceKind,
        project: &str,
        file_path: &str,
    ) -> UsageEvent {
        UsageEvent {
            timestamp: Utc
                .with_ymd_and_hms(day.year(), day.month(), day.day(), hour, minute, 0)
                .single()
                .unwrap(),
            source,
            model: "gpt-5.3-codex".to_string(),
            session: format!("session-{project}"),
            project: Some(project.to_string()),
            file_path: file_path.to_string(),
            usage: UsageAccumulator::default(),
        }
    }
}
