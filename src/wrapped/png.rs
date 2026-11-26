use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use anyhow::{Context, Result};
use image::{Rgba, RgbaImage, imageops::FilterType};
use resvg::{tiny_skia, usvg};

use crate::cli::WrappedTheme;
use crate::tui_theme::{format_compact_tokens, format_compact_usd};

use super::data::WrappedReport;

const CANVAS_W: u32 = 1200;
const CANVAS_H: u32 = 1500;
const POSTER_FONT_BYTES: &[u8] = include_bytes!("../../assets/fonts/Audiowide-Regular.ttf");
const LOGO_SVG_BYTES: &[u8] = include_bytes!("../../assets/branding/tokenusage-logomark.svg");
static FONT_CACHE: OnceLock<Option<FontArc>> = OnceLock::new();

#[derive(Clone, Copy)]
struct PosterBox {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: f32,
}

impl PosterBox {
    const fn new(x: i32, y: i32, w: i32, h: i32, r: f32) -> Self {
        Self { x, y, w, h, r }
    }
}

#[derive(Clone, Copy)]
struct ThemePalette {
    is_carbon_weave: bool,
    bg_top: [f32; 3],
    bg_bottom: [f32; 3],
    orb_a: [f32; 3],
    orb_b: [f32; 3],
    shell_bg: Rgba<u8>,
    core_bg: Rgba<u8>,
    elevated_bg: Rgba<u8>,
    shell_border: Rgba<u8>,
    primary: Rgba<u8>,
    secondary: Rgba<u8>,
    highlight: Rgba<u8>,
    success: Rgba<u8>,
    text: Rgba<u8>,
    muted: Rgba<u8>,
    dim: Rgba<u8>,
    heat: [Rgba<u8>; 5],
}

impl ThemePalette {
    fn from_theme(theme: WrappedTheme) -> Self {
        match theme {
            WrappedTheme::Cyber => Self {
                is_carbon_weave: false,
                bg_top: [7.0, 10.0, 20.0],
                bg_bottom: [11.0, 16.0, 32.0],
                orb_a: [0.0, 229.0, 255.0],
                orb_b: [167.0, 139.0, 250.0],
                shell_bg: rgba(11, 16, 29),
                core_bg: rgba(15, 23, 40),
                elevated_bg: rgba(24, 34, 56),
                shell_border: rgba(42, 58, 92),
                primary: rgba(0, 229, 255),
                secondary: rgba(167, 139, 250),
                highlight: rgba(251, 191, 36),
                success: rgba(16, 185, 129),
                text: rgba(244, 248, 255),
                muted: rgba(154, 170, 196),
                dim: rgba(108, 124, 152),
                heat: [
                    rgba(20, 28, 44),
                    rgba(14, 72, 98),
                    rgba(0, 136, 174),
                    rgba(0, 194, 228),
                    rgba(82, 246, 255),
                ],
            },
            WrappedTheme::Carbon => Self {
                is_carbon_weave: true,
                bg_top: [10.0, 11.0, 14.0],
                bg_bottom: [18.0, 19.0, 24.0],
                orb_a: [245.0, 158.0, 11.0],
                orb_b: [249.0, 115.0, 22.0],
                shell_bg: rgba(15, 17, 21),
                core_bg: rgba(22, 25, 31),
                elevated_bg: rgba(34, 38, 46),
                shell_border: rgba(68, 72, 82),
                primary: rgba(245, 158, 11),
                secondary: rgba(249, 115, 22),
                highlight: rgba(251, 191, 36),
                success: rgba(52, 211, 153),
                text: rgba(245, 247, 250),
                muted: rgba(180, 185, 196),
                dim: rgba(126, 132, 145),
                heat: [
                    rgba(28, 30, 36),
                    rgba(92, 46, 14),
                    rgba(168, 86, 16),
                    rgba(234, 138, 18),
                    rgba(255, 204, 64),
                ],
            },
            WrappedTheme::Matrix => Self {
                is_carbon_weave: false,
                bg_top: [4.0, 10.0, 6.0],
                bg_bottom: [7.0, 18.0, 11.0],
                orb_a: [16.0, 185.0, 129.0],
                orb_b: [74.0, 222.0, 128.0],
                shell_bg: rgba(7, 17, 11),
                core_bg: rgba(10, 24, 16),
                elevated_bg: rgba(16, 38, 25),
                shell_border: rgba(30, 74, 48),
                primary: rgba(74, 222, 128),
                secondary: rgba(16, 185, 129),
                highlight: rgba(163, 230, 53),
                success: rgba(52, 211, 153),
                text: rgba(236, 253, 243),
                muted: rgba(134, 194, 158),
                dim: rgba(84, 142, 108),
                heat: [
                    rgba(14, 28, 19),
                    rgba(16, 68, 38),
                    rgba(22, 128, 66),
                    rgba(34, 197, 94),
                    rgba(134, 255, 168),
                ],
            },
            WrappedTheme::Solar => Self {
                is_carbon_weave: false,
                bg_top: [14.0, 8.0, 22.0],
                bg_bottom: [24.0, 12.0, 32.0],
                orb_a: [251.0, 191.0, 36.0],
                orb_b: [251.0, 113.0, 133.0],
                shell_bg: rgba(20, 12, 30),
                core_bg: rgba(28, 17, 42),
                elevated_bg: rgba(44, 26, 62),
                shell_border: rgba(88, 52, 108),
                primary: rgba(251, 191, 36),
                secondary: rgba(251, 113, 133),
                highlight: rgba(56, 189, 248),
                success: rgba(52, 211, 153),
                text: rgba(255, 248, 244),
                muted: rgba(206, 178, 196),
                dim: rgba(148, 118, 148),
                heat: [
                    rgba(32, 20, 44),
                    rgba(116, 36, 68),
                    rgba(198, 66, 94),
                    rgba(249, 132, 64),
                    rgba(255, 218, 96),
                ],
            },
        }
    }
}

