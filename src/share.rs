use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::OnceLock;

use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use font8x8::UnicodeFonts;
use image::{ImageBuffer, Rgba, RgbaImage, imageops::FilterType};
use plotters::prelude::*;
use resvg::{tiny_skia, usvg};

use crate::cli::{ImgArgs, ImgPeriod};
use crate::pipeline::collect_usage_snapshot;
use crate::types::{TokenCounts, UsageEvent};

const POSTER_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/Audiowide-Regular.ttf");
const DEFAULT_LOGO_SVG_BYTES: &[u8] = include_bytes!("../assets/branding/tokenusage-logomark.svg");
static POSTER_FONT: OnceLock<Option<FontArc>> = OnceLock::new();
const CHART_HEADROOM: f64 = 1.28;

pub(crate) async fn run_share(args: ImgArgs) -> Result<()> {
    if args.common.json || args.common.jq.is_some() {
        bail!("tu img does not support --json/--jq");
    }
    if args.bars < 7 {
        bail!("--bars must be >= 7");
    }
    let mut canvas_w = args.width;
    let mut canvas_h = args.height;
    if args.portrait {
        if args.width == 1600 && args.height == 900 {
            canvas_w = 1080;
            canvas_h = 1920;
        } else if canvas_w >= canvas_h {
            std::mem::swap(&mut canvas_w, &mut canvas_h);
        }
    }
    let short_edge = canvas_w.min(canvas_h);
    let long_edge = canvas_w.max(canvas_h);
    if short_edge < 620 || long_edge < 960 {
        bail!("--width/--height too small; minimum edges are 620x960");
    }

    match args.period {
        ImgPeriod::Daily | ImgPeriod::Weekly => {
            let output = resolve_output_abs(&args.output)?;
            let period = args.period;
            let snapshot =
                render_share_image_for_period(&args, period, &output, canvas_w, canvas_h).await?;
            println!(
                "share image written: {} ({} view)",
                output.display(),
                snapshot.period_label
            );
        }
        ImgPeriod::Both => {
            let (daily_output, weekly_output) = derive_dual_outputs(&args.output)?;
            render_share_image_for_period(
                &args,
                ImgPeriod::Daily,
                &daily_output,
                canvas_w,
                canvas_h,
            )
            .await?;
            render_share_image_for_period(
                &args,
                ImgPeriod::Weekly,
                &weekly_output,
                canvas_w,
                canvas_h,
            )
            .await?;
            // Print full generated paths on two lines for easy copy.
            println!("{}", daily_output.display());
            println!("{}", weekly_output.display());
        }
    }
    Ok(())
}

async fn render_share_image_for_period(
    args: &ImgArgs,
    period: ImgPeriod,
    output: &Path,
    canvas_w: u32,
    canvas_h: u32,
) -> Result<ShareSnapshot> {
    let mut common = args.common.clone();
    apply_default_img_range(&mut common, period)?;

    let mut loaded = collect_usage_snapshot(common.clone()).await?;
    let _ = loaded.stats.files_discovered;
    if let Some(project_filter) = args.project.as_deref() {
        loaded.events.retain(|event| {
            event
                .project
                .as_deref()
                .is_some_and(|p| p.contains(project_filter))
        });
    }

    if loaded.events.is_empty() {
        bail!("No usage data found in selected range; nothing to render");
    }

    let mut args_for_period = args.clone();
    args_for_period.period = period;
    let snapshot =
        ShareSnapshot::build(&loaded.events, &loaded.timezone, period, args.bars, &common)?;

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create output dir: {}", parent.display()))?;
    }

    let mut img: RgbaImage = ImageBuffer::from_pixel(canvas_w, canvas_h, rgba(8, 12, 24));
    draw_gradient_background(&mut img, rgba(8, 12, 24), rgba(16, 26, 52));
    draw_share_card(&mut img, &snapshot, &args_for_period)?;
    img.save(output)
        .with_context(|| format!("Failed to save image: {}", output.display()))?;

    Ok(snapshot)
}

fn resolve_output_abs(output: &str) -> Result<PathBuf> {
    let expanded = expand_user_path(output);
    if expanded.is_absolute() {
        return Ok(expanded);
    }
    Ok(std::env::current_dir()
        .context("Failed to read current directory")?
        .join(expanded))
}

fn derive_dual_outputs(base_output: &str) -> Result<(PathBuf, PathBuf)> {
    let base = resolve_output_abs(base_output)?;
    let parent = base.parent().unwrap_or_else(|| Path::new("."));
    let stem = base
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("tokenusage");
    let ext = base.extension().and_then(|s| s.to_str()).unwrap_or("png");
    let daily = parent.join(format!("{stem}-daily.{ext}"));
    let weekly = parent.join(format!("{stem}-weekly.{ext}"));
    Ok((daily, weekly))
}

#[derive(Clone)]
struct SharePoint {
    label: String,
    tokens: u64,
}

#[derive(Clone)]
struct ShareSnapshot {
    period_label: String,
    range_label: String,
    total_tokens: u64,
    total_cost_usd: f64,
    peak_tokens: (String, u64),
    top_model: Option<(String, u64)>,
    models_used: Vec<String>,
    points: Vec<SharePoint>,
}

