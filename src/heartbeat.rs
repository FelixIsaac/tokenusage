use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, NaiveDate, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

#[cfg(feature = "cli")]
use comfy_table::{
    Attribute, Cell as TableCell, Color as TableColor, ContentArrangement, Row as TableRow,
    Table as TextTable, modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL,
};
#[cfg(feature = "cli")]
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
#[cfg(feature = "cli")]
use std::io::IsTerminal;
#[cfg(feature = "cli")]
use std::process::{Command, Stdio};

#[cfg(feature = "cli")]
use crate::activity::{dataset_from_heartbeats, format_activity_duration};
#[cfg(feature = "cli")]
use crate::cli::{
    HeartbeatArgs, HeartbeatCommand, HeartbeatPingArgs, HeartbeatStatsArgs, HeartbeatWatchArgs,
};
use crate::pipeline::TimeZoneMode;

pub(crate) const DEFAULT_HEARTBEAT_PULSE_SECS: u16 = 120;
pub(crate) const DEFAULT_HEARTBEAT_TIMEOUT_SECS: u16 = 900;

const HEARTBEAT_FILE_PADDING_DAYS: i64 = 1;
const WATCH_IGNORE_COMPONENTS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    ".venv",
    "venv",
];

static HEARTBEAT_FILE_CACHE: LazyLock<Mutex<HashMap<PathBuf, CachedHeartbeatFile>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
struct CachedHeartbeatFile {
    modified: Option<SystemTime>,
    records: Vec<HeartbeatRecord>,
}

#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum HeartbeatEntityKind {
    #[default]
    File,
    App,
    Domain,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HeartbeatRecord {
    pub(crate) timestamp: DateTime<Utc>,
    pub(crate) entity: String,
    pub(crate) kind: HeartbeatEntityKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) branch: Option<String>,
    pub(crate) origin: String,
    pub(crate) is_write: bool,
    pub(crate) pulse_seconds: u16,
    pub(crate) timeout_seconds: u16,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HeartbeatLoadOptions<'a> {
    pub(crate) start: NaiveDate,
    pub(crate) end: NaiveDate,
    pub(crate) tz: &'a TimeZoneMode,
    pub(crate) project_filter: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
struct HeartbeatDailyRow {
    date: String,
    heartbeats: usize,
    coding_seconds: u64,
    coding: String,
    top_project: Option<String>,
    top_language: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct HeartbeatBreakdownRow {
    name: String,
    count: usize,
    percent: f64,
}

#[derive(Debug, Clone, Serialize)]
struct HeartbeatStatsOut {
    start: String,
    end: String,
    heartbeats: usize,
    coding_seconds: u64,
    coding: String,
    avg_per_day_seconds: u64,
    avg_per_day: String,
    top_project: Option<String>,
    top_language: Option<String>,
    top_origin: Option<String>,
    daily: Vec<HeartbeatDailyRow>,
    origins: Vec<HeartbeatBreakdownRow>,
    stats_path: String,
}

#[cfg(feature = "cli")]
pub(crate) async fn run(args: HeartbeatArgs) -> Result<()> {
    match args.command {
        HeartbeatCommand::Ping(args) => run_ping(args).await,
        HeartbeatCommand::Watch(args) => run_watch(args).await,
        HeartbeatCommand::Stats(args) => run_stats(args).await,
    }
}

pub(crate) async fn append_heartbeat(record: HeartbeatRecord) -> Result<PathBuf> {
    let path = heartbeat_file_path_for_utc_day(record.timestamp.date_naive())?;
    let serialized = serde_json::to_string(&record)?;
    let path_for_cache = path.clone();
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create heartbeat dir: {}", parent.display()))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("Failed to open heartbeat file: {}", path.display()))?;
        file.write_all(serialized.as_bytes())
            .context("Failed to write heartbeat JSON line")?;
        file.write_all(b"\n")
            .context("Failed to terminate heartbeat JSON line")?;
        invalidate_heartbeat_cache(&path_for_cache);
        Ok(path)
    })
    .await?
}

