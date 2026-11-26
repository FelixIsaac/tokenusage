use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use crate::carbon::format_commas_u64;
use crate::cli::WrappedTheme;
use crate::output::TuiSession;
use crate::tui_theme::{
    ACCENT_AMBER, ACCENT_CYAN, ACCENT_EMERALD, ACCENT_ROSE, ACCENT_VIOLET, BG_CANVAS, BG_ELEVATED,
    BG_SURFACE, BORDER_FOCUS, BORDER_SUBTLE, HEATMAP_COLORS, TEXT_MUTED, TEXT_PRIMARY,
    TEXT_SECONDARY, card_block, centered_rect, ease_out_cubic, format_compact_tokens,
    format_compact_usd, keybar_line, provider_accent, render_big_text, render_halfblock_heatmap,
    render_segmented_bar, render_sparkline_24h,
};

use super::data::WrappedBundle;
use super::{html, open_in_default_viewer, png};

const SLIDE_ANIM_MS: u128 = 480;
const AUTO_ADVANCE_SECS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slide {
    Cover = 0,
    Rhythm = 1,
    Arsenal = 2,
    Cache = 3,
    ScaleImpact = 4,
    HallOfFame = 5,
    SharePoster = 6,
}

impl Slide {
    const ALL: [Slide; 7] = [
        Slide::Cover,
        Slide::Rhythm,
        Slide::Arsenal,
        Slide::Cache,
        Slide::ScaleImpact,
        Slide::HallOfFame,
        Slide::SharePoster,
    ];

    fn title(self) -> &'static str {
        match self {
            Slide::Cover => "01 · Cover",
            Slide::Rhythm => "02 · Rhythm",
            Slide::Arsenal => "03 · Arsenal",
            Slide::Cache => "04 · Cache Engine",
            Slide::ScaleImpact => "05 · Scale & Planet",
            Slide::HallOfFame => "06 · Hall of Fame",
            Slide::SharePoster => "07 · Share & Export",
        }
    }

    fn next(self) -> Self {
        let idx = (self as usize + 1) % Self::ALL.len();
        Self::ALL[idx]
    }

    fn prev(self) -> Self {
        let idx = (self as usize + Self::ALL.len() - 1) % Self::ALL.len();
        Self::ALL[idx]
    }
}

struct WrappedTuiState<'a> {
    bundle: &'a mut WrappedBundle,
    theme: WrappedTheme,
    slide: Slide,
    slide_entered_at: Instant,
    auto_play: bool,
    auto_tick: Instant,
    show_help: bool,
    toast: Option<(String, Instant)>,
}

impl<'a> WrappedTuiState<'a> {
    fn set_slide(&mut self, slide: Slide) {
        if self.slide != slide {
            self.slide = slide;
            self.slide_entered_at = Instant::now();
            self.auto_tick = Instant::now();
        }
    }

    fn anim_factor(&self) -> f64 {
        let elapsed = self.slide_entered_at.elapsed().as_millis();
        if elapsed >= SLIDE_ANIM_MS {
            1.0
        } else {
            ease_out_cubic(elapsed as f64 / SLIDE_ANIM_MS as f64)
        }
    }

    fn is_animating(&self) -> bool {
        self.slide_entered_at.elapsed().as_millis() < SLIDE_ANIM_MS
    }

    fn set_toast(&mut self, msg: String) {
        self.toast = Some((msg, Instant::now()));
    }
}