impl ShareSnapshot {
    fn build(
        events: &[UsageEvent],
        timezone: &crate::pipeline::TimeZoneMode,
        period: ImgPeriod,
        max_points: usize,
        common: &crate::cli::CommonArgs,
    ) -> Result<Self> {
        let mut total = TokenCounts::default();
        let mut models: BTreeMap<String, TokenCounts> = BTreeMap::new();

        for event in events {
            let counts = event.usage.to_counts();
            total.add_assign(counts.clone());

            let key = sanitize_model_name(&event.model);
            let model = models.entry(key).or_default();
            model.add_assign(counts);
        }

        let points = match period {
            ImgPeriod::Daily => {
                let target =
                    parse_date(common.since.as_deref())?.unwrap_or_else(|| timezone.now_date());
                aggregate_hourly(events, timezone, target)
            }
            ImgPeriod::Weekly => {
                let end =
                    parse_date(common.until.as_deref())?.unwrap_or_else(|| timezone.now_date());
                let since = parse_date(common.since.as_deref())?
                    .unwrap_or(end - chrono::TimeDelta::days(6));
                aggregate_daily(events, timezone, since, end, max_points)
            }
            ImgPeriod::Both => {
                let end =
                    parse_date(common.until.as_deref())?.unwrap_or_else(|| timezone.now_date());
                let since = parse_date(common.since.as_deref())?
                    .unwrap_or(end - chrono::TimeDelta::days(6));
                aggregate_daily(events, timezone, since, end, max_points)
            }
        };

        if points.is_empty() {
            bail!("No points generated for chart");
        }

        let mut model_totals = models.into_iter().collect::<Vec<_>>();
        model_totals.sort_by(|(a_model, a_counts), (b_model, b_counts)| {
            b_counts
                .total_tokens
                .cmp(&a_counts.total_tokens)
                .then_with(|| {
                    a_model
                        .to_ascii_lowercase()
                        .cmp(&b_model.to_ascii_lowercase())
                })
        });
        let top_model = model_totals
            .first()
            .map(|(name, counts)| (name.clone(), counts.total_tokens));
        let models_used = model_totals
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();

        let peak_tokens = points
            .iter()
            .max_by_key(|p| p.tokens)
            .map(|p| (p.label.clone(), p.tokens))
            .unwrap_or_else(|| ("-".to_string(), 0));

        let (period_label, range_label) = match period {
            ImgPeriod::Daily => {
                let target =
                    parse_date(common.since.as_deref())?.unwrap_or_else(|| timezone.now_date());
                (
                    "daily (hourly)".to_string(),
                    format!("{} 00:00-23:59", target.format("%Y-%m-%d")),
                )
            }
            ImgPeriod::Weekly => {
                let end =
                    parse_date(common.until.as_deref())?.unwrap_or_else(|| timezone.now_date());
                let since = parse_date(common.since.as_deref())?
                    .unwrap_or(end - chrono::TimeDelta::days(6));
                (
                    "weekly (daily)".to_string(),
                    format!("{} -> {}", since.format("%Y-%m-%d"), end.format("%Y-%m-%d")),
                )
            }
            ImgPeriod::Both => {
                let end =
                    parse_date(common.until.as_deref())?.unwrap_or_else(|| timezone.now_date());
                let since = parse_date(common.since.as_deref())?
                    .unwrap_or(end - chrono::TimeDelta::days(6));
                (
                    "weekly (daily)".to_string(),
                    format!("{} -> {}", since.format("%Y-%m-%d"), end.format("%Y-%m-%d")),
                )
            }
        };

        Ok(Self {
            period_label,
            range_label,
            total_tokens: total.total_tokens,
            total_cost_usd: total.cost_usd,
            peak_tokens,
            top_model,
            models_used,
            points,
        })
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl Rect {
    fn right(self) -> u32 {
        self.x + self.w
    }

    fn bottom(self) -> u32 {
        self.y + self.h
    }

    fn inset(self, padding: u32) -> Rect {
        Rect {
            x: self.x + padding,
            y: self.y + padding,
            w: self.w.saturating_sub(padding * 2),
            h: self.h.saturating_sub(padding * 2),
        }
    }
}

fn draw_share_card(img: &mut RgbaImage, snapshot: &ShareSnapshot, args: &ImgArgs) -> Result<()> {
    let portrait = img.height() > img.width();
    if portrait {
        return draw_share_card_portrait(img, snapshot, args);
    }
    let margin = ((img.width() as f32) * 0.036).round() as u32;
    let gap = ((img.height() as f32) * 0.014).round() as u32;
    let layout = Rect {
        x: margin,
        y: margin,
        w: img.width().saturating_sub(margin * 2),
        h: img.height().saturating_sub(margin * 2),
    };

    let header_h = if portrait {
        ((layout.h as f32) * 0.40).round() as u32
    } else {
        ((layout.h as f32) * 0.47).round() as u32
    };
    let footer_h = 26u32;
    let chart_h = layout
        .h
        .saturating_sub(header_h)
        .saturating_sub(footer_h)
        .saturating_sub(gap * 2);

    let header = Rect {
        x: layout.x,
        y: layout.y,
        w: layout.w,
        h: header_h,
    };
    let chart = Rect {
        x: layout.x,
        y: header.bottom() + gap,
        w: layout.w,
        h: chart_h,
    };
    let footer = Rect {
        x: layout.x,
        y: chart.bottom() + gap,
        w: layout.w,
        h: footer_h,
    };

    draw_overlay_glow(
        img,
        layout.x + layout.w / 2,
        layout.y + ((layout.h as f32) * 0.32) as u32,
        ((layout.w.min(layout.h) as f32) * 0.37) as u32,
        Rgba([62, 174, 255, 38]),
    );
    draw_overlay_glow(
        img,
        layout.x + ((layout.w as f32) * 0.76) as u32,
        layout.y + ((layout.h as f32) * 0.22) as u32,
        ((layout.w.min(layout.h) as f32) * 0.25) as u32,
        Rgba([90, 255, 166, 28]),
    );

    draw_header(img, header, snapshot, args, portrait)?;
    draw_line_chart(img, chart, "trend", &snapshot.points, rgba(98, 245, 154));
    draw_footer(img, footer, args);
    Ok(())
}

fn draw_share_card_portrait(
    img: &mut RgbaImage,
    snapshot: &ShareSnapshot,
    args: &ImgArgs,
) -> Result<()> {
    let margin = ((img.width() as f32) * 0.04).round() as u32;
    let layout = Rect {
        x: margin,
        y: margin,
        w: img.width().saturating_sub(margin * 2),
        h: img.height().saturating_sub(margin * 2),
    };
    fill_rect(img, layout, rgba(5, 10, 20));
    draw_cyber_grid(img, layout, 34, Rgba([54, 145, 128, 34]));

    draw_overlay_glow(
        img,
        layout.x + layout.w / 2,
        layout.y + ((layout.h as f32) * 0.24) as u32,
        ((layout.w.min(layout.h) as f32) * 0.38) as u32,
        Rgba([74, 255, 188, 36]),
    );
    draw_overlay_glow(
        img,
        layout.x + layout.w / 2,
        layout.y + ((layout.h as f32) * 0.62) as u32,
        ((layout.w.min(layout.h) as f32) * 0.30) as u32,
        Rgba([59, 186, 255, 24]),
    );

    let top_y = layout.y as i32 + 8;
    draw_text(
        img,
        layout.x as i32 + 8,
        top_y,
        &truncate_chars(&args.brand, 16),
        3,
        rgba(150, 174, 216),
    );
    let end_tag = snapshot
        .range_label
        .split("->")
        .last()
        .map(str::trim)
        .unwrap_or(snapshot.range_label.as_str())
        .to_string();
    let short_end = if end_tag.len() >= 5 {
        end_tag[end_tag.len() - 5..].to_string()
    } else {
        end_tag
    };
    draw_text(
        img,
        (layout.right().saturating_sub(90)) as i32,
        top_y,
        &short_end,
        3,
        rgba(190, 205, 230),
    );

    let logo_size = ((layout.w as f32) * 0.16).round() as u32;
    let logo_x = layout.x + (layout.w.saturating_sub(logo_size)) / 2;
    let logo_y = layout.y + 24;
    draw_overlay_glow(
        img,
        logo_x + logo_size / 2,
        logo_y + logo_size / 2,
        logo_size + 18,
        Rgba([96, 247, 183, 28]),
    );
    draw_logo_box(img, logo_x, logo_y, logo_size, args)?;

    let meta_y = logo_y + logo_size + 78;
    draw_text(
        img,
        layout.x as i32 + 16,
        meta_y as i32,
        &format!(
            "{}  |  {}",
            snapshot.period_label.to_uppercase(),
            snapshot.range_label
        ),
        3,
        rgba(176, 201, 236),
    );

    let big_number = format_compact_u64(snapshot.total_tokens);
    let big_color = rgba(112, 241, 166);
    draw_text(
        img,
        layout.x as i32 + 14,
        (meta_y + 42) as i32,
        &big_number,
        10,
        big_color,
    );
    let total_usage_y = (meta_y + 42) as i32 + line_height_px(10) + 4;
    draw_text(
        img,
        layout.x as i32 + 14,
        total_usage_y,
        "TOTAL USAGE",
        5,
        rgba(204, 220, 242),
    );

    let cards_top = (total_usage_y + line_height_px(5) + 14).max(layout.y as i32) as u32;
    let cards_h = ((layout.h as f32) * 0.145) as u32;
    let card_gap = 16u32;
    let card_w = (layout.w.saturating_sub(card_gap)) / 2;
    let card_h = (cards_h.saturating_sub(card_gap)) / 2;
    let avg_tokens = if snapshot.points.is_empty() {
        0
    } else {
        snapshot.total_tokens / snapshot.points.len() as u64
    };
    let avg_unit = if snapshot.period_label.starts_with("daily") {
        "hour"
    } else {
        "day"
    };

    draw_stat_box(
        img,
        Rect {
            x: layout.x,
            y: cards_top,
            w: card_w,
            h: card_h,
        },
        "range total",
        &format_compact_u64_spaced(snapshot.total_tokens),
    );
    draw_stat_box(
        img,
        Rect {
            x: layout.x + card_w + card_gap,
            y: cards_top,
            w: card_w,
            h: card_h,
        },
        "average",
        &format!("{}/{}", format_compact_u64_spaced(avg_tokens), avg_unit),
    );
    draw_stat_box(
        img,
        Rect {
            x: layout.x,
            y: cards_top + card_h + card_gap,
            w: card_w,
            h: card_h,
        },
        "peak",
        &format!(
            "{} @ {}",
            format_compact_u64_spaced(snapshot.peak_tokens.1),
            format_peak_bucket_label(&snapshot.period_label, &snapshot.peak_tokens.0)
        ),
    );
    draw_models_box(
        img,
        Rect {
            x: layout.x + card_w + card_gap,
            y: cards_top + card_h + card_gap,
            w: card_w,
            h: card_h,
        },
        "models",
        &snapshot.models_used,
    );

    let footer_h = 112u32;
    let footer_y = layout.bottom().saturating_sub(footer_h);
    let chart_top = cards_top + cards_h + 12;
    let chart_h = footer_y.saturating_sub(chart_top + 16);
    draw_plotters_chart_panel(
        img,
        Rect {
            x: layout.x,
            y: chart_top,
            w: layout.w,
            h: chart_h,
        },
        &snapshot.points,
    )?;

    let footer = Rect {
        x: layout.x,
        y: footer_y,
        w: layout.w,
        h: footer_h,
    };
    fill_rect(img, footer, rgba(18, 24, 38));
    stroke_rect(img, footer, rgba(83, 176, 150), 2);
    let brand_line = args.brand.to_uppercase();
    let brand_scale = 5;
    let brand_w = text_width(&brand_line, brand_scale) as u32;
    let brand_x = if brand_w + 12 >= footer.w {
        footer.x as i32 + 6
    } else {
        (footer.x + (footer.w - brand_w) / 2) as i32
    };
    draw_text(
        img,
        brand_x,
        footer.y as i32 + 16,
        &brand_line,
        brand_scale,
        rgba(157, 241, 206),
    );

    let mut url_scale = 3u32;
    while url_scale > 1 && text_width(&args.brand_url, url_scale) as u32 + 12 >= footer.w {
        url_scale -= 1;
    }
    let url_w = text_width(&args.brand_url, url_scale) as u32;
    let url_x = if url_w + 12 >= footer.w {
        footer.x as i32 + 6
    } else {
        (footer.x + (footer.w - url_w) / 2) as i32
    };
    draw_text(
        img,
        url_x,
        footer.y as i32 + 70,
        &args.brand_url,
        url_scale,
        rgba(184, 214, 203),
    );
    Ok(())
}

fn draw_header(
    img: &mut RgbaImage,
    area: Rect,
    snapshot: &ShareSnapshot,
    args: &ImgArgs,
    portrait: bool,
) -> Result<()> {
    let area = area.inset(2);
    let logo_size = if portrait {
        ((area.h as f32) * 0.14).round() as u32
    } else {
        ((area.h as f32) * 0.34).round() as u32
    };
    let logo_x = area.right().saturating_sub(logo_size);
    let logo_y = area.y + 2;

    if portrait {
        draw_text(
            img,
            area.x as i32,
            area.y as i32 + 6,
            &args.brand,
            2,
            rgba(132, 154, 196),
        );
        draw_text(
            img,
            area.x as i32,
            area.y as i32 + 30,
            "TOKEN PERFORMANCE",
            4,
            rgba(222, 232, 251),
        );
        draw_text(
            img,
            area.x as i32,
            area.y as i32 + 62,
            &format!("{} · {}", snapshot.period_label, snapshot.range_label),
            2,
            rgba(164, 187, 229),
        );
    } else {
        draw_text(
            img,
            area.x as i32,
            area.y as i32 + 6,
            &format!("{} · {}", args.brand, snapshot.period_label),
            2,
            rgba(132, 154, 196),
        );
        draw_text(
            img,
            area.x as i32,
            area.y as i32 + 34,
            "TOKEN PERFORMANCE",
            4,
            rgba(222, 232, 251),
        );
        draw_text(
            img,
            area.x as i32,
            area.y as i32 + 68,
            &snapshot.range_label,
            3,
            rgba(164, 187, 229),
        );
    }

    draw_logo_box(img, logo_x, logo_y, logo_size, args)?;

    if !portrait {
        let brand_w = text_width(&args.brand, 2) as u32;
        draw_text(
            img,
            logo_x.saturating_sub(brand_w + 10) as i32,
            logo_y as i32 + (logo_size as i32 / 2 - 8),
            &args.brand,
            2,
            rgba(146, 240, 255),
        );
    }

    draw_text(
        img,
        area.x as i32,
        area.y as i32 + 112,
        &format_compact_u64(snapshot.total_tokens),
        if portrait { 11 } else { 10 },
        rgba(125, 235, 168),
    );
    draw_text(
        img,
        area.x as i32,
        area.y as i32 + if portrait { 218 } else { 200 },
        "TOTAL USAGE",
        3,
        rgba(156, 224, 185),
    );
    if portrait {
        let base_y = area.y as i32 + 286;
        draw_text(
            img,
            area.x as i32,
            base_y,
            &format!("total tokens  {}", format_u64(snapshot.total_tokens)),
            3,
            rgba(228, 235, 249),
        );
        draw_text(
            img,
            area.x as i32,
            base_y + 34,
            &format!("cost (usd)  {}", format_usd(snapshot.total_cost_usd)),
            3,
            rgba(190, 208, 240),
        );
        draw_text(
            img,
            area.x as i32,
            base_y + 68,
            &format!(
                "peak  {} ({})",
                snapshot.peak_tokens.0,
                format_u64(snapshot.peak_tokens.1)
            ),
            3,
            rgba(228, 235, 249),
        );
        if let Some((model, tokens)) = snapshot.top_model.as_ref() {
            draw_text(
                img,
                area.x as i32,
                base_y + 102,
                &format!("top model  {}", model),
                2,
                rgba(190, 208, 240),
            );
            draw_text(
                img,
                area.x as i32,
                base_y + 132,
                &format!("model tokens  {}", format_u64(*tokens)),
                2,
                rgba(156, 178, 220),
            );
        }
    } else {
        let left_col_x = area.x as i32;
        let right_col_x = (area.x + area.w / 2) as i32;
        draw_text(
            img,
            left_col_x,
            area.y as i32 + 252,
            &format!("total tokens  {}", format_u64(snapshot.total_tokens)),
            3,
            rgba(228, 235, 249),
        );
        draw_text(
            img,
            left_col_x,
            area.y as i32 + 288,
            &format!("cost (usd)  {}", format_usd(snapshot.total_cost_usd)),
            3,
            rgba(190, 208, 240),
        );
        draw_text(
            img,
            right_col_x,
            area.y as i32 + 252,
            &format!(
                "peak  {} ({})",
                snapshot.peak_tokens.0,
                format_u64(snapshot.peak_tokens.1)
            ),
            3,
            rgba(228, 235, 249),
        );
        if let Some((model, tokens)) = snapshot.top_model.as_ref() {
            draw_text(
                img,
                right_col_x,
                area.y as i32 + 288,
                &format!("top model  {}", model),
                2,
                rgba(190, 208, 240),
            );
            draw_text(
                img,
                right_col_x,
                area.y as i32 + 320,
                &format!("model tokens  {}", format_u64(*tokens)),
                2,
                rgba(156, 178, 220),
            );
        }
    }

    Ok(())
}

fn draw_line_chart(
    img: &mut RgbaImage,
    area: Rect,
    title: &str,
    points: &[SharePoint],
    line_color: Rgba<u8>,
) {
    fill_rect(img, area, rgba(14, 20, 38));
    stroke_rect(img, area, rgba(45, 63, 104), 1);
    draw_text(
        img,
        area.x as i32 + 12,
        area.y as i32 + 10,
        title,
        3,
        rgba(181, 198, 229),
    );

    let inner = area.inset(14);
    let plot = Rect {
        x: inner.x,
        y: inner.y + 28,
        w: inner.w,
        h: inner.h.saturating_sub(42),
    };
    fill_rect(img, plot, rgba(10, 15, 31));
    stroke_rect(img, plot, rgba(38, 54, 90), 1);

    if points.is_empty() {
        return;
    }

    for step in 1..=3u32 {
        let y = plot.y + ((plot.h.saturating_sub(28) * step) / 4);
        for x in (plot.x + 1)..plot.right().saturating_sub(1) {
            if x % 8 < 4 {
                blend_pixel(img, x as i32, y as i32, Rgba([44, 60, 95, 95]));
            }
        }
    }

    let values = points.iter().map(|p| p.tokens).collect::<Vec<_>>();
    let max_value = values.iter().copied().max().unwrap_or(1).max(1);
    let n = values.len().max(1);
    let x_start = plot.x + 12;
    let x_end = plot.right().saturating_sub(12);
    let y_bottom = plot.bottom().saturating_sub(26);
    let y_top = plot.y + 14;
    let x_span = (x_end.saturating_sub(x_start)).max(1) as f64;
    let slot_w = (x_span / n as f64).max(1.0);
    let bar_w = (slot_w * 0.56).clamp(3.0, 28.0).round() as i32;
    let y_span = (y_bottom.saturating_sub(y_top)).max(1) as f64;

    draw_line(
        img,
        x_start as i32,
        y_bottom as i32,
        x_end as i32,
        y_bottom as i32,
        Rgba([75, 247, 181, 138]),
    );

    let value_scale = if n <= 12 { 3u32 } else { 2u32 };
    let label_step = if n <= 10 {
        1
    } else if n <= 18 {
        2
    } else {
        3
    };

    for (idx, value) in values.iter().enumerate() {
        if *value == 0 {
            continue;
        }
        let center_x = x_start as f64 + (idx as f64 + 0.5) * slot_w;
        let h = ((*value as f64 / max_value as f64) * y_span).round() as i32;
        let h = h.max(1);
        let x0 = center_x.round() as i32 - bar_w / 2;
        let y0 = y_bottom as i32 - h;

        draw_rect_clamped_i32(img, x0, y0, bar_w, h, line_color);
        draw_rect_clamped_i32(img, x0, y0, bar_w, 1, rgba(170, 255, 213));

        if idx % label_step != 0 && idx + 1 != n {
            continue;
        }
        let value_text = format_compact_u64(*value);
        let value_w = text_width(&value_text, value_scale) as i32;
        let mut tx = center_x.round() as i32 - value_w / 2;
        let min_x = plot.x as i32 + 4;
        let max_x = plot.right().saturating_sub(value_w as u32 + 4) as i32;
        if tx < min_x {
            tx = min_x;
        }
        if tx > max_x {
            tx = max_x;
        }
        let text_h = line_height_px(value_scale).max(1);
        let ty = (y0 - text_h - 3).max(plot.y as i32 + 3);
        draw_rect_clamped_i32(
            img,
            tx - 2,
            ty - 1,
            value_w + 4,
            text_h + 2,
            Rgba([6, 14, 27, 220]),
        );
        draw_text(img, tx, ty, &value_text, value_scale, rgba(204, 248, 223));
    }

    let start_label = points.first().map(|p| p.label.as_str()).unwrap_or("-");
    let end_label = points.last().map(|p| p.label.as_str()).unwrap_or("-");
    draw_text(
        img,
        plot.x as i32 + 6,
        plot.bottom() as i32 - 22,
        start_label,
        3,
        rgba(145, 167, 217),
    );
    let end_w = text_width(end_label, 3) as u32;
    draw_text(
        img,
        plot.right().saturating_sub(end_w + 6) as i32,
        plot.bottom() as i32 - 22,
        end_label,
        3,
        rgba(145, 167, 217),
    );

    let range_line = format_u64(max_value);
    let range_w = text_width(&range_line, 3) as u32;
    draw_text(
        img,
        plot.x
            .saturating_add(plot.w.saturating_sub(range_w) / 2)
            .saturating_sub(2) as i32,
        plot.bottom() as i32 - 22,
        &range_line,
        3,
        rgba(145, 167, 217),
    );
}

fn draw_footer(img: &mut RgbaImage, area: Rect, args: &ImgArgs) {
    let narrow = area.w < 1000;
    let text = if narrow {
        format!("{} · {}", args.brand, truncate_chars(&args.brand_url, 16))
    } else {
        format!("Generated by {} · {}", args.brand, args.brand_url)
    };
    let scale = if narrow {
        1
    } else if area.w < 1200 {
        2
    } else {
        3
    };
    let width = text_width(&text, scale) as u32;
    let x = if width + 2 >= area.w {
        area.x as i32
    } else {
        area.right().saturating_sub(width + 2) as i32
    };
    draw_text(img, x, area.y as i32 + 1, &text, scale, rgba(124, 145, 194));
}

fn draw_cyber_grid(img: &mut RgbaImage, area: Rect, spacing: u32, color: Rgba<u8>) {
    if spacing == 0 {
        return;
    }
    let mut y = area.y;
    while y < area.bottom() {
        for x in area.x..area.right() {
            blend_pixel(img, x as i32, y as i32, color);
        }
        y = y.saturating_add(spacing);
    }
    let mut x = area.x;
    while x < area.right() {
        for y in area.y..area.bottom() {
            blend_pixel(img, x as i32, y as i32, color);
        }
        x = x.saturating_add(spacing);
    }
}

fn draw_stat_box(img: &mut RgbaImage, rect: Rect, title: &str, value: &str) {
    let title_pad_x = 12i32;
    let value_pad_x = 16i32;
    fill_rect(img, rect, rgba(6, 18, 28));
    stroke_rect(img, rect, rgba(83, 245, 179), 1);
    draw_overlay_glow(
        img,
        rect.x + rect.w / 2,
        rect.y + rect.h / 2,
        rect.w.min(rect.h) / 2,
        Rgba([83, 245, 179, 16]),
    );
    draw_text(
        img,
        rect.x as i32 + title_pad_x,
        rect.y as i32 + 8,
        title,
        3,
        rgba(172, 221, 207),
    );
    draw_text(
        img,
        rect.x as i32 + value_pad_x,
        rect.y as i32 + 40,
        &truncate_chars(value, 22),
        5,
        rgba(236, 251, 244),
    );
}

fn draw_models_box(img: &mut RgbaImage, rect: Rect, title: &str, models: &[String]) {
    let title_pad_x = 12i32;
    let value_pad_x = 14i32;
    fill_rect(img, rect, rgba(6, 18, 28));
    stroke_rect(img, rect, rgba(83, 245, 179), 1);
    draw_overlay_glow(
        img,
        rect.x + rect.w / 2,
        rect.y + rect.h / 2,
        rect.w.min(rect.h) / 2,
        Rgba([83, 245, 179, 16]),
    );
    draw_text(
        img,
        rect.x as i32 + title_pad_x,
        rect.y as i32 + 8,
        title,
        3,
        rgba(172, 221, 207),
    );

    if models.is_empty() {
        draw_text(
            img,
            rect.x as i32 + value_pad_x,
            rect.y as i32 + 40,
            "-",
            4,
            rgba(236, 251, 244),
        );
        return;
    }

    let max_w = rect.w.saturating_sub(24);
    let max_h = rect.h.saturating_sub(40);
    let mut chosen_scale = 1u32;
    let mut chosen_lines = models.to_vec();
    for scale in (1u32..=5u32).rev() {
        let Some(lines) = layout_models_lines_limited(models, scale, max_w, 2) else {
            continue;
        };
        let needed_h = (lines.len() as i32) * line_height_px(scale);
        if needed_h as u32 <= max_h {
            chosen_scale = scale;
            chosen_lines = lines;
            break;
        }
    }

    let line_h = line_height_px(chosen_scale).max(1);
    let content_bottom = rect.bottom() as i32 - 4;
    let mut y = rect.y as i32 + 36;
    for line in &chosen_lines {
        if y + line_h > content_bottom {
            break;
        }
        draw_text(
            img,
            rect.x as i32 + value_pad_x,
            y,
            line,
            chosen_scale,
            rgba(236, 251, 244),
        );
        y += line_h;
    }
}

fn layout_models_lines_limited(
    models: &[String],
    scale: u32,
    max_w: u32,
    max_lines: usize,
) -> Option<Vec<String>> {
    if models.is_empty() {
        return Some(vec!["-".to_string()]);
    }
    if max_lines == 0 {
        return None;
    }

    if max_lines == 2 {
        let joined = models.join(" · ");
        if text_width(&joined, scale) as u32 <= max_w {
            return Some(vec![joined]);
        }
        if models.len() >= 2 {
            let mut best: Option<(u32, Vec<String>)> = None;
            for split in 1..models.len() {
                let left = models[..split].join(" · ");
                let right = models[split..].join(" · ");
                let lw = text_width(&left, scale) as u32;
                let rw = text_width(&right, scale) as u32;
                if lw <= max_w && rw <= max_w {
                    let score = lw.max(rw);
                    match &best {
                        Some((best_score, _)) if score >= *best_score => {}
                        _ => best = Some((score, vec![left, right])),
                    }
                }
            }
            if let Some((_, lines)) = best {
                return Some(lines);
            }
        }
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    for model in models {
        if text_width(model, scale) as u32 > max_w {
            return None;
        }
        let candidate = if current.is_empty() {
            model.clone()
        } else {
            format!("{current} · {model}")
        };
        if text_width(&candidate, scale) as u32 <= max_w {
            current = candidate;
            continue;
        }
        lines.push(std::mem::take(&mut current));
        if lines.len() >= max_lines {
            return None;
        }
        current = model.clone();
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() || lines.len() > max_lines {
        return None;
    }
    Some(lines)
}

fn draw_plotters_chart_panel(img: &mut RgbaImage, area: Rect, points: &[SharePoint]) -> Result<()> {
    fill_rect(img, area, rgba(7, 16, 30));
    stroke_rect(img, area, rgba(62, 136, 126), 1);

    let inner = area.inset(12);
    let axis_band_h = 44u32;
    let plot = Rect {
        x: inner.x,
        y: inner.y + 10,
        w: inner.w,
        h: inner.h.saturating_sub(axis_band_h + 10),
    };
    let axis_band = Rect {
        x: inner.x,
        y: plot.bottom() + 2,
        w: inner.w,
        h: axis_band_h,
    };
    fill_rect(img, plot, rgba(5, 12, 24));
    fill_rect(img, axis_band, rgba(6, 14, 27));

    if points.is_empty() {
        return Ok(());
    }

    match render_chart_bitmap(points, plot.w, plot.h) {
        Ok(chart_img) => overlay_image(img, &chart_img, plot.x, plot.y),
        Err(_) => {
            // fallback if plotters chart generation fails
            let max_value = points.iter().map(|p| p.tokens).max().unwrap_or(1).max(1) as f64;
            let n = points.len() as u32;
            let slot_w = (plot.w.saturating_sub(20) / n.max(1)).max(6);
            let bar_w = ((slot_w as f32) * 0.62).round() as u32;
            let bar_w = bar_w.clamp(4, 22);
            let base_y = plot.bottom().saturating_sub(18);
            let max_h = plot.h.saturating_sub(30);
            let start_x = plot.x + 10;
            for (idx, point) in points.iter().enumerate() {
                let slot_x = start_x + idx as u32 * slot_w;
                let x = slot_x + slot_w.saturating_sub(bar_w) / 2;
                let h = ((point.tokens as f64 / max_value) * max_h as f64).round() as u32;
                let y = base_y.saturating_sub(h);
                fill_rect(
                    img,
                    Rect {
                        x,
                        y,
                        w: bar_w,
                        h: h.max(1),
                    },
                    rgba(98, 245, 154),
                );
            }
        }
    }

    fill_rect(
        img,
        Rect {
            x: axis_band.x,
            y: axis_band.y + 2,
            w: axis_band.w,
            h: 2,
        },
        rgba(75, 247, 181),
    );

    let n = points.len().max(1);
    let label_scale = 3u32;
    let mut max_label_w = 0u32;
    for p in points {
        max_label_w = max_label_w.max(text_width(&p.label, label_scale) as u32);
    }
    let slot_w = (axis_band.w as f32 / n as f32).max(1.0);
    let mut step = ((max_label_w as f32 + 8.0) / slot_w).ceil() as usize;
    if step == 0 {
        step = 1;
    }
    if n <= 8 {
        step = 1;
    }

    for (idx, point) in points.iter().enumerate() {
        if idx % step != 0 && idx + 1 != n {
            continue;
        }
        let center_x = axis_band.x as f32 + (idx as f32 + 0.5) * (axis_band.w as f32 / n as f32);
        let label_w = text_width(&point.label, label_scale) as u32;
        let mut label_x = center_x.round() as i32 - (label_w as i32 / 2);
        let min_x = axis_band.x as i32 + 2;
        let max_x = axis_band.right().saturating_sub(label_w + 2) as i32;
        if label_x < min_x {
            label_x = min_x;
        }
        if label_x > max_x {
            label_x = max_x;
        }
        draw_text(
            img,
            label_x,
            axis_band.y as i32 + 10,
            &point.label,
            label_scale,
            rgba(151, 203, 185),
        );
    }

    // Value labels above bars (compact style like 12.8B / 1.6M).
    let max_tokens = points.iter().map(|p| p.tokens).max().unwrap_or(1).max(1) as f64;
    let upper = ((max_tokens * CHART_HEADROOM).ceil() as u64 + 1) as f64;
    let count = points.len().max(1);
    let slot = 10.0f64;
    let x_max = count as f64 * slot;
    let margin_px = 8.0f64;
    let x_plot_w = (plot.w as f64 - margin_px * 2.0).max(1.0);
    let y_plot_h = (plot.h as f64 - margin_px * 2.0).max(1.0);
    let value_label_scale = if count <= 10 { 4u32 } else { 3u32 };
    let mut value_step = 1usize;
    if count > 10 {
        value_step = 2;
    }
    if count > 18 {
        value_step = 3;
    }
    let slot_px = (x_plot_w / count as f64).max(1.0);
    let bar_w_px = (slot_px * 0.40).max(4.0);
    let bar_bottom = plot.y as f64 + margin_px + y_plot_h;
    let mut bar_rects = Vec::with_capacity(points.len());
    for (idx, point) in points.iter().enumerate() {
        let base = idx as f64 * slot;
        let center = base + 5.0;
        let px = plot.x as f64 + margin_px + (center / x_max) * x_plot_w;
        let py = plot.y as f64 + margin_px + (1.0 - (point.tokens as f64 / upper)) * y_plot_h;
        let rect = (
            (px - bar_w_px * 0.5).floor() as i32,
            py.round() as i32,
            bar_w_px.ceil().max(1.0) as i32,
            (bar_bottom - py).ceil().max(1.0) as i32,
        );
        bar_rects.push(rect);
    }

    for (idx, point) in points.iter().enumerate() {
        if idx % value_step != 0 && idx + 1 != count {
            continue;
        }
        if point.tokens == 0 {
            continue;
        }
        let base = idx as f64 * slot;
        let center = base + 5.0;
        let px = plot.x as f64 + margin_px + (center / x_max) * x_plot_w;
        let py = plot.y as f64 + margin_px + (1.0 - (point.tokens as f64 / upper)) * y_plot_h;
        let value = format_compact_u64(point.tokens);
        let w = text_width(&value, value_label_scale) as i32;
        let mut tx = px.round() as i32 - w / 2;
        let min_x = plot.x as i32 + 4;
        let max_x = plot.right().saturating_sub((w as u32) + 4) as i32;
        if tx < min_x {
            tx = min_x;
        }
        if tx > max_x {
            tx = max_x;
        }
        let label_h = line_height_px(value_label_scale).max(1);
        let mut ty = (py.round() as i32 - label_h - 2).max(plot.y as i32 + 4);
        let top_limit = plot.y as i32 + 2;
        for _ in 0..10 {
            let label_rect = (tx - 2, ty - 1, w + 4, label_h + 2);
            let mut intersects = false;
            for (bar_idx, bar_rect) in bar_rects.iter().enumerate() {
                if bar_idx == idx {
                    continue;
                }
                if rects_overlap_i32(label_rect, *bar_rect) {
                    intersects = true;
                    break;
                }
            }
            if !intersects {
                break;
            }
            ty -= (label_h / 2).max(2);
            if ty <= top_limit {
                ty = top_limit;
                break;
            }
        }
        draw_rect_clamped_i32(
            img,
            tx - 2,
            ty - 1,
            w + 4,
            label_h + 2,
            Rgba([6, 14, 27, 224]),
        );
        draw_text(
            img,
            tx + 1,
            ty + 1,
            &value,
            value_label_scale,
            rgba(10, 18, 32),
        );
        draw_text(img, tx, ty, &value, value_label_scale, rgba(204, 248, 223));
    }
    Ok(())
}

fn rects_overlap_i32(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    if aw <= 0 || ah <= 0 || bw <= 0 || bh <= 0 {
        return false;
    }
    let ar = ax + aw;
    let ab = ay + ah;
    let br = bx + bw;
    let bb = by + bh;
    ax < br && ar > bx && ay < bb && ab > by
}

fn draw_rect_clamped_i32(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, color: Rgba<u8>) {
    if w <= 0 || h <= 0 {
        return;
    }
    let left = x.max(0) as u32;
    let top = y.max(0) as u32;
    let right = (x + w).max(0) as u32;
    let bottom = (y + h).max(0) as u32;
    let right = right.min(img.width());
    let bottom = bottom.min(img.height());
    if right <= left || bottom <= top {
        return;
    }
    fill_rect(
        img,
        Rect {
            x: left,
            y: top,
            w: right - left,
            h: bottom - top,
        },
        color,
    );
}

fn draw_overlay_glow(img: &mut RgbaImage, cx: u32, cy: u32, radius: u32, color: Rgba<u8>) {
    if radius == 0 {
        return;
    }
    let min_x = cx.saturating_sub(radius);
    let max_x = (cx + radius).min(img.width().saturating_sub(1));
    let min_y = cy.saturating_sub(radius);
    let max_y = (cy + radius).min(img.height().saturating_sub(1));

    let r2 = (radius as f32) * (radius as f32);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let dx = x as f32 - cx as f32;
            let dy = y as f32 - cy as f32;
            let d2 = dx * dx + dy * dy;
            if d2 > r2 {
                continue;
            }
            let alpha = (1.0 - d2 / r2).powf(1.9) * (color[3] as f32 / 255.0);
            if alpha <= 0.01 {
                continue;
            }
            blend_pixel(
                img,
                x as i32,
                y as i32,
                Rgba([color[0], color[1], color[2], (alpha * 255.0).round() as u8]),
            );
        }
    }
}

fn render_chart_bitmap(points: &[SharePoint], width: u32, height: u32) -> Result<RgbaImage> {
    let w = width.max(32);
    let h = height.max(32);
    let mut rgb = vec![0u8; (w * h * 3) as usize];

    {
        let backend = BitMapBackend::with_buffer(&mut rgb, (w, h));
        let root = backend.into_drawing_area();
        root.fill(&RGBColor(5, 12, 24))?;

        let max = points.iter().map(|p| p.tokens).max().unwrap_or(1).max(1);
        let upper = ((max as f64) * CHART_HEADROOM).ceil() as u64 + 1;
        let count = points.len().max(1) as i32;
        let slot = 10i32;
        let chart_x_max = count * slot;

        let mut chart = ChartBuilder::on(&root)
            .margin(8)
            .x_label_area_size(0)
            .y_label_area_size(0)
            .build_cartesian_2d(0..chart_x_max, 0u64..upper)?;

        chart.draw_series(points.iter().enumerate().map(|(i, p)| {
            let base = i as i32 * slot;
            let x0 = base + 3;
            let x1 = base + 7;
            Rectangle::new(
                [(x0, 0u64), (x1, p.tokens)],
                RGBColor(98, 245, 154).filled(),
            )
        }))?;

        root.present()?;
    }

    let mut rgba_img: RgbaImage = ImageBuffer::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let idx = ((y * w + x) * 3) as usize;
            rgba_img.put_pixel(x, y, Rgba([rgb[idx], rgb[idx + 1], rgb[idx + 2], 255]));
        }
    }
    Ok(rgba_img)
}