pub(crate) async fn load_heartbeat_records(
    options: HeartbeatLoadOptions<'_>,
) -> Result<Vec<HeartbeatRecord>> {
    let start = options
        .start
        .checked_sub_signed(TimeDelta::days(HEARTBEAT_FILE_PADDING_DAYS))
        .unwrap_or(options.start);
    let end = options
        .end
        .checked_add_signed(TimeDelta::days(HEARTBEAT_FILE_PADDING_DAYS))
        .unwrap_or(options.end);
    let tz = options.tz.clone();
    let project_filter = options.project_filter.map(ToOwned::to_owned);
    let requested_start = options.start;
    let requested_end = options.end;

    tokio::task::spawn_blocking(move || -> Result<Vec<HeartbeatRecord>> {
        let mut records = Vec::new();
        let mut cursor = start;
        while cursor <= end {
            let path = heartbeat_file_path_for_utc_day(cursor)?;
            if path.is_file() {
                let parsed = read_heartbeat_file_cached(&path).with_context(|| {
                    format!("Failed to read heartbeat file: {}", path.display())
                })?;
                records.extend(parsed.into_iter().filter(|record| {
                    let day = heartbeat_local_date(record.timestamp, &tz);
                    if day < requested_start || day > requested_end {
                        return false;
                    }
                    match project_filter.as_deref() {
                        Some(project_filter) => record
                            .project
                            .as_deref()
                            .is_some_and(|project| project.contains(project_filter)),
                        None => true,
                    }
                }));
            }
            cursor = cursor
                .checked_add_signed(TimeDelta::days(1))
                .unwrap_or(end.succ_opt().unwrap_or(end));
        }
        records.sort_by_key(|record| record.timestamp);
        Ok(records)
    })
    .await?
}

fn invalidate_heartbeat_cache(path: &Path) {
    if let Ok(mut cache) = HEARTBEAT_FILE_CACHE.lock() {
        cache.remove(path);
    }
}

fn read_heartbeat_file_cached(path: &Path) -> Result<Vec<HeartbeatRecord>> {
    let modified = fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok());

    if let Ok(cache) = HEARTBEAT_FILE_CACHE.lock()
        && let Some(entry) = cache.get(path)
        && entry.modified == modified
    {
        return Ok(entry.records.clone());
    }

    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<HeartbeatRecord>(trimmed) {
            records.push(record);
        }
    }

    if let Ok(mut cache) = HEARTBEAT_FILE_CACHE.lock() {
        cache.insert(
            path.to_path_buf(),
            CachedHeartbeatFile {
                modified,
                records: records.clone(),
            },
        );
    }

    Ok(records)
}

// ---------------------------------------------------------------------------
// Shared utilities (used by library path)
// ---------------------------------------------------------------------------

fn heartbeat_file_path_for_utc_day(day: NaiveDate) -> Result<PathBuf> {
    Ok(heartbeat_dir()?.join(format!("{day}.jsonl")))
}

fn heartbeat_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("TU_HEARTBEAT_DIR") {
        return Ok(PathBuf::from(path));
    }
    if let Some(base) = dirs::data_local_dir() {
        return Ok(base.join("tokenusage").join("heartbeats"));
    }
    if let Some(home) = dirs::home_dir() {
        return Ok(home.join(".tokenusage").join("heartbeats"));
    }
    bail!("Failed to resolve heartbeat data directory");
}

fn heartbeat_local_date(ts: DateTime<Utc>, tz: &TimeZoneMode) -> NaiveDate {
    match tz {
        TimeZoneMode::Local => ts.with_timezone(&Local).date_naive(),
        TimeZoneMode::Utc => ts.date_naive(),
        TimeZoneMode::Named(tz) => ts.with_timezone(tz).date_naive(),
    }
}

// ---------------------------------------------------------------------------
// CLI-only: everything below is gated behind the `cli` feature.
// ---------------------------------------------------------------------------

