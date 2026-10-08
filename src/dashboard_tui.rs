#[cfg(feature = "cli")]
use std::collections::{BTreeMap, HashMap};
use std::io::{self, IsTerminal};
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{Datelike, NaiveDate};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, Tabs};

use crate::cli::{CommonArgs, ScalePeriodArg, WeekStart};
use crate::insights::{
    ReportInsights, compute_report_insights, sanitize_project_label, sanitize_session_label,
};
use crate::pipeline::TimeZoneMode;
use crate::pipeline::commands::{
    RankDayItem, RankLeaderboardOut, RankModelItem, RankProjectItem, RankSessionItem,
};
use crate::pipeline::week_start;
use crate::scale::InformationScale;
use crate::types::{DailyReport, DailyRow, ParseStats, TokenCounts, UsageEvent};

type BreakdownGroup = (u64, f64, HashMap<String, (u64, f64)>);
type BreakdownMap = HashMap<String, BreakdownGroup>;
type BreakdownEntries = Vec<(String, BreakdownGroup)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashboardTab {
    ScaleOverview = 0,
    Leaderboard = 1,
    Breakdown = 2,
    Timeline = 3,
}

impl DashboardTab {
    pub fn all() -> &'static [DashboardTab] {
        &[
            DashboardTab::ScaleOverview,
            DashboardTab::Leaderboard,
            DashboardTab::Breakdown,
            DashboardTab::Timeline,
        ]
    }

    pub fn title(&self) -> &'static str {
        match self {
            DashboardTab::ScaleOverview => " 🌐 1: Human Scale & Overview ",
            DashboardTab::Leaderboard => " 🏆 2: Hall of Fame & Records ",
            DashboardTab::Breakdown => " 📊 3: Cross Breakdown ",
            DashboardTab::Timeline => " 📅 4: Usage Timeline ",
        }
    }
}

pub struct DashboardState {
    pub selected_tab: DashboardTab,
    pub period: ScalePeriodArg,
    pub all_events: Vec<UsageEvent>,
    pub tz: TimeZoneMode,
    pub common: CommonArgs,
    pub project_filter: Option<String>,
    pub breakdown_by_model: bool,
    pub scroll_offset: usize,
    pub show_help: bool,

    // Filtered data cache
    pub current_events: Vec<UsageEvent>,
    pub scale: InformationScale,
    pub rank_out: RankLeaderboardOut,
    pub report: DailyReport,
    pub insights: ReportInsights,
}

impl DashboardState {
    pub fn new(
        all_events: Vec<UsageEvent>,
        tz: TimeZoneMode,
        common: CommonArgs,
        initial_tab: DashboardTab,
        period: ScalePeriodArg,
        project_filter: Option<String>,
    ) -> Self {
        let mut state = Self {
            selected_tab: initial_tab,
            period,
            all_events,
            tz,
            common,
            project_filter,
            breakdown_by_model: false,
            scroll_offset: 0,
            show_help: false,
            current_events: Vec::new(),
            scale: InformationScale::calculate(0, 0),
            rank_out: RankLeaderboardOut {
                total_tokens: 0,
                total_cost_usd: 0.0,
                top_models: Vec::new(),
                top_projects: Vec::new(),
                peak_days: Vec::new(),
                top_sessions: Vec::new(),
            },
            report: DailyReport::default(),
            insights: ReportInsights::default(),
        };
        state.recompute_filtered_data();
        state
    }

