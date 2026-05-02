use std::collections::HashMap;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::Result;
use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color as TuiColor, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::cli::TopArgs;
use crate::types::{SourceKind, TokenCounts};

use super::display::*;
use super::live::BlocksLiveSession;
use super::parsing::save_incremental_cache;
use super::parsing::{ParseFilesConfig, extract_session_title, load_usage, parse_files_with_cache};
use super::*;

#[derive(Clone)]
pub(super) struct TopSession {
    session_id: String,
    /// Stable key for rate tracking (session_id when unmerged, project name when merged)
    rate_key: String,
    source: SourceKind,
    project: Option<String>,
    primary_model: String,
    totals: TokenCounts,
    last_activity: DateTime<Utc>,
    output_tokens_total: u64,
    title: String,
    session_count: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TopSortKey {
    Cost,
    OutputTokens,
    Rate,
    Recent,
}

pub(crate) async fn run_top(args: TopArgs) -> Result<()> {
    if args.smoke_check {
        let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
        let loaded = load_usage(&args.common, &tz).await?;
        println!(
            "top-smoke-ok: events={} files={} parsed_lines={}",
            loaded.events.len(),
            loaded.stats.files_discovered,
            loaded.stats.lines_parsed
        );
        return Ok(());
    }
    if !std::io::stdout().is_terminal() {
        bail!("tu top requires an interactive terminal");
    }

    let refresh_every = args.refresh_interval.max(1);
    let limit = args.limit;
    let active_window_secs = args.active_hours as i64 * 3600;
    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;

    let mut session = BlocksLiveSession::enter()?;
    let mut sort_key = TopSortKey::Recent;
    let mut show_all = active_window_secs == 0;
    let mut merge_projects = true;

    // Previous snapshot for rate calculation
    let mut prev_session_output: HashMap<String, u64> = HashMap::new();
    let mut prev_snapshot_at: Option<Instant> = None;
    let mut rates: HashMap<String, f64> = HashMap::new();
    let mut title_cache: HashMap<String, String> = HashMap::new();

    // Background runtime init (same pattern as tu live)
    let common_for_init = args.common.clone();
    let mut pending_runtime_task: Option<tokio::task::JoinHandle<Result<LiveUsageRuntime>>> =
        Some(tokio::spawn(async move {
            LiveUsageRuntime::new(&common_for_init, refresh_every, false).await
        }));
    let mut live_runtime: Option<LiveUsageRuntime> = None;

    let tick_rate = Duration::from_secs(refresh_every);
    let mut last_data_refresh = Instant::now();
    let mut sessions_display: Vec<TopSession> = Vec::new();
    let mut total_display = TokenCounts::default();
    let mut total_rate: f64 = 0.0;

    loop {
        // Check if runtime init finished
        if let Some(task) = pending_runtime_task.as_mut() {
            if task.is_finished() {
                let task = pending_runtime_task.take().unwrap();
                match task.await {
                    Ok(Ok(rt)) => live_runtime = Some(rt),
                    Ok(Err(e)) => {
                        drop(session);
                        bail!("Failed to initialise runtime: {e}");
                    }
                    Err(e) => {
                        drop(session);
                        bail!("Runtime init task panicked: {e}");
                    }
                }
            }
        }

        // Refresh data
        let should_refresh = live_runtime.is_some() && last_data_refresh.elapsed() >= tick_rate;
        if should_refresh {
            let rt = live_runtime.as_mut().unwrap();
            rt.maybe_refresh_discovery();

            let parsed = parse_files_with_cache(
                &rt.files_cache,
                &mut rt.cache_store,
                ParseFilesConfig {
                    filter: rt.filter,
                    timezone: &tz,
                    pricing: rt.pricing.clone(),
                    worker_count: rt.worker_count,
                    cache_enabled: rt.cache_enabled,
                    sort_events: false,
                },
            );
            if parsed.cache_dirty {
                rt.cache_dirty = true;
            }

            // Group events by session
            let mut grouped: HashMap<String, (GroupAggregate, u64, String)> = HashMap::new();
            for event in &parsed.loaded.events {
                let entry = grouped
                    .entry(event.session.clone())
                    .or_insert_with(|| (GroupAggregate::default(), 0u64, event.file_path.clone()));
                entry.0.add_event(event);
                entry.1 += event.usage.output_tokens;
            }

            // Build sessions list
            let now = Instant::now();
            let mut new_sessions: Vec<TopSession> = grouped
                .into_iter()
                .map(|(sid, (agg, out_total, fpath))| {
                    let primary_model = agg
                        .by_model
                        .iter()
                        .max_by_key(|(_, v)| v.output_tokens)
                        .map(|(k, _)| k.clone())
                        .unwrap_or_default();
                    // Use file mtime as last_activity (more accurate than last event timestamp)
                    let last = std::fs::metadata(&fpath)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| {
                            let dur = t.duration_since(UNIX_EPOCH).ok()?;
                            DateTime::from_timestamp(dur.as_secs() as i64, dur.subsec_nanos())
                        })
                        .or(agg.last_activity)
                        .unwrap_or_else(Utc::now);
                    let source = agg
                        .by_source
                        .keys()
                        .next()
                        .copied()
                        .unwrap_or(SourceKind::Claude);
                    // Lookup cached title or extract
                    let title = title_cache
                        .entry(sid.clone())
                        .or_insert_with(|| extract_session_title(source, Path::new(&fpath)))
                        .clone();
                    TopSession {
                        rate_key: sid.clone(),
                        session_id: sid,
                        source,
                        project: agg.project,
                        primary_model,
                        totals: agg.totals.to_counts(),
                        last_activity: last,
                        output_tokens_total: out_total,
                        title,
                        session_count: 1,
                    }
                })
                .collect();

            // Filter to active sessions only
            if !show_all && active_window_secs > 0 {
                let cutoff = Utc::now() - chrono::Duration::seconds(active_window_secs);
                new_sessions.retain(|s| s.last_activity >= cutoff);
            }

            // Merge sessions by project if enabled
            if merge_projects {
                let mut by_project: HashMap<String, TopSession> = HashMap::new();
                for s in new_sessions {
                    let key = s.project.clone().unwrap_or_else(|| s.session_id.clone());
                    match by_project.entry(key.clone()) {
                        std::collections::hash_map::Entry::Occupied(mut e) => {
                            let m = e.get_mut();
                            m.totals.add_assign(s.totals);
                            m.output_tokens_total += s.output_tokens_total;
                            m.session_count += 1;
                            if s.last_activity > m.last_activity {
                                m.last_activity = s.last_activity;
                                m.primary_model = s.primary_model;
                                // Prefer title from most recent session; keep old if newest is empty
                                if !s.title.is_empty() {
                                    m.title = s.title;
                                }
                            }
                        }
                        std::collections::hash_map::Entry::Vacant(e) => {
                            let mut merged = s;
                            merged.rate_key = key;
                            e.insert(merged);
                        }
                    }
                }
                new_sessions = by_project.into_values().collect();
            }

            // Calculate rates (use rate_key for stable tracking across merges)
            if let Some(prev_at) = prev_snapshot_at {
                let elapsed_secs = now.duration_since(prev_at).as_secs_f64();
                if elapsed_secs > 0.5 {
                    rates.clear();
                    for s in &new_sessions {
                        let prev = prev_session_output.get(&s.rate_key).copied().unwrap_or(0);
                        let delta = s.output_tokens_total.saturating_sub(prev);
                        let rate = (delta as f64 / elapsed_secs) * 60.0;
                        rates.insert(s.rate_key.clone(), rate);
                    }
                }
            }

            // Store current snapshot for next rate calc
            prev_session_output.clear();
            for s in &new_sessions {
                prev_session_output.insert(s.rate_key.clone(), s.output_tokens_total);
            }
            prev_snapshot_at = Some(now);

            // Sort
            match sort_key {
                TopSortKey::Cost => new_sessions.sort_by(|a, b| {
                    b.totals
                        .cost_usd
                        .partial_cmp(&a.totals.cost_usd)
                        .unwrap_or(std::cmp::Ordering::Equal)
                }),
                TopSortKey::OutputTokens => {
                    new_sessions.sort_by(|a, b| b.output_tokens_total.cmp(&a.output_tokens_total))
                }
                TopSortKey::Rate => new_sessions.sort_by(|a, b| {
                    let ra = rates.get(&a.rate_key).copied().unwrap_or(0.0);
                    let rb = rates.get(&b.rate_key).copied().unwrap_or(0.0);
                    rb.partial_cmp(&ra).unwrap_or(std::cmp::Ordering::Equal)
                }),
                TopSortKey::Recent => {
                    new_sessions.sort_by(|a, b| b.last_activity.cmp(&a.last_activity))
                }
            }

            new_sessions.truncate(limit);

            total_display = new_sessions
                .iter()
                .fold(TokenCounts::default(), |mut acc, s| {
                    acc.add_assign(s.totals.clone());
                    acc
                });
            total_rate = new_sessions
                .iter()
                .map(|s| rates.get(&s.rate_key).copied().unwrap_or(0.0))
                .sum();

            sessions_display = new_sessions;
            last_data_refresh = Instant::now();

            // Flush cache periodically
            if rt.cache_dirty && rt.last_cache_flush_at.elapsed() > Duration::from_secs(30) {
                if let Some(ref path) = rt.cache_path {
                    save_incremental_cache(path, &rt.cache_store);
                    rt.cache_dirty = false;
                    rt.last_cache_flush_at = Instant::now();
                }
            }
        }

        // Render
        render_top_frame(
            &mut session,
            &TopFrameState {
                sessions: &sessions_display,
                rates: &rates,
                total: &total_display,
                total_rate,
                sort_key,
                tz: &tz,
                loading: live_runtime.is_none(),
                show_all,
                merge_projects,
            },
        )?;

        // Handle input (non-blocking)
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char('c') => sort_key = TopSortKey::Cost,
                    KeyCode::Char('o') => sort_key = TopSortKey::OutputTokens,
                    KeyCode::Char('r') => sort_key = TopSortKey::Rate,
                    KeyCode::Char('t') => sort_key = TopSortKey::Recent,
                    KeyCode::Char('a') => {
                        show_all = !show_all;
                        last_data_refresh = Instant::now() - tick_rate;
                    }
                    KeyCode::Char('m') => {
                        merge_projects = !merge_projects;
                        // Reset rate tracking since keys change between modes
                        prev_session_output.clear();
                        prev_snapshot_at = None;
                        rates.clear();
                        last_data_refresh = Instant::now() - tick_rate;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

pub(super) struct TopFrameState<'a> {
    sessions: &'a [TopSession],
    rates: &'a HashMap<String, f64>,
    total: &'a TokenCounts,
    total_rate: f64,
    sort_key: TopSortKey,
    tz: &'a TimeZoneMode,
    loading: bool,
    show_all: bool,
    merge_projects: bool,
}

pub(super) fn render_top_frame(
    session: &mut BlocksLiveSession,
    state: &TopFrameState<'_>,
) -> Result<()> {
    let TopFrameState {
        sessions,
        rates,
        total,
        total_rate,
        sort_key,
        tz,
        loading,
        show_all,
        merge_projects,
    } = state;
    session.terminal.draw(|frame: &mut ratatui::Frame| {
        let area = frame.area();

        // Layout: header(3) + table(rest) + footer(1)
        let chunks = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(1),
            ])
            .split(area);

        // Header
        let status = if *loading { " [loading...]" } else { "" };
        let mode = if *show_all { "all" } else { "active" };
        let header_text = format!(
            " tokenusage top — {} sessions ({mode}) | ${:.2} | {:.0} tok/min{}",
            sessions.len(),
            total.cost_usd,
            total_rate,
            status,
        );
        let header = Paragraph::new(Line::from(vec![Span::styled(
            header_text,
            Style::default()
                .fg(TuiColor::Cyan)
                .add_modifier(Modifier::BOLD),
        )]))
        .block(Block::default().borders(Borders::BOTTOM));
        frame.render_widget(header, chunks[0]);

        // Table
        let header_row = Row::new(vec![
            Cell::from("Source"),
            Cell::from("Project"),
            Cell::from("Model"),
            Cell::from("Tok/min"),
            Cell::from("Input"),
            Cell::from("Output"),
            Cell::from("Cost"),
            Cell::from("Last Active"),
        ])
        .style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(TuiColor::Yellow),
        )
        .height(1);

        let rows: Vec<Row> = sessions
            .iter()
            .map(|s| {
                let rate = rates.get(&s.rate_key).copied().unwrap_or(0.0);
                let source_color = match s.source {
                    SourceKind::Claude => TuiColor::Cyan,
                    SourceKind::Codex => TuiColor::Blue,
                    SourceKind::Gemini => TuiColor::Green,
                    SourceKind::OpenCode => TuiColor::Magenta,
                };
                let project = s.project.as_deref().unwrap_or("-");
                let project_short = if project.len() > 20 {
                    &project[project.len() - 20..]
                } else {
                    project
                };
                let project_label = project_short.to_string();
                let model_short = shorten_model_name(&s.primary_model);

                // Rate bar visualization
                let rate_str = if rate > 0.5 {
                    let bar_len = ((rate / 1000.0) * 5.0).min(8.0) as usize;
                    let bar: String = "█".repeat(bar_len.max(1));
                    format!("{bar} {:.0}", rate)
                } else {
                    "  -".to_string()
                };

                let rate_color = if rate > 5000.0 {
                    TuiColor::Red
                } else if rate > 1000.0 {
                    TuiColor::Yellow
                } else if rate > 0.5 {
                    TuiColor::Green
                } else {
                    TuiColor::DarkGray
                };

                let last_active = format_display_datetime(s.last_activity, tz);
                // Show only time portion if today
                let last_short = if last_active.len() > 5 {
                    last_active
                        .rsplit_once(' ')
                        .map(|(_, t)| t)
                        .unwrap_or(&last_active)
                } else {
                    &last_active
                };

                // Build project cell: project name + title subtitle
                let project_lines = if s.title.is_empty() {
                    Line::from(Span::raw(project_label.clone()))
                } else {
                    Line::from(vec![
                        Span::raw(project_label.clone()),
                        Span::styled(
                            format!("  {}", truncate_str(&s.title, 40)),
                            Style::default().fg(TuiColor::DarkGray),
                        ),
                    ])
                };

                Row::new(vec![
                    Cell::from(s.source.as_str()).style(Style::default().fg(source_color)),
                    Cell::from(project_lines),
                    Cell::from(model_short),
                    Cell::from(rate_str).style(Style::default().fg(rate_color)),
                    Cell::from(format_token_compact(s.totals.input_tokens)),
                    Cell::from(format_token_compact(s.output_tokens_total)),
                    Cell::from(format!("${:.2}", s.totals.cost_usd)),
                    Cell::from(last_short.to_string())
                        .style(Style::default().fg(TuiColor::DarkGray)),
                ])
            })
            .collect();

        let widths = [
            Constraint::Length(8),
            Constraint::Fill(1),
            Constraint::Length(16),
            Constraint::Length(14),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(10),
        ];

        let table = Table::new(rows, widths)
            .header(header_row)
            .block(Block::default());
        frame.render_widget(table, chunks[1]);

        // Footer with sort key hints
        let sort_keys = [
            ("c", "cost", TopSortKey::Cost),
            ("o", "output", TopSortKey::OutputTokens),
            ("r", "rate", TopSortKey::Rate),
            ("t", "recent", TopSortKey::Recent),
        ];
        let mut footer_spans: Vec<Span> = vec![Span::raw(" Sort: ")];
        for (i, (key, label, sk)) in sort_keys.iter().enumerate() {
            if i > 0 {
                footer_spans.push(Span::raw("  "));
            }
            let style = if *sk == *sort_key {
                Style::default()
                    .fg(TuiColor::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(TuiColor::DarkGray)
            };
            footer_spans.push(Span::styled(format!("[{key}]{label}"), style));
        }
        let all_label = if *show_all { "active" } else { "all" };
        footer_spans.push(Span::styled(
            format!("  [a]{all_label}"),
            Style::default().fg(TuiColor::DarkGray),
        ));
        let merge_style = if *merge_projects {
            Style::default()
                .fg(TuiColor::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TuiColor::DarkGray)
        };
        footer_spans.push(Span::styled("  [m]merge", merge_style));
        footer_spans.push(Span::styled(
            "  [q]quit",
            Style::default().fg(TuiColor::DarkGray),
        ));

        let footer = Paragraph::new(Line::from(footer_spans));
        frame.render_widget(footer, chunks[2]);
    })?;
    Ok(())
}