cfg_if::cfg_if! {
if #[cfg(feature = "cli")] {

async fn run_ping(args: HeartbeatPingArgs) -> Result<()> {
    let record = build_ping_record(&args)?;
    let saved_path = append_heartbeat(record.clone()).await?;
    println!("saved heartbeat");
    println!(
        "{:<12} {}",
        "time",
        record
            .timestamp
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
    );
    println!("{:<12} {}", "entity", record.entity);
    println!(
        "{:<12} {}",
        "kind",
        format!("{:?}", record.kind).to_lowercase()
    );
    if let Some(project) = record.project.as_deref() {
        println!("{:<12} {}", "project", project);
    }
    if let Some(language) = record.language.as_deref() {
        println!("{:<12} {}", "language", language);
    }
    println!("{:<12} {}", "origin", record.origin);
    println!("{:<12} {}", "write", record.is_write);
    println!("{:<12} {}", "path", saved_path.display());
    Ok(())
}

async fn run_watch(args: HeartbeatWatchArgs) -> Result<()> {
    tokio::task::spawn_blocking(move || run_watch_blocking(args)).await?
}

fn run_watch_blocking(args: HeartbeatWatchArgs) -> Result<()> {
    let mut paths = if args.paths.is_empty() {
        vec![std::env::current_dir().context("Failed to resolve current directory")?]
    } else {
        args.paths.iter().map(PathBuf::from).collect::<Vec<_>>()
    };
    paths.sort();
    paths.dedup();

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher: RecommendedWatcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default().with_poll_interval(std::time::Duration::from_millis(args.debounce_ms)),
    )
    .context("Failed to initialize heartbeat watcher")?;

    for path in &paths {
        watcher
            .watch(path, RecursiveMode::Recursive)
            .with_context(|| format!("Failed to watch {}", path.display()))?;
    }

    println!("heartbeat watch");
    for path in &paths {
        println!("{:<12} {}", "path", path.display());
    }
    println!("{:<12} {}s", "pulse", args.pulse_seconds);
    println!("{:<12} {}", "origin", args.origin);
    println!("{:<12} {}", "writes-only", args.writes_only);
    println!("ctrl+c to stop");

    let mut last_emitted = HashMap::<String, DateTime<Utc>>::new();
    loop {
        match rx.recv() {
            Ok(Ok(event)) => {
                if let Some(is_write) = classify_watch_event(&event, args.writes_only) {
                    for path in event.paths {
                        if should_ignore_watch_path(&path) {
                            continue;
                        }
                        let entity = normalize_entity_string(&path);
                        let now = Utc::now();
                        let should_emit = match last_emitted.get(&entity) {
                            Some(last) => {
                                (now - *last).num_seconds() >= i64::from(args.pulse_seconds)
                            }
                            None => true,
                        };
                        if !should_emit {
                            continue;
                        }

                        let record = heartbeat_record_from_path(
                            &path,
                            now,
                            args.project.as_deref(),
                            Some(&args.origin),
                            is_write,
                            args.pulse_seconds,
                            DEFAULT_HEARTBEAT_TIMEOUT_SECS,
                        );
                        let saved_path = append_heartbeat_blocking(&record)?;
                        last_emitted.insert(entity.clone(), now);
                        println!(
                            "{}  {}  {}  {}",
                            now.with_timezone(&Local).format("%H:%M:%S"),
                            if is_write { "write" } else { "touch" },
                            entity,
                            saved_path.display()
                        );
                    }
                }
            }
            Ok(Err(err)) => eprintln!("watch error: {err}"),
            Err(_) => break,
        }
    }

    Ok(())
}

fn append_heartbeat_blocking(record: &HeartbeatRecord) -> Result<PathBuf> {
    let path = heartbeat_file_path_for_utc_day(record.timestamp.date_naive())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create heartbeat dir: {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("Failed to open heartbeat file: {}", path.display()))?;
    let serialized = serde_json::to_string(record)?;
    file.write_all(serialized.as_bytes())
        .context("Failed to write heartbeat JSON line")?;
    file.write_all(b"\n")
        .context("Failed to terminate heartbeat JSON line")?;
    invalidate_heartbeat_cache(&path);
    Ok(path)
}

fn classify_watch_event(event: &Event, writes_only: bool) -> Option<bool> {
    match event.kind {
        EventKind::Create(_) => Some(true),
        EventKind::Modify(_) => Some(true),
        EventKind::Access(_) if !writes_only => Some(false),
        _ => None,
    }
}

fn should_ignore_watch_path(path: &Path) -> bool {
    if path.as_os_str().is_empty() {
        return true;
    }
    if let Ok(base) = heartbeat_dir()
        && path.starts_with(base)
    {
        return true;
    }
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        WATCH_IGNORE_COMPONENTS
            .iter()
            .any(|ignored| name == *ignored)
    })
}