pub(crate) fn write_wrapped_png(
    path: &Path,
    report: &WrappedReport,
    theme: WrappedTheme,
) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    let img = render_wrapped_png(report, theme)?;
    img.save(path)
        .with_context(|| format!("Failed to save PNG poster {}", path.display()))
}

pub(crate) fn render_wrapped_png(report: &WrappedReport, theme: WrappedTheme) -> Result<RgbaImage> {
    let pal = ThemePalette::from_theme(theme);
    let mut img = RgbaImage::new(CANVAS_W, CANVAS_H);
    paint_poster_canvas(&mut img, &pal);

    // Outer frame bevel
    stroke_rounded_rect_aa(
        &mut img,
        PosterBox::new(24, 24, CANVAS_W as i32 - 48, CANVAS_H as i32 - 48, 28.0),
        pal.shell_border,
        2.0,
    );

    // 1. Header bar (y=48..128)
    if let Ok(logo) = render_svg_logo(LOGO_SVG_BYTES, 50, 50) {
        overlay(&mut img, &logo, 64, 54);
    }
    draw_text(
        &mut img,
        130,
        54,
        &format!("TOKENUSAGE WRAPPED {}", report.period_label),
        21.0,
        pal.primary,
    );
    draw_text(
        &mut img,
        130,
        84,
        &format!(
            "{}  ·  {}",
            theme.as_str().to_ascii_uppercase(),
            report.timezone
        ),
        14.5,
        pal.dim,
    );

    // Tier pill & privacy pill on top-right
    let tier_pill = PosterBox::new(664, 54, 472, 34, 17.0);
    draw_neon_pill(&mut img, tier_pill, pal.elevated_bg, pal.highlight);
    draw_text_centered(
        &mut img,
        tier_pill.x + tier_pill.w / 2,
        tier_pill.y + 9,
        &report.archetype.tier.to_ascii_uppercase(),
        14.5,
        pal.highlight,
    );

    let priv_lbl = if report.include_projects {
        "PROJECTS VISIBLE (OPT-IN)"
    } else {
        "PRIVACY SAFE · PROJECTS REDACTED"
    };
    draw_text_right(&mut img, 1134, 96, priv_lbl, 13.0, pal.success);

    // 2. Hero Double-Bezel Card (y=142..424)
    let hero_box = PosterBox::new(56, 142, 1088, 282, 24.0);
    draw_double_bezel_card(&mut img, hero_box, &pal, true);

    draw_glow_text(
        &mut img,
        88,
        166,
        &report.archetype.title.to_ascii_uppercase(),
        26.0,
        pal.highlight,
        glow_rgba(pal.highlight, 95),
    );
    draw_text_right(
        &mut img,
        1110,
        171,
        "TOTAL TOKENS DIGESTED",
        15.0,
        pal.muted,
    );

    let hero_val = format!(
        "{} TOKENS",
        format_compact_tokens(report.totals.total_tokens)
    );
    let mut hero_px = 84.0f32;
    while hero_px > 54.0 && text_width(&hero_val, hero_px) > 1015.0 {
        hero_px -= 2.0;
    }
    draw_glow_text(
        &mut img,
        88,
        210,
        &hero_val,
        hero_px,
        pal.text,
        glow_rgba(pal.primary, 135),
    );

    draw_text(
        &mut img,
        88,
        312,
        &format!(
            "{} API value   ·   ~{} cache saved ({:.1}% hit / {:.1}× reuse)",
            format_compact_usd(report.totals.cost_usd),
            format_compact_usd(report.cache.savings_usd),
            report.cache.hit_pct,
            report.cache.reuse_ratio,
        ),
        20.0,
        pal.success,
    );

    // Pop-culture flex banner strip inside Hero card
    let flex_pill = PosterBox::new(84, 352, 1032, 48, 12.0);
    draw_neon_pill(&mut img, flex_pill, pal.elevated_bg, pal.primary);
    draw_text_fit(
        &mut img,
        104,
        367,
        992.0,
        &format!(
            "{}   ·   {}",
            report.pop_culture_scale, report.archetype.tagline
        ),
        16.5,
        13.0,
        pal.primary,
    );

    // 3. 4 Double-Bezel KPI Tiles (y=444..576)
    let kpis = [
        (
            "LONGEST STREAK",
            format!("{} DAYS", report.longest_streak_days),
            format!("{} active days logged", report.active_days),
            pal.highlight,
        ),
        (
            "CHRONOTYPE",
            report.chronotype.label.clone(),
            format!(
                "Peak {:02}:00 · {:.0}% night",
                report.chronotype.peak_hour, report.chronotype.night_pct
            ),
            pal.secondary,
        ),
        (
            "PEAK DAY",
            report
                .peak_day
                .as_ref()
                .map_or("-".to_string(), |d| format_compact_tokens(d.tokens)),
            report.peak_day.as_ref().map_or("-".to_string(), |d| {
                format!("{} ({})", d.date, format_compact_usd(d.cost_usd))
            }),
            pal.primary,
        ),
        (
            "OUTPUT WRITTEN",
            format!("{} WORDS", format_compact_tokens(report.scale.output_words)),
            format!("{:.0} novels equiv", report.scale.creation_books),
            pal.success,
        ),
    ];
    let tile_w = 257;
    let tile_gap = 20;
    for (idx, (lbl, val, sub, color)) in kpis.iter().enumerate() {
        let tx = 56 + idx as i32 * (tile_w + tile_gap);
        draw_double_bezel_card(
            &mut img,
            PosterBox::new(tx, 444, tile_w, 132, 18.0),
            &pal,
            false,
        );
        draw_text(&mut img, tx + 20, 464, lbl, 13.5, pal.muted);
        draw_text_fit(
            &mut img,
            tx + 20,
            496,
            (tile_w - 40) as f32,
            val,
            24.0,
            14.0,
            *color,
        );
        draw_text_fit(
            &mut img,
            tx + 20,
            538,
            (tile_w - 40) as f32,
            sub,
            13.5,
            11.5,
            pal.dim,
        );
    }

    // 4. Glowing 53-Week Activity Pulse (y=596..824)
    draw_double_bezel_card(
        &mut img,
        PosterBox::new(56, 596, 1088, 228, 20.0),
        &pal,
        false,
    );
    draw_text(&mut img, 88, 620, "53-WEEK ACTIVITY PULSE", 17.5, pal.text);
    draw_text_right(
        &mut img,
        1112,
        622,
        &format!(
            "{} -> {}   |   {} active days",
            report.heatmap.start_date, report.heatmap.end_date, report.active_days
        ),
        14.0,
        pal.dim,
    );
    draw_glow_heatmap(&mut img, 88, 658, 1024, &report.heatmap.grid, &pal);

    // 5. Split Middle: Models (left) + 24h Circadian Equalizer & Providers (right) (y=844..1168)
    draw_double_bezel_card(
        &mut img,
        PosterBox::new(56, 844, 618, 324, 20.0),
        &pal,
        false,
    );
    draw_text(&mut img, 88, 868, "TOP FRONTIER MODELS", 17.5, pal.text);
    for (idx, m) in report.top_models.iter().take(5).enumerate() {
        let ry = 910 + idx as i32 * 48;
        let m_color = provider_or_theme_color(&m.raw_name, &pal);
        draw_text(
            &mut img,
            88,
            ry,
            &format!("#{} {}", idx + 1, truncate_chars(&m.name, 25)),
            18.5,
            pal.text,
        );
        draw_text_right(
            &mut img,
            642,
            ry,
            &format!("{} ({:.0}%)", format_compact_tokens(m.tokens), m.share_pct),
            17.5,
            m_color,
        );
        draw_neon_progress_bar(
            &mut img,
            PosterBox::new(88, ry + 25, 554, 9, 4.5),
            m.share_pct,
            pal.elevated_bg,
            m_color,
        );
    }

    draw_double_bezel_card(
        &mut img,
        PosterBox::new(694, 844, 450, 324, 20.0),
        &pal,
        false,
    );
    draw_text(
        &mut img,
        724,
        868,
        "24H CIRCADIAN & ARSENAL",
        17.5,
        pal.text,
    );
    draw_circadian_equalizer(
        &mut img,
        724,
        904,
        390,
        104,
        &report.hourly_tokens,
        report.chronotype.peak_hour,
        &pal,
    );
    draw_text(&mut img, 724, 1018, "00:00", 12.0, pal.dim);
    draw_text_centered(&mut img, 724 + 195, 1018, "12:00", 12.0, pal.dim);
    draw_text_right(&mut img, 1114, 1018, "23:00", 12.0, pal.dim);

    for (idx, p) in report.top_providers.iter().take(3).enumerate() {
        let py = 1048 + idx as i32 * 36;
        let p_color = provider_or_theme_color(&p.name, &pal);
        draw_text(
            &mut img,
            724,
            py,
            &truncate_chars(&p.name, 16),
            16.0,
            pal.text,
        );
        draw_text_right(
            &mut img,
            1114,
            py,
            &format!("{} ({:.1}%)", format_compact_tokens(p.tokens), p.share_pct),
            15.5,
            p_color,
        );
        draw_neon_progress_bar(
            &mut img,
            PosterBox::new(724, py + 21, 390, 6, 3.0),
            p.share_pct,
            pal.elevated_bg,
            p_color,
        );
    }

    // 6. Unlocked Badges & Footprint Card (y=1188..1376)
    draw_double_bezel_card(
        &mut img,
        PosterBox::new(56, 1188, 1088, 188, 20.0),
        &pal,
        false,
    );
    draw_text(
        &mut img,
        88,
        1210,
        "ACHIEVEMENTS & PLANET FOOTPRINT",
        17.0,
        pal.text,
    );

    let badge_count = report.badges.len().min(4);
    if badge_count > 0 {
        let total_w = 1024;
        let gap = 14;
        let pill_w = (total_w - gap * (badge_count as i32 - 1)) / badge_count as i32;
        for (idx, badge) in report.badges.iter().take(4).enumerate() {
            let bx = 88 + idx as i32 * (pill_w + gap);
            let pill = PosterBox::new(bx, 1244, pill_w, 40, 12.0);
            draw_neon_pill(&mut img, pill, pal.elevated_bg, pal.secondary);
            draw_text_fit(
                &mut img,
                bx + 12,
                1256,
                (pill_w - 24) as f32,
                &badge.title.to_ascii_uppercase(),
                14.0,
                11.0,
                pal.highlight,
            );
        }
    }

    draw_text_fit(
        &mut img,
        88,
        1304,
        1024.0,
        &report.scale_headline,
        17.0,
        13.5,
        pal.primary,
    );
    draw_text_fit(
        &mut img,
        88,
        1336,
        1024.0,
        &report.carbon.summary,
        14.5,
        12.0,
        pal.muted,
    );

    // 7. Bottom Viral CTA Footer (y=1402..1452)
    draw_glow_text(
        &mut img,
        58,
        1408,
        "GET YOURS -> brew install FelixIsaac/tap/tokenusage && tu wrapped",
        18.0,
        pal.highlight,
        glow_rgba(pal.highlight, 60),
    );
    draw_text_right(
        &mut img,
        1142,
        1411,
        "github.com/FelixIsaac/tokenusage",
        14.5,
        pal.dim,
    );

    Ok(img)
}