fn draw_line(img: &mut RgbaImage, mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: Rgba<u8>) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        blend_pixel(img, x0, y0, color);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = err * 2;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn draw_circle(img: &mut RgbaImage, cx: i32, cy: i32, radius: i32, color: Rgba<u8>) {
    if radius <= 0 {
        return;
    }
    let r2 = radius * radius;
    for y in (cy - radius)..=(cy + radius) {
        for x in (cx - radius)..=(cx + radius) {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy <= r2 {
                blend_pixel(img, x, y, color);
            }
        }
    }
}

fn aggregate_hourly(
    events: &[UsageEvent],
    timezone: &crate::pipeline::TimeZoneMode,
    day: NaiveDate,
) -> Vec<SharePoint> {
    let mut buckets = vec![TokenCounts::default(); 24];
    for event in events {
        if timezone.date_of(event.timestamp) != day {
            continue;
        }
        let hour = timezone.hour_of(event.timestamp) as usize;
        buckets[hour].add_assign(event.usage.to_counts());
    }

    (0..24)
        .map(|h| SharePoint {
            label: format!("{h:02}"),
            tokens: buckets[h].total_tokens,
        })
        .collect()
}

fn aggregate_daily(
    events: &[UsageEvent],
    timezone: &crate::pipeline::TimeZoneMode,
    since: NaiveDate,
    until: NaiveDate,
    max_points: usize,
) -> Vec<SharePoint> {
    let mut day = since;
    let mut buckets = BTreeMap::<NaiveDate, TokenCounts>::new();
    while day <= until {
        buckets.insert(day, TokenCounts::default());
        day += chrono::TimeDelta::days(1);
    }

    for event in events {
        let day = timezone.date_of(event.timestamp);
        if day < since || day > until {
            continue;
        }
        if let Some(bucket) = buckets.get_mut(&day) {
            bucket.add_assign(event.usage.to_counts());
        }
    }

    let mut points = buckets
        .into_iter()
        .map(|(day, counts)| SharePoint {
            label: day.format("%m-%d").to_string(),
            tokens: counts.total_tokens,
        })
        .collect::<Vec<_>>();

    if points.len() > max_points {
        points = points[points.len() - max_points..].to_vec();
    }

    points
}

