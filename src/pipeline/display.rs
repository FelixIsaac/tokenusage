use std::io::IsTerminal;

use comfy_table::{
    Attribute, Cell as TableCell, Color as TableColor, ContentArrangement,
    Table as TextTable, modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL,
};
use terminal_size::{Width, terminal_size};


use super::*;
use super::activity_report::ActivityOverview;

pub(super) fn use_styled_output() -> bool {
    std::io::stdout().is_terminal() || std::env::var("CLICOLOR_FORCE").is_ok()
}

pub(super) fn create_text_table() -> TextTable {
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

pub(super) fn header_cell(text: &str) -> TableCell {
    let mut cell = TableCell::new(text).add_attribute(Attribute::Bold);
    if use_styled_output() {
        cell = cell.fg(TableColor::Cyan);
    }
    cell
}

pub(super) fn value_cell(text: &str, color: Option<TableColor>) -> TableCell {
    let mut cell = TableCell::new(text);
    if use_styled_output()
        && let Some(color) = color
    {
        cell = cell.fg(color);
    }
    cell
}

pub(super) fn metric_value_cell(
    label: &str,
    value: &str,
    color: Option<TableColor>,
    inline: bool,
) -> TableCell {
    let content = if inline {
        format!("{label}: {value}")
    } else {
        format!("{label}\n{value}")
    };
    let mut cell = TableCell::new(content);
    if inline {
        cell = cell.add_attribute(Attribute::Bold);
    }
    if use_styled_output()
        && let Some(color) = color
    {
        cell = cell.fg(color);
    }
    cell
}

pub(super) fn should_render_activity_metrics_inline(overview: &ActivityOverview) -> bool {
    let width = detect_terminal_width();
    let row_1 = estimated_metric_row_width(&[
        (
            "Coding",
            overview
                .activity
                .as_ref()
                .map(|summary| summary.text.as_str())
                .unwrap_or("-"),
        ),
        (
            "Avg / day",
            overview.avg_coding_per_day.as_deref().unwrap_or("-"),
        ),
        ("Tokens", &format_u64(overview.totals.total_tokens)),
        ("Cost", &format_usd(overview.totals.cost_usd)),
    ]);
    let row_2 = estimated_metric_row_width(&[
        (
            "Tok / hr",
            &overview
                .tokens_per_hour
                .map(format_u64)
                .unwrap_or_else(|| "-".to_string()),
        ),
        (
            "Cost / hr",
            &overview
                .cost_per_hour
                .map(format_usd)
                .unwrap_or_else(|| "-".to_string()),
        ),
        ("Top model", overview.top_model.as_deref().unwrap_or("-")),
        (
            "Top project",
            overview
                .activity
                .as_ref()
                .and_then(|activity| activity.top_project.as_deref())
                .unwrap_or("-"),
        ),
    ]);
    let range_text = if overview.start == overview.end {
        overview.start.clone()
    } else {
        format!("{} -> {}", overview.start, overview.end)
    };
    let row_3 = estimated_metric_row_width(&[
        (
            "Top lang",
            overview
                .activity
                .as_ref()
                .and_then(|activity| activity.top_language.as_deref())
                .unwrap_or("-"),
        ),
        (
            "Top source",
            overview
                .activity
                .as_ref()
                .and_then(|activity| activity.top_source.as_deref())
                .unwrap_or("-"),
        ),
        ("Range", &range_text),
        ("", ""),
    ]);
    width >= row_1.max(row_2).max(row_3)
}

pub(super) fn estimated_metric_row_width(metrics: &[(&str, &str)]) -> usize {
    let content = metrics
        .iter()
        .map(|(label, value)| {
            if label.is_empty() {
                4usize
            } else {
                label.chars().count() + value.chars().count() + 6
            }
        })
        .sum::<usize>();
    content + metrics.len().saturating_mul(3) + 8
}

pub(super) fn detect_terminal_width() -> usize {
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

pub(super) fn pretty_axis_upper_bound(value: u64) -> u64 {
    if value == 0 {
        return 1;
    }
    let magnitude = 10u64.pow(value.ilog10());
    for factor in [1u64, 2, 5, 10] {
        let candidate = magnitude.saturating_mul(factor);
        if candidate >= value {
            return candidate;
        }
    }
    value
}

pub(super) fn ratatui_buffer_lines(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    buffer
        .content
        .chunks(buffer.area.width as usize)
        .map(|cells| {
            let mut line = String::new();
            for cell in cells {
                line.push_str(cell.symbol());
            }
            line.trim_end().to_string()
        })
        .collect()
}

pub(super) fn render_progress_meter(ratio: f64, width: usize) -> String {
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

pub(super) fn truncate_display_text(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let mut out = value.chars().take(keep).collect::<String>();
    out.push('…');
    out
}

pub(super) fn format_token_compact(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{n}")
    }
}

pub(super) fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

pub(super) fn shorten_model_name(model: &str) -> String {
    let s = model
        .replace("claude-", "")
        .replace("codex-", "cx-")
        .replace("-20251001", "")
        .replace("-20250131", "");
    if s.len() > 16 { s[..16].to_string() } else { s }
}