fn paint_poster_canvas(img: &mut RgbaImage, pal: &ThemePalette) {
    let orbs = [
        (
            CANVAS_W as f32 * 0.16,
            CANVAS_H as f32 * 0.10,
            520.0f32,
            pal.orb_a,
            0.18f32,
        ),
        (
            CANVAS_W as f32 * 0.86,
            CANVAS_H as f32 * 0.24,
            460.0f32,
            pal.orb_b,
            0.14f32,
        ),
        (
            CANVAS_W as f32 * 0.50,
            CANVAS_H as f32 * 0.92,
            540.0f32,
            pal.orb_a,
            0.11f32,
        ),
    ];

    for y in 0..CANVAS_H {
        let ty = y as f32 / CANVAS_H as f32;
        let base_r = pal.bg_top[0] + (pal.bg_bottom[0] - pal.bg_top[0]) * ty;
        let base_g = pal.bg_top[1] + (pal.bg_bottom[1] - pal.bg_top[1]) * ty;
        let base_b = pal.bg_top[2] + (pal.bg_bottom[2] - pal.bg_top[2]) * ty;
        let is_grid_y = y % 60 == 0;

        for x in 0..CANVAS_W {
            let mut r = base_r;
            let mut g = base_g;
            let mut b = base_b;

            // Optional forged carbon 2x2 diagonal twill weave
            if pal.is_carbon_weave {
                let phase = ((x / 3) + (y / 3)) % 4;
                let weave = if phase < 2 { 5.0 } else { -3.0 };
                let sub = if ((x / 3).wrapping_sub(y / 3)) % 2 == 0 {
                    1.5
                } else {
                    -1.5
                };
                r += weave + sub;
                g += weave + sub;
                b += weave + sub + 0.5;
            } else if is_grid_y || x % 60 == 0 {
                // Subtle architectural blueprint grid
                r += 4.5;
                g += 5.5;
                b += 9.0;
            }

            // Radial plasma glow orbs
            let fx = x as f32;
            let fy = y as f32;
            for &(ox, oy, rad, col, strength) in &orbs {
                let dx = fx - ox;
                let dy = fy - oy;
                let d2 = dx * dx + dy * dy;
                let r2 = rad * rad;
                if d2 < r2 {
                    let k = (1.0 - d2 / r2).powi(2) * strength;
                    r += col[0] * k;
                    g += col[1] * k;
                    b += col[2] * k;
                }
            }

            // Subtle deterministic micro film grain
            let hash = x
                .wrapping_mul(374_761_393)
                .wrapping_add(y.wrapping_mul(668_265_263));
            let grain = ((hash ^ (hash >> 13)) & 3) as f32 - 1.5;

            img.put_pixel(
                x,
                y,
                Rgba([
                    (r + grain).clamp(0.0, 255.0) as u8,
                    (g + grain).clamp(0.0, 255.0) as u8,
                    (b + grain).clamp(0.0, 255.0) as u8,
                    255,
                ]),
            );
        }
    }
}