fn apply_default_img_range(common: &mut crate::cli::CommonArgs, period: ImgPeriod) -> Result<()> {
    let tz = parse_timezone_arg(common.timezone.as_deref())?;
    let today = tz.now_date();

    match period {
        ImgPeriod::Daily => match (common.since.as_ref(), common.until.as_ref()) {
            (None, None) => {
                let day = today.format("%Y-%m-%d").to_string();
                common.since = Some(day.clone());
                common.until = Some(day);
            }
            (Some(since), None) => {
                common.until = Some(since.clone());
            }
            (None, Some(until)) => {
                common.since = Some(until.clone());
            }
            (Some(since), Some(until)) => {
                if since != until {
                    bail!("tu img --period daily expects one day (use same --since/--until)");
                }
            }
        },
        ImgPeriod::Weekly => match (common.since.as_ref(), common.until.as_ref()) {
            (None, None) => {
                let since = (today - chrono::TimeDelta::days(6))
                    .format("%Y-%m-%d")
                    .to_string();
                let until = today.format("%Y-%m-%d").to_string();
                common.since = Some(since);
                common.until = Some(until);
            }
            (Some(since), None) => {
                let since_day = parse_date(Some(since.as_str()))?.unwrap_or(today);
                common.until = Some(
                    (since_day + chrono::TimeDelta::days(6))
                        .format("%Y-%m-%d")
                        .to_string(),
                );
            }
            (None, Some(until)) => {
                let until_day = parse_date(Some(until.as_str()))?.unwrap_or(today);
                common.since = Some(
                    (until_day - chrono::TimeDelta::days(6))
                        .format("%Y-%m-%d")
                        .to_string(),
                );
            }
            (Some(_), Some(_)) => {}
        },
        ImgPeriod::Both => {}
    }

    Ok(())
}