async fn run_stats(args: HeartbeatStatsArgs) -> Result<()> {
    let tz = parse_timezone_mode(args.timezone.as_deref())?;
    let (start, end) = heartbeat_stats_range(&args, &tz)?;
    let records = load_heartbeat_records(HeartbeatLoadOptions {
        start,
        end,
        tz: &tz,
        project_filter: args.project.as_deref(),
    })
    .await?;
    let dataset = dataset_from_heartbeats(&tz, &records);
    let daily = build_daily_rows(&dataset, &records, start, end, &tz);
    let summary = dataset.summary_for_range(start, end);
    let heartbeats = records.len();
    let days = (end - start).num_days().max(0) as u32 + 1;
    let total_seconds = summary
        .as_ref()
        .map(|value| value.total_seconds)
        .unwrap_or(0);
    let top_origin = top_origin(&records);
    let out = HeartbeatStatsOut {
        start: start.to_string(),
        end: end.to_string(),
        heartbeats,
        coding_seconds: total_seconds,
        coding: format_activity_duration(total_seconds),
        avg_per_day_seconds: if days == 0 {
            0
        } else {
            total_seconds / u64::from(days)
        },
        avg_per_day: format_activity_duration(if days == 0 {
            0
        } else {
            total_seconds / u64::from(days)
        }),
        top_project: summary.as_ref().and_then(|value| value.top_project.clone()),
        top_language: summary
            .as_ref()
            .and_then(|value| value.top_language.clone()),
        top_origin,
        daily,
        origins: origin_breakdown(&records, args.limit.max(1)),
        stats_path: heartbeat_dir()?.display().to_string(),
    };

    if args.json || args.jq.is_some() {
        emit_json(&out, args.jq.as_deref())
    } else {
        print_heartbeat_stats(&out);
        Ok(())
    }
}

fn build_ping_record(args: &HeartbeatPingArgs) -> Result<HeartbeatRecord> {
    let timestamp = match args.time.as_deref() {
        Some(raw) => DateTime::parse_from_rfc3339(raw)
            .with_context(|| format!("Invalid --time RFC3339 value: {raw}"))?
            .with_timezone(&Utc),
        None => Utc::now(),
    };

    let entity = match args.entity.as_deref() {
        Some(entity) => normalize_entity_string(Path::new(entity)),
        None => normalize_entity_string(
            &std::env::current_dir().context("Failed to resolve current directory")?,
        ),
    };
    let path = PathBuf::from(&entity);
    let mut record = heartbeat_record_from_path(
        &path,
        timestamp,
        args.project.as_deref(),
        Some(&args.origin),
        args.write,
        args.pulse_seconds,
        args.timeout_seconds,
    )
    .with_kind(args.kind);
    if let Some(language) = args.language.as_ref() {
        record.language = Some(language.clone());
    }
    if let Some(branch) = args.branch.as_ref() {
        record.branch = Some(branch.clone());
    }
    if args.kind != HeartbeatEntityKind::File {
        if args.language.is_none() {
            record.language = None;
        }
        if args.project.is_none() {
            record.project = None;
        }
    }
    Ok(record)
}

fn heartbeat_stats_range(
    args: &HeartbeatStatsArgs,
    tz: &TimeZoneMode,
) -> Result<(NaiveDate, NaiveDate)> {
    let today = heartbeat_local_date(Utc::now(), tz);
    let since = parse_date_filter(args.since.as_deref())?;
    let until = parse_date_filter(args.until.as_deref())?;

    let range = match (since, until) {
        (Some(since), Some(until)) => (since.min(until), since.max(until)),
        (Some(since), None) => (since, today),
        (None, Some(until)) => {
            let start = until
                .checked_sub_signed(TimeDelta::days(i64::from(args.days.max(1) - 1)))
                .unwrap_or(until);
            (start, until)
        }
        (None, None) => {
            let end = today;
            let start = end
                .checked_sub_signed(TimeDelta::days(i64::from(args.days.max(1) - 1)))
                .unwrap_or(end);
            (start, end)
        }
    };

    Ok(range)
}