fn draw_double_bezel_card(img: &mut RgbaImage, b: PosterBox, pal: &ThemePalette, hero_glow: bool) {
    if hero_glow {
        draw_outer_bloom(img, b, pal.primary, 16, 48);
    }
    fill_rounded_rect_aa(img, b, pal.shell_bg);
    stroke_rounded_rect_aa(img, b, pal.shell_border, 1.5);

    let inner = PosterBox::new(b.x + 6, b.y + 6, b.w - 12, b.h - 12, (b.r - 4.0).max(4.0));
    fill_rounded_rect_aa(img, inner, pal.core_bg);

    let rim_color = if hero_glow {
        glow_rgba(pal.primary, 165)
    } else {
        Rgba([255, 255, 255, 28])
    };
    stroke_rounded_rect_aa(img, inner, rim_color, 1.0);
}

fn draw_neon_pill(img: &mut RgbaImage, b: PosterBox, bg: Rgba<u8>, border: Rgba<u8>) {
    fill_rounded_rect_aa(img, b, bg);
    stroke_rounded_rect_aa(img, b, glow_rgba(border, 165), 1.5);
}

fn draw_outer_bloom(
    img: &mut RgbaImage,
    b: PosterBox,
    color: Rgba<u8>,
    spread: i32,
    peak_alpha: u8,
) {
    let x0 = (b.x - spread).max(0);
    let y0 = (b.y - spread).max(0);
    let x1 = (b.x + b.w + spread).min(CANVAS_W as i32);
    let y1 = (b.y + b.h + spread).min(CANVAS_H as i32);
    let spread_f = spread.max(1) as f32;

    for py in y0..y1 {
        for px in x0..x1 {
            let d = rounded_rect_sdf(px as f32 + 0.5, py as f32 + 0.5, b);
            if d > 0.0 && d < spread_f {
                let falloff = (1.0 - d / spread_f).powi(2);
                let alpha = (peak_alpha as f32 * falloff).round() as u8;
                if alpha > 0 {
                    blend_pixel(img, px, py, Rgba([color[0], color[1], color[2], alpha]));
                }
            }
        }
    }
}

