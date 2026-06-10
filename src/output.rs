use std::collections::BTreeMap;
use std::io::{self, IsTerminal};
use std::time::Duration;

use anyhow::{Result, bail};
use comfy_table::{
    Attribute, Cell, Color, ContentArrangement, Row, Table, modifiers::UTF8_ROUND_CORNERS,
    presets::UTF8_FULL,
};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color as TuiColor, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Row as TuiRow, Table as TuiTable, Wrap};
use terminal_size::{Width, terminal_size};

use crate::ReportInsights;
use crate::types::{DailyReport, DailyRow, TableLayout, TokenCounts};
use std::collections::BTreeSet;

pub(crate) fn print_report_table_with_options(
    report: &DailyReport,
    force_compact: bool,
    show_breakdown: bool,
) {
    let terminal_width = detect_terminal_width();
    let show_activity = report_has_activity(report);
    let total_models_cell = report_unique_model_count_by_source_multiline(report);
    let layout = if force_compact {
        TableLayout::Compact
    } else {
        choose_layout(report, show_activity, show_breakdown, terminal_width)
    };
    let mut daily_table = create_table(terminal_width);
    set_layout_header(&mut daily_table, layout, show_activity);

    for row in &report.daily {
        daily_table.add_row(primary_row(row, layout, show_activity));
        if show_breakdown {
            add_breakdown_rows(&mut daily_table, row, layout, show_activity);
        }
    }

    daily_table.add_row(layout_total_row(
        report,
        layout,
        show_activity,
        &total_models_cell,
    ));
    println!("{daily_table}");

    if let Some(insights) = report.insights.as_ref() {
        print_report_insights(insights);
    }
}

/// Column header titles for a layout, in render order. The matching value cells
/// are produced by `primary_row` / `layout_total_row` in the same order.
fn layout_columns(layout: TableLayout, show_activity: bool) -> Vec<&'static str> {
    let mut cols = vec!["Date", "Models"];
    if show_activity {
        cols.push("Coding");
    }
    match layout {
        TableLayout::Compact => {}
        TableLayout::Standard => {
            if show_activity {
                cols.push("Tok/hr");
            } else {
                cols.push("Input");
                cols.push("Output");
            }
        }
        TableLayout::Full => {
            if show_activity {
                cols.push("Tok/hr");
            }
            cols.push("Input");
            cols.push("Output");
            cols.push("Cache Create");
            cols.push("Cache Read");
        }
    }
    cols.push("Total Tokens");
    cols.push("Cost (USD)");
    cols
}

fn set_layout_header(table: &mut Table, layout: TableLayout, show_activity: bool) {
    table.set_header(
        layout_columns(layout, show_activity)
            .into_iter()
            .map(|title| header_cell(title, Color::Cyan))
            .collect::<Vec<_>>(),
    );
}

fn layout_total_row(
    report: &DailyReport,
    layout: TableLayout,
    show_activity: bool,
    total_models_cell: &str,
) -> Row {
    let bold = |text: String| {
        Cell::new(text)
            .add_attribute(Attribute::Bold)
            .fg(Color::Yellow)
    };
    let t = &report.totals;
    let mut cells = vec![bold("TOTAL".to_string()), bold(total_models_cell.to_string())];
    if show_activity {
        cells.push(bold(format_activity_text(report.activity_totals.as_ref())));
    }
    match layout {
        TableLayout::Compact => {}
        TableLayout::Standard => {
            if show_activity {
                cells.push(bold(format_tokens_per_hour(
                    report.activity_totals.as_ref(),
                    t.total_tokens,
                )));
            } else {
                cells.push(bold(format_u64(t.input_tokens)));
                cells.push(bold(format_u64(t.output_tokens)));
            }
        }
        TableLayout::Full => {
            if show_activity {
                cells.push(bold(format_tokens_per_hour(
                    report.activity_totals.as_ref(),
                    t.total_tokens,
                )));
            }
            cells.push(bold(format_u64(t.input_tokens)));
            cells.push(bold(format_u64(t.output_tokens)));
            cells.push(bold(format_u64(t.cache_creation_input_tokens)));
            cells.push(bold(format_u64(t.cache_read_input_tokens)));
        }
    }
    cells.push(bold(format_u64(t.total_tokens)));
    cells.push(bold(format_usd(t.cost_usd)));
    Row::from(cells)
}

