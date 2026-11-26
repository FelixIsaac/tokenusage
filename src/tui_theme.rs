use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};

use crate::carbon::format_commas_u64;

pub(crate) const BG_CANVAS: Color = Color::Rgb(10, 14, 23);
pub(crate) const BG_SURFACE: Color = Color::Rgb(17, 24, 39);
pub(crate) const BG_ELEVATED: Color = Color::Rgb(26, 35, 50);
pub(crate) const BORDER_SUBTLE: Color = Color::Rgb(38, 49, 71);
pub(crate) const BORDER_FOCUS: Color = Color::Rgb(0, 229, 255);

pub(crate) const TEXT_PRIMARY: Color = Color::Rgb(241, 245, 249);
pub(crate) const TEXT_SECONDARY: Color = Color::Rgb(148, 163, 184);
pub(crate) const TEXT_MUTED: Color = Color::Rgb(100, 116, 139);

pub(crate) const ACCENT_CYAN: Color = Color::Rgb(0, 229, 255);
pub(crate) const ACCENT_AMBER: Color = Color::Rgb(251, 191, 36);
pub(crate) const ACCENT_EMERALD: Color = Color::Rgb(16, 185, 129);
pub(crate) const ACCENT_VIOLET: Color = Color::Rgb(167, 139, 250);
pub(crate) const ACCENT_ROSE: Color = Color::Rgb(251, 113, 133);

pub(crate) const HEATMAP_COLORS: [Color; 5] = [
    Color::Rgb(22, 27, 34),
    Color::Rgb(14, 68, 89),
    Color::Rgb(0, 125, 158),
    Color::Rgb(0, 184, 217),
    Color::Rgb(56, 242, 255),
];

pub(crate) fn provider_accent(label: &str) -> Color {
    let lower = label.trim().to_ascii_lowercase();
    if lower.contains("claude") {
        Color::Rgb(249, 115, 22)
    } else if lower.contains("codex") || lower.contains("openai") || lower.contains("gpt") {
        Color::Rgb(16, 185, 129)
    } else if lower.contains("gemini") || lower.contains("antigravity") {
        Color::Rgb(56, 189, 248)
    } else if lower.contains("opencode") {
        Color::Rgb(167, 139, 250)
    } else if lower.contains("grok") {
        Color::Rgb(251, 113, 133)
    } else if lower == "pi" || lower.contains("pi ") || lower.ends_with(":pi") {
        Color::Rgb(251, 191, 36)
    } else if lower.contains("muse") {
        Color::Rgb(232, 121, 249)
    } else {
        ACCENT_CYAN
    }
}

pub(crate) fn card_block(title: &str, focused: bool) -> Block<'_> {
    let border_color = if focused { BORDER_FOCUS } else { BORDER_SUBTLE };
    let title_color = if focused { ACCENT_CYAN } else { TEXT_PRIMARY };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(BG_SURFACE).fg(TEXT_PRIMARY))
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(title_color)
                .add_modifier(Modifier::BOLD),
        ))
}

#[inline]
pub(crate) fn ease_out_cubic(t: f64) -> f64 {
    let clamped = t.clamp(0.0, 1.0);
    1.0 - (1.0 - clamped).powi(3)
}

pub(crate) fn format_compact_tokens(n: u64) -> String {
    if n >= 1_000_000_000_000 {
        format_compact_decimal(n as f64 / 1_000_000_000_000.0, "T")
    } else if n >= 1_000_000_000 {
        format_compact_decimal(n as f64 / 1_000_000_000.0, "B")
    } else if n >= 1_000_000 {
        format_compact_decimal(n as f64 / 1_000_000.0, "M")
    } else if n >= 1_000 {
        format_compact_decimal(n as f64 / 1_000.0, "K")
    } else {
        n.to_string()
    }
}

fn format_compact_decimal(val: f64, suffix: &str) -> String {
    let rounded = (val * 10.0).round() / 10.0;
    if (rounded - rounded.trunc()).abs() < f64::EPSILON {
        format!("{}{}", rounded as u64, suffix)
    } else {
        format!("{rounded:.1}{suffix}")
    }
}

pub(crate) fn format_compact_usd(usd: f64) -> String {
    if usd >= 1_000.0 {
        format!("${}", format_commas_u64(usd.round() as u64))
    } else if usd >= 100.0 {
        format!("${usd:.0}")
    } else {
        format!("${usd:.2}")
    }
}