fn parse_timezone_arg(input: Option<&str>) -> Result<crate::pipeline::TimeZoneMode> {
    let Some(raw) = input else {
        return Ok(crate::pipeline::TimeZoneMode::Local);
    };

    if raw.eq_ignore_ascii_case("local") {
        return Ok(crate::pipeline::TimeZoneMode::Local);
    }
    if raw.eq_ignore_ascii_case("utc") {
        return Ok(crate::pipeline::TimeZoneMode::Utc);
    }

    let tz = chrono_tz::Tz::from_str(raw)
        .with_context(|| format!("Invalid timezone: {raw}. Use e.g. UTC or Asia/Tokyo"))?;
    Ok(crate::pipeline::TimeZoneMode::Named(tz))
}

fn parse_date(input: Option<&str>) -> Result<Option<NaiveDate>> {
    let Some(raw) = input.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if raw.len() == 10 {
        return Ok(Some(
            NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .with_context(|| format!("Invalid date: {raw} (expected YYYY-MM-DD)"))?,
        ));
    }
    if raw.len() == 8 {
        return Ok(Some(
            NaiveDate::parse_from_str(raw, "%Y%m%d")
                .with_context(|| format!("Invalid date: {raw} (expected YYYYMMDD)"))?,
        ));
    }
    bail!("Invalid date: {raw} (expected YYYY-MM-DD or YYYYMMDD)");
}

