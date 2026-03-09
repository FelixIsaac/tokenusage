mod activity;
mod cli;
mod config;
mod gui;
mod heartbeat;
mod output;
mod pipeline;
mod share;
mod types;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Commands, DailyArgs, normalize_cli_args};

pub async fn run() -> Result<()> {
    let cli = Cli::parse_from(normalize_cli_args(std::env::args().collect()));

    let command = cli.command.unwrap_or(Commands::Daily(DailyArgs::default()));
    let command = config::apply_config(command)?;

    let throttle_ms = extract_throttle(&command);
    if throttle_ms > 0 && std::env::var("_TU_THROTTLE_ACTIVE").is_err() {
        run_with_throttle(throttle_ms)
    } else {
        dispatch(command).await
    }
}

fn extract_throttle(cmd: &Commands) -> u64 {
    match cmd {
        Commands::Daily(a) | Commands::Codex(a) | Commands::Claude(a) => a.common.slow,
        Commands::Today(a) => a.common.slow,
        Commands::Activity(a) => a.common.slow,
        Commands::Monthly(a) => a.common.slow,
        Commands::Weekly(a) => a.common.slow,
        Commands::Img(a) => a.common.slow,
        Commands::Session(a) => a.common.slow,
        Commands::Blocks(a) => a.common.slow,
        Commands::Live(a) => a.common.slow,
        Commands::Top(a) => a.common.slow,
        Commands::Statusline(a) => a.common.slow,
        Commands::Gui(a) => a.common.slow,
        Commands::Heartbeat(a) => match &a.command {
            cli::HeartbeatCommand::Stats(s) => s.slow,
            _ => None,
        },
        _ => None,
    }
    .unwrap_or(0)
}

/// Re-invoke the same binary without `--throttle`, piping stdout line-by-line
/// with a delay between each line. Pure Rust, no libc/unsafe needed.
fn run_with_throttle(delay_ms: u64) -> Result<()> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    let exe = std::env::current_exe()?;
    let args = args_without_slow();
    let delay = Duration::from_millis(delay_ms);

    let mut child = Command::new(exe)
        .args(&args)
        .env("_TU_THROTTLE_ACTIVE", "1")
        .env("CLICOLOR_FORCE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let mut out = std::io::stdout().lock();

    for line in reader.lines().map_while(Result::ok) {
        let _ = writeln!(out, "{}", line);
        let _ = out.flush();
        std::thread::sleep(delay);
    }

    let status = child.wait()?;
    if !status.success() {
        anyhow::bail!("child exited with {}", status);
    }
    Ok(())
}

fn args_without_slow() -> Vec<String> {
    let mut result = Vec::new();
    let mut skip_next = false;
    for arg in std::env::args().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--slow" || arg == "-S" {
            skip_next = true;
            continue;
        }
        if arg.starts_with("--slow=") {
            continue;
        }
        result.push(arg);
    }
    result
}

async fn dispatch(command: Commands) -> Result<()> {
    match command {
        Commands::Daily(args) => pipeline::run_daily(args).await,
        Commands::Today(args) => pipeline::run_today(args).await,
        Commands::Activity(args) => pipeline::run_activity(args).await,
        Commands::Heartbeat(args) => heartbeat::run(args).await,
        Commands::Codex(mut args) => {
            args.common.no_claude = true;
            args.common.no_codex = false;
            pipeline::run_daily(args).await
        }
        Commands::Claude(mut args) => {
            args.common.no_codex = true;
            args.common.no_claude = false;
            pipeline::run_daily(args).await
        }
        Commands::Antigravity(args) => pipeline::run_antigravity(args).await,
        Commands::Monthly(args) => pipeline::run_monthly(args).await,
        Commands::Weekly(args) => pipeline::run_weekly(args).await,
        Commands::Img(args) => share::run_share(args).await,
        Commands::Session(args) => pipeline::run_session(args).await,
        Commands::Blocks(args) => pipeline::run_blocks(args).await,
        Commands::Live(args) => pipeline::run_blocks(args.into()).await,
        Commands::Top(args) => pipeline::run_top(args).await,
        Commands::Statusline(args) => pipeline::run_statusline(args).await,
        Commands::Gui(args) => gui::run_gui(args),
    }
}