pub(crate) fn keybar_line(bindings: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::with_capacity(bindings.len() * 4);
    for (idx, (key, label)) in bindings.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled("  ", Style::default().fg(TEXT_MUTED)));
        }
        spans.push(Span::styled(
            format!(" {key} "),
            Style::default()
                .bg(BG_ELEVATED)
                .fg(ACCENT_CYAN)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {label}"),
            Style::default().fg(TEXT_SECONDARY),
        ));
    }
    Line::from(spans)
}

pub(crate) fn render_segmented_bar(pct: f64, width: usize, filled_color: Color) -> Line<'static> {
    let width = width.max(4);
    let clamped = pct.clamp(0.0, 100.0);
    let total_eighths = ((clamped / 100.0) * (width * 8) as f64).round() as usize;
    let full_blocks = total_eighths / 8;
    let rem = total_eighths % 8;

    let mut filled = "█".repeat(full_blocks.min(width));
    if rem > 0 && full_blocks < width {
        let partial = match rem {
            1 => "▏",
            2 => "▎",
            3 => "▍",
            4 => "▌",
            5 => "▋",
            6 => "▊",
            _ => "▉",
        };
        filled.push_str(partial);
    }
    let used_cells = filled.chars().count();
    let empty = "░".repeat(width.saturating_sub(used_cells));

    Line::from(vec![
        Span::styled(filled, Style::default().fg(filled_color)),
        Span::styled(empty, Style::default().fg(BORDER_SUBTLE)),
    ])
}

pub(crate) fn render_sparkline_24h(buckets: &[u64; 24], color: Color) -> Line<'static> {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max_val = buckets.iter().copied().max().unwrap_or(0);
    let spans = buckets
        .iter()
        .map(|&v| {
            if max_val == 0 || v == 0 {
                Span::styled("▁▁", Style::default().fg(BORDER_SUBTLE))
            } else {
                let idx = (((v as f64 / max_val as f64) * 7.0).round() as usize).min(7);
                let ch = BARS[idx];
                let c = if idx >= 6 { ACCENT_AMBER } else { color };
                Span::styled(format!("{ch}{ch}"), Style::default().fg(c))
            }
        })
        .collect::<Vec<_>>();
    Line::from(spans)
}