fn draw_logo_box(img: &mut RgbaImage, x: u32, y: u32, size: u32, args: &ImgArgs) -> Result<()> {
    let logo_bg = rgba(25, 40, 71);
    fill_rect(
        img,
        Rect {
            x,
            y,
            w: size,
            h: size,
        },
        logo_bg,
    );
    stroke_rect(
        img,
        Rect {
            x,
            y,
            w: size,
            h: size,
        },
        rgba(84, 121, 191),
        1,
    );

    if let Some(path) = args.logo.as_deref() {
        let logo_path = expand_user_path(path);
        if logo_path.is_file() {
            let rendered = if is_svg_path(&logo_path) {
                let bytes = fs::read(&logo_path)
                    .with_context(|| format!("Failed to read logo SVG: {}", logo_path.display()))?;
                render_svg_logo(&bytes, size.saturating_sub(8), size.saturating_sub(8))
                    .with_context(|| {
                        format!("Failed to render logo SVG: {}", logo_path.display())
                    })?
            } else {
                image::open(&logo_path)
                    .with_context(|| format!("Failed to open logo image: {}", logo_path.display()))?
                    .resize(
                        size.saturating_sub(8),
                        size.saturating_sub(8),
                        FilterType::Triangle,
                    )
                    .to_rgba8()
            };
            let ox = x + (size.saturating_sub(rendered.width())) / 2;
            let oy = y + (size.saturating_sub(rendered.height())) / 2;
            overlay_image(img, &rendered, ox, oy);
            return Ok(());
        }
    }

    match render_svg_logo(
        DEFAULT_LOGO_SVG_BYTES,
        size.saturating_sub(8),
        size.saturating_sub(8),
    ) {
        Ok(rendered) => {
            let ox = x + (size.saturating_sub(rendered.width())) / 2;
            let oy = y + (size.saturating_sub(rendered.height())) / 2;
            overlay_image(img, &rendered, ox, oy);
        }
        Err(_) => {
            draw_default_logo_mark(img, x, y, size, logo_bg);
        }
    }
    Ok(())
}