#[inline]
fn rounded_rect_sdf(px: f32, py: f32, b: PosterBox) -> f32 {
    let cx = b.x as f32 + b.w as f32 * 0.5;
    let cy = b.y as f32 + b.h as f32 * 0.5;
    let hx = b.w as f32 * 0.5 - b.r;
    let hy = b.h as f32 * 0.5 - b.r;
    let qx = (px - cx).abs() - hx;
    let qy = (py - cy).abs() - hy;
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - b.r
}

fn fill_rounded_rect_aa(img: &mut RgbaImage, b: PosterBox, color: Rgba<u8>) {
    let x0 = (b.x - 1).max(0);
    let y0 = (b.y - 1).max(0);
    let x1 = (b.x + b.w + 2).min(CANVAS_W as i32);
    let y1 = (b.y + b.h + 2).min(CANVAS_H as i32);

    for py in y0..y1 {
        for px in x0..x1 {
            let d = rounded_rect_sdf(px as f32 + 0.5, py as f32 + 0.5, b);
            if d <= -0.7 {
                blend_pixel(img, px, py, color);
            } else if d < 0.7 {
                let cov = (0.5 - d * 0.7).clamp(0.0, 1.0);
                let alpha = ((color[3] as f32) * cov).round() as u8;
                if alpha > 0 {
                    blend_pixel(img, px, py, Rgba([color[0], color[1], color[2], alpha]));
                }
            }
        }
    }
}