    pub fn recompute_filtered_data(&mut self) {
        let today_date = self.tz.now_date();
        let monday_date = week_start(today_date, WeekStart::Monday);
        let month_start_date =
            NaiveDate::from_ymd_opt(today_date.year(), today_date.month(), 1).unwrap_or(today_date);
        let seven_days_ago = today_date - chrono::TimeDelta::days(7);

        self.current_events = self
            .all_events
            .iter()
            .filter(|e| {
                if let Some(pf) = self.project_filter.as_deref() {
                    if !e.project.as_deref().is_some_and(|p| p.contains(pf)) {
                        return false;
                    }
                }
                let event_date = self.tz.date_of(e.timestamp);
                match self.period {
                    ScalePeriodArg::Today => {
                        if event_date != today_date {
                            return false;
                        }
                    }
                    ScalePeriodArg::Daily => {
                        if event_date < seven_days_ago {
                            return false;
                        }
                    }
                    ScalePeriodArg::Weekly => {
                        if event_date < monday_date {
                            return false;
                        }
                    }
                    ScalePeriodArg::Monthly => {
                        if event_date < month_start_date {
                            return false;
                        }
                    }
                    ScalePeriodArg::All => {}
                }
                true
            })
            .cloned()
            .collect();

        // 1. Scale
        self.scale = InformationScale::from_events(&self.current_events);

        // 2. Rank
        let mut total_tokens = 0u64;
        let mut total_cost = 0.0;
        let mut model_tokens: HashMap<String, u64> = HashMap::new();
        let mut model_cost: HashMap<String, f64> = HashMap::new();
        let mut project_tokens: HashMap<String, u64> = HashMap::new();
        let mut project_cost: HashMap<String, f64> = HashMap::new();
        let mut project_model_tokens: HashMap<String, HashMap<String, u64>> = HashMap::new();
        let mut day_tokens: HashMap<String, u64> = HashMap::new();
        let mut day_cost: HashMap<String, f64> = HashMap::new();
        let mut day_top_model: HashMap<String, HashMap<String, u64>> = HashMap::new();
        let mut session_tokens: HashMap<String, u64> = HashMap::new();
        let mut session_cost: HashMap<String, f64> = HashMap::new();
        let mut session_meta: HashMap<String, (String, String)> = HashMap::new();

        for e in &self.current_events {
            let tok = e.usage.total_tokens();
            let cost = e.usage.cost_usd;
            total_tokens += tok;
            total_cost += cost;

            *model_tokens.entry(e.model.clone()).or_insert(0) += tok;
            *model_cost.entry(e.model.clone()).or_insert(0.0) += cost;

            let proj = e.project.as_deref().unwrap_or("-").to_string();
            *project_tokens.entry(proj.clone()).or_insert(0) += tok;
            *project_cost.entry(proj.clone()).or_insert(0.0) += cost;
            *project_model_tokens
                .entry(proj)
                .or_default()
                .entry(e.model.clone())
                .or_insert(0) += tok;

            let day_str = self.tz.date_of(e.timestamp).format("%Y-%m-%d").to_string();
            *day_tokens.entry(day_str.clone()).or_insert(0) += tok;
            *day_cost.entry(day_str.clone()).or_insert(0.0) += cost;
            *day_top_model
                .entry(day_str)
                .or_default()
                .entry(e.model.clone())
                .or_insert(0) += tok;

            if !e.session.trim().is_empty() {
                *session_tokens.entry(e.session.clone()).or_insert(0) += tok;
                *session_cost.entry(e.session.clone()).or_insert(0.0) += cost;
                session_meta.entry(e.session.clone()).or_insert_with(|| {
                    let d = self.tz.date_of(e.timestamp).format("%Y-%m-%d").to_string();
                    let p = e.project.as_deref().unwrap_or("-").to_string();
                    (d, p)
                });
            }
        }

        let mut sorted_models: Vec<(String, u64)> = model_tokens.into_iter().collect();
        sorted_models.sort_by_key(|a| std::cmp::Reverse(a.1));

        let top_models: Vec<RankModelItem> = sorted_models
            .into_iter()
            .take(15)
            .enumerate()
            .map(|(i, (model, tok))| {
                let cost = model_cost.get(&model).copied().unwrap_or(0.0);
                let share_pct = if total_tokens > 0 {
                    (tok as f64 / total_tokens as f64) * 100.0
                } else {
                    0.0
                };
                let cost_pct = if total_cost > 0.0 {
                    (cost / total_cost) * 100.0
                } else {
                    0.0
                };
                let tier = if tok >= 10_000_000_000 {
                    "Titan"
                } else if tok >= 1_000_000_000 {
                    "Heavyweight"
                } else if tok >= 100_000_000 {
                    "Workhorse"
                } else if tok >= 10_000_000 {
                    "Regular"
                } else {
                    "Lightweight"
                }
                .to_string();
                RankModelItem {
                    rank: i + 1,
                    model,
                    tokens: tok,
                    share_pct,
                    cost_usd: cost,
                    cost_pct,
                    tier,
                }
            })
            .collect();

        let mut sorted_projects: Vec<(String, u64)> = project_tokens.into_iter().collect();
        sorted_projects.sort_by_key(|a| std::cmp::Reverse(a.1));

        let top_projects: Vec<RankProjectItem> = sorted_projects
            .into_iter()
            .take(15)
            .enumerate()
            .map(|(i, (proj, tok))| {
                let cost = project_cost.get(&proj).copied().unwrap_or(0.0);
                let share_pct = if total_tokens > 0 {
                    (tok as f64 / total_tokens as f64) * 100.0
                } else {
                    0.0
                };
                let primary_model = project_model_tokens
                    .get(&proj)
                    .and_then(|m| m.iter().max_by_key(|(_, t)| **t).map(|(k, _)| k.clone()))
                    .unwrap_or_else(|| "-".to_string());
                let clean_proj = sanitize_project_label(&proj);
                RankProjectItem {
                    rank: i + 1,
                    project: clean_proj,
                    tokens: tok,
                    share_pct,
                    cost_usd: cost,
                    primary_model,
                }
            })
            .collect();

        let mut sorted_days: Vec<(String, u64)> = day_tokens.into_iter().collect();
        sorted_days.sort_by_key(|a| std::cmp::Reverse(a.1));

        let peak_days: Vec<RankDayItem> = sorted_days
            .into_iter()
            .take(10)
            .enumerate()
            .map(|(i, (date, tok))| {
                let cost = day_cost.get(&date).copied().unwrap_or(0.0);
                let top_model = day_top_model
                    .get(&date)
                    .and_then(|m| m.iter().max_by_key(|(_, t)| **t).map(|(k, _)| k.clone()))
                    .unwrap_or_else(|| "-".to_string());
                RankDayItem {
                    rank: i + 1,
                    date,
                    tokens: tok,
                    cost_usd: cost,
                    top_model,
                }
            })
            .collect();

        let mut sorted_sessions: Vec<(String, u64)> = session_tokens.into_iter().collect();
        sorted_sessions.sort_by_key(|a| std::cmp::Reverse(a.1));

        let top_sessions: Vec<RankSessionItem> = sorted_sessions
            .into_iter()
            .take(10)
            .enumerate()
            .map(|(i, (session, tok))| {
                let cost = session_cost.get(&session).copied().unwrap_or(0.0);
                let (date, proj) = session_meta
                    .get(&session)
                    .cloned()
                    .unwrap_or_else(|| ("-".to_string(), "-".to_string()));
                let clean_session = sanitize_session_label(&session);
                let clean_proj = sanitize_project_label(&proj);
                RankSessionItem {
                    rank: i + 1,
                    session: clean_session,
                    date,
                    project: clean_proj,
                    tokens: tok,
                    cost_usd: cost,
                }
            })
            .collect();

        self.rank_out = RankLeaderboardOut {
            total_tokens,
            total_cost_usd: total_cost,
            top_models,
            top_projects,
            peak_days,
            top_sessions,
        };

        // 3. Daily / Timeline Report
        let mut daily_map: BTreeMap<String, DailyRow> = BTreeMap::new();
        for e in &self.current_events {
            let day_str = self.tz.date_of(e.timestamp).format("%Y-%m-%d").to_string();
            let row = daily_map
                .entry(day_str.clone())
                .or_insert_with(|| DailyRow {
                    date: day_str,
                    totals: TokenCounts::default(),
                    models: BTreeMap::new(),
                    sources: BTreeMap::new(),
                    models_by_source: BTreeMap::new(),
                    activity: None,
                });
            row.totals.add_assign(e.usage.to_counts());
            row.models
                .entry(e.model.clone())
                .or_default()
                .add_assign(e.usage.to_counts());
            let src = e.source.as_str().to_string();
            row.sources
                .entry(src.clone())
                .or_default()
                .add_assign(e.usage.to_counts());
            row.models_by_source
                .entry(src)
                .or_default()
                .insert(e.model.clone());
        }

        let mut daily_rows: Vec<DailyRow> = daily_map.into_values().collect();
        daily_rows.sort_by(|a, b| b.date.cmp(&a.date));

        let mut grand_totals = TokenCounts::default();
        for r in &daily_rows {
            grand_totals.add_assign(r.totals.clone());
        }

        self.insights = compute_report_insights(&daily_rows, &grand_totals, None);
        self.report = DailyReport {
            daily: daily_rows,
            totals: grand_totals,
            activity_totals: None,
            stats: ParseStats::default(),
            insights: Some(self.insights.clone()),
        };
    }
}