fn is_svg_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
}

fn render_svg_logo(svg_bytes: &[u8], target_w: u32, target_h: u32) -> Result<RgbaImage> {
    let target_w = target_w.max(1);
    let target_h = target_h.max(1);
    let tree = usvg::Tree::from_data(svg_bytes, &usvg::Options::default())
        .context("Invalid SVG document")?;
    let svg_size = tree.size();

    let scale_x = target_w as f32 / svg_size.width();
    let scale_y = target_h as f32 / svg_size.height();
    let scale = scale_x.min(scale_y).max(0.01);

    let render_w = (svg_size.width() * scale).round().max(1.0) as u32;
    let render_h = (svg_size.height() * scale).round().max(1.0) as u32;
    let mut pixmap =
        tiny_skia::Pixmap::new(render_w, render_h).context("Failed to allocate SVG pixmap")?;
    let transform = tiny_skia::Transform::from_scale(scale, scale);
    let mut canvas = pixmap.as_mut();
    resvg::render(&tree, transform, &mut canvas);

    let raw = pixmap.data();
    let mut out = RgbaImage::new(render_w, render_h);
    for y in 0..render_h {
        for x in 0..render_w {
            let idx = ((y * render_w + x) * 4) as usize;
            out.put_pixel(
                x,
                y,
                Rgba([raw[idx], raw[idx + 1], raw[idx + 2], raw[idx + 3]]),
            );
        }
    }
    Ok(out)
}

fn draw_default_logo_mark(img: &mut RgbaImage, x: u32, y: u32, size: u32, bg: Rgba<u8>) {
    if size < 24 {
        return;
    }

    let accent = rgba(78, 236, 214);
    let cx = x as f32 + size as f32 * 0.5;

    // Top crescent.
    let crescent_cx = cx;
    let crescent_cy = y as f32 + size as f32 * 0.34;
    let crescent_rx_outer = size as f32 * 0.38;
    let crescent_ry_outer = size as f32 * 0.17;
    let crescent_rx_inner = size as f32 * 0.31;
    let crescent_ry_inner = size as f32 * 0.12;
    draw_filled_ellipse(
        img,
        crescent_cx,
        crescent_cy,
        crescent_rx_outer,
        crescent_ry_outer,
        accent,
    );
    draw_filled_ellipse(
        img,
        crescent_cx,
        crescent_cy + size as f32 * 0.03,
        crescent_rx_inner,
        crescent_ry_inner,
        bg,
    );

    // Vertical T stem.
    let stem_w = ((size as f32) * 0.08).max(4.0).round() as i32;
    let stem_h = ((size as f32) * 0.24).max(8.0).round() as i32;
    let stem_x = cx.round() as i32 - stem_w / 2;
    let stem_y = (y as f32 + size as f32 * 0.42).round() as i32;
    let stem_r = (stem_w / 2).max(1);
    fill_rect(
        img,
        Rect {
            x: stem_x.max(0) as u32,
            y: stem_y.max(0) as u32,
            w: stem_w as u32,
            h: stem_h as u32,
        },
        accent,
    );
    draw_circle(img, stem_x + stem_r, stem_y, stem_r, accent);
    draw_circle(img, stem_x + stem_r, stem_y + stem_h, stem_r, accent);

    // Bottom smile U: thick arc.
    let u_cx = cx.round() as i32;
    let u_cy = (y as f32 + size as f32 * 0.72).round() as i32;
    let u_radius = ((size as f32) * 0.24).max(10.0);
    let u_thickness = ((size as f32) * 0.17).max(4.0).round() as i32;
    // Draw the lower smile arc (∪). 18..162 degrees keeps the arc in the lower half.
    draw_arc_stroke(
        img,
        (u_cx, u_cy),
        u_radius,
        18.0,
        162.0,
        u_thickness,
        accent,
    );
}

fn draw_filled_ellipse(img: &mut RgbaImage, cx: f32, cy: f32, rx: f32, ry: f32, color: Rgba<u8>) {
    if rx <= 0.5 || ry <= 0.5 {
        return;
    }
    let min_x = (cx - rx).floor() as i32;
    let max_x = (cx + rx).ceil() as i32;
    let min_y = (cy - ry).floor() as i32;
    let max_y = (cy + ry).ceil() as i32;
    let inv_rx = 1.0 / rx;
    let inv_ry = 1.0 / ry;
    for py in min_y..=max_y {
        for px in min_x..=max_x {
            let nx = (px as f32 + 0.5 - cx) * inv_rx;
            let ny = (py as f32 + 0.5 - cy) * inv_ry;
            if nx * nx + ny * ny <= 1.0 {
                blend_pixel(img, px, py, color);
            }
        }
    }
}

fn draw_arc_stroke(
    img: &mut RgbaImage,
    center: (i32, i32),
    radius: f32,
    start_deg: f32,
    end_deg: f32,
    thickness: i32,
    color: Rgba<u8>,
) {
    if radius <= 0.5 || thickness <= 0 {
        return;
    }
    let dot_r = (thickness / 2).max(1);
    let (cx, cy) = center;
    let mut deg = start_deg;
    while deg <= end_deg {
        let rad = deg.to_radians();
        let px = cx as f32 + radius * rad.cos();
        let py = cy as f32 + radius * rad.sin();
        draw_circle(img, px.round() as i32, py.round() as i32, dot_r, color);
        deg += 0.7;
    }
}

fn draw_gradient_background(img: &mut RgbaImage, top: Rgba<u8>, bottom: Rgba<u8>) {
    let h = img.height().max(1);
    for y in 0..h {
        let t = y as f64 / (h.saturating_sub(1)) as f64;
        let c = mix(top, bottom, t);
        for x in 0..img.width() {
            img.put_pixel(x, y, c);
        }
    }
}

fn fill_rect(img: &mut RgbaImage, rect: Rect, color: Rgba<u8>) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let max_x = rect.right().min(img.width());
    let max_y = rect.bottom().min(img.height());
    for y in rect.y..max_y {
        for x in rect.x..max_x {
            blend_pixel(img, x as i32, y as i32, color);
        }
    }
}

fn stroke_rect(img: &mut RgbaImage, rect: Rect, color: Rgba<u8>, thickness: u32) {
    if rect.w == 0 || rect.h == 0 || thickness == 0 {
        return;
    }
    fill_rect(
        img,
        Rect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: thickness.min(rect.h),
        },
        color,
    );
    fill_rect(
        img,
        Rect {
            x: rect.x,
            y: rect.bottom().saturating_sub(thickness),
            w: rect.w,
            h: thickness.min(rect.h),
        },
        color,
    );
    fill_rect(
        img,
        Rect {
            x: rect.x,
            y: rect.y,
            w: thickness.min(rect.w),
            h: rect.h,
        },
        color,
    );
    fill_rect(
        img,
        Rect {
            x: rect.right().saturating_sub(thickness),
            y: rect.y,
            w: thickness.min(rect.w),
            h: rect.h,
        },
        color,
    );
}

fn draw_text(img: &mut RgbaImage, x: i32, y: i32, text: &str, scale: u32, color: Rgba<u8>) {
    let line_h = line_height_px(scale);
    for (line_idx, line) in text.split('\n').enumerate() {
        draw_text_line(img, x, y + line_h * line_idx as i32, line, scale, color);
    }
}

