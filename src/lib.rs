mod cli;
mod config;
mod gui;
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

    match command {
        Commands::Daily(args) => pipeline::run_daily(args).await,
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
        Commands::Monthly(args) => pipeline::run_monthly(args).await,
        Commands::Weekly(args) => pipeline::run_weekly(args).await,
        Commands::Img(args) => share::run_share(args).await,
        Commands::Session(args) => pipeline::run_session(args).await,
        Commands::Blocks(args) => pipeline::run_blocks(args).await,
        Commands::Live(args) => pipeline::run_blocks(args.into()).await,
        Commands::Statusline(args) => pipeline::run_statusline(args).await,
        Commands::Gui(args) => gui::run_gui(args),
    }
}