pub(crate) fn run_wrapped_tui(bundle: &mut WrappedBundle, theme: WrappedTheme) -> Result<()> {
    if !io::stdout().is_terminal() {
        bail!(
            "tu wrapped TUI requires an interactive terminal (use --no-tui, --json, or --html/--png)"
        );
    }

    let mut session = TuiSession::enter()?;
    let now = Instant::now();
    let mut state = WrappedTuiState {
        bundle,
        theme,
        slide: Slide::Cover,
        slide_entered_at: now,
        auto_play: false,
        auto_tick: now,
        show_help: false,
        toast: None,
    };

    loop {
        if let Some((_, created)) = state.toast
            && created.elapsed() > Duration::from_secs(4)
        {
            state.toast = None;
        }

        if state.auto_play && state.auto_tick.elapsed() >= Duration::from_secs(AUTO_ADVANCE_SECS) {
            let next = state.slide.next();
            state.set_slide(next);
        }

        session
            .terminal
            .draw(|frame| draw_wrapped_frame(frame, &state))?;

        let poll_ms = if state.is_animating() || state.auto_play {
            33
        } else {
            200
        };
        if !event::poll(Duration::from_millis(poll_ms))? {
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
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') | KeyCode::Enter => {
                state.auto_play = false;
                state.set_slide(state.slide.next());
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => {
                state.auto_play = false;
                state.set_slide(state.slide.prev());
            }
            KeyCode::Char('1') => {
                state.auto_play = false;
                state.set_slide(Slide::Cover);
            }
            KeyCode::Char('2') => {
                state.auto_play = false;
                state.set_slide(Slide::Rhythm);
            }
            KeyCode::Char('3') => {
                state.auto_play = false;
                state.set_slide(Slide::Arsenal);
            }
            KeyCode::Char('4') => {
                state.auto_play = false;
                state.set_slide(Slide::Cache);
            }
            KeyCode::Char('5') => {
                state.auto_play = false;
                state.set_slide(Slide::ScaleImpact);
            }
            KeyCode::Char('6') => {
                state.auto_play = false;
                state.set_slide(Slide::HallOfFame);
            }
            KeyCode::Char('7') => {
                state.auto_play = false;
                state.set_slide(Slide::SharePoster);
            }
            KeyCode::Char('a') => {
                state.auto_play = !state.auto_play;
                state.auto_tick = Instant::now();
                state.set_toast(if state.auto_play {
                    "Auto-play ON (5s per slide)".to_string()
                } else {
                    "Auto-play OFF".to_string()
                });
            }
            KeyCode::Char('p') => {
                let next = !state.bundle.report.include_projects;
                state.bundle.set_include_projects(next);
                state.set_toast(if next {
                    "Project & session names UNREDACTED (local view)".to_string()
                } else {
                    "Privacy ON: project & session names redacted".to_string()
                });
            }
            KeyCode::Char('e') => {
                let period_slug = state
                    .bundle
                    .report
                    .period_label
                    .to_ascii_lowercase()
                    .replace(|c: char| !c.is_ascii_alphanumeric(), "-");
                let html_path = PathBuf::from(format!("tokenusage-wrapped-{period_slug}.html"));
                let png_path = PathBuf::from(format!("tokenusage-wrapped-{period_slug}.png"));
                let res_html =
                    html::write_wrapped_html(&html_path, &state.bundle.report, state.theme);
                let res_png = png::write_wrapped_png(&png_path, &state.bundle.report, state.theme);
                match (res_html, res_png) {
                    (Ok(()), Ok(())) => {
                        open_in_default_viewer(&html_path);
                        state.set_toast(format!(
                            "Exported {} + {}",
                            html_path.display(),
                            png_path.display()
                        ));
                    }
                    (Err(err), _) | (_, Err(err)) => {
                        state.set_toast(format!("Export failed: {err}"));
                    }
                }
            }
            _ => {}
        }
    }

    drop(session);
    Ok(())
}

fn draw_wrapped_frame(frame: &mut ratatui::Frame<'_>, state: &WrappedTuiState<'_>) {
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(BG_CANVAS)), area);

    if area.width < 64 || area.height < 18 {
        let msg = Paragraph::new(
            "Terminal too small for tu wrapped (needs at least 64×18). Resize or pass --no-tui / --html.",
        )
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(ACCENT_AMBER).bg(BG_CANVAS));
        frame.render_widget(msg, area);
        return;
    }

    let [header_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(12),
        Constraint::Length(2),
    ])
    .areas(area);

    draw_header(frame, header_area, state);

    match state.slide {
        Slide::Cover => draw_slide_cover(frame, body_area, state),
        Slide::Rhythm => draw_slide_rhythm(frame, body_area, state),
        Slide::Arsenal => draw_slide_arsenal(frame, body_area, state),
        Slide::Cache => draw_slide_cache(frame, body_area, state),
        Slide::ScaleImpact => draw_slide_scale(frame, body_area, state),
        Slide::HallOfFame => draw_slide_hall_of_fame(frame, body_area, state),
        Slide::SharePoster => draw_slide_share(frame, body_area, state),
    }

    draw_footer(frame, footer_area, state);

    if state.show_help {
        draw_help_popup(frame, area);
    }
}