/// Natural rendered width of a layout (content + padding + borders), with no
/// wrapping, so layout selection can pick the richest layout that actually fits.
fn measure_layout_width(
    report: &DailyReport,
    layout: TableLayout,
    show_activity: bool,
    show_breakdown: bool,
) -> usize {
    let mut probe = Table::new();
    // Default arrangement (Disabled) => columns take full content width, no wrap.
    probe.load_preset(UTF8_FULL).apply_modifier(UTF8_ROUND_CORNERS);
    set_layout_header(&mut probe, layout, show_activity);
    for row in &report.daily {
        probe.add_row(primary_row(row, layout, show_activity));
        if show_breakdown {
            add_breakdown_rows(&mut probe, row, layout, show_activity);
        }
    }
    let total_models_cell = report_unique_model_count_by_source_multiline(report);
    probe.add_row(layout_total_row(report, layout, show_activity, &total_models_cell));

    let widths = probe.column_max_content_widths();
    let n = widths.len();
    let content: usize = widths.iter().map(|w| *w as usize).sum();
    // comfy-table default per-column padding (1,1) = 2; UTF8_FULL borders = n + 1.
    content + 2 * n + (n + 1)
}

/// Pick the richest layout whose natural width fits the terminal; Compact is the floor.
fn choose_layout(
    report: &DailyReport,
    show_activity: bool,
    show_breakdown: bool,
    terminal_width: usize,
) -> TableLayout {
    for layout in [TableLayout::Full, TableLayout::Standard] {
        if measure_layout_width(report, layout, show_activity, show_breakdown) <= terminal_width {
            return layout;
        }
    }
    TableLayout::Compact
}

fn print_report_insights(insights: &ReportInsights) {
    let mut parts = Vec::new();
    if let Some(cache_share) = insights.cache_share_pct {
        parts.push(format!("cache share {:.1}%", cache_share));
    }
    if let Some(output_share) = insights.output_share_pct {
        parts.push(format!("output share {:.1}%", output_share));
    }
    if let Some(cost_per_mtoken) = insights.cost_per_mtoken {
        parts.push(format!("${:.2}/1M tok", cost_per_mtoken));
    }
    if let Some(tokens_per_usd) = insights.tokens_per_usd {
        parts.push(format!("{} tok/$", format_u64(tokens_per_usd)));
    }
    if let Some(top_source) = insights.top_source.as_deref() {
        if let Some(share) = insights.top_source_share_pct {
            parts.push(format!("top source {top_source} ({share:.1}%)"));
        } else {
            parts.push(format!("top source {top_source}"));
        }
    }
    if let Some(top_model) = insights.top_model.as_deref() {
        if let Some(share) = insights.top_model_share_pct {
            parts.push(format!("top model {top_model} ({share:.1}%)"));
        } else {
            parts.push(format!("top model {top_model}"));
        }
    }
    if let Some(avg) = insights.avg_tokens_per_active_day {
        parts.push(format!("avg {} tok/active day", format_u64(avg)));
    }
    if let Some(avg) = insights.avg_cost_per_active_day {
        parts.push(format!("avg {}/active day", format_usd(avg)));
    }
    if let Some(streak) = insights.current_streak_days {
        parts.push(format!("streak {}d", streak));
    }
    if let Some(peak) = insights.peak_period.as_ref() {
        parts.push(format!(
            "peak {} ({} tok, {})",
            peak.date,
            format_u64(peak.total_tokens),
            format_usd(peak.cost_usd)
        ));
    }
    if !insights.spikes.is_empty() {
        let spike = &insights.spikes[0];
        let mut text = format!(
            "spike {} ({} tok; med {})",
            spike.date,
            format_u64(spike.total_tokens),
            format_u64(spike.baseline_median)
        );
        if spike.top_source.is_some() || spike.top_model.is_some() {
            let src = spike.top_source.as_deref().unwrap_or("-");
            let model = spike.top_model.as_deref().unwrap_or("-");
            text.push_str(&format!(" [{src} / {model}]"));
        }
        if spike.top_project.is_some() || spike.top_session.is_some() {
            let project = spike.top_project.as_deref().unwrap_or("-");
            let session = spike.top_session.as_deref().unwrap_or("-");
            text.push_str(&format!(" {{{project} / {session}}}"));
        }
        parts.push(text);
    }
    if !insights.anomalies.is_empty() {
        let a = &insights.anomalies[0];
        let mut text = format!("anomaly {} (z={:.1})", a.date, a.robust_z.max(0.0));
        if a.top_source.is_some() || a.top_model.is_some() {
            let src = a.top_source.as_deref().unwrap_or("-");
            let model = a.top_model.as_deref().unwrap_or("-");
            text.push_str(&format!(" [{src} / {model}]"));
        }
        if a.top_project.is_some() || a.top_session.is_some() {
            let project = a.top_project.as_deref().unwrap_or("-");
            let session = a.top_session.as_deref().unwrap_or("-");
            text.push_str(&format!(" {{{project} / {session}}}"));
        }
        parts.push(text);
    }
    if !insights.mix_tokens_pct.is_empty() {
        let mut top = insights
            .mix_tokens_pct
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect::<Vec<_>>();
        top.sort_by(|a, b| b.1.total_cmp(&a.1));
        top.truncate(2);
        let mix = top
            .into_iter()
            .map(|(k, v)| format!("{k} {v:.0}%"))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!("mix {mix}"));
    }

    if !parts.is_empty() {
        println!("Insights: {}", parts.join(" · "));
    }
}