fn stroke_rounded_rect_aa(img: &mut RgbaImage, b: PosterBox, color: Rgba<u8>, thickness: f32) {
    let pad = (thickness + 2.0).ceil() as i32;
    let x0 = (b.x - pad).max(0);
    let y0 = (b.y - pad).max(0);
    let x1 = (b.x + b.w + pad).min(CANVAS_W as i32);
    let y1 = (b.y + b.h + pad).min(CANVAS_H as i32);
    let half = thickness * 0.5;

    for py in y0..y1 {
        for px in x0..x1 {
            let d = rounded_rect_sdf(px as f32 + 0.5, py as f32 + 0.5, b).abs();
            if d <= half + 0.7 {
                let cov = ((half + 0.5) - d).clamp(0.0, 1.0);
                let alpha = ((color[3] as f32) * cov).round() as u8;
                if alpha > 0 {
                    blend_pixel(img, px, py, Rgba([color[0], color[1], color[2], alpha]));
                }
            }
        }
    }
}

fn draw_neon_progress_bar(
    img: &mut RgbaImage,
    b: PosterBox,
    pct: f64,
    track: Rgba<u8>,
    fill: Rgba<u8>,
) {
    fill_rounded_rect_aa(img, b, track);
    let fw = ((b.w as f64) * (pct.clamp(0.0, 100.0) / 100.0)).round() as i32;
    if fw >= 4 {
        let fb = PosterBox::new(b.x, b.y, fw, b.h, b.r.min(fw as f32 * 0.5));
        draw_outer_bloom(img, fb, fill, 6, 45);
        fill_rounded_rect_aa(img, fb, fill);
    }
}

