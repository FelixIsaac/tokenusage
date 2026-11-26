pub(crate) mod data;
pub(crate) mod html;
pub(crate) mod png;
pub(crate) mod tui;

use std::io::{self, IsTerminal};
use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::cli::WrappedArgs;
use crate::pipeline::{emit_json, load_usage_unsorted, parse_timezone_mode, should_emit_json};

use data::{WrappedBuildOptions, build_wrapped_bundle, print_wrapped_static};

pub(crate) async fn run_wrapped(args: WrappedArgs) -> Result<()> {
    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let timezone_label = args
        .common
        .timezone
        .clone()
        .unwrap_or_else(|| "local".to_string());

    let loaded = load_usage_unsorted(&args.common, &tz).await?;
    let mut bundle = build_wrapped_bundle(WrappedBuildOptions {
        events: &loaded.events,
        tz: &tz,
        timezone_label,
        pricing: &loaded.pricing,
        target_year: args.year,
        all_time: args.all_time,
        include_projects: args.include_projects,
        common: Some(&args.common),
    });

    if should_emit_json(&args.common) {
        return emit_json(&bundle.report, args.common.jq.as_deref());
    }

    if args.prompt {
        println!("{}", bundle.report.agent_prompt);
        return Ok(());
    }

    let mut exported_any = false;
    if let Some(html_path) = args.html.as_deref() {
        html::write_wrapped_html(html_path, &bundle.report, args.theme)?;
        println!("HTML micro-site written: {}", html_path.display());
        if args.open {
            open_in_default_viewer(html_path);
        }
        exported_any = true;
    }

    if let Some(png_path) = args.png.as_deref() {
        png::write_wrapped_png(png_path, &bundle.report, args.theme)?;
        println!("Share poster PNG written: {}", png_path.display());
        if args.open && args.html.is_none() {
            open_in_default_viewer(png_path);
        }
        exported_any = true;
    }

    if exported_any || args.no_tui || !io::stdout().is_terminal() {
        print_wrapped_static(&bundle.report);
        return Ok(());
    }

    tui::run_wrapped_tui(&mut bundle, args.theme)
}

pub(crate) fn open_in_default_viewer(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(path).spawn();

    #[cfg(target_os = "linux")]
    let _ = Command::new("xdg-open").arg(path).spawn();

    #[cfg(target_os = "windows")]
    let _ = Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn();
}
