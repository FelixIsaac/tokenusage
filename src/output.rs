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

use crate::types::{DailyReport, DailyRow, TableLayout, TokenCounts};

pub(crate) fn print_report_table_with_options(
    report: &DailyReport,
    force_compact: bool,
    show_breakdown: bool,
) {
    let terminal_width = detect_terminal_width();
    let layout = if force_compact {
        TableLayout::Compact
    } else {
        TableLayout::from_terminal_width(terminal_width)
    };
    let mut daily_table = create_table(terminal_width);
    match layout {
        TableLayout::Compact => {
            daily_table.set_header(vec![
                header_cell("Date", Color::Cyan),
                header_cell("Models", Color::Cyan),
                header_cell("Total Tokens", Color::Cyan),
                header_cell("Cost (USD)", Color::Cyan),
            ]);
        }
        TableLayout::Standard => {
            daily_table.set_header(vec![
                header_cell("Date", Color::Cyan),
                header_cell("Models", Color::Cyan),
                header_cell("Input", Color::Cyan),
                header_cell("Output", Color::Cyan),
                header_cell("Total Tokens", Color::Cyan),
                header_cell("Cost (USD)", Color::Cyan),
            ]);
        }
        TableLayout::Full => {
            daily_table.set_header(vec![
                header_cell("Date", Color::Cyan),
                header_cell("Models", Color::Cyan),
                header_cell("Input", Color::Cyan),
                header_cell("Output", Color::Cyan),
                header_cell("Cache Create", Color::Cyan),
                header_cell("Cache Read", Color::Cyan),
                header_cell("Total Tokens", Color::Cyan),
                header_cell("Cost (USD)", Color::Cyan),
            ]);
        }
    }

    for row in &report.daily {
        daily_table.add_row(primary_row(row, layout));
        if show_breakdown {
            add_breakdown_rows(&mut daily_table, row, layout);
        }
    }

    match layout {
        TableLayout::Compact => {
            daily_table.add_row(Row::from(vec![
                Cell::new("TOTAL")
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new("-")
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new(format_u64(report.totals.total_tokens))
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new(format_usd(report.totals.cost_usd))
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
            ]));
        }
        TableLayout::Standard => {
            daily_table.add_row(Row::from(vec![
                Cell::new("TOTAL")
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new("-")
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new(format_u64(report.totals.input_tokens))
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new(format_u64(report.totals.output_tokens))
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new(format_u64(report.totals.total_tokens))
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new(format_usd(report.totals.cost_usd))
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
            ]));
        }
        TableLayout::Full => {
            daily_table.add_row(Row::from(vec![
                Cell::new("TOTAL")
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new("-")
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new(format_u64(report.totals.input_tokens))
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new(format_u64(report.totals.output_tokens))
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new(format_u64(report.totals.cache_creation_input_tokens))
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new(format_u64(report.totals.cache_read_input_tokens))
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new(format_u64(report.totals.total_tokens))
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
                Cell::new(format_usd(report.totals.cost_usd))
                    .add_attribute(Attribute::Bold)
                    .fg(Color::Yellow),
            ]));
        }
    }
    println!("{daily_table}");
}

fn primary_row(row: &DailyRow, layout: TableLayout) -> Row {
    match layout {
        TableLayout::Compact => Row::from(vec![
            Cell::new(format_date_cell(&row.date)),
            Cell::new(format_model_list(&row.models, layout)),
            Cell::new(format_u64(row.totals.total_tokens)),
            Cell::new(format_usd(row.totals.cost_usd)).fg(Color::Green),
        ]),
        TableLayout::Standard => Row::from(vec![
            Cell::new(format_date_cell(&row.date)),
            Cell::new(format_model_list(&row.models, layout)),
            Cell::new(format_u64(row.totals.input_tokens)),
            Cell::new(format_u64(row.totals.output_tokens)),
            Cell::new(format_u64(row.totals.total_tokens)),
            Cell::new(format_usd(row.totals.cost_usd)).fg(Color::Green),
        ]),
        TableLayout::Full => Row::from(vec![
            Cell::new(format_date_cell(&row.date)),
            Cell::new(format_model_list(&row.models, layout)),
            Cell::new(format_u64(row.totals.input_tokens)),
            Cell::new(format_u64(row.totals.output_tokens)),
            Cell::new(format_u64(row.totals.cache_creation_input_tokens)),
            Cell::new(format_u64(row.totals.cache_read_input_tokens)),
            Cell::new(format_u64(row.totals.total_tokens)),
            Cell::new(format_usd(row.totals.cost_usd)).fg(Color::Green),
        ]),
    }
}