fn draw_glow_heatmap(
    img: &mut RgbaImage,
    x0: i32,
    y0: i32,
    avail_w: i32,
    grid: &[Vec<u8>],
    pal: &ThemePalette,
) {
    let num_cols = grid.first().map_or(53, Vec::len).clamp(1, 54) as i32;
    let gap = 4;
    let cell = ((avail_w + gap) / num_cols - gap).clamp(12, 18);
    let step_x = cell + gap;
    let cell_h = 16;
    let step_y = cell_h + 5;

    for (row, cols) in grid.iter().enumerate().take(7) {
        for (col, &lvl) in cols.iter().enumerate().take(54) {
            let color = pal.heat[(lvl as usize).min(4)];
            let cb = PosterBox::new(
                x0 + col as i32 * step_x,
                y0 + row as i32 * step_y,
                cell,
                cell_h,
                3.5,
            );
            if lvl == 4 {
                draw_outer_bloom(img, cb, color, 5, 58);
            }
            fill_rounded_rect_aa(img, cb, color);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_circadian_equalizer(
    img: &mut RgbaImage,
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    hourly: &[u64; 24],
    peak_hour: u32,
    pal: &ThemePalette,
) {
    let max_val = hourly.iter().copied().max().unwrap_or(1).max(1);
    let gap = 4;
    let bar_w = (w - gap * 23) / 24;
    let step_x = bar_w + gap;

    for (hr, &val) in hourly.iter().enumerate() {
        let ratio = val as f64 / max_val as f64;
        let bh = ((h as f64 * ratio).round() as i32).max(6);
        let bx = x0 + hr as i32 * step_x;
        let by = y0 + h - bh;
        let bar_box = PosterBox::new(bx, by, bar_w, bh, 3.0);
        let color = if hr as u32 == peak_hour {
            pal.highlight
        } else if ratio >= 0.6 {
            pal.primary
        } else {
            pal.secondary
        };
        if hr as u32 == peak_hour {
            draw_outer_bloom(img, bar_box, pal.highlight, 6, 65);
        }
        fill_rounded_rect_aa(img, bar_box, color);
    }
}

fn provider_or_theme_color(label: &str, pal: &ThemePalette) -> Rgba<u8> {
    let lower = label.to_ascii_lowercase();
    if pal.is_carbon_weave {
        if lower.contains("claude") {
            rgba(249, 115, 22)
        } else if lower.contains("grok") {
            rgba(251, 113, 133)
        } else if lower.contains("codex") || lower.contains("gpt") {
            rgba(251, 191, 36)
        } else {
            rgba(226, 232, 240)
        }
    } else if lower.contains("claude") {
        rgba(249, 115, 22)
    } else if lower.contains("codex") || lower.contains("gpt") {
        rgba(16, 185, 129)
    } else if lower.contains("gemini") {
        rgba(56, 189, 248)
    } else if lower.contains("opencode") {
        rgba(167, 139, 250)
    } else if lower.contains("grok") {
        rgba(251, 113, 133)
    } else if lower.contains("pi") {
        rgba(251, 191, 36)
    } else {
        pal.primary
    }
}

fn rgba(r: u8, g: u8, b: u8) -> Rgba<u8> {
    Rgba([r, g, b, 255])
}

fn glow_rgba(c: Rgba<u8>, alpha: u8) -> Rgba<u8> {
    Rgba([c[0], c[1], c[2], alpha])
}

fn poster_font() -> Option<&'static FontArc> {
    FONT_CACHE
        .get_or_init(|| FontArc::try_from_slice(POSTER_FONT_BYTES).ok())
        .as_ref()
}

fn normalize_poster_text(text: &str) -> String {
    text.replace('₂', "2").replace('…', "...")
}

fn text_width(text: &str, px: f32) -> f32 {
    let Some(font) = poster_font() else {
        return text.chars().count() as f32 * px * 0.6;
    };
    let clean = normalize_poster_text(text);
    let scaled = font.as_scaled(PxScale::from(px));
    let mut w = 0.0f32;
    let mut prev = None;
    for ch in clean.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(prev_id) = prev {
            w += scaled.kern(prev_id, id);
        }
        w += scaled.h_advance(id);
        prev = Some(id);
    }
    w
}

fn draw_text_right(
    img: &mut RgbaImage,
    right_x: i32,
    y: i32,
    text: &str,
    px: f32,
    color: Rgba<u8>,
) {
    let w = text_width(text, px).round() as i32;
    draw_text(img, right_x - w, y, text, px, color);
}

fn draw_text_centered(
    img: &mut RgbaImage,
    center_x: i32,
    y: i32,
    text: &str,
    px: f32,
    color: Rgba<u8>,
) {
    let w = text_width(text, px).round() as i32;
    draw_text(img, center_x - w / 2, y, text, px, color);
}

#[allow(clippy::too_many_arguments)]
fn draw_text_fit(
    img: &mut RgbaImage,
    x: i32,
    y: i32,
    max_w: f32,
    text: &str,
    mut px: f32,
    min_px: f32,
    color: Rgba<u8>,
) {
    while px > min_px && text_width(text, px) > max_w {
        px -= 0.5;
    }
    draw_text(img, x, y, text, px, color);
}

fn draw_glow_text(
    img: &mut RgbaImage,
    x: i32,
    y: i32,
    text: &str,
    px: f32,
    fg: Rgba<u8>,
    halo: Rgba<u8>,
) {
    let offsets = [
        (-2, 0),
        (2, 0),
        (0, -2),
        (0, 2),
        (-2, -2),
        (2, 2),
        (-2, 2),
        (2, -2),
    ];
    let soft = Rgba([halo[0], halo[1], halo[2], halo[3] / 4]);
    for (ox, oy) in offsets {
        draw_text(img, x + ox, y + oy, text, px, soft);
    }
    draw_text(img, x, y, text, px, fg);
}

fn draw_text(img: &mut RgbaImage, x: i32, y: i32, text: &str, px: f32, color: Rgba<u8>) {
    let Some(font) = poster_font() else {
        return;
    };
    let clean = normalize_poster_text(text);
    let scale = PxScale::from(px);
    let scaled = font.as_scaled(scale);
    let mut caret_x = x as f32;
    let baseline_y = y as f32 + scaled.ascent();
    let mut prev = None;
    let master_alpha = color[3] as f32 / 255.0;

    for ch in clean.chars() {
        let glyph_id = scaled.glyph_id(ch);
        if let Some(prev_id) = prev {
            caret_x += scaled.kern(prev_id, glyph_id);
        }
        let glyph = glyph_id.with_scale_and_position(scale, point(caret_x, baseline_y));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            let ox = bounds.min.x.floor() as i32;
            let oy = bounds.min.y.floor() as i32;
            outline.draw(|gx, gy, coverage| {
                let alpha = (coverage * master_alpha * 255.0).round() as u8;
                if alpha == 0 {
                    return;
                }
                blend_pixel(
                    img,
                    ox + gx as i32,
                    oy + gy as i32,
                    Rgba([color[0], color[1], color[2], alpha]),
                );
            });
        }
        caret_x += scaled.h_advance(glyph_id);
        prev = Some(glyph_id);
    }
}