fn primary_row(row: &DailyRow, layout: TableLayout, show_activity: bool) -> Row {
    match layout {
        TableLayout::Compact => {
            if show_activity {
                Row::from(vec![
                    Cell::new(format_date_cell(&row.date)),
                    Cell::new(format_model_list(&row.models, layout)),
                    Cell::new(format_activity_text(row.activity.as_ref())).fg(Color::DarkGrey),
                    Cell::new(format_u64(row.totals.total_tokens)),
                    Cell::new(format_usd(row.totals.cost_usd)).fg(Color::Green),
                ])
            } else {
                Row::from(vec![
                    Cell::new(format_date_cell(&row.date)),
                    Cell::new(format_model_list(&row.models, layout)),
                    Cell::new(format_u64(row.totals.total_tokens)),
                    Cell::new(format_usd(row.totals.cost_usd)).fg(Color::Green),
                ])
            }
        }
        TableLayout::Standard => {
            if show_activity {
                Row::from(vec![
                    Cell::new(format_date_cell(&row.date)),
                    Cell::new(format_model_list(&row.models, layout)),
                    Cell::new(format_activity_text(row.activity.as_ref())).fg(Color::DarkGrey),
                    Cell::new(format_tokens_per_hour(
                        row.activity.as_ref(),
                        row.totals.total_tokens,
                    )),
                    Cell::new(format_u64(row.totals.total_tokens)),
                    Cell::new(format_usd(row.totals.cost_usd)).fg(Color::Green),
                ])
            } else {
                Row::from(vec![
                    Cell::new(format_date_cell(&row.date)),
                    Cell::new(format_model_list(&row.models, layout)),
                    Cell::new(format_u64(row.totals.input_tokens)),
                    Cell::new(format_u64(row.totals.output_tokens)),
                    Cell::new(format_u64(row.totals.total_tokens)),
                    Cell::new(format_usd(row.totals.cost_usd)).fg(Color::Green),
                ])
            }
        }
        TableLayout::Full => {
            if show_activity {
                Row::from(vec![
                    Cell::new(format_date_cell(&row.date)),
                    Cell::new(format_model_list(&row.models, layout)),
                    Cell::new(format_activity_text(row.activity.as_ref())).fg(Color::DarkGrey),
                    Cell::new(format_tokens_per_hour(
                        row.activity.as_ref(),
                        row.totals.total_tokens,
                    )),
                    Cell::new(format_u64(row.totals.input_tokens)),
                    Cell::new(format_u64(row.totals.output_tokens)),
                    Cell::new(format_u64(row.totals.cache_creation_input_tokens)),
                    Cell::new(format_u64(row.totals.cache_read_input_tokens)),
                    Cell::new(format_u64(row.totals.total_tokens)),
                    Cell::new(format_usd(row.totals.cost_usd)).fg(Color::Green),
                ])
            } else {
                Row::from(vec![
                    Cell::new(format_date_cell(&row.date)),
                    Cell::new(format_model_list(&row.models, layout)),
                    Cell::new(format_u64(row.totals.input_tokens)),
                    Cell::new(format_u64(row.totals.output_tokens)),
                    Cell::new(format_u64(row.totals.cache_creation_input_tokens)),
                    Cell::new(format_u64(row.totals.cache_read_input_tokens)),
                    Cell::new(format_u64(row.totals.total_tokens)),
                    Cell::new(format_usd(row.totals.cost_usd)).fg(Color::Green),
                ])
            }
        }
    }
}