fn draw_text_line(
    img: &mut RgbaImage,
    mut x: i32,
    y: i32,
    text: &str,
    scale: u32,
    color: Rgba<u8>,
) {
    if let Some(font) = poster_font() {
        draw_text_line_ttf(img, font, x, y, text, scale, color);
        return;
    }

    let step = (8 * scale + scale) as i32;
    for ch in text.chars() {
        if ch == ' ' {
            x += step;
            continue;
        }
        if let Some(glyph) = font8x8::BASIC_FONTS.get(ch) {
            for (row_idx, bits) in glyph.iter().enumerate() {
                for col_idx in 0..8 {
                    if bits & (1 << col_idx) == 0 {
                        continue;
                    }
                    let px = x + (col_idx * scale as i32);
                    let py = y + (row_idx as i32 * scale as i32);
                    for dy in 0..scale {
                        for dx in 0..scale {
                            blend_pixel(img, px + dx as i32, py + dy as i32, color);
                        }
                    }
                }
            }
        }
        x += step;
    }
}

fn text_width(text: &str, scale: u32) -> usize {
    if let Some(font) = poster_font() {
        return text_width_ttf(font, text, scale);
    }

    let per_char = (8 * scale + scale) as usize;
    text.chars().count() * per_char
}

fn poster_font() -> Option<&'static FontArc> {
    POSTER_FONT
        .get_or_init(|| FontArc::try_from_slice(POSTER_FONT_BYTES).ok())
        .as_ref()
}

fn font_px(scale: u32) -> f32 {
    (scale.max(1) as f32 * 9.2).max(9.0)
}

fn line_height_px(scale: u32) -> i32 {
    (font_px(scale) * 1.20).round() as i32
}

fn draw_text_line_ttf(
    img: &mut RgbaImage,
    font: &FontArc,
    x: i32,
    y: i32,
    text: &str,
    scale: u32,
    color: Rgba<u8>,
) {
    let px = font_px(scale);
    let scale = PxScale::from(px);
    let scaled = font.as_scaled(scale);
    let mut caret_x = x as f32;
    let baseline_y = y as f32 + scaled.ascent();
    let mut prev = None;

    for ch in text.chars() {
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
                let alpha = (coverage * (color[3] as f32 / 255.0) * 255.0).round() as u8;
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

fn text_width_ttf(font: &FontArc, text: &str, scale: u32) -> usize {
    let px = font_px(scale);
    let scale = PxScale::from(px);
    let scaled = font.as_scaled(scale);
    let mut width = 0.0f32;
    let mut prev = None;
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(prev_id) = prev {
            width += scaled.kern(prev_id, id);
        }
        width += scaled.h_advance(id);
        prev = Some(id);
    }
    width.max(0.0).round() as usize
}

fn blend_pixel(img: &mut RgbaImage, x: i32, y: i32, src: Rgba<u8>) {
    if x < 0 || y < 0 {
        return;
    }
    let x = x as u32;
    let y = y as u32;
    if x >= img.width() || y >= img.height() {
        return;
    }
    if src[3] == 255 {
        img.put_pixel(x, y, src);
        return;
    }

    let dst = *img.get_pixel(x, y);
    let alpha = src[3] as f32 / 255.0;
    let inv = 1.0 - alpha;
    let out = Rgba([
        (src[0] as f32 * alpha + dst[0] as f32 * inv).round() as u8,
        (src[1] as f32 * alpha + dst[1] as f32 * inv).round() as u8,
        (src[2] as f32 * alpha + dst[2] as f32 * inv).round() as u8,
        255,
    ]);
    img.put_pixel(x, y, out);
}

fn overlay_image(base: &mut RgbaImage, logo: &RgbaImage, x: u32, y: u32) {
    for ly in 0..logo.height() {
        for lx in 0..logo.width() {
            let px = x + lx;
            let py = y + ly;
            if px >= base.width() || py >= base.height() {
                continue;
            }
            let src = *logo.get_pixel(lx, ly);
            blend_pixel(base, px as i32, py as i32, src);
        }
    }
}

fn rgba(r: u8, g: u8, b: u8) -> Rgba<u8> {
    Rgba([r, g, b, 255])
}

fn mix(a: Rgba<u8>, b: Rgba<u8>, t: f64) -> Rgba<u8> {
    let clamped = t.clamp(0.0, 1.0);
    let lerp = |x: u8, y: u8| -> u8 { (x as f64 + (y as f64 - x as f64) * clamped).round() as u8 };
    Rgba([lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2]), 255])
}

fn sanitize_model_name(input: &str) -> String {
    let mut model = input.trim();
    for _ in 0..2 {
        if let Some(rest) = model.strip_prefix("claude:") {
            model = rest;
            continue;
        }
        if let Some(rest) = model.strip_prefix("codex:") {
            model = rest;
            continue;
        }
    }
    let mut out = model.to_string();
    for prefix in ["claude-", "codex-"] {
        if let Some(rest) = out.strip_prefix(prefix) {
            out = rest.to_string();
            break;
        }
    }
    if let Some((base, suffix)) = out.rsplit_once('-') {
        if suffix.len() == 8 && suffix.chars().all(|c| c.is_ascii_digit()) {
            out = base.to_string();
        }
    }
    normalize_model_version_dots(&out)
}

fn normalize_model_version_dots(input: &str) -> String {
    let mut parts = input.split('-').map(str::to_string).collect::<Vec<_>>();
    for idx in 0..parts.len().saturating_sub(1) {
        let a = &parts[idx];
        let b = &parts[idx + 1];
        let is_short_num =
            |s: &str| !s.is_empty() && s.len() <= 2 && s.chars().all(|c| c.is_ascii_digit());
        if is_short_num(a) && is_short_num(b) {
            parts[idx] = format!("{a}.{b}");
            parts.remove(idx + 1);
            break;
        }
    }
    parts.join("-")
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
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

fn format_u64(value: u64) -> String {
    let raw = value.to_string();
    let mut out = String::with_capacity(raw.len() + raw.len() / 3);
    for (idx, ch) in raw.chars().enumerate() {
        out.push(ch);
        let remain = raw.len().saturating_sub(idx + 1);
        if remain > 0 && remain.is_multiple_of(3) {
            out.push(',');
        }
    }
    out
}

fn format_compact_u64(value: u64) -> String {
    const K: f64 = 1_000.0;
    const M: f64 = 1_000_000.0;
    const B: f64 = 1_000_000_000.0;
    let v = value as f64;
    if v >= B {
        format_compact_decimal(v / B, "B")
    } else if v >= M {
        format_compact_decimal(v / M, "M")
    } else if v >= K {
        format_compact_decimal(v / K, "K")
    } else {
        format_u64(value)
    }
}

fn format_compact_u64_spaced(value: u64) -> String {
    const K: f64 = 1_000.0;
    const M: f64 = 1_000_000.0;
    const B: f64 = 1_000_000_000.0;
    let v = value as f64;
    if v >= B {
        format_compact_decimal_spaced(v / B, "B")
    } else if v >= M {
        format_compact_decimal_spaced(v / M, "M")
    } else if v >= K {
        format_compact_decimal_spaced(v / K, "K")
    } else {
        format_u64(value)
    }
}

fn format_compact_decimal(value: f64, suffix: &str) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if (rounded - rounded.trunc()).abs() < f64::EPSILON {
        format!("{}{}", rounded as u64, suffix)
    } else {
        format!("{rounded:.1}{suffix}")
    }
}

fn format_compact_decimal_spaced(value: f64, suffix: &str) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if (rounded - rounded.trunc()).abs() < f64::EPSILON {
        format!("{} {}", rounded as u64, suffix)
    } else {
        format!("{rounded:.1} {suffix}")
    }
}

fn format_peak_bucket_label(period_label: &str, raw: &str) -> String {
    if period_label.starts_with("daily")
        && let Ok(hour) = raw.parse::<u32>()
        && hour < 24
    {
        return format!("{hour:02}:00-{:02}:00", hour + 1);
    }
    raw.to_string()
}

fn format_usd(value: f64) -> String {
    format!("${value:.2}")
}

fn expand_user_path(input: &str) -> PathBuf {
    if input == "~"
        && let Some(home) = dirs::home_dir()
    {
        return home;
    }
    if let Some(rest) = input.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    Path::new(input).to_path_buf()
}