fn blend_pixel(img: &mut RgbaImage, x: i32, y: i32, src: Rgba<u8>) {
    if x < 0 || y < 0 || x as u32 >= CANVAS_W || y as u32 >= CANVAS_H {
        return;
    }
    if src[3] == 255 {
        img.put_pixel(x as u32, y as u32, src);
        return;
    }
    let dst = *img.get_pixel(x as u32, y as u32);
    let a = src[3] as f32 / 255.0;
    let inv = 1.0 - a;
    img.put_pixel(
        x as u32,
        y as u32,
        Rgba([
            (src[0] as f32 * a + dst[0] as f32 * inv).round() as u8,
            (src[1] as f32 * a + dst[1] as f32 * inv).round() as u8,
            (src[2] as f32 * a + dst[2] as f32 * inv).round() as u8,
            255,
        ]),
    );
}

fn render_svg_logo(svg_bytes: &[u8], w: u32, h: u32) -> Result<RgbaImage> {
    let tree = usvg::Tree::from_data(svg_bytes, &usvg::Options::default())?;
    let sz = tree.size();
    let scale = (w as f32 / sz.width())
        .min(h as f32 / sz.height())
        .max(0.01);
    let rw = (sz.width() * scale).round().max(1.0) as u32;
    let rh = (sz.height() * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(rw, rh).context("Pixmap alloc failed")?;
    let mut canvas = pixmap.as_mut();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut canvas,
    );
    let raw = pixmap.data();
    let mut out = RgbaImage::new(rw, rh);
    for py in 0..rh {
        for px in 0..rw {
            let idx = ((py * rw + px) * 4) as usize;
            out.put_pixel(
                px,
                py,
                Rgba([raw[idx], raw[idx + 1], raw[idx + 2], raw[idx + 3]]),
            );
        }
    }
    Ok(image::imageops::resize(&out, w, h, FilterType::Lanczos3))
}

fn overlay(base: &mut RgbaImage, top: &RgbaImage, x0: i32, y0: i32) {
    for ly in 0..top.height() {
        for lx in 0..top.width() {
            let px = *top.get_pixel(lx, ly);
            if px[3] > 0 {
                blend_pixel(base, x0 + lx as i32, y0 + ly as i32, px);
            }
        }
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::CommonArgs;
    use crate::pipeline::TimeZoneMode;
    use crate::types::PricingTable;
    use crate::wrapped::data::{WrappedBuildOptions, build_wrapped_bundle};

    #[test]
    fn poster_png_has_valid_magic_and_dimensions_for_cyber_and_carbon() {
        let tz = TimeZoneMode::Utc;
        let pricing = PricingTable::default_table();
        let common = CommonArgs::default();
        let bundle = build_wrapped_bundle(WrappedBuildOptions {
            events: &[],
            tz: &tz,
            timezone_label: "UTC".to_string(),
            pricing: &pricing,
            target_year: Some(2026),
            all_time: false,
            include_projects: false,
            common: Some(&common),
        });

        for theme in [WrappedTheme::Cyber, WrappedTheme::Carbon] {
            let img = render_wrapped_png(&bundle.report, theme).unwrap();
            assert_eq!(img.width(), 1200);
            assert_eq!(img.height(), 1500);
        }

        let tmp = std::env::temp_dir().join("tu-wrapped-test-poster.png");
        write_wrapped_png(&tmp, &bundle.report, WrappedTheme::Carbon).unwrap();
        let bytes = fs::read(&tmp).unwrap();
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]));
        let _ = fs::remove_file(tmp);
    }
}