/// Renders a 7-row × N-col contribution grid (intensity 0..=4) into 4 terminal
/// lines by packing pairs of vertical cells into upper half-blocks `▀`.
pub(crate) fn render_halfblock_heatmap(grid: &[Vec<u8>]) -> Vec<Line<'static>> {
    let cols = grid.iter().map(Vec::len).max().unwrap_or(0);
    let mut lines = Vec::with_capacity(4);
    for pair in 0..4 {
        let top_row = pair * 2;
        let bot_row = top_row + 1;
        let mut spans = Vec::with_capacity(cols);
        for col in 0..cols {
            let top_lvl = grid
                .get(top_row)
                .and_then(|r| r.get(col))
                .copied()
                .unwrap_or(0)
                .min(4) as usize;
            let top_color = HEATMAP_COLORS[top_lvl];

            if bot_row < 7 {
                let bot_lvl = grid
                    .get(bot_row)
                    .and_then(|r| r.get(col))
                    .copied()
                    .unwrap_or(0)
                    .min(4) as usize;
                let bot_color = HEATMAP_COLORS[bot_lvl];
                spans.push(Span::styled(
                    "▀",
                    Style::default().fg(top_color).bg(bot_color),
                ));
            } else {
                spans.push(Span::styled(
                    "▀",
                    Style::default().fg(top_color).bg(BG_SURFACE),
                ));
            }
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// Renders large 5-row geometric block typography for hero KPI counters.
pub(crate) fn render_big_text(text: &str, color: Color) -> Vec<Line<'static>> {
    let mut rows = vec![String::new(); 5];
    for (idx, ch) in text.chars().enumerate() {
        let glyph = glyph_5row(ch);
        if idx > 0 {
            for r in &mut rows {
                r.push(' ');
            }
        }
        for (row_idx, part) in glyph.iter().enumerate() {
            rows[row_idx].push_str(part);
        }
    }

    rows.into_iter()
        .map(|row| {
            Line::from(Span::styled(
                row,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ))
        })
        .collect()
}

fn glyph_5row(ch: char) -> [&'static str; 5] {
    match ch.to_ascii_uppercase() {
        '0' => ["███", "█ █", "█ █", "█ █", "███"],
        '1' => [" ██", "  █", "  █", "  █", " ██"],
        '2' => ["███", "  █", "███", "█  ", "███"],
        '3' => ["███", "  █", " ██", "  █", "███"],
        '4' => ["█ █", "█ █", "███", "  █", "  █"],
        '5' => ["███", "█  ", "███", "  █", "███"],
        '6' => ["███", "█  ", "███", "█ █", "███"],
        '7' => ["███", "  █", "  █", " █ ", " █ "],
        '8' => ["███", "█ █", "███", "█ █", "███"],
        '9' => ["███", "█ █", "███", "  █", "███"],
        'A' => ["███", "█ █", "███", "█ █", "█ █"],
        'B' => ["██ ", "█ █", "██ ", "█ █", "██ "],
        'C' => ["███", "█  ", "█  ", "█  ", "███"],
        'D' => ["██ ", "█ █", "█ █", "█ █", "██ "],
        'E' => ["███", "█  ", "██ ", "█  ", "███"],
        'F' => ["███", "█  ", "██ ", "█  ", "█  "],
        'G' => ["███", "█  ", "█ █", "█ █", "███"],
        'H' => ["█ █", "█ █", "███", "█ █", "█ █"],
        'I' => ["███", " █ ", " █ ", " █ ", "███"],
        'J' => ["███", "  █", "  █", "█ █", "███"],
        'K' => ["█ █", "█ █", "██ ", "█ █", "█ █"],
        'L' => ["█  ", "█  ", "█  ", "█  ", "███"],
        'M' => ["█   █", "██ ██", "█ █ █", "█   █", "█   █"],
        'N' => ["█  █", "██ █", "█ ██", "█  █", "█  █"],
        'O' => ["███", "█ █", "█ █", "█ █", "███"],
        'P' => ["███", "█ █", "███", "█  ", "█  "],
        'Q' => ["███", "█ █", "█ █", " ██", "  █"],
        'R' => ["███", "█ █", "██ ", "█ █", "█ █"],
        'S' => ["███", "█  ", "███", "  █", "███"],
        'T' => ["███", " █ ", " █ ", " █ ", " █ "],
        'U' => ["█ █", "█ █", "█ █", "█ █", "███"],
        'V' => ["█ █", "█ █", "█ █", " █ ", " █ "],
        'W' => ["█   █", "█   █", "█ █ █", "██ ██", "█   █"],
        'X' => ["█ █", " █ ", " █ ", " █ ", "█ █"],
        'Y' => ["█ █", "█ █", " █ ", " █ ", " █ "],
        'Z' => ["███", "  █", " █ ", "█  ", "███"],
        '.' => [" ", " ", " ", " ", "█"],
        ',' => [" ", " ", " ", "█", "█"],
        ':' => [" ", "█", " ", "█", " "],
        '$' => ["███", "█▄ ", "███", " ▄█", "███"],
        '%' => ["█ █", "  █", " █ ", "█  ", "█ █"],
        '+' => ["   ", " █ ", "███", " █ ", "   "],
        '-' => ["   ", "   ", "███", "   ", "   "],
        '/' => ["  █", "  █", " █ ", "█  ", "█  "],
        '×' | '*' => ["   ", "█ █", " █ ", "█ █", "   "],
        ' ' => ["  ", "  ", "  ", "  ", "  "],
        _ => ["██", "█ ", "█ ", "█ ", "██"],
    }
}

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_big_text_always_produces_five_rows() {
        let lines = render_big_text("164.3B", ACCENT_CYAN);
        assert_eq!(lines.len(), 5);
        assert!(lines.iter().all(|l| !l.spans.is_empty()));
    }

    #[test]
    fn render_halfblock_heatmap_packs_seven_rows_into_four_lines() {
        let grid = vec![vec![0, 1, 2, 3, 4]; 7];
        let lines = render_halfblock_heatmap(&grid);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].spans.len(), 5);
    }

    #[test]
    fn ease_out_cubic_bounds_and_monotonic() {
        assert!((ease_out_cubic(-0.5) - 0.0).abs() < f64::EPSILON);
        assert!((ease_out_cubic(0.0) - 0.0).abs() < f64::EPSILON);
        assert!(ease_out_cubic(0.5) > 0.5);
        assert!((ease_out_cubic(1.0) - 1.0).abs() < f64::EPSILON);
        assert!((ease_out_cubic(2.0) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn compact_formatters_scale_cleanly() {
        assert_eq!(format_compact_tokens(950), "950");
        assert_eq!(format_compact_tokens(1_500), "1.5K");
        assert_eq!(format_compact_tokens(12_000_000), "12M");
        assert_eq!(format_compact_tokens(164_300_000_000), "164.3B");
        assert_eq!(format_compact_usd(42.5), "$42.50");
        assert_eq!(format_compact_usd(1240.2), "$1,240");
    }
}