pub fn run_dashboard_tui(
    all_events: Vec<UsageEvent>,
    tz: TimeZoneMode,
    common: CommonArgs,
    initial_tab: DashboardTab,
    initial_period: ScalePeriodArg,
    project_filter: Option<String>,
) -> Result<()> {
    if !io::stdout().is_terminal() {
        bail!("--tui requires an interactive terminal");
    }

    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;

    let mut state = DashboardState::new(
        all_events,
        tz,
        common,
        initial_tab,
        initial_period,
        project_filter,
    );

    let res = run_dashboard_event_loop(&mut terminal, &mut state);

    let _ = terminal.show_cursor();
    let _ = crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen
    );
    let _ = crossterm::terminal::disable_raw_mode();

    res
}

fn run_dashboard_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut DashboardState,
) -> Result<()> {
    loop {
        terminal.draw(|frame| draw_dashboard(frame, state))?;

        if !event::poll(Duration::from_millis(150))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if state.show_help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter
            ) {
                state.show_help = false;
            }
            continue;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => break,
            KeyCode::Char('?') => state.show_help = true,

            // Tab switching
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                let tabs = DashboardTab::all();
                let idx = (state.selected_tab as usize + 1) % tabs.len();
                state.selected_tab = tabs[idx];
                state.scroll_offset = 0;
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                let tabs = DashboardTab::all();
                let idx = (state.selected_tab as usize + tabs.len() - 1) % tabs.len();
                state.selected_tab = tabs[idx];
                state.scroll_offset = 0;
            }
            KeyCode::Char('1') => {
                state.selected_tab = DashboardTab::ScaleOverview;
                state.scroll_offset = 0;
            }
            KeyCode::Char('2') => {
                state.selected_tab = DashboardTab::Leaderboard;
                state.scroll_offset = 0;
            }
            KeyCode::Char('3') => {
                state.selected_tab = DashboardTab::Breakdown;
                state.scroll_offset = 0;
            }
            KeyCode::Char('4') => {
                state.selected_tab = DashboardTab::Timeline;
                state.scroll_offset = 0;
            }

            // Period quick-switchers
            KeyCode::Char('a') => {
                state.period = ScalePeriodArg::All;
                state.recompute_filtered_data();
            }
            KeyCode::Char('m') => {
                if state.selected_tab == DashboardTab::Breakdown {
                    state.breakdown_by_model = !state.breakdown_by_model;
                } else {
                    state.period = ScalePeriodArg::Monthly;
                    state.recompute_filtered_data();
                }
            }
            KeyCode::Char('w') => {
                state.period = ScalePeriodArg::Weekly;
                state.recompute_filtered_data();
            }
            KeyCode::Char('d') => {
                state.period = ScalePeriodArg::Daily;
                state.recompute_filtered_data();
            }
            KeyCode::Char('t') => {
                state.period = ScalePeriodArg::Today;
                state.recompute_filtered_data();
            }

            // Scrolling
            KeyCode::Down | KeyCode::Char('j') => {
                state.scroll_offset = state.scroll_offset.saturating_add(1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.scroll_offset = state.scroll_offset.saturating_sub(1);
            }
            KeyCode::PageDown => {
                state.scroll_offset = state.scroll_offset.saturating_add(8);
            }
            KeyCode::PageUp => {
                state.scroll_offset = state.scroll_offset.saturating_sub(8);
            }
            KeyCode::Home => {
                state.scroll_offset = 0;
            }

            _ => {}
        }
    }

    Ok(())
}