fn build_daily_rows(
    dataset: &crate::activity::ActivityDataset,
    records: &[HeartbeatRecord],
    start: NaiveDate,
    end: NaiveDate,
    tz: &TimeZoneMode,
) -> Vec<HeartbeatDailyRow> {
    let mut counts_by_day = BTreeMap::<NaiveDate, usize>::new();
    for record in records {
        let day = heartbeat_local_date(record.timestamp, tz);
        *counts_by_day.entry(day).or_default() += 1;
    }

    let mut rows = Vec::new();
    let mut cursor = start;
    while cursor <= end {
        let summary = dataset.summary_for_day(cursor);
        let coding_seconds = summary
            .as_ref()
            .map(|value| value.total_seconds)
            .unwrap_or(0);
        if coding_seconds > 0 || counts_by_day.contains_key(&cursor) {
            rows.push(HeartbeatDailyRow {
                date: cursor.to_string(),
                heartbeats: counts_by_day.get(&cursor).copied().unwrap_or(0),
                coding_seconds,
                coding: format_activity_duration(coding_seconds),
                top_project: summary.as_ref().and_then(|value| value.top_project.clone()),
                top_language: summary
                    .as_ref()
                    .and_then(|value| value.top_language.clone()),
            });
        }
        cursor = cursor
            .checked_add_signed(TimeDelta::days(1))
            .unwrap_or(end.succ_opt().unwrap_or(end));
    }
    rows
}

fn origin_breakdown(records: &[HeartbeatRecord], limit: usize) -> Vec<HeartbeatBreakdownRow> {
    if records.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut counts = HashMap::<String, usize>::new();
    for record in records {
        *counts.entry(record.origin.clone()).or_default() += 1;
    }
    let total = records.len() as f64;
    let mut rows = counts.into_iter().collect::<Vec<_>>();
    rows.sort_by(|(name_a, count_a), (name_b, count_b)| {
        count_b.cmp(count_a).then_with(|| name_a.cmp(name_b))
    });
    rows.truncate(limit);
    rows.into_iter()
        .map(|(name, count)| HeartbeatBreakdownRow {
            name,
            count,
            percent: (count as f64 / total) * 100.0,
        })
        .collect()
}

fn top_origin(records: &[HeartbeatRecord]) -> Option<String> {
    origin_breakdown(records, 1)
        .into_iter()
        .next()
        .map(|row| row.name)
}