fn draw_header(frame: &mut ratatui::Frame<'_>, area: Rect, state: &WrappedTuiState<'_>) {
    let report = &state.bundle.report;
    let privacy_pill = if report.include_projects {
        Span::styled(
            " PROJECTS VISIBLE ",
            Style::default()
                .bg(ACCENT_AMBER)
                .fg(BG_CANVAS)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " PRIVACY SAFE ",
            Style::default()
                .bg(BG_ELEVATED)
                .fg(ACCENT_EMERALD)
                .add_modifier(Modifier::BOLD),
        )
    };

    let auto_span = if state.auto_play {
        Span::styled(
            " ▶ AUTO ",
            Style::default()
                .bg(ACCENT_VIOLET)
                .fg(BG_CANVAS)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };

    let mut dots = Vec::with_capacity(Slide::ALL.len());
    for s in Slide::ALL {
        let active = s == state.slide;
        dots.push(Span::styled(
            if active { " ● " } else { " ○ " },
            Style::default().fg(if active { ACCENT_CYAN } else { TEXT_MUTED }),
        ));
    }

    let mut row = vec![
        Span::styled(
            format!(" ✦ TOKENUSAGE WRAPPED {} ", report.period_label),
            Style::default()
                .fg(ACCENT_CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(TEXT_MUTED)),
        Span::styled(
            state.slide.title(),
            Style::default()
                .fg(TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        privacy_pill,
        Span::raw(" "),
        auto_span,
        Span::raw("  "),
    ];
    row.extend(dots);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(BG_SURFACE))
        .border_style(Style::default().fg(BORDER_SUBTLE));

    frame.render_widget(Paragraph::new(Line::from(row)).block(block), area);
}

fn draw_footer(frame: &mut ratatui::Frame<'_>, area: Rect, state: &WrappedTuiState<'_>) {
    let line = if let Some((msg, _)) = &state.toast {
        Line::from(vec![
            Span::styled(
                " ✓ ",
                Style::default()
                    .fg(ACCENT_EMERALD)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(msg.clone(), Style::default().fg(TEXT_PRIMARY)),
        ])
    } else {
        keybar_line(&[
            ("←/→", "Slide"),
            ("1-7", "Jump"),
            ("e", "Export HTML+PNG"),
            ("p", "Toggle Privacy"),
            ("a", "Autoplay"),
            ("?", "Help"),
            ("q", "Quit"),
        ])
    };
    let footer = Paragraph::new(line)
        .alignment(Alignment::Center)
        .style(Style::default().bg(BG_CANVAS));
    frame.render_widget(footer, area);
}

fn draw_slide_cover(frame: &mut ratatui::Frame<'_>, area: Rect, state: &WrappedTuiState<'_>) {
    let report = &state.bundle.report;
    let f = state.anim_factor();
    let anim_tokens = ((report.totals.total_tokens as f64) * f).round() as u64;
    let anim_cost = report.totals.cost_usd * f;
    let anim_days = ((report.active_days as f64) * f).round() as u32;
    let anim_sessions = ((report.total_sessions as f64) * f).round() as u64;

    let [hero_area, kpi_area] =
        Layout::vertical([Constraint::Min(10), Constraint::Length(6)]).areas(area);

    let hero_str = format_compact_tokens(anim_tokens);
    let mut hero_lines = vec![
        Line::from(Span::styled(
            format!("YOUR {} AI ENGINEERING REWIND", report.period_label),
            Style::default()
                .fg(TEXT_SECONDARY)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    hero_lines.extend(render_big_text(&hero_str, ACCENT_CYAN));
    hero_lines.push(Line::from(""));
    hero_lines.push(Line::from(vec![
        Span::styled(
            " ARCHETYPE ",
            Style::default()
                .bg(ACCENT_AMBER)
                .fg(BG_CANVAS)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}  ", report.archetype.title),
            Style::default()
                .fg(TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", report.archetype.tier),
            Style::default()
                .bg(BG_ELEVATED)
                .fg(ACCENT_CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", report.archetype.tagline),
            Style::default().fg(TEXT_SECONDARY),
        ),
    ]));
    hero_lines.push(Line::from(Span::styled(
        report.archetype.reason.clone(),
        Style::default().fg(TEXT_MUTED),
    )));

    let hero = Paragraph::new(hero_lines)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(card_block("Headline Volume", true));
    frame.render_widget(hero, hero_area);

    let cols = Layout::horizontal([
        Constraint::Percentage(25),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
    ])
    .split(kpi_area);

    render_stat_tile(
        frame,
        cols[0],
        "API Value",
        format_compact_usd(anim_cost),
        format!(
            "Saved ~{}",
            format_compact_usd(report.cache.savings_usd * f)
        ),
        ACCENT_EMERALD,
    );
    render_stat_tile(
        frame,
        cols[1],
        "Active Days",
        format!("{anim_days}d"),
        format!("Longest streak: {}d", report.longest_streak_days),
        ACCENT_AMBER,
    );
    render_stat_tile(
        frame,
        cols[2],
        "Sessions",
        format_commas_u64(anim_sessions),
        format!(
            "{} events logged",
            format_compact_tokens(report.total_events)
        ),
        ACCENT_VIOLET,
    );
    render_stat_tile(
        frame,
        cols[3],
        "Chronotype",
        report.chronotype.label.clone(),
        format!(
            "Peak {:02}:00 · {:.0}% night",
            report.chronotype.peak_hour, report.chronotype.night_pct
        ),
        ACCENT_ROSE,
    );
}

fn draw_slide_rhythm(frame: &mut ratatui::Frame<'_>, area: Rect, state: &WrappedTuiState<'_>) {
    let report = &state.bundle.report;
    let [heatmap_area, lower_area] =
        Layout::vertical([Constraint::Length(9), Constraint::Min(8)]).areas(area);

    let mut hm_lines = vec![Line::from(vec![
        Span::styled("Less ", Style::default().fg(TEXT_MUTED)),
        Span::styled("▪", Style::default().fg(HEATMAP_COLORS[0])),
        Span::styled("▪", Style::default().fg(HEATMAP_COLORS[1])),
        Span::styled("▪", Style::default().fg(HEATMAP_COLORS[2])),
        Span::styled("▪", Style::default().fg(HEATMAP_COLORS[3])),
        Span::styled("▪", Style::default().fg(HEATMAP_COLORS[4])),
        Span::styled(" More   ", Style::default().fg(TEXT_MUTED)),
        Span::styled(
            format!(
                "{} → {}   ({} active days · max {}/day)",
                report.heatmap.start_date,
                report.heatmap.end_date,
                report.active_days,
                format_compact_tokens(report.heatmap.max_day_tokens)
            ),
            Style::default().fg(TEXT_SECONDARY),
        ),
    ])];
    hm_lines.push(Line::from(""));
    hm_lines.extend(render_halfblock_heatmap(&report.heatmap.grid));

    frame.render_widget(
        Paragraph::new(hm_lines)
            .alignment(Alignment::Center)
            .block(card_block(
                "Contribution Pulse (Half-Block 53×7 Grid)",
                true,
            )),
        heatmap_area,
    );

    let [left, right] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .areas(lower_area);

    let mut left_lines = vec![
        Line::from(Span::styled(
            "24-Hour Circadian Token Velocity (00:00 → 23:00)",
            Style::default().fg(TEXT_SECONDARY),
        )),
        Line::from(""),
        render_sparkline_24h(&report.hourly_tokens, ACCENT_CYAN),
        Line::from(Span::styled(
            "00      04      08      12      16      20      23",
            Style::default().fg(TEXT_MUTED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Chronotype: ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(
                format!("{} ({})", report.chronotype.label, report.chronotype.window),
                Style::default()
                    .fg(ACCENT_AMBER)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            report.chronotype.detail.clone(),
            Style::default().fg(TEXT_MUTED),
        )),
    ];
    if let Some(peak) = &report.peak_day {
        left_lines.push(Line::from(""));
        left_lines.push(Line::from(vec![
            Span::styled("Peak Single Day: ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(
                format!(
                    "{} · {} ({})",
                    peak.date,
                    format_compact_tokens(peak.tokens),
                    peak.top_model
                ),
                Style::default().fg(ACCENT_EMERALD),
            ),
        ]));
    }
    frame.render_widget(
        Paragraph::new(left_lines)
            .wrap(Wrap { trim: true })
            .block(card_block("Circadian Rhythm", false)),
        left,
    );

    const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let max_wd = report
        .weekday_tokens
        .iter()
        .copied()
        .max()
        .unwrap_or(1)
        .max(1);
    let mut right_lines = vec![Line::from(Span::styled(
        format!("Weekend share: {:.1}%", report.weekend_share_pct),
        Style::default().fg(ACCENT_VIOLET),
    ))];
    for (idx, &tok) in report.weekday_tokens.iter().enumerate() {
        let pct = (tok as f64 / max_wd as f64) * 100.0;
        let color = if idx >= 5 { ACCENT_VIOLET } else { ACCENT_CYAN };
        let bar = render_segmented_bar(pct, 18, color);
        let mut spans = vec![Span::styled(
            format!("{:<4}", DAY_NAMES[idx]),
            Style::default().fg(TEXT_SECONDARY),
        )];
        spans.extend(bar.spans);
        spans.push(Span::styled(
            format!(" {}", format_compact_tokens(tok)),
            Style::default().fg(TEXT_PRIMARY),
        ));
        right_lines.push(Line::from(spans));
    }
    frame.render_widget(
        Paragraph::new(right_lines).block(card_block("Day-of-Week Balance", false)),
        right,
    );
}

fn draw_slide_arsenal(frame: &mut ratatui::Frame<'_>, area: Rect, state: &WrappedTuiState<'_>) {
    let report = &state.bundle.report;
    let [eras_area, main_area] =
        Layout::vertical([Constraint::Length(7), Constraint::Min(10)]).areas(area);

    let era_cols = Layout::horizontal([
        Constraint::Percentage(25),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
    ])
    .split(eras_area);

    for (idx, col) in era_cols.iter().enumerate() {
        if let Some(era) = report.quarterly_eras.get(idx) {
            let color = provider_accent(&era.top_provider);
            let lines = vec![
                Line::from(Span::styled(
                    era.title.clone(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    era.top_model.clone(),
                    Style::default().fg(TEXT_PRIMARY),
                )),
                Line::from(Span::styled(
                    format!(
                        "{} · {:.0}%",
                        format_compact_tokens(era.tokens),
                        era.share_pct
                    ),
                    Style::default().fg(TEXT_SECONDARY),
                )),
            ];
            frame.render_widget(
                Paragraph::new(lines).block(card_block(&era.quarter, era.tokens > 0)),
                *col,
            );
        }
    }

    let [prov_area, model_area] =
        Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
            .areas(main_area);

    let mut prov_lines = Vec::new();
    for p in report.top_providers.iter().take(6) {
        let color = provider_accent(&p.name);
        prov_lines.push(Line::from(vec![
            Span::styled(
                format!("{:<13}", p.name),
                Style::default()
                    .fg(TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>7} ", format_compact_tokens(p.tokens)),
                Style::default().fg(color),
            ),
            Span::styled(
                format!("({:>4.1}% · {}d)", p.share_pct, p.active_days),
                Style::default().fg(TEXT_MUTED),
            ),
        ]));
        prov_lines.push(render_segmented_bar(p.share_pct, 28, color));
    }
    frame.render_widget(
        Paragraph::new(prov_lines).block(card_block("Provider Mix", false)),
        prov_area,
    );

    let mut model_lines = Vec::new();
    for (idx, m) in report.top_models.iter().take(7).enumerate() {
        let color = provider_accent(&m.raw_name);
        model_lines.push(Line::from(vec![
            Span::styled(format!("#{} ", idx + 1), Style::default().fg(ACCENT_AMBER)),
            Span::styled(
                format!("{:<26}", truncate_text(&m.name, 26)),
                Style::default().fg(TEXT_PRIMARY),
            ),
            Span::styled(
                format!("{:>7}", format_compact_tokens(m.tokens)),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "  {:>4.1}%  {}",
                    m.share_pct,
                    format_compact_usd(m.cost_usd)
                ),
                Style::default().fg(TEXT_SECONDARY),
            ),
        ]));
    }
    frame.render_widget(
        Paragraph::new(model_lines).block(card_block("Top Models", true)),
        model_area,
    );
}

fn draw_slide_cache(frame: &mut ratatui::Frame<'_>, area: Rect, state: &WrappedTuiState<'_>) {
    let report = &state.bundle.report;
    let f = state.anim_factor();
    let anim_savings = report.cache.savings_usd * f;

    let [hero_area, detail_area] =
        Layout::vertical([Constraint::Length(10), Constraint::Min(8)]).areas(area);

    let mut hero_lines = vec![
        Line::from(Span::styled(
            "ESTIMATED PROMPT CACHE SAVINGS",
            Style::default()
                .fg(TEXT_SECONDARY)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    hero_lines.extend(render_big_text(
        &format_compact_usd(anim_savings),
        ACCENT_EMERALD,
    ));
    frame.render_widget(
        Paragraph::new(hero_lines)
            .alignment(Alignment::Center)
            .block(card_block("The Cache Engine", true)),
        hero_area,
    );

    let [left, right] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .areas(detail_area);

    let max_cost = report
        .cache
        .uncached_cost_usd
        .max(report.totals.cost_usd)
        .max(0.01);
    let actual_pct = (report.totals.cost_usd / max_cost) * 100.0;
    let left_lines = vec![
        Line::from(vec![
            Span::styled("Without Caching: ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(
                format_compact_usd(report.cache.uncached_cost_usd),
                Style::default()
                    .fg(ACCENT_ROSE)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        render_segmented_bar(100.0, 34, ACCENT_ROSE),
        Line::from(""),
        Line::from(vec![
            Span::styled("Actual API Cost: ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(
                format_compact_usd(report.totals.cost_usd),
                Style::default()
                    .fg(ACCENT_EMERALD)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        render_segmented_bar(actual_pct, 34, ACCENT_EMERALD),
    ];
    frame.render_widget(
        Paragraph::new(left_lines).block(card_block("Counterfactual Bill Comparison", false)),
        left,
    );

    let right_lines = vec![
        Line::from(vec![
            Span::styled(
                "Prompt Cache Hit Rate : ",
                Style::default().fg(TEXT_SECONDARY),
            ),
            Span::styled(
                format!("{:.1}%", report.cache.hit_pct),
                Style::default()
                    .fg(ACCENT_CYAN)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "Cache Reuse Ratio     : ",
                Style::default().fg(TEXT_SECONDARY),
            ),
            Span::styled(
                format!("{:.1}×", report.cache.reuse_ratio),
                Style::default()
                    .fg(ACCENT_AMBER)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "Cache Reads           : ",
                Style::default().fg(TEXT_SECONDARY),
            ),
            Span::styled(
                format_compact_tokens(report.cache.cache_read_tokens),
                Style::default().fg(TEXT_PRIMARY),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "Cache Creation        : ",
                Style::default().fg(TEXT_SECONDARY),
            ),
            Span::styled(
                format_compact_tokens(report.cache.cache_write_tokens),
                Style::default().fg(TEXT_PRIMARY),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Coding-agent norm: ~91–96.6% aggregate hit (98%+ = elite)",
            Style::default().fg(TEXT_MUTED),
        )),
    ];
    frame.render_widget(
        Paragraph::new(right_lines).block(card_block("Efficiency Telemetry", false)),
        right,
    );
}

fn draw_slide_scale(frame: &mut ratatui::Frame<'_>, area: Rect, state: &WrappedTuiState<'_>) {
    let report = &state.bundle.report;
    let [top_row, bot_row] =
        Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)]).areas(area);

    let scale = &report.scale;
    let scale_lines = vec![
        Line::from(Span::styled(
            "ACTIVE CREATION (OUTPUT TOKENS)",
            Style::default()
                .fg(ACCENT_AMBER)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            report.pop_culture_scale.clone(),
            Style::default()
                .fg(TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            scale.creation_headline(),
            Style::default().fg(TEXT_SECONDARY),
        )),
        Line::from(Span::styled(
            format!(
                "  {} words · {:.0} pages · {:.1}h @ 80 WPM",
                format_commas_u64(scale.output_words),
                scale.creation_pages,
                scale.creation_typing_hours
            ),
            Style::default().fg(TEXT_MUTED),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "TOTAL COGITATION (ALL TOKENS DIGESTED)",
            Style::default()
                .fg(ACCENT_CYAN)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            report.scale_headline.clone(),
            Style::default().fg(TEXT_PRIMARY),
        )),
        Line::from(Span::styled(
            format!(
                "  {:.2}× Wikipedia · {:.1}× Shakespeare · {:.1}m paper stack",
                scale.digestion_wikipedia_multiples,
                scale.digestion_shakespeare_works,
                scale.digestion_paper_stack_km * 1000.0
            ),
            Style::default().fg(TEXT_MUTED),
        )),
    ];
    frame.render_widget(
        Paragraph::new(scale_lines)
            .wrap(Wrap { trim: true })
            .block(card_block("Human Information Scale", true)),
        top_row,
    );

    let c = &report.carbon;
    let carbon_lines = vec![
        Line::from(vec![
            Span::styled("Estimated Energy   : ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(
                format!("{:.2} kWh", c.energy_kwh),
                Style::default()
                    .fg(ACCENT_AMBER)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Phone Charges      : ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(
                format!(
                    "~{}",
                    format_commas_u64(c.smartphone_charges.round() as u64)
                ),
                Style::default().fg(ACCENT_CYAN),
            ),
        ]),
        Line::from(vec![
            Span::styled("Tree-Months Offset : ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(
                format!("{:.1} tree-months", c.tree_months),
                Style::default().fg(ACCENT_EMERALD),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            c.summary.clone(),
            Style::default().fg(TEXT_MUTED),
        )),
    ];
    frame.render_widget(
        Paragraph::new(carbon_lines)
            .wrap(Wrap { trim: true })
            .block(card_block("Energy & Planet Footprint", false)),
        bot_row,
    );
}

fn draw_slide_hall_of_fame(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    state: &WrappedTuiState<'_>,
) {
    let report = &state.bundle.report;
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)]).areas(area);

    let mut badge_lines = Vec::new();
    if report.badges.is_empty() {
        badge_lines.push(Line::from(Span::styled(
            "Keep shipping to unlock achievements!",
            Style::default().fg(TEXT_MUTED),
        )));
    } else {
        for b in &report.badges {
            badge_lines.push(Line::from(vec![
                Span::styled(format!("{} ", b.glyph), Style::default().fg(ACCENT_AMBER)),
                Span::styled(
                    b.title.clone(),
                    Style::default()
                        .fg(TEXT_PRIMARY)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            badge_lines.push(Line::from(Span::styled(
                format!("     {}", b.detail),
                Style::default().fg(TEXT_SECONDARY),
            )));
            badge_lines.push(Line::from(""));
        }
    }
    frame.render_widget(
        Paragraph::new(badge_lines)
            .wrap(Wrap { trim: true })
            .block(card_block("Unlocked Badges", true)),
        left,
    );

    let mut rec_lines = Vec::new();
    if let Some(d) = &report.peak_day {
        rec_lines.push(Line::from(vec![
            Span::styled("Highest Volume Day : ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(
                format!(
                    "{} ({} · {})",
                    d.date,
                    format_compact_tokens(d.tokens),
                    d.top_model
                ),
                Style::default().fg(ACCENT_CYAN),
            ),
        ]));
    }
    if let Some(d) = &report.peak_spend_day {
        rec_lines.push(Line::from(vec![
            Span::styled("Peak Spend Day     : ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(
                format!("{} ({})", d.date, format_compact_usd(d.cost_usd)),
                Style::default().fg(ACCENT_EMERALD),
            ),
        ]));
    }
    if let Some(s) = &report.peak_session {
        let label = s.label.as_deref().unwrap_or("[redacted · press p]");
        rec_lines.push(Line::from(vec![
            Span::styled("Marathon Session   : ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(
                format!(
                    "{} · {} ({})",
                    format_compact_tokens(s.tokens),
                    s.date,
                    label
                ),
                Style::default().fg(ACCENT_AMBER),
            ),
        ]));
    }

    if report.include_projects && !report.top_projects.is_empty() {
        rec_lines.push(Line::from(""));
        rec_lines.push(Line::from(Span::styled(
            "Top Projects (p toggles privacy):",
            Style::default()
                .fg(ACCENT_VIOLET)
                .add_modifier(Modifier::BOLD),
        )));
        for p in report.top_projects.iter().take(4) {
            rec_lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:<24}", truncate_text(&p.name, 24)),
                    Style::default().fg(TEXT_PRIMARY),
                ),
                Span::styled(
                    format!(" {} ({:.1}%)", format_compact_tokens(p.tokens), p.share_pct),
                    Style::default().fg(TEXT_SECONDARY),
                ),
            ]));
        }
    } else {
        rec_lines.push(Line::from(""));
        rec_lines.push(Line::from(Span::styled(
            "Project & session identifiers are redacted by default (press [p] to preview).",
            Style::default().fg(TEXT_MUTED),
        )));
    }

    frame.render_widget(
        Paragraph::new(rec_lines)
            .wrap(Wrap { trim: true })
            .block(card_block("Personal Records", false)),
        right,
    );
}

fn draw_slide_share(frame: &mut ratatui::Frame<'_>, area: Rect, state: &WrappedTuiState<'_>) {
    let report = &state.bundle.report;
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);

    let blurb_lines = report
        .share_blurb
        .lines()
        .map(|line| {
            Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(TEXT_PRIMARY),
            ))
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(blurb_lines)
            .wrap(Wrap { trim: false })
            .block(card_block("Ready-to-Share Social Card", true)),
        left,
    );

    let action_lines = vec![
        Line::from(Span::styled(
            "SHARE & VIRAL WORKFLOW",
            Style::default()
                .fg(ACCENT_CYAN)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " [e] ",
                Style::default()
                    .bg(ACCENT_CYAN)
                    .fg(BG_CANVAS)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " Export interactive HTML micro-site + 1200×1500 PNG poster & open in browser",
                Style::default().fg(TEXT_PRIMARY),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " CLI ",
                Style::default()
                    .bg(BG_ELEVATED)
                    .fg(ACCENT_AMBER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" tu wrapped --prompt", Style::default().fg(ACCENT_AMBER)),
            Span::styled(
                "  prints a ready-to-paste AI roast/story prompt with your anonymized metrics.",
                Style::default().fg(TEXT_SECONDARY),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!(
                "Timezone: {} · Privacy: {}",
                report.timezone,
                if report.include_projects {
                    "Projects shown"
                } else {
                    "Redacted"
                }
            ),
            Style::default().fg(TEXT_MUTED),
        )),
    ];

    frame.render_widget(
        Paragraph::new(action_lines)
            .wrap(Wrap { trim: true })
            .block(card_block("One-Click Export", false)),
        right,
    );
}

fn render_stat_tile(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    title: &str,
    big_value: String,
    subtitle: String,
    color: ratatui::style::Color,
) {
    let lines = vec![
        Line::from(Span::styled(
            big_value,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(subtitle, Style::default().fg(TEXT_MUTED))),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(card_block(title, false)),
        area,
    );
}

fn draw_help_popup(frame: &mut ratatui::Frame<'_>, area: Rect) {
    let popup = centered_rect(58, 52, area);
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from(Span::styled(
            "tu wrapped — Controls",
            Style::default()
                .fg(ACCENT_CYAN)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  [←]/[→] / [Space]    Previous / Next slide"),
        Line::from("  [1] - [7]            Jump straight to slide 1–7"),
        Line::from("  [e]                  Export HTML micro-site + PNG poster"),
        Line::from("  [p]                  Toggle project & session privacy"),
        Line::from("  [a]                  Toggle 5-second auto-advance"),
        Line::from("  [?]                  Close help"),
        Line::from("  [q] / [Esc]          Quit presentation"),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(BG_SURFACE).fg(TEXT_PRIMARY))
        .border_style(Style::default().fg(BORDER_FOCUS))
        .title(" Help ");
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn truncate_text(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}