fn add_breakdown_rows(table: &mut Table, row: &DailyRow, layout: TableLayout) {
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
                table.add_row(Row::from(vec![
                    Cell::new("").fg(Color::DarkGrey),
                    model_cell,
                    Cell::new(format_u64(counts.total_tokens)).fg(Color::DarkGrey),
                    Cell::new(format_usd(counts.cost_usd)).fg(Color::DarkGrey),
                ]));
            }
            TableLayout::Standard => {
                table.add_row(Row::from(vec![
                    Cell::new("").fg(Color::DarkGrey),
                    model_cell,
                    Cell::new(format_u64(counts.input_tokens)).fg(Color::DarkGrey),
                    Cell::new(format_u64(counts.output_tokens)).fg(Color::DarkGrey),
                    Cell::new(format_u64(counts.total_tokens)).fg(Color::DarkGrey),
                    Cell::new(format_usd(counts.cost_usd)).fg(Color::DarkGrey),
                ]));
            }
            TableLayout::Full => {
                table.add_row(Row::from(vec![
                    Cell::new("").fg(Color::DarkGrey),
                    model_cell,
                    Cell::new(format_u64(counts.input_tokens)).fg(Color::DarkGrey),
                    Cell::new(format_u64(counts.output_tokens)).fg(Color::DarkGrey),
                    Cell::new(format_u64(counts.cache_creation_input_tokens)).fg(Color::DarkGrey),
                    Cell::new(format_u64(counts.cache_read_input_tokens)).fg(Color::DarkGrey),
                    Cell::new(format_u64(counts.total_tokens)).fg(Color::DarkGrey),
                    Cell::new(format_usd(counts.cost_usd)).fg(Color::DarkGrey),
                ]));
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
    let headers = tui_headers(layout);
    let constraints = tui_constraints(layout);
    let all_rows = tui_rows(report, layout);

    let visible_rows = visible_body_rows(usize::from(table_area.height));
    let max_offset = all_rows.len().saturating_sub(visible_rows);
    let start = offset.min(max_offset);
    let end = (start + visible_rows).min(all_rows.len());

    let table = TuiTable::new(
        all_rows[start..end].iter().cloned().map(TuiRow::new),
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
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("tu daily (sticky header)"),
    );

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

fn tui_headers(layout: TableLayout) -> Vec<&'static str> {
    match layout {
        TableLayout::Compact => vec!["Date", "Models", "Total Tokens", "Cost (USD)"],
        TableLayout::Standard => vec![
            "Date",
            "Models",
            "Input",
            "Output",
            "Total Tokens",
            "Cost (USD)",
        ],
        TableLayout::Full => vec![
            "Date",
            "Models",
            "Input",
            "Output",
            "Cache Create",
            "Cache Read",
            "Total Tokens",
            "Cost (USD)",
        ],
    }
}

fn tui_constraints(layout: TableLayout) -> Vec<Constraint> {
    match layout {
        TableLayout::Compact => vec![
            Constraint::Length(12),
            Constraint::Percentage(48),
            Constraint::Length(15),
            Constraint::Length(12),
        ],
        TableLayout::Standard => vec![
            Constraint::Length(12),
            Constraint::Percentage(36),
            Constraint::Length(13),
            Constraint::Length(13),
            Constraint::Length(15),
            Constraint::Length(12),
        ],
        TableLayout::Full => vec![
            Constraint::Length(12),
            Constraint::Percentage(30),
            Constraint::Length(13),
            Constraint::Length(13),
            Constraint::Length(13),
            Constraint::Length(13),
            Constraint::Length(15),
            Constraint::Length(12),
        ],
    }
}

fn tui_rows(report: &DailyReport, layout: TableLayout) -> Vec<Vec<String>> {
    let mut rows = report
        .daily
        .iter()
        .map(|row| tui_day_row(row, layout))
        .collect::<Vec<_>>();
    rows.push(tui_total_row(report, layout));
    rows
}

fn tui_day_row(row: &DailyRow, layout: TableLayout) -> Vec<String> {
    let date = row.date.clone();
    let models = format_model_inline(&row.models, layout);
    match layout {
        TableLayout::Compact => vec![
            date,
            models,
            format_u64(row.totals.total_tokens),
            format_usd(row.totals.cost_usd),
        ],
        TableLayout::Standard => vec![
            date,
            models,
            format_u64(row.totals.input_tokens),
            format_u64(row.totals.output_tokens),
            format_u64(row.totals.total_tokens),
            format_usd(row.totals.cost_usd),
        ],
        TableLayout::Full => vec![
            date,
            models,
            format_u64(row.totals.input_tokens),
            format_u64(row.totals.output_tokens),
            format_u64(row.totals.cache_creation_input_tokens),
            format_u64(row.totals.cache_read_input_tokens),
            format_u64(row.totals.total_tokens),
            format_usd(row.totals.cost_usd),
        ],
    }
}

fn tui_total_row(report: &DailyReport, layout: TableLayout) -> Vec<String> {
    match layout {
        TableLayout::Compact => vec![
            "TOTAL".to_string(),
            "-".to_string(),
            format_u64(report.totals.total_tokens),
            format_usd(report.totals.cost_usd),
        ],
        TableLayout::Standard => vec![
            "TOTAL".to_string(),
            "-".to_string(),
            format_u64(report.totals.input_tokens),
            format_u64(report.totals.output_tokens),
            format_u64(report.totals.total_tokens),
            format_usd(report.totals.cost_usd),
        ],
        TableLayout::Full => vec![
            "TOTAL".to_string(),
            "-".to_string(),
            format_u64(report.totals.input_tokens),
            format_u64(report.totals.output_tokens),
            format_u64(report.totals.cache_creation_input_tokens),
            format_u64(report.totals.cache_read_input_tokens),
            format_u64(report.totals.total_tokens),
            format_usd(report.totals.cost_usd),
        ],
    }
}

fn format_model_inline(models: &BTreeMap<String, TokenCounts>, layout: TableLayout) -> String {
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

    let model_limit = layout.model_inline_limit();
    let mut parts = sorted
        .iter()
        .take(model_limit)
        .map(|(model, _)| truncate_text(model, layout.model_inline_char_limit()))
        .collect::<Vec<_>>();
    if sorted.len() > model_limit {
        parts.push(format!("+{}", sorted.len() - model_limit));
    }

    truncate_text(&parts.join(", "), layout.model_inline_char_limit())
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
        .set_content_arrangement(ContentArrangement::DynamicFullWidth);
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
