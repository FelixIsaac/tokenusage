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
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, ListState, Paragraph, Row as TuiRow, Table as TuiTable, Wrap,
};
use terminal_size::{Width, terminal_size};
use unicode_width::UnicodeWidthStr;

use crate::ReportInsights;
use crate::types::{DailyReport, DailyRow, TableLayout, TokenCounts};
use std::collections::BTreeSet;

/// Headline-first one-liner for muscle-memory checks (`--brief`): range, tokens,
/// cost, top model — no table.
fn print_report_brief(report: &DailyReport) {
    let range = match (report.daily.first(), report.daily.last()) {
        (Some(first), Some(last)) if first.date != last.date => {
            format!("{} -> {}", first.date, last.date)
        }
        (Some(first), _) => first.date.clone(),
        _ => "-".to_string(),
    };
    let top_model = report
        .insights
        .as_ref()
        .and_then(|i| i.top_model.as_deref())
        .unwrap_or("-");
    println!(
        "{range}  {} tok  {}  top {top_model}",
        format_u64(report.totals.total_tokens),
        format_usd(report.totals.cost_usd),
    );
}

pub(crate) fn print_report_table_with_options(
    report: &DailyReport,
    force_compact: bool,
    show_breakdown: bool,
    brief: bool,
) {
    if brief {
        print_report_brief(report);
        return;
    }
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
    let rendered_table = daily_table.to_string();
    let table_width = rendered_table
        .lines()
        .next()
        .map(|line| strip_ansi_codes(line).as_str().width())
        .unwrap_or(terminal_width);
    println!("{rendered_table}");

    if let Some(insights) = report.insights.as_ref() {
        print_report_insights(insights, Some(table_width));
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
    let mut cells = vec![
        bold("TOTAL".to_string()),
        bold(total_models_cell.to_string()),
    ];
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
    probe
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS);
    set_layout_header(&mut probe, layout, show_activity);
    for row in &report.daily {
        probe.add_row(primary_row(row, layout, show_activity));
        if show_breakdown {
            add_breakdown_rows(&mut probe, row, layout, show_activity);
        }
    }
    let total_models_cell = report_unique_model_count_by_source_multiline(report);
    probe.add_row(layout_total_row(
        report,
        layout,
        show_activity,
        &total_models_cell,
    ));

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

pub fn print_boxed_card(title: &str, sections: &[(&str, Vec<String>)]) {
    print_boxed_card_with_width(title, sections, None);
}

pub fn print_boxed_card_with_width(
    title: &str,
    sections: &[(&str, Vec<String>)],
    target_width: Option<usize>,
) {
    println!();
    println!(
        "{}",
        render_boxed_card_with_width(title, sections, target_width)
    );
}

pub fn render_boxed_card_with_width(
    title: &str,
    sections: &[(&str, Vec<String>)],
    target_width: Option<usize>,
) -> String {
    let term_width = detect_terminal_width();
    let title_plain = strip_ansi_codes(title);
    let title_len = title_plain.as_str().width();
    let min_width = (title_len + 4).min(term_width);

    let width = match target_width {
        Some(tw) => tw.max(min_width),
        None => term_width.clamp(72.min(term_width), 100.min(term_width)),
    };
    let inner_width = width.saturating_sub(4);
    let border_color = "\x1b[36m";
    let reset = "\x1b[0m";

    let header_fill = width.saturating_sub(title_len + 3);
    let mut lines_out = Vec::new();

    lines_out.push(format!(
        "{}╭─{}{}╮{}",
        border_color,
        title,
        "─".repeat(header_fill),
        reset
    ));

    for (sec_idx, (sec_title, lines)) in sections.iter().enumerate() {
        if sec_idx > 0 {
            lines_out.push(format!(
                "{}│{}│{}",
                border_color,
                " ".repeat(width.saturating_sub(2)),
                reset
            ));
        }
        let sec_header = format!("  \x1b[1m{}\x1b[0m", sec_title);
        lines_out.push(format_card_line_exact(
            &sec_header,
            width,
            border_color,
            reset,
        ));

        for line in lines {
            let plain = strip_ansi_codes(line);
            let display_w = plain.as_str().width();
            if display_w + 5 <= width.saturating_sub(2) {
                let item = format!("     {}", line);
                lines_out.push(format_card_line_exact(&item, width, border_color, reset));
            } else {
                // Word wrap long line
                let words: Vec<&str> = line.split_whitespace().collect();
                let mut current = String::new();
                let mut is_first = true;
                let continuation_indent = "       ";

                for word in words {
                    let candidate = if current.is_empty() {
                        word.to_string()
                    } else {
                        format!("{} {}", current, word)
                    };
                    let candidate_plain = strip_ansi_codes(&candidate);
                    let target_max = if is_first {
                        inner_width.saturating_sub(4)
                    } else {
                        inner_width.saturating_sub(continuation_indent.len())
                    };

                    if candidate_plain.as_str().width() <= target_max {
                        current = candidate;
                    } else {
                        if !current.is_empty() {
                            let formatted = if is_first {
                                format!("     {}", current)
                            } else {
                                format!("{}{}", continuation_indent, current)
                            };
                            lines_out.push(format_card_line_exact(
                                &formatted,
                                width,
                                border_color,
                                reset,
                            ));
                            is_first = false;
                        }
                        current = word.to_string();
                    }
                }
                if !current.is_empty() {
                    let formatted = if is_first {
                        format!("     {}", current)
                    } else {
                        format!("{}{}", continuation_indent, current)
                    };
                    lines_out.push(format_card_line_exact(
                        &formatted,
                        width,
                        border_color,
                        reset,
                    ));
                }
            }
        }
    }

    lines_out.push(format!(
        "{}╰{}╯{}",
        border_color,
        "─".repeat(width.saturating_sub(2)),
        reset
    ));

    lines_out.join("\n")
}

fn print_report_insights(insights: &ReportInsights, target_width: Option<usize>) {
    let mut sections: Vec<(&str, Vec<String>)> = Vec::new();

    // 1. Spending & Efficiency
    let mut spend_lines = Vec::new();
    if let Some(cache_share) = insights.cache_share_pct {
        let mut text = format!("• Cache Efficiency: {:.1}% hit rate", cache_share);
        if let Some(savings) = insights.cache_savings_usd
            && savings.abs() >= 1.0
        {
            if savings >= 0.0 {
                text.push_str(&format!(
                    " · Saved ~{} vs uncached rates",
                    format_usd(savings)
                ));
            } else {
                text.push_str(&format!(
                    " · Premium ~{} paid for cache writes",
                    format_usd(-savings)
                ));
            }
        }
        if let Some(reuse) = insights.cache_reuse_ratio
            && reuse < 1.0
        {
            text.push_str(&format!(" (low reuse {:.1}×)", reuse));
        }
        spend_lines.push(text);
    }
    if let Some(cost_m) = insights.cost_per_mtoken {
        let mut text = format!("• Effective Rate:   ${:.2} / 1M tokens", cost_m);
        if let Some(tok_per_usd) = insights.tokens_per_usd {
            text.push_str(&format!(" (avg {} tok/$)", format_u64(tok_per_usd)));
        }
        spend_lines.push(text);
    }
    if let Some(conc) = &insights.cost_concentration
        && conc.cost_pct - conc.token_pct >= 15.0
    {
        spend_lines.push(format!(
            "• Cost Divergence:  {} {:.0}% tok → {:.0}% cost",
            conc.label, conc.token_pct, conc.cost_pct
        ));
    }
    if !spend_lines.is_empty() {
        sections.push(("💰 Spending & Efficiency", spend_lines));
    }

    // 2. Top Drivers
    let mut driver_lines = Vec::new();
    if let Some(top_src) = insights.top_source.as_deref() {
        let mut text = if let Some(share) = insights.top_source_share_pct {
            format!("• Primary Provider: {top_src} ({share:.1}% tokens)")
        } else {
            format!("• Primary Provider: {top_src}")
        };
        if !insights.mix_tokens_pct.is_empty() {
            let mut top = insights
                .mix_tokens_pct
                .iter()
                .map(|(k, v)| (k.as_str(), *v))
                .collect::<Vec<_>>();
            top.sort_by(|a, b| b.1.total_cmp(&a.1));
            let mix_str = top
                .iter()
                .take(3)
                .map(|(k, v)| format!("{k} {v:.0}%"))
                .collect::<Vec<_>>()
                .join(", ");
            text.push_str(&format!(" · Mix: {mix_str}"));
        }
        driver_lines.push(text);
    }
    if let Some(top_mod) = insights.top_model.as_deref() {
        let text = if let Some(share) = insights.top_model_share_pct {
            format!("• Dominant Model:   {top_mod} ({share:.1}% of all tokens)")
        } else {
            format!("• Dominant Model:   {top_mod}")
        };
        driver_lines.push(text);
    }
    if !driver_lines.is_empty() {
        sections.push(("🏆 Top Drivers", driver_lines));
    }

    // 3. Records & Milestones
    let mut record_lines = Vec::new();
    if let Some(spend) = &insights.peak_spend {
        record_lines.push(format!(
            "• Peak Spend:       {} ({} · {} tok)",
            spend.date,
            format_usd(spend.cost_usd),
            format_u64(spend.total_tokens)
        ));
    }
    if let Some(vol) = &insights.peak_period {
        let show_vol = match &insights.peak_spend {
            Some(spend) => spend.date != vol.date,
            None => true,
        };
        if show_vol {
            record_lines.push(format!(
                "• Peak Volume:      {} ({} tok · {})",
                vol.date,
                format_u64(vol.total_tokens),
                format_usd(vol.cost_usd)
            ));
        }
    }
    if !insights.spikes.is_empty() {
        let spike = &insights.spikes[0];
        let ratio = if spike.baseline_median > 0 {
            spike.total_tokens as f64 / spike.baseline_median as f64
        } else {
            1.0
        };
        let mut meta = Vec::new();
        if let Some(src) = &spike.top_source {
            meta.push(src.clone());
        }
        if let Some(m) = &spike.top_model {
            meta.push(m.clone());
        }
        if let Some(p) = &spike.top_project {
            meta.push(crate::insights::sanitize_project_label(p));
        }
        let meta_str = if !meta.is_empty() {
            format!(" [{}]", meta.join(" / "))
        } else {
            String::new()
        };
        record_lines.push(format!(
            "• Volume Spike:     {} was {:.1}× baseline median ({} tok){}",
            spike.date,
            ratio,
            format_u64(spike.baseline_median),
            meta_str
        ));
    }
    if let Some(streak) = insights.current_streak_days {
        let mut text = format!("• Active Streak:    {} days", streak);
        if let Some(avg) = insights.avg_tokens_per_active_day {
            text.push_str(&format!(" (avg {} tok/day)", format_u64(avg)));
        }
        record_lines.push(text);
    }
    if !record_lines.is_empty() {
        sections.push(("📈 Records & Milestones", record_lines));
    }

    // 4. Human Scale & Equivalences
    if let Some(scale) = &insights.scale {
        let mut scale_lines = Vec::new();
        scale_lines.push(format!(
            "• Context Digested: {}",
            scale.digestion_headline()
        ));
        scale_lines.push(format!("• Active Written:   {}", scale.creation_headline()));
        sections.push(("🌐 Human Scale & Equivalences", scale_lines));
    }

    if sections.is_empty() {
        return;
    }

    print_boxed_card_with_width(" Insights & Highlights ", &sections, target_width);
}

pub fn strip_ansi_codes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn format_card_line_exact(
    content: &str,
    total_width: usize,
    border_color: &str,
    reset: &str,
) -> String {
    let plain = strip_ansi_codes(content);
    let plain_len = plain.as_str().width();
    let inner_width = total_width.saturating_sub(2);
    if plain_len > inner_width {
        let truncated = truncate_text(&plain, inner_width);
        let trunc_len = truncated.as_str().width();
        let padding = " ".repeat(inner_width.saturating_sub(trunc_len));
        format!(
            "{}│{}{}{}{}│{}",
            border_color, reset, truncated, padding, border_color, reset
        )
    } else {
        let padding = " ".repeat(inner_width - plain_len);
        format!(
            "{}│{}{}{}{}│{}",
            border_color, reset, content, padding, border_color, reset
        )
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

// ---------------------------------------------------------------------------
// Interactive command menu (bare `tu` on a TTY) — category-grouped picker.
// Keep in sync with the Commands enum / help legend in cli.rs.
// ---------------------------------------------------------------------------

enum MenuRow {
    Header(&'static str),
    Cmd {
        name: &'static str,
        desc: &'static str,
    },
}

fn menu_rows() -> Vec<MenuRow> {
    use MenuRow::{Cmd, Header};
    let mut rows = vec![
        Header("Reporting"),
        Cmd {
            name: "today",
            desc: "Today's usage + coding activity",
        },
        Cmd {
            name: "daily",
            desc: "Per-day token usage and cost",
        },
        Cmd {
            name: "weekly",
            desc: "Per-week token usage",
        },
        Cmd {
            name: "monthly",
            desc: "Per-month token usage",
        },
        Cmd {
            name: "activity",
            desc: "Coding-activity view with per-day breakdowns",
        },
        Cmd {
            name: "blocks",
            desc: "Usage grouped by 5-hour billing blocks",
        },
        Cmd {
            name: "session",
            desc: "Per-session token usage",
        },
        Cmd {
            name: "carbon",
            desc: "Carbon footprint, energy (kWh), and water report",
        },
        Cmd {
            name: "scale",
            desc: "Human scale & information equivalences (words, books, Wikipedias)",
        },
        Cmd {
            name: "rank",
            desc: "Local leaderboard & Hall of Fame records",
        },
        Cmd {
            name: "breakdown",
            desc: "Cross-stratified breakdown (Projects × Models)",
        },
        Cmd {
            name: "tui",
            desc: "Interactive unified TUI dashboard (Scale, Hall of Fame, Matrix, Timeline)",
        },
        Header("Live"),
        Cmd {
            name: "live",
            desc: "Live-updating usage view",
        },
        Cmd {
            name: "top",
            desc: "Real-time per-session viewer (htop for tokens)",
        },
        Cmd {
            name: "gui",
            desc: "Desktop GUI (Iced)",
        },
        Header("Integration"),
        Cmd {
            name: "statusline init",
            desc: "Set up the Claude Code status line (backs up settings.json, asks first)",
        },
        Cmd {
            name: "statusline",
            desc: "Print the one-line status (what the statusLine hook renders)",
        },
        Cmd {
            name: "img",
            desc: "Render a usage report as a PNG",
        },
        Cmd {
            name: "heartbeat",
            desc: "Editor-activity heartbeat collector and stats",
        },
        Header("Diagnostics"),
        Cmd {
            name: "doctor",
            desc: "Inspect roots, files, cache and pricing health",
        },
        Cmd {
            name: "parity",
            desc: "Compare tu totals against ccusage",
        },
        Cmd {
            name: "completions",
            desc: "Generate shell completion scripts (bash, zsh, fish, etc.)",
        },
    ];

    // Balances: only surface providers whose API key is actually configured, so
    // the menu shows what you can use, not 6 commands that just error. The env
    // var names match each provider's documented convention. Antigravity is a
    // local language-server probe (no key) so it's always shown.
    let has = |key: &str| std::env::var_os(key).is_some();
    let mut balances: Vec<MenuRow> = Vec::new();
    // Antigravity detection is macOS/Linux-only in tu (ps/lsof) — hide on Windows.
    if !cfg!(windows) {
        balances.push(Cmd {
            name: "antigravity",
            desc: "Show Antigravity plan and usage limits",
        });
    }
    if has("DEEPSEEK_API_KEY") {
        balances.push(Cmd {
            name: "deepseek",
            desc: "Show DeepSeek API credit balance",
        });
    }
    if has("OPENROUTER_API_KEY") {
        balances.push(Cmd {
            name: "openrouter",
            desc: "Show OpenRouter API credit balance",
        });
    }
    if has("XAI_API_KEY") {
        balances.push(Cmd {
            name: "grok",
            desc: "Show Grok (xAI) credit balance",
        });
    }
    if has("MOONSHOT_API_KEY") {
        balances.push(Cmd {
            name: "kimi",
            desc: "Show Kimi (Moonshot) credit balance",
        });
    }
    if has("ANTHROPIC_API_KEY") || has("ANTHROPIC_ADMIN_KEY") {
        balances.push(Cmd {
            name: "anthropic-api",
            desc: "Show Anthropic API usage today",
        });
    }
    if !balances.is_empty() {
        rows.push(Header("Balances"));
        rows.extend(balances);
    }

    rows
}

fn menu_visible(rows: &[MenuRow], filter: &str) -> Vec<usize> {
    if filter.is_empty() {
        return (0..rows.len()).collect();
    }
    let needle = filter.to_ascii_lowercase();
    rows.iter()
        .enumerate()
        .filter_map(|(i, row)| match row {
            MenuRow::Cmd { name, desc }
                if name.to_ascii_lowercase().contains(&needle)
                    || desc.to_ascii_lowercase().contains(&needle) =>
            {
                Some(i)
            }
            _ => None,
        })
        .collect()
}

fn first_cmd_pos(rows: &[MenuRow], visible: &[usize]) -> usize {
    visible
        .iter()
        .position(|&ri| matches!(rows[ri], MenuRow::Cmd { .. }))
        .unwrap_or(0)
}

fn step_cmd_pos(rows: &[MenuRow], visible: &[usize], pos: usize, forward: bool) -> usize {
    let mut p = pos;
    loop {
        let next = if forward {
            if p + 1 >= visible.len() {
                return pos;
            }
            p + 1
        } else {
            if p == 0 {
                return pos;
            }
            p - 1
        };
        p = next;
        if matches!(rows[visible[p]], MenuRow::Cmd { .. }) {
            return p;
        }
    }
}

/// Bare `tu` on an interactive terminal opens this picker. Returns the chosen
/// command name, or None if the user quit.
pub(crate) fn run_command_menu() -> Result<Option<String>> {
    if !io::stdout().is_terminal() {
        return Ok(None);
    }
    let rows = menu_rows();
    let mut filter = String::new();
    let mut visible = menu_visible(&rows, &filter);
    let mut pos = first_cmd_pos(&rows, &visible);

    let mut session = TuiSession::enter()?;
    let chosen = loop {
        session
            .terminal
            .draw(|frame| draw_menu(frame, &rows, &visible, pos, &filter))?;

        if !event::poll(Duration::from_millis(150))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Esc => break None,
            KeyCode::Enter => {
                if let Some(&ri) = visible.get(pos)
                    && let MenuRow::Cmd { name, .. } = rows[ri]
                {
                    break Some(name.to_string());
                }
            }
            KeyCode::Down => pos = step_cmd_pos(&rows, &visible, pos, true),
            KeyCode::Up => pos = step_cmd_pos(&rows, &visible, pos, false),
            KeyCode::Backspace => {
                filter.pop();
                visible = menu_visible(&rows, &filter);
                pos = first_cmd_pos(&rows, &visible);
            }
            KeyCode::Char(c) => {
                filter.push(c);
                visible = menu_visible(&rows, &filter);
                pos = first_cmd_pos(&rows, &visible);
            }
            _ => {}
        }
    };
    drop(session);
    Ok(chosen)
}

fn draw_menu(
    frame: &mut ratatui::Frame<'_>,
    rows: &[MenuRow],
    visible: &[usize],
    pos: usize,
    filter: &str,
) {
    let [list_area, footer] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(frame.area());

    let items: Vec<ListItem> = visible
        .iter()
        .map(|&ri| match &rows[ri] {
            MenuRow::Header(title) => ListItem::new(Line::from(Span::styled(
                (*title).to_string(),
                Style::default()
                    .fg(TuiColor::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))),
            MenuRow::Cmd { name, desc } => ListItem::new(Line::from(vec![
                Span::styled(format!("{name:<17}"), Style::default().fg(TuiColor::White)),
                Span::styled((*desc).to_string(), Style::default().fg(TuiColor::DarkGray)),
            ])),
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(pos.min(visible.len().saturating_sub(1))));
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" tu — pick a command "),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("❯ ");
    frame.render_stateful_widget(list, list_area, &mut state);

    let hint = if filter.is_empty() {
        "↑/↓ move · enter run · type to filter · esc quit".to_string()
    } else {
        format!("/{filter}    enter run · esc quit")
    };
    frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(TuiColor::DarkGray)),
        footer,
    );
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

pub(crate) fn format_u64(value: u64) -> String {
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

pub(crate) fn format_usd(value: f64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_boxed_card_exact_target_width() {
        let sections = vec![
            (
                "💰 Spending & Efficiency",
                vec![
                    "• Cache Efficiency: 98.7% hit rate · Saved ~$147481.80 vs uncached rates"
                        .to_string(),
                    "• Effective Rate:   $0.68 / 1M tokens (avg 1,469,422 tok/$)".to_string(),
                ],
            ),
            (
                "🏆 Top Drivers",
                vec!["• Dominant Model:   claude-sonnet-5 (32.0% of all tokens)".to_string()],
            ),
        ];

        for target in [65, 72, 80, 95] {
            let rendered =
                render_boxed_card_with_width(" Insights & Highlights ", &sections, Some(target));
            for line in rendered.lines() {
                let plain = strip_ansi_codes(line);
                assert_eq!(
                    plain.as_str().width(),
                    target,
                    "Line {:?} does not match target width {}",
                    plain,
                    target
                );
            }
        }
    }
}