fn draw_dashboard(frame: &mut ratatui::Frame<'_>, state: &DashboardState) {
    let area = frame.area();
    if area.width < 50 || area.height < 12 {
        let msg = Paragraph::new("Terminal window too small for dashboard")
            .style(Style::default().fg(Color::Yellow));
        frame.render_widget(msg, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Top Navigation Bar & Period
            Constraint::Min(8),    // Main Active Tab Content
            Constraint::Length(1), // Keybindings Footer
        ])
        .split(area);

    // 1. Top Header Tabs & Period Indicator
    draw_header(frame, chunks[0], state);

    // 2. Active Tab Content
    match state.selected_tab {
        DashboardTab::ScaleOverview => draw_scale_overview_tab(frame, chunks[1], state),
        DashboardTab::Leaderboard => draw_leaderboard_tab(frame, chunks[1], state),
        DashboardTab::Breakdown => draw_breakdown_tab(frame, chunks[1], state),
        DashboardTab::Timeline => draw_timeline_tab(frame, chunks[1], state),
    }

    // 3. Footer
    draw_footer(frame, chunks[2], state);

    // 4. Modal popup if Help is active
    if state.show_help {
        draw_help_modal(frame, area);
    }
}

fn draw_header(frame: &mut ratatui::Frame<'_>, area: Rect, state: &DashboardState) {
    let titles: Vec<Line> = DashboardTab::all()
        .iter()
        .map(|t| Line::from(Span::raw(t.title())))
        .collect();

    let tab_idx = state.selected_tab as usize;
    let period_str = match state.period {
        ScalePeriodArg::All => "All-Time (A)",
        ScalePeriodArg::Monthly => "Monthly (M)",
        ScalePeriodArg::Weekly => "Weekly (W)",
        ScalePeriodArg::Daily => "7-Days (D)",
        ScalePeriodArg::Today => "Today (T)",
    };

    let title_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            format!(
                " ⚡ tokenusage v{} · Timeframe: [{}] ",
                env!("CARGO_PKG_VERSION"),
                period_str
            ),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));

    let tabs = Tabs::new(titles)
        .block(title_block)
        .select(tab_idx)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        );

    frame.render_widget(tabs, area);
}