fn add_breakdown_rows(table: &mut Table, row: &DailyRow, layout: TableLayout, show_activity: bool) {
    let mut models = row.models.iter().collect::<Vec<_>>();
    models.sort_by(|(model_a, counts_a), (model_b, counts_b)| {
        counts_b
            .total_tokens
            .cmp(&counts_a.total_tokens)
            .then_with(|| model_a.cmp(model_b))
    });

    for (model, counts) in models {
        let model_cell = Cell::new(format!(
            "  └─ {}",
            truncate_text(model, layout.model_char_limit())
        ))
        .fg(Color::DarkGrey);
        match layout {
            TableLayout::Compact => {
                if show_activity {
                    table.add_row(Row::from(vec![
                        Cell::new("").fg(Color::DarkGrey),
                        model_cell,
                        Cell::new("").fg(Color::DarkGrey),
                        Cell::new(format_u64(counts.total_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_usd(counts.cost_usd)).fg(Color::DarkGrey),
                    ]));
                } else {
                    table.add_row(Row::from(vec![
                        Cell::new("").fg(Color::DarkGrey),
                        model_cell,
                        Cell::new(format_u64(counts.total_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_usd(counts.cost_usd)).fg(Color::DarkGrey),
                    ]));
                }
            }
            TableLayout::Standard => {
                if show_activity {
                    table.add_row(Row::from(vec![
                        Cell::new("").fg(Color::DarkGrey),
                        model_cell,
                        Cell::new("").fg(Color::DarkGrey),
                        Cell::new("").fg(Color::DarkGrey),
                        Cell::new(format_u64(counts.total_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_usd(counts.cost_usd)).fg(Color::DarkGrey),
                    ]));
                } else {
                    table.add_row(Row::from(vec![
                        Cell::new("").fg(Color::DarkGrey),
                        model_cell,
                        Cell::new(format_u64(counts.input_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_u64(counts.output_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_u64(counts.total_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_usd(counts.cost_usd)).fg(Color::DarkGrey),
                    ]));
                }
            }
            TableLayout::Full => {
                if show_activity {
                    table.add_row(Row::from(vec![
                        Cell::new("").fg(Color::DarkGrey),
                        model_cell,
                        Cell::new("").fg(Color::DarkGrey),
                        Cell::new("").fg(Color::DarkGrey),
                        Cell::new(format_u64(counts.input_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_u64(counts.output_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_u64(counts.cache_creation_input_tokens))
                            .fg(Color::DarkGrey),
                        Cell::new(format_u64(counts.cache_read_input_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_u64(counts.total_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_usd(counts.cost_usd)).fg(Color::DarkGrey),
                    ]));
                } else {
                    table.add_row(Row::from(vec![
                        Cell::new("").fg(Color::DarkGrey),
                        model_cell,
                        Cell::new(format_u64(counts.input_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_u64(counts.output_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_u64(counts.cache_creation_input_tokens))
                            .fg(Color::DarkGrey),
                        Cell::new(format_u64(counts.cache_read_input_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_u64(counts.total_tokens)).fg(Color::DarkGrey),
                        Cell::new(format_usd(counts.cost_usd)).fg(Color::DarkGrey),
                    ]));
                }
            }
        }
    }
}

pub(crate) fn run_report_tui(report: &DailyReport) -> Result<()> {
    if !io::stdout().is_terminal() {
        bail!("--tui requires an interactive terminal");
    }

    let mut session = TuiSession::enter()?;
    let mut offset = 0usize;

    loop {
        let size = session.terminal.size()?;
        let total_rows = report.daily.len().saturating_add(1);
        let page_rows = visible_body_rows(usize::from(size.height));
        let max_offset = total_rows.saturating_sub(page_rows);
        offset = offset.min(max_offset);

        session
            .terminal
            .draw(|frame| draw_report_tui(frame, report, offset))?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => break,
            KeyCode::Down | KeyCode::Char('j') => offset = (offset + 1).min(max_offset),
            KeyCode::Up | KeyCode::Char('k') => offset = offset.saturating_sub(1),
            KeyCode::PageDown => offset = (offset + page_rows).min(max_offset),
            KeyCode::PageUp => offset = offset.saturating_sub(page_rows),
            KeyCode::Home => offset = 0,
            KeyCode::End => offset = max_offset,
            _ => {}
        }
    }

    Ok(())
}

fn draw_report_tui(frame: &mut ratatui::Frame<'_>, report: &DailyReport, offset: usize) {
    let root = frame.area();
    let [table_area, footer_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(root);

    let layout = TableLayout::from_terminal_width(usize::from(table_area.width));
    let show_activity = report_has_activity(report);
    let headers = tui_headers(layout, show_activity);
    let constraints = tui_constraints(layout, show_activity);
    let all_rows = tui_rows(report, layout, show_activity);

    let visible_rows = visible_body_rows(usize::from(table_area.height));
    let max_offset = all_rows.len().saturating_sub(visible_rows);
    let start = offset.min(max_offset);
    let end = (start + visible_rows).min(all_rows.len());

    let table = TuiTable::new(
        all_rows[start..end]
            .iter()
            .map(|cells| TuiRow::new(cells.clone()).height(tui_row_height(cells))),
        constraints,
    )
    .header(
        TuiRow::new(headers).style(
            Style::default()
                .fg(TuiColor::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1)
    .block(Block::default().borders(Borders::ALL).title("tu daily"));

    frame.render_widget(table, table_area);

    let status = format!(
        "rows {}-{} / {} | ↑↓ j/k PgUp/PgDn Home/End | q exit",
        if all_rows.is_empty() { 0 } else { start + 1 },
        end,
        all_rows.len()
    );
    let footer = Paragraph::new(Line::raw(status))
        .style(Style::default().fg(TuiColor::Gray))
        .wrap(Wrap { trim: true });
    frame.render_widget(footer, footer_area);
}

fn tui_headers(layout: TableLayout, show_activity: bool) -> Vec<&'static str> {
    match layout {
        TableLayout::Compact => {
            if show_activity {
                vec!["Date", "Models", "Coding", "Total Tokens", "Cost (USD)"]
            } else {
                vec!["Date", "Models", "Total Tokens", "Cost (USD)"]
            }
        }
        TableLayout::Standard => {
            if show_activity {
                vec![
                    "Date",
                    "Models",
                    "Coding",
                    "Tok/hr",
                    "Total Tokens",
                    "Cost (USD)",
                ]
            } else {
                vec![
                    "Date",
                    "Models",
                    "Input",
                    "Output",
                    "Total Tokens",
                    "Cost (USD)",
                ]
            }
        }
        TableLayout::Full => {
            if show_activity {
                vec![
                    "Date",
                    "Models",
                    "Coding",
                    "Tok/hr",
                    "Input",
                    "Output",
                    "Cache Create",
                    "Cache Read",
                    "Total Tokens",
                    "Cost (USD)",
                ]
            } else {
                vec![
                    "Date",
                    "Models",
                    "Input",
                    "Output",
                    "Cache Create",
                    "Cache Read",
                    "Total Tokens",
                    "Cost (USD)",
                ]
            }
        }
    }
}

fn tui_constraints(layout: TableLayout, show_activity: bool) -> Vec<Constraint> {
    match layout {
        TableLayout::Compact => {
            if show_activity {
                vec![
                    Constraint::Length(12),
                    Constraint::Percentage(44),
                    Constraint::Length(11),
                    Constraint::Length(15),
                    Constraint::Length(12),
                ]
            } else {
                vec![
                    Constraint::Length(12),
                    Constraint::Percentage(54),
                    Constraint::Length(15),
                    Constraint::Length(12),
                ]
            }
        }
        TableLayout::Standard => {
            if show_activity {
                vec![
                    Constraint::Length(12),
                    Constraint::Percentage(34),
                    Constraint::Length(11),
                    Constraint::Length(14),
                    Constraint::Length(15),
                    Constraint::Length(12),
                ]
            } else {
                vec![
                    Constraint::Length(12),
                    Constraint::Percentage(30),
                    Constraint::Length(13),
                    Constraint::Length(13),
                    Constraint::Length(15),
                    Constraint::Length(12),
                ]
            }
        }
        TableLayout::Full => {
            if show_activity {
                vec![
                    Constraint::Length(12),
                    Constraint::Percentage(26),
                    Constraint::Length(11),
                    Constraint::Length(14),
                    Constraint::Length(13),
                    Constraint::Length(13),
                    Constraint::Length(13),
                    Constraint::Length(13),
                    Constraint::Length(15),
                    Constraint::Length(12),
                ]
            } else {
                vec![
                    Constraint::Length(12),
                    Constraint::Percentage(30),
                    Constraint::Length(13),
                    Constraint::Length(13),
                    Constraint::Length(13),
                    Constraint::Length(13),
                    Constraint::Length(15),
                    Constraint::Length(12),
                ]
            }
        }
    }
}

fn tui_rows(report: &DailyReport, layout: TableLayout, show_activity: bool) -> Vec<Vec<String>> {
    let mut sorted_daily = report.daily.iter().collect::<Vec<_>>();
    sorted_daily.sort_by(|a, b| b.date.cmp(&a.date));

    let mut rows = sorted_daily
        .into_iter()
        .map(|row| tui_day_row(row, layout, show_activity))
        .collect::<Vec<_>>();
    rows.push(tui_total_row(report, layout, show_activity));
    rows
}

fn tui_day_row(row: &DailyRow, layout: TableLayout, show_activity: bool) -> Vec<String> {
    let date = row.date.clone();
    let models = format_model_multiline(&row.models, layout);
    match layout {
        TableLayout::Compact => {
            if show_activity {
                vec![
                    date,
                    models,
                    format_activity_text(row.activity.as_ref()),
                    format_u64(row.totals.total_tokens),
                    format_usd(row.totals.cost_usd),
                ]
            } else {
                vec![
                    date,
                    models,
                    format_u64(row.totals.total_tokens),
                    format_usd(row.totals.cost_usd),
                ]
            }
        }
        TableLayout::Standard => {
            if show_activity {
                vec![
                    date,
                    models,
                    format_activity_text(row.activity.as_ref()),
                    format_tokens_per_hour(row.activity.as_ref(), row.totals.total_tokens),
                    format_u64(row.totals.total_tokens),
                    format_usd(row.totals.cost_usd),
                ]
            } else {
                vec![
                    date,
                    models,
                    format_u64(row.totals.input_tokens),
                    format_u64(row.totals.output_tokens),
                    format_u64(row.totals.total_tokens),
                    format_usd(row.totals.cost_usd),
                ]
            }
        }
        TableLayout::Full => {
            if show_activity {
                vec![
                    date,
                    models,
                    format_activity_text(row.activity.as_ref()),
                    format_tokens_per_hour(row.activity.as_ref(), row.totals.total_tokens),
                    format_u64(row.totals.input_tokens),
                    format_u64(row.totals.output_tokens),
                    format_u64(row.totals.cache_creation_input_tokens),
                    format_u64(row.totals.cache_read_input_tokens),
                    format_u64(row.totals.total_tokens),
                    format_usd(row.totals.cost_usd),
                ]
            } else {
                vec![
                    date,
                    models,
                    format_u64(row.totals.input_tokens),
                    format_u64(row.totals.output_tokens),
                    format_u64(row.totals.cache_creation_input_tokens),
                    format_u64(row.totals.cache_read_input_tokens),
                    format_u64(row.totals.total_tokens),
                    format_usd(row.totals.cost_usd),
                ]
            }
        }
    }
}

fn tui_total_row(report: &DailyReport, layout: TableLayout, show_activity: bool) -> Vec<String> {
    let models_cell = report_unique_model_count_by_source_multiline(report);
    match layout {
        TableLayout::Compact => {
            if show_activity {
                vec![
                    "TOTAL".to_string(),
                    models_cell,
                    format_activity_text(report.activity_totals.as_ref()),
                    format_u64(report.totals.total_tokens),
                    format_usd(report.totals.cost_usd),
                ]
            } else {
                vec![
                    "TOTAL".to_string(),
                    models_cell,
                    format_u64(report.totals.total_tokens),
                    format_usd(report.totals.cost_usd),
                ]
            }
        }
        TableLayout::Standard => {
            if show_activity {
                vec![
                    "TOTAL".to_string(),
                    models_cell,
                    format_activity_text(report.activity_totals.as_ref()),
                    format_tokens_per_hour(
                        report.activity_totals.as_ref(),
                        report.totals.total_tokens,
                    ),
                    format_u64(report.totals.total_tokens),
                    format_usd(report.totals.cost_usd),
                ]
            } else {
                vec![
                    "TOTAL".to_string(),
                    models_cell,
                    format_u64(report.totals.input_tokens),
                    format_u64(report.totals.output_tokens),
                    format_u64(report.totals.total_tokens),
                    format_usd(report.totals.cost_usd),
                ]
            }
        }
        TableLayout::Full => {
            if show_activity {
                vec![
                    "TOTAL".to_string(),
                    models_cell,
                    format_activity_text(report.activity_totals.as_ref()),
                    format_tokens_per_hour(
                        report.activity_totals.as_ref(),
                        report.totals.total_tokens,
                    ),
                    format_u64(report.totals.input_tokens),
                    format_u64(report.totals.output_tokens),
                    format_u64(report.totals.cache_creation_input_tokens),
                    format_u64(report.totals.cache_read_input_tokens),
                    format_u64(report.totals.total_tokens),
                    format_usd(report.totals.cost_usd),
                ]
            } else {
                vec![
                    "TOTAL".to_string(),
                    models_cell,
                    format_u64(report.totals.input_tokens),
                    format_u64(report.totals.output_tokens),
                    format_u64(report.totals.cache_creation_input_tokens),
                    format_u64(report.totals.cache_read_input_tokens),
                    format_u64(report.totals.total_tokens),
                    format_usd(report.totals.cost_usd),
                ]
            }
        }
    }
}

fn report_unique_model_count_by_source_multiline(report: &DailyReport) -> String {
    let mut per_source: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in &report.daily {
        for (source, models) in &row.models_by_source {
            per_source
                .entry(source.clone())
                .or_default()
                .extend(models.iter().cloned());
        }
    }

    if per_source.is_empty() {
        return "-".to_string();
    }

    per_source
        .into_iter()
        .map(|(source, models)| format!("{source}: {} models", models.len()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn report_has_activity(report: &DailyReport) -> bool {
    report
        .activity_totals
        .as_ref()
        .is_some_and(|summary| summary.total_seconds > 0)
        || report.daily.iter().any(|row| {
            row.activity
                .as_ref()
                .is_some_and(|summary| summary.total_seconds > 0)
        })
}

fn format_model_multiline(models: &BTreeMap<String, TokenCounts>, layout: TableLayout) -> String {
    if models.is_empty() {
        return "-".to_string();
    }

    let mut sorted = models.iter().collect::<Vec<_>>();
    sorted.sort_by(|(model_a, counts_a), (model_b, counts_b)| {
        counts_b
            .total_tokens
            .cmp(&counts_a.total_tokens)
            .then_with(|| model_a.cmp(model_b))
    });

    let model_limit = layout.model_line_limit();
    let mut lines = sorted
        .iter()
        .take(model_limit)
        .map(|(model, _)| format!("- {}", truncate_text(model, layout.model_char_limit())))
        .collect::<Vec<_>>();
    if sorted.len() > model_limit {
        lines.push(format!("... +{} more", sorted.len() - model_limit));
    }

    lines.join("\n")
}

fn format_activity_text(activity: Option<&crate::types::ActivitySummary>) -> String {
    activity
        .map(|summary| summary.text.clone())
        .unwrap_or_else(|| "-".to_string())
}

fn format_tokens_per_hour(
    activity: Option<&crate::types::ActivitySummary>,
    total_tokens: u64,
) -> String {
    let Some(activity) = activity else {
        return "-".to_string();
    };
    if activity.total_seconds == 0 {
        return "-".to_string();
    }
    let hourly = (total_tokens as f64 * 3600.0 / activity.total_seconds as f64)
        .round()
        .max(0.0) as u64;
    format_u64(hourly)
}

fn tui_row_height(cells: &[String]) -> u16 {
    cells
        .iter()
        .map(|cell| cell.lines().count() as u16)
        .max()
        .unwrap_or(1)
        .max(1)
}

fn visible_body_rows(area_height: usize) -> usize {
    area_height.saturating_sub(4).max(1)
}

struct TuiSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TuiSession {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.hide_cursor()?;
        Ok(Self { terminal })
    }
}

impl Drop for TuiSession {
    fn drop(&mut self) {
        let _ = self.terminal.show_cursor();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn create_table(width: usize) -> Table {
    let mut table = Table::new();
    let table_width = width.clamp(20, u16::MAX as usize) as u16;
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_width(table_width)
        .set_content_arrangement(ContentArrangement::Dynamic);
    if !io::stdout().is_terminal() && std::env::var("CLICOLOR_FORCE").is_ok() {
        table.enforce_styling();
    }
    table
}

fn header_cell(text: &str, color: Color) -> Cell {
    Cell::new(text).fg(color).add_attribute(Attribute::Bold)
}

fn format_u64(value: u64) -> String {
    let raw = value.to_string();
    let mut out = String::with_capacity(raw.len() + (raw.len() / 3));
    let total = raw.len();
    for (idx, ch) in raw.chars().enumerate() {
        out.push(ch);
        let remain = total.saturating_sub(idx + 1);
        if remain > 0 && remain.is_multiple_of(3) {
            out.push(',');
        }
    }
    out
}

fn format_usd(value: f64) -> String {
    format!("${value:.2}")
}

fn format_date_cell(date: &str) -> String {
    let mut parts = date.splitn(3, '-');
    let year = parts.next();
    let month = parts.next();
    let day = parts.next();

    match (year, month, day) {
        (Some(y), Some(m), Some(d)) => format!("{y}\n{m}-{d}"),
        _ => date.to_string(),
    }
}

fn format_model_list(models: &BTreeMap<String, TokenCounts>, layout: TableLayout) -> String {
    if models.is_empty() {
        return "-".to_string();
    }

    let mut sorted = models.iter().collect::<Vec<_>>();
    sorted.sort_by(|(model_a, counts_a), (model_b, counts_b)| {
        counts_b
            .total_tokens
            .cmp(&counts_a.total_tokens)
            .then_with(|| model_a.cmp(model_b))
    });

    let limit = layout.model_line_limit();
    let char_limit = layout.model_char_limit();
    let mut lines = Vec::new();
    for (idx, (model, _)) in sorted.iter().enumerate() {
        if idx >= limit {
            break;
        }
        lines.push(format!("- {}", truncate_text(model, char_limit)));
    }
    if sorted.len() > limit {
        lines.push(format!("... +{} more", sorted.len() - limit));
    }

    lines.join("\n")
}

fn truncate_text(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }

    let mut out = String::with_capacity(max_chars);
    for (idx, ch) in input.chars().enumerate() {
        if idx >= max_chars - 3 {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn detect_terminal_width() -> usize {
    if let Ok(raw) = std::env::var("COLUMNS")
        && let Ok(cols) = raw.parse::<usize>()
        && cols > 0
    {
        return cols;
    }
    if let Some((Width(cols), _)) = terminal_size() {
        return usize::from(cols);
    }
    160
}