fn print_heartbeat_stats(out: &HeartbeatStatsOut) {
    if out.start == out.end {
        println!("Heartbeat {}", out.start);
    } else {
        println!("Heartbeat {} -> {}", out.start, out.end);
    }
    let active_days = out.daily.len();
    let total_days = match (
        NaiveDate::parse_from_str(&out.end, "%Y-%m-%d"),
        NaiveDate::parse_from_str(&out.start, "%Y-%m-%d"),
    ) {
        (Ok(end), Ok(start)) => ((end - start).num_days().max(0) as usize) + 1,
        _ => active_days.max(1),
    };
    println!(
        "{:<12} {} {:>5.1}%  {}/{}",
        "Active days",
        render_progress_meter(active_days as f64 / total_days.max(1) as f64, 18),
        active_days as f64 / total_days.max(1) as f64 * 100.0,
        active_days,
        total_days
    );

    let mut summary = create_text_table();
    summary.set_header(vec![
        header_cell("Heartbeats"),
        header_cell("Coding"),
        header_cell("Avg / day"),
        header_cell("Top project"),
    ]);
    summary.add_row(TableRow::from(vec![
        value_cell(&out.heartbeats.to_string(), Some(TableColor::Cyan)),
        value_cell(&out.coding, Some(TableColor::Green)),
        value_cell(&out.avg_per_day, Some(TableColor::Green)),
        value_cell(
            out.top_project.as_deref().unwrap_or("-"),
            Some(TableColor::White),
        ),
    ]));
    summary.add_row(TableRow::from(vec![
        value_label_cell("Top lang"),
        value_label_cell("Top origin"),
        value_label_cell("Store"),
        value_label_cell(""),
    ]));
    summary.add_row(TableRow::from(vec![
        value_cell(
            out.top_language.as_deref().unwrap_or("-"),
            Some(TableColor::White),
        ),
        value_cell(
            out.top_origin.as_deref().unwrap_or("-"),
            Some(TableColor::White),
        ),
        value_cell(
            &truncate_text(&out.stats_path, 40),
            Some(TableColor::DarkGrey),
        ),
        value_cell("", None),
    ]));
    println!("{summary}");

    if !out.daily.is_empty() {
        println!();
        let mut daily = create_text_table();
        daily.set_header(vec![
            header_cell("Date"),
            header_cell("Heartbeats"),
            header_cell("Coding"),
            header_cell("Top project"),
            header_cell("Top lang"),
        ]);
        for row in &out.daily {
            daily.add_row(TableRow::from(vec![
                value_cell(&row.date, Some(TableColor::White)),
                value_cell(&row.heartbeats.to_string(), Some(TableColor::Cyan)),
                value_cell(&row.coding, Some(TableColor::Green)),
                value_cell(
                    &truncate_text(row.top_project.as_deref().unwrap_or("-"), 24),
                    Some(TableColor::White),
                ),
                value_cell(
                    &truncate_text(row.top_language.as_deref().unwrap_or("-"), 16),
                    Some(TableColor::White),
                ),
            ]));
        }
        println!("{daily}");
    }

    if !out.origins.is_empty() {
        println!();
        let mut origins = create_text_table();
        origins.set_header(vec![
            header_cell("Origin"),
            header_cell("Count"),
            header_cell("Share"),
        ]);
        for row in &out.origins {
            origins.add_row(TableRow::from(vec![
                value_cell(&row.name, Some(TableColor::White)),
                value_cell(&row.count.to_string(), Some(TableColor::Cyan)),
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
        println!("{origins}");
    }
}

fn use_styled_output() -> bool {
    std::io::stdout().is_terminal() || std::env::var("CLICOLOR_FORCE").is_ok()
}

fn create_text_table() -> TextTable {
    let mut table = TextTable::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic);
    if !std::io::stdout().is_terminal() && std::env::var("CLICOLOR_FORCE").is_ok() {
        table.enforce_styling();
    }
    table
}

fn header_cell(text: &str) -> TableCell {
    let mut cell = TableCell::new(text).add_attribute(Attribute::Bold);
    if use_styled_output() {
        cell = cell.fg(TableColor::Cyan);
    }
    cell
}

fn value_label_cell(text: &str) -> TableCell {
    let mut cell = TableCell::new(text).add_attribute(Attribute::Bold);
    if use_styled_output() {
        cell = cell.fg(TableColor::DarkGrey);
    }
    cell
}

fn value_cell(text: &str, color: Option<TableColor>) -> TableCell {
    let mut cell = TableCell::new(text);
    if use_styled_output()
        && let Some(color) = color
    {
        cell = cell.fg(color);
    }
    cell
}

fn render_progress_meter(ratio: f64, width: usize) -> String {
    let clamped = ratio.clamp(0.0, 1.0);
    let filled = (clamped * width as f64).round() as usize;
    let mut bar = String::with_capacity(width);
    for idx in 0..width {
        if idx < filled {
            bar.push('█');
        } else {
            bar.push('░');
        }
    }
    format!("[{bar}]")
}

fn emit_json<T: Serialize>(value: &T, jq: Option<&str>) -> Result<()> {
    let pretty = serde_json::to_string_pretty(value)?;
    if let Some(filter) = jq {
        let mut child = Command::new("jq")
            .arg(filter)
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .spawn()
            .context("Failed to run jq")?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(pretty.as_bytes())
                .context("Failed to write jq stdin")?;
        }
        let status = child.wait().context("Failed to wait for jq")?;
        if !status.success() {
            bail!("jq exited with status {}", status);
        }
        Ok(())
    } else {
        println!("{pretty}");
        Ok(())
    }
}

fn parse_timezone_mode(input: Option<&str>) -> Result<TimeZoneMode> {
    let Some(raw) = input else {
        return Ok(TimeZoneMode::Local);
    };
    if raw.eq_ignore_ascii_case("local") {
        return Ok(TimeZoneMode::Local);
    }
    if raw.eq_ignore_ascii_case("utc") {
        return Ok(TimeZoneMode::Utc);
    }
    let tz: chrono_tz::Tz = raw
        .parse()
        .with_context(|| format!("Invalid timezone: {raw}"))?;
    Ok(TimeZoneMode::Named(tz))
}

fn parse_date_filter(input: Option<&str>) -> Result<Option<NaiveDate>> {
    crate::pipeline::parse_date_filter(input)
}

fn heartbeat_record_from_path(
    path: &Path,
    timestamp: DateTime<Utc>,
    project_override: Option<&str>,
    origin: Option<&str>,
    is_write: bool,
    pulse_seconds: u16,
    timeout_seconds: u16,
) -> HeartbeatRecord {
    let project = project_override
        .map(ToOwned::to_owned)
        .or_else(|| detect_project_name(path));
    let language = infer_language_from_path(path);
    HeartbeatRecord {
        timestamp,
        entity: normalize_entity_string(path),
        kind: if path.extension().is_some() {
            HeartbeatEntityKind::File
        } else {
            HeartbeatEntityKind::Other
        },
        project,
        language,
        branch: None,
        origin: origin.unwrap_or("manual").to_string(),
        is_write,
        pulse_seconds,
        timeout_seconds,
    }
}

impl HeartbeatRecord {
    fn with_kind(mut self, kind: HeartbeatEntityKind) -> Self {
        self.kind = kind;
        self
    }
}

fn normalize_entity_string(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn detect_project_name(path: &Path) -> Option<String> {
    let starting = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };

    for candidate in starting.ancestors() {
        if candidate.join(".git").exists() {
            let name = candidate.file_name()?.to_string_lossy().trim().to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }

    if starting.as_os_str().is_empty() {
        return None;
    }

    let name = starting.file_name()?.to_string_lossy().trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

fn infer_language_from_path(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    let language = match ext.as_str() {
        "rs" => "Rust",
        "py" => "Python",
        "js" => "JavaScript",
        "jsx" => "JavaScript React",
        "ts" => "TypeScript",
        "tsx" => "TypeScript React",
        "go" => "Go",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "c" | "h" => "C",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "C++",
        "cs" => "C#",
        "rb" => "Ruby",
        "php" => "PHP",
        "lua" => "Lua",
        "sh" | "bash" | "zsh" => "Shell",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "md" | "mdx" => "Markdown",
        "html" => "HTML",
        "css" => "CSS",
        "scss" => "SCSS",
        "sql" => "SQL",
        _ => return None,
    };
    Some(language.to_string())
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

} // if cfg(feature = "cli")
} // cfg_if!

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{
        HeartbeatEntityKind, HeartbeatRecord, detect_project_name, infer_language_from_path,
    };

    #[test]
    fn infers_language_from_common_extensions() {
        assert_eq!(
            infer_language_from_path(std::path::Path::new("src/main.rs")).as_deref(),
            Some("Rust")
        );
        assert_eq!(
            infer_language_from_path(std::path::Path::new("app/page.tsx")).as_deref(),
            Some("TypeScript React")
        );
        assert_eq!(
            infer_language_from_path(std::path::Path::new("README")).as_deref(),
            None
        );
    }

    #[test]
    fn heartbeat_record_serializes() {
        let record = HeartbeatRecord {
            timestamp: Utc.with_ymd_and_hms(2026, 3, 8, 10, 0, 0).single().unwrap(),
            entity: "/tmp/example.rs".to_string(),
            kind: HeartbeatEntityKind::File,
            project: Some("tokenusage".to_string()),
            language: Some("Rust".to_string()),
            branch: None,
            origin: "manual".to_string(),
            is_write: true,
            pulse_seconds: 120,
            timeout_seconds: 900,
        };
        let text = serde_json::to_string(&record).unwrap();
        let decoded: HeartbeatRecord = serde_json::from_str(&text).unwrap();
        assert_eq!(decoded.project.as_deref(), Some("tokenusage"));
        assert_eq!(decoded.language.as_deref(), Some("Rust"));
    }

    #[test]
    fn project_detection_falls_back_to_parent_dir() {
        let root = std::env::temp_dir().join(format!("tu-heartbeat-{}", std::process::id()));
        let child = root.join("src").join("main.rs");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(child.parent().unwrap()).unwrap();
        assert_eq!(detect_project_name(&child).as_deref(), Some("src"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