fn draw_footer(frame: &mut ratatui::Frame<'_>, area: Rect, state: &DashboardState) {
    let shortcuts = match state.selected_tab {
        DashboardTab::Breakdown => {
            "[Tab/1-4] Tabs  [a/m/w/d/t] Period  [m] Flip Matrix  [↑/↓/j/k] Scroll  [?] Help  [q] Quit"
        }
        _ => "[Tab/1-4] Tabs  [a/m/w/d/t] Period  [↑/↓/j/k] Scroll  [?] Help  [q] Quit",
    };
    let footer = Paragraph::new(shortcuts)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, area);
}

fn draw_scale_overview_tab(frame: &mut ratatui::Frame<'_>, area: Rect, state: &DashboardState) {
    let vertical_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // Spending & Efficiency + Drivers
            Constraint::Min(10),   // Active Generation vs Context Digestion
            Constraint::Length(5), // Milestones & Streaks
        ])
        .split(area);

    // Row 1: Economics & Provider Mix
    let row1_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(vertical_chunks[0]);

    // 💰 Spending Economics
    let cost_m = state.insights.cost_per_mtoken.unwrap_or(0.0);
    let savings = state.insights.cache_savings_usd.unwrap_or(0.0);

    let spend_text = vec![
        Line::from(vec![
            Span::styled("Total Cost: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("${:.2}", state.rank_out.total_cost_usd),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  (${:.2}/1M tok)", cost_m),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("Prompt Cache Saved: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("~${:.2}", savings),
                Style::default().fg(Color::Green),
            ),
            Span::styled(" vs uncached rates", Style::default().fg(Color::DarkGray)),
        ]),
    ];

    let spend_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" 💰 Spending & Economics ")
        .border_style(Style::default().fg(Color::Blue));

    let spend_para = Paragraph::new(spend_text).block(spend_block);
    frame.render_widget(spend_para, row1_chunks[0]);

    // 🏆 Drivers & Mix
    let top_src = state.insights.top_source.as_deref().unwrap_or("-");
    let top_mod = state.insights.top_model.as_deref().unwrap_or("-");
    let mod_share = state.insights.top_model_share_pct.unwrap_or(0.0);

    let driver_text = vec![
        Line::from(vec![
            Span::styled("Primary Provider: ", Style::default().fg(Color::Gray)),
            Span::styled(
                top_src,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Dominant Model:   ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{} ({:.1}%)", top_mod, mod_share),
                Style::default().fg(Color::Yellow),
            ),
        ]),
    ];

    let driver_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" 🏆 Drivers & Provider Mix ")
        .border_style(Style::default().fg(Color::Blue));

    let driver_para = Paragraph::new(driver_text).block(driver_block);
    frame.render_widget(driver_para, row1_chunks[1]);

    // Row 2: Human Scale (Active Creation vs Context Digested)
    let row2_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(vertical_chunks[1]);

    // Left: Active Generation
    let s = &state.scale;
    let create_lines = vec![
        Line::from(vec![
            Span::styled("Total Generated: ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!(
                    "{} tokens",
                    crate::carbon::format_commas_u64(s.output_tokens)
                ),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    " (~{} words)",
                    crate::carbon::format_commas_u64(s.output_words)
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("📚 Books Written:  ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("~{:.0} full novels", s.creation_books),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("   Harry Potter:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("~{:.1}× complete 7-book series", s.creation_hp_series),
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(vec![
            Span::styled("   Typing Effort:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "~{:.1} years non-stop (24/7 @ 80 wpm)",
                    s.creation_typing_years_247
                ),
                Style::default().fg(Color::LightCyan),
            ),
        ]),
    ];

    let create_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" ✍️ Active Creation (AI Output) ")
        .border_style(Style::default().fg(Color::Cyan));

    let create_para = Paragraph::new(create_lines).block(create_block);
    frame.render_widget(create_para, row2_chunks[0]);

    // Right: Context Digested
    let digest_lines = vec![
        Line::from(vec![
            Span::styled("Total Digested:  ", Style::default().fg(Color::Magenta)),
            Span::styled(
                format!(
                    "{} tokens",
                    crate::carbon::format_commas_u64(s.total_tokens)
                ),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    " (~{} words)",
                    crate::carbon::format_commas_u64(s.total_words)
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("🌐 World Knowledge:", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!(
                    " ~{:.1}× English Wikipedia (all 6.8M articles)",
                    s.digestion_wikipedia_multiples
                ),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("🏛️ Library Scope:  ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!(
                    " ~{:.1} public city libraries (40k vols)",
                    s.digestion_public_libraries
                ),
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(vec![
            Span::styled("📖 Reading Time:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    " ~{:.0} human reading years (8h/day @ 250 wpm)",
                    s.digestion_reading_years_8h
                ),
                Style::default().fg(Color::LightCyan),
            ),
        ]),
        Line::from(vec![
            Span::styled("📄 Physical Stack: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    " ~{:.2} km tall double-sided paper stack",
                    s.digestion_paper_stack_km
                ),
                Style::default().fg(Color::LightMagenta),
            ),
        ]),
    ];

    let digest_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" 🧠 Context Digested (World Scale) ")
        .border_style(Style::default().fg(Color::Magenta));

    let digest_para = Paragraph::new(digest_lines).block(digest_block);
    frame.render_widget(digest_para, row2_chunks[1]);

    // Row 3: Milestones & Records
    let mut milestone_spans = Vec::new();
    if let Some(spend) = &state.insights.peak_spend {
        milestone_spans.push(Span::styled(
            "Peak Spend: ",
            Style::default().fg(Color::Gray),
        ));
        milestone_spans.push(Span::styled(
            format!("{} (${:.2})  ", spend.date, spend.cost_usd),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(vol) = &state.insights.peak_period {
        milestone_spans.push(Span::styled(
            "Peak Volume: ",
            Style::default().fg(Color::Gray),
        ));
        milestone_spans.push(Span::styled(
            format!(
                "{} ({} tok)  ",
                vol.date,
                crate::carbon::format_commas_u64(vol.total_tokens)
            ),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(streak) = state.insights.current_streak_days {
        milestone_spans.push(Span::styled(
            "Active Streak: ",
            Style::default().fg(Color::Gray),
        ));
        milestone_spans.push(Span::styled(
            format!("{} days", streak),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let milestone_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" 📈 Records & Milestones ")
        .border_style(Style::default().fg(Color::DarkGray));

    let milestone_para = Paragraph::new(Line::from(milestone_spans)).block(milestone_block);
    frame.render_widget(milestone_para, vertical_chunks[2]);
}

fn draw_leaderboard_tab(frame: &mut ratatui::Frame<'_>, area: Rect, state: &DashboardState) {
    let col_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left Column: Top Models (Top) & Top Projects (Bottom)
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(col_chunks[0]);

    // 1. Top Models Table
    let model_rows = state.rank_out.top_models.iter().map(|m| {
        let badge = match m.rank {
            1 => "🥇",
            2 => "🥈",
            3 => "🥉",
            _ => "  ",
        };
        Row::new(vec![
            Cell::from(format!("{} {}", badge, m.rank)),
            Cell::from(m.model.clone()),
            Cell::from(crate::carbon::format_commas_u64(m.tokens)),
            Cell::from(format!("{:.1}%", m.share_pct)),
            Cell::from(format!("${:.2}", m.cost_usd)),
            Cell::from(Span::styled(
                m.tier.clone(),
                Style::default().fg(Color::Cyan),
            )),
        ])
    });

    let model_table = Table::new(
        model_rows,
        [
            Constraint::Length(6),
            Constraint::Min(16),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new(vec!["Rank", "Model", "Tokens", "Share", "Cost", "Tier"]).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" 🤖 Top Models (Volume & Spend) ")
            .border_style(Style::default().fg(Color::Cyan)),
    );

    frame.render_widget(model_table, left_chunks[0]);

    // 2. Top Projects Table
    let project_rows = state.rank_out.top_projects.iter().map(|p| {
        let badge = match p.rank {
            1 => "🥇",
            2 => "🥈",
            3 => "🥉",
            _ => "  ",
        };
        Row::new(vec![
            Cell::from(format!("{} {}", badge, p.rank)),
            Cell::from(p.project.clone()),
            Cell::from(crate::carbon::format_commas_u64(p.tokens)),
            Cell::from(format!("{:.1}%", p.share_pct)),
            Cell::from(format!("${:.2}", p.cost_usd)),
            Cell::from(p.primary_model.clone()),
        ])
    });

    let project_table = Table::new(
        project_rows,
        [
            Constraint::Length(6),
            Constraint::Min(16),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(14),
        ],
    )
    .header(
        Row::new(vec![
            "Rank",
            "Project",
            "Tokens",
            "Share",
            "Cost",
            "Primary Model",
        ])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" 🏗️ Top Projects (Consumption) ")
            .border_style(Style::default().fg(Color::Green)),
    );

    frame.render_widget(project_table, left_chunks[1]);

    // Right Column: Peak Days (Top) & Monster Sessions (Bottom)
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(col_chunks[1]);

    // 3. Peak Days Table
    let day_rows = state.rank_out.peak_days.iter().map(|d| {
        let badge = match d.rank {
            1 => "🥇",
            2 => "🥈",
            3 => "🥉",
            _ => "  ",
        };
        Row::new(vec![
            Cell::from(format!("{} {}", badge, d.rank)),
            Cell::from(d.date.clone()),
            Cell::from(crate::carbon::format_commas_u64(d.tokens)),
            Cell::from(format!("${:.2}", d.cost_usd)),
            Cell::from(d.top_model.clone()),
        ])
    });

    let day_table = Table::new(
        day_rows,
        [
            Constraint::Length(6),
            Constraint::Length(12),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Min(16),
        ],
    )
    .header(
        Row::new(vec!["Rank", "Date", "Tokens", "Cost", "Top Model"]).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" ⚡ All-Time Record Days ")
            .border_style(Style::default().fg(Color::Yellow)),
    );

    frame.render_widget(day_table, right_chunks[0]);

    // 4. Monster Sessions Table
    let sess_rows = state.rank_out.top_sessions.iter().map(|s| {
        let badge = match s.rank {
            1 => "🥇",
            2 => "🥈",
            3 => "🥉",
            _ => "  ",
        };
        Row::new(vec![
            Cell::from(format!("{} {}", badge, s.rank)),
            Cell::from(s.session.clone()),
            Cell::from(s.date.clone()),
            Cell::from(s.project.clone()),
            Cell::from(crate::carbon::format_commas_u64(s.tokens)),
            Cell::from(format!("${:.2}", s.cost_usd)),
        ])
    });

    let sess_table = Table::new(
        sess_rows,
        [
            Constraint::Length(6),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Min(14),
            Constraint::Length(12),
            Constraint::Length(9),
        ],
    )
    .header(
        Row::new(vec!["Rank", "Session", "Date", "Project", "Tokens", "Cost"]).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" 🔥 Monster Sessions ")
            .border_style(Style::default().fg(Color::Red)),
    );

    frame.render_widget(sess_table, right_chunks[1]);
}

fn draw_breakdown_tab(frame: &mut ratatui::Frame<'_>, area: Rect, state: &DashboardState) {
    let mode_title = if state.breakdown_by_model {
        " 📊 Models × Projects Stratification (Press [m] to flip to Projects × Models) "
    } else {
        " 📊 Projects × Models Stratification (Press [m] to flip to Models × Projects) "
    };

    let mut table_rows = Vec::new();

    if state.breakdown_by_model {
        // Models -> Projects
        let mut model_map: BreakdownMap = HashMap::new();
        for e in &state.current_events {
            let tok = e.usage.total_tokens();
            let cost = e.usage.cost_usd;
            let proj = sanitize_project_label(e.project.as_deref().unwrap_or("-"));
            let entry = model_map
                .entry(e.model.clone())
                .or_insert_with(|| (0, 0.0, HashMap::new()));
            entry.0 += tok;
            entry.1 += cost;
            let sub = entry.2.entry(proj).or_insert((0, 0.0));
            sub.0 += tok;
            sub.1 += cost;
        }

        let mut sorted: BreakdownEntries = model_map.into_iter().collect();
        sorted.sort_by_key(|a| std::cmp::Reverse((a.1).0));

        for (model_name, (m_tokens, m_cost, sub_map)) in sorted {
            table_rows.push(
                Row::new(vec![
                    Cell::from(Span::styled(
                        format!("📦 {}", model_name),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Cell::from(""),
                    Cell::from(Span::styled(
                        crate::carbon::format_commas_u64(m_tokens),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Cell::from("100.0%"),
                    Cell::from(Span::styled(
                        format!("${:.2}", m_cost),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )),
                ])
                .style(Style::default().bg(Color::Rgb(20, 30, 45))),
            );

            let mut sub_items: Vec<(String, (u64, f64))> = sub_map.into_iter().collect();
            sub_items.sort_by_key(|a| std::cmp::Reverse((a.1).0));

            for (proj_name, (p_tokens, p_cost)) in sub_items {
                let pct = if m_tokens > 0 {
                    (p_tokens as f64 / m_tokens as f64) * 100.0
                } else {
                    0.0
                };
                table_rows.push(Row::new(vec![
                    Cell::from(""),
                    Cell::from(format!("  └─ {}", proj_name)),
                    Cell::from(crate::carbon::format_commas_u64(p_tokens)),
                    Cell::from(format!("{:.1}%", pct)),
                    Cell::from(format!("${:.2}", p_cost)),
                ]));
            }
        }
    } else {
        // Projects -> Models
        let mut project_map: BreakdownMap = HashMap::new();
        for e in &state.current_events {
            let tok = e.usage.total_tokens();
            let cost = e.usage.cost_usd;
            let proj = sanitize_project_label(e.project.as_deref().unwrap_or("-"));
            let entry = project_map
                .entry(proj)
                .or_insert_with(|| (0, 0.0, HashMap::new()));
            entry.0 += tok;
            entry.1 += cost;
            let sub = entry.2.entry(e.model.clone()).or_insert((0, 0.0));
            sub.0 += tok;
            sub.1 += cost;
        }

        let mut sorted: BreakdownEntries = project_map.into_iter().collect();
        sorted.sort_by_key(|a| std::cmp::Reverse((a.1).0));

        for (proj_name, (p_tokens, p_cost, sub_map)) in sorted {
            table_rows.push(
                Row::new(vec![
                    Cell::from(Span::styled(
                        format!("📦 {}", proj_name),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Cell::from(""),
                    Cell::from(Span::styled(
                        crate::carbon::format_commas_u64(p_tokens),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Cell::from("100.0%"),
                    Cell::from(Span::styled(
                        format!("${:.2}", p_cost),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )),
                ])
                .style(Style::default().bg(Color::Rgb(30, 30, 30))),
            );

            let mut sub_items: Vec<(String, (u64, f64))> = sub_map.into_iter().collect();
            sub_items.sort_by_key(|a| std::cmp::Reverse((a.1).0));

            for (model_name, (m_tokens, m_cost)) in sub_items {
                let pct = if p_tokens > 0 {
                    (m_tokens as f64 / p_tokens as f64) * 100.0
                } else {
                    0.0
                };
                table_rows.push(Row::new(vec![
                    Cell::from(""),
                    Cell::from(format!("  └─ {}", model_name)),
                    Cell::from(crate::carbon::format_commas_u64(m_tokens)),
                    Cell::from(format!("{:.1}%", pct)),
                    Cell::from(format!("${:.2}", m_cost)),
                ]));
            }
        }
    }

    let skip_count = state.scroll_offset.min(table_rows.len().saturating_sub(1));
    let visible_rows = table_rows.into_iter().skip(skip_count);

    let table = Table::new(
        visible_rows,
        [
            Constraint::Length(28),
            Constraint::Min(24),
            Constraint::Length(16),
            Constraint::Length(10),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new(vec![
            "Group / Entity",
            "Sub-Item Breakdown",
            "Tokens",
            "Share",
            "Cost",
        ])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(mode_title)
            .border_style(Style::default().fg(Color::Cyan)),
    );

    frame.render_widget(table, area);
}

fn draw_timeline_tab(frame: &mut ratatui::Frame<'_>, area: Rect, state: &DashboardState) {
    let rows = state
        .report
        .daily
        .iter()
        .skip(state.scroll_offset)
        .map(|r| {
            let models_str = r
                .models
                .keys()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            Row::new(vec![
                Cell::from(r.date.clone()),
                Cell::from(models_str),
                Cell::from(crate::carbon::format_commas_u64(r.totals.input_tokens)),
                Cell::from(crate::carbon::format_commas_u64(r.totals.output_tokens)),
                Cell::from(crate::carbon::format_commas_u64(
                    r.totals.cache_read_input_tokens,
                )),
                Cell::from(crate::carbon::format_commas_u64(r.totals.total_tokens)),
                Cell::from(format!("${:.2}", r.totals.cost_usd)),
            ])
        });

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Min(20),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(vec![
            "Date",
            "Primary Models",
            "Input",
            "Output",
            "Cache Read",
            "Total Tokens",
            "Cost",
        ])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" 📅 Usage Timeline & Daily Records ")
            .border_style(Style::default().fg(Color::Cyan)),
    );

    frame.render_widget(table, area);
}

fn draw_help_modal(frame: &mut ratatui::Frame<'_>, area: Rect) {
    let popup_area = centered_rect(60, 50, area);
    frame.render_widget(Clear, popup_area);

    let help_text = vec![
        Line::from(Span::styled(
            "✨ Keyboard Shortcuts & Navigation",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  [Tab] / [Shift-Tab]  Switch active view tab"),
        Line::from("  [1] - [4]            Jump directly to tab 1-4"),
        Line::from(""),
        Line::from("  [a]                  All-Time Lifetime data"),
        Line::from("  [m]                  Monthly data (or toggle Matrix in Breakdown)"),
        Line::from("  [w]                  Weekly data (This week)"),
        Line::from("  [d]                  Daily data (Last 7 days)"),
        Line::from("  [t]                  Today's usage only"),
        Line::from(""),
        Line::from("  [↑] / [↓] / [j] / [k] Scroll tables & lists"),
        Line::from("  [PageUp] / [PageDown] Scroll by page"),
        Line::from("  [Home]               Jump to top"),
        Line::from(""),
        Line::from("  [?]                  Toggle this help popup"),
        Line::from("  [q] / [Esc]          Exit dashboard"),
    ];

    let help_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" ❓ Help & Navigation ")
        .border_style(Style::default().fg(Color::Yellow));

    let help_para = Paragraph::new(help_text).block(help_block);
    frame.render_widget(help_para, popup_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
