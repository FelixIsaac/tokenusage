use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::cli::{
    ActivityArgs, BlocksArgs, Commands, CommonArgs, CostSource, DailyArgs, GuiArgs, GuiPeriod,
    ImgArgs, ImgPeriod, LiveArgs, MonthlyArgs, SessionArgs, SortOrder, StatuslineArgs, TodayArgs,
    VisualBurnRate, WeekStart, WeeklyArgs,
};

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    defaults: Option<CommonConfig>,
    commands: Option<CommandConfigs>,
}

#[derive(Debug, Default, Deserialize)]
struct CommandConfigs {
    daily: Option<DailyConfig>,
    today: Option<TodayConfig>,
    activity: Option<ActivityConfig>,
    codex: Option<DailyConfig>,
    claude: Option<DailyConfig>,
    monthly: Option<MonthlyConfig>,
    weekly: Option<WeeklyConfig>,
    img: Option<ImgConfig>,
    session: Option<SessionConfig>,
    blocks: Option<BlocksConfig>,
    live: Option<LiveConfig>,
    statusline: Option<StatuslineConfig>,
    gui: Option<GuiConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct CommonConfig {
    since: Option<String>,
    until: Option<String>,
    json: Option<bool>,
    jq: Option<String>,
    debug: Option<bool>,
    #[serde(alias = "debugSamples")]
    debug_samples: Option<usize>,
    order: Option<SortOrder>,
    breakdown: Option<bool>,
    offline: Option<bool>,
    timezone: Option<String>,
    locale: Option<String>,
    compact: Option<bool>,
    workers: Option<usize>,
    #[serde(alias = "noClaude")]
    no_claude: Option<bool>,
    #[serde(alias = "noCodex")]
    no_codex: Option<bool>,
    #[serde(alias = "noAntigravity")]
    no_antigravity: Option<bool>,
    #[serde(alias = "claudeProjectsDir")]
    claude_projects_dir: Option<Vec<String>>,
    #[serde(alias = "codexSessionsDir")]
    codex_sessions_dir: Option<Vec<String>>,
    #[serde(alias = "ignorePath")]
    ignore_path: Option<Vec<String>>,
    #[serde(alias = "noDefaultIgnores")]
    no_default_ignores: Option<bool>,
    #[serde(alias = "noIncrementalCache")]
    no_incremental_cache: Option<bool>,
    #[serde(alias = "rebuildCache")]
    rebuild_cache: Option<bool>,
    #[serde(alias = "pricingFile")]
    pricing_file: Option<String>,
    #[serde(alias = "withActivity")]
    with_activity: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct DailyConfig {
    #[serde(flatten)]
    common: CommonConfig,
    tui: Option<bool>,
    instances: Option<bool>,
    project: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct TodayConfig {
    #[serde(flatten)]
    common: CommonConfig,
    project: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ActivityConfig {
    #[serde(flatten)]
    common: CommonConfig,
    project: Option<String>,
    days: Option<u32>,
    limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct MonthlyConfig {
    #[serde(flatten)]
    common: CommonConfig,
}

#[derive(Debug, Default, Deserialize)]
struct WeeklyConfig {
    #[serde(flatten)]
    common: CommonConfig,
    #[serde(alias = "startOfWeek")]
    start_of_week: Option<WeekStart>,
}

#[derive(Debug, Default, Deserialize)]
struct SessionConfig {
    #[serde(flatten)]
    common: CommonConfig,
    id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct BlocksConfig {
    #[serde(flatten)]
    common: CommonConfig,
    active: Option<bool>,
    recent: Option<bool>,
    #[serde(alias = "tokenLimit")]
    token_limit: Option<String>,
    #[serde(alias = "sessionLength")]
    session_length: Option<u32>,
    live: Option<bool>,
    #[serde(alias = "refreshInterval")]
    refresh_interval: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct LiveConfig {
    #[serde(flatten)]
    common: CommonConfig,
    #[serde(alias = "tokenLimit")]
    token_limit: Option<String>,
    #[serde(alias = "sessionLength")]
    session_length: Option<u32>,
    #[serde(alias = "refreshInterval")]
    refresh_interval: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct StatuslineConfig {
    #[serde(flatten)]
    common: CommonConfig,
    cache: Option<bool>,
    #[serde(alias = "refreshInterval")]
    refresh_interval: Option<u64>,
    #[serde(alias = "visualBurnRate")]
    visual_burn_rate: Option<VisualBurnRate>,
    #[serde(alias = "costSource")]
    cost_source: Option<CostSource>,
    #[serde(alias = "contextLowThreshold")]
    context_low_threshold: Option<u8>,
    #[serde(alias = "contextMediumThreshold")]
    context_medium_threshold: Option<u8>,
}

#[derive(Debug, Default, Deserialize)]
struct GuiConfig {
    #[serde(flatten)]
    common: CommonConfig,
    period: Option<GuiPeriod>,
    instances: Option<bool>,
    project: Option<String>,
    #[serde(alias = "startOfWeek")]
    start_of_week: Option<WeekStart>,
}

#[derive(Debug, Default, Deserialize)]
struct ImgConfig {
    #[serde(flatten)]
    common: CommonConfig,
    period: Option<ImgPeriod>,
    project: Option<String>,
    output: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    bars: Option<usize>,
    brand: Option<String>,
    #[serde(alias = "brandUrl")]
    brand_url: Option<String>,
    logo: Option<String>,
}

pub(crate) fn apply_config(command: Commands) -> Result<Commands> {
    let cfg_path = resolve_config_path(&command)?;
    let Some(config_path) = cfg_path else {
        return Ok(command);
    };

    let body = std::fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
    let config: ConfigFile = serde_json::from_str(&body)
        .with_context(|| format!("Invalid config JSON: {}", config_path.display()))?;

    let merged = match command {
        Commands::Daily(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            if let Some(daily_cfg) = config.commands.as_ref().and_then(|c| c.daily.as_ref()) {
                apply_common_config(&mut args.common, Some(&daily_cfg.common));
                apply_daily_config(&mut args, daily_cfg);
            }
            Commands::Daily(args)
        }
        Commands::Today(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            if let Some(today_cfg) = config.commands.as_ref().and_then(|c| c.today.as_ref()) {
                apply_common_config(&mut args.common, Some(&today_cfg.common));
                apply_today_config(&mut args, today_cfg);
            }
            Commands::Today(args)
        }
        Commands::Activity(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            if let Some(activity_cfg) = config.commands.as_ref().and_then(|c| c.activity.as_ref()) {
                apply_common_config(&mut args.common, Some(&activity_cfg.common));
                apply_activity_config(&mut args, activity_cfg);
            }
            Commands::Activity(args)
        }
        Commands::Heartbeat(args) => Commands::Heartbeat(args),
        Commands::Codex(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            if let Some(daily_cfg) = config.commands.as_ref().and_then(|c| c.daily.as_ref()) {
                apply_common_config(&mut args.common, Some(&daily_cfg.common));
                apply_daily_config(&mut args, daily_cfg);
            }
            if let Some(codex_cfg) = config.commands.as_ref().and_then(|c| c.codex.as_ref()) {
                apply_common_config(&mut args.common, Some(&codex_cfg.common));
                apply_daily_config(&mut args, codex_cfg);
            }
            Commands::Codex(args)
        }
        Commands::Claude(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            if let Some(daily_cfg) = config.commands.as_ref().and_then(|c| c.daily.as_ref()) {
                apply_common_config(&mut args.common, Some(&daily_cfg.common));
                apply_daily_config(&mut args, daily_cfg);
            }
            if let Some(claude_cfg) = config.commands.as_ref().and_then(|c| c.claude.as_ref()) {
                apply_common_config(&mut args.common, Some(&claude_cfg.common));
                apply_daily_config(&mut args, claude_cfg);
            }
            Commands::Claude(args)
        }
        Commands::Antigravity(args) => Commands::Antigravity(args),
        Commands::Monthly(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            if let Some(monthly_cfg) = config.commands.as_ref().and_then(|c| c.monthly.as_ref()) {
                apply_common_config(&mut args.common, Some(&monthly_cfg.common));
                apply_monthly_config(&mut args, monthly_cfg);
            }
            Commands::Monthly(args)
        }
        Commands::Weekly(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            if let Some(weekly_cfg) = config.commands.as_ref().and_then(|c| c.weekly.as_ref()) {
                apply_common_config(&mut args.common, Some(&weekly_cfg.common));
                apply_weekly_config(&mut args, weekly_cfg);
            }
            Commands::Weekly(args)
        }
        Commands::Img(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            if let Some(img_cfg) = config.commands.as_ref().and_then(|c| c.img.as_ref()) {
                apply_common_config(&mut args.common, Some(&img_cfg.common));
                apply_img_config(&mut args, img_cfg);
            }
            Commands::Img(args)
        }
        Commands::Session(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            if let Some(session_cfg) = config.commands.as_ref().and_then(|c| c.session.as_ref()) {
                apply_common_config(&mut args.common, Some(&session_cfg.common));
                apply_session_config(&mut args, session_cfg);
            }
            Commands::Session(args)
        }
        Commands::Blocks(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            if let Some(blocks_cfg) = config.commands.as_ref().and_then(|c| c.blocks.as_ref()) {
                apply_common_config(&mut args.common, Some(&blocks_cfg.common));
                apply_blocks_config(&mut args, blocks_cfg);
            }
            Commands::Blocks(args)
        }
        Commands::Live(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            if let Some(blocks_cfg) = config.commands.as_ref().and_then(|c| c.blocks.as_ref()) {
                apply_common_config(&mut args.common, Some(&blocks_cfg.common));
                apply_live_from_blocks_config(&mut args, blocks_cfg);
            }
            if let Some(live_cfg) = config.commands.as_ref().and_then(|c| c.live.as_ref()) {
                apply_common_config(&mut args.common, Some(&live_cfg.common));
                apply_live_config(&mut args, live_cfg);
            }
            Commands::Live(args)
        }
        Commands::Top(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            Commands::Top(args)
        }
        Commands::Statusline(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            if let Some(statusline_cfg) =
                config.commands.as_ref().and_then(|c| c.statusline.as_ref())
            {
                apply_common_config(&mut args.common, Some(&statusline_cfg.common));
                apply_statusline_config(&mut args, statusline_cfg);
            }
            Commands::Statusline(args)
        }
        Commands::Gui(mut args) => {
            apply_common_config(&mut args.common, config.defaults.as_ref());
            if let Some(gui_cfg) = config.commands.as_ref().and_then(|c| c.gui.as_ref()) {
                apply_common_config(&mut args.common, Some(&gui_cfg.common));
                apply_gui_config(&mut args, gui_cfg);
            }
            Commands::Gui(args)
        }
    };

    Ok(merged)
}

fn resolve_config_path(command: &Commands) -> Result<Option<PathBuf>> {
    let explicit = match command {
        Commands::Daily(args) => args.common.config.as_deref(),
        Commands::Today(args) => args.common.config.as_deref(),
        Commands::Activity(args) => args.common.config.as_deref(),
        Commands::Heartbeat(_) => None,
        Commands::Codex(args) => args.common.config.as_deref(),
        Commands::Claude(args) => args.common.config.as_deref(),
        Commands::Antigravity(args) => args.config.as_deref(),
        Commands::Monthly(args) => args.common.config.as_deref(),
        Commands::Weekly(args) => args.common.config.as_deref(),
        Commands::Img(args) => args.common.config.as_deref(),
        Commands::Session(args) => args.common.config.as_deref(),
        Commands::Blocks(args) => args.common.config.as_deref(),
        Commands::Live(args) => args.common.config.as_deref(),
        Commands::Top(args) => args.common.config.as_deref(),
        Commands::Statusline(args) => args.common.config.as_deref(),
        Commands::Gui(args) => args.common.config.as_deref(),
    };

    if let Some(path) = explicit {
        let p = expand_user_path(path);
        if p.is_file() {
            return Ok(Some(p));
        }
        bail!("Config file not found: {}", p.display());
    }

    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(".tu").join("tu.json"));
    }

    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".config").join("tu").join("tu.json"));
        candidates.push(
            home.join(".config")
                .join("tokenusage")
                .join("tokenusage.json"),
        );
    }

    Ok(candidates.into_iter().find(|p| p.is_file()))
}

fn apply_common_config(common: &mut CommonArgs, cfg: Option<&CommonConfig>) {
    let Some(cfg) = cfg else {
        return;
    };

    merge_if_none(&mut common.since, &cfg.since);
    merge_if_none(&mut common.until, &cfg.until);
    merge_if_false(&mut common.json, cfg.json);
    merge_if_none(&mut common.jq, &cfg.jq);
    merge_if_false(&mut common.debug, cfg.debug);
    merge_if_usize_default(&mut common.debug_samples, cfg.debug_samples, 5);
    merge_if_default(&mut common.order, cfg.order, SortOrder::Asc);
    merge_if_false(&mut common.breakdown, cfg.breakdown);
    merge_if_false(&mut common.offline, cfg.offline);
    merge_if_none(&mut common.timezone, &cfg.timezone);
    merge_if_none(&mut common.locale, &cfg.locale);
    merge_if_false(&mut common.compact, cfg.compact);
    merge_if_none(&mut common.workers, &cfg.workers);
    merge_if_false(&mut common.no_claude, cfg.no_claude);
    merge_if_false(&mut common.no_codex, cfg.no_codex);
    merge_if_false(&mut common.no_antigravity, cfg.no_antigravity);
    if common.claude_projects_dir.is_empty()
        && let Some(values) = cfg.claude_projects_dir.as_ref()
    {
        common.claude_projects_dir = values.clone();
    }
    if common.codex_sessions_dir.is_empty()
        && let Some(values) = cfg.codex_sessions_dir.as_ref()
    {
        common.codex_sessions_dir = values.clone();
    }
    if common.ignore_path.is_empty()
        && let Some(values) = cfg.ignore_path.as_ref()
    {
        common.ignore_path = values.clone();
    }
    merge_if_false(&mut common.no_default_ignores, cfg.no_default_ignores);
    merge_if_false(&mut common.no_incremental_cache, cfg.no_incremental_cache);
    merge_if_false(&mut common.rebuild_cache, cfg.rebuild_cache);
    merge_if_none(&mut common.pricing_file, &cfg.pricing_file);
    merge_if_false(&mut common.with_activity, cfg.with_activity);
}

fn apply_daily_config(args: &mut DailyArgs, cfg: &DailyConfig) {
    merge_if_false(&mut args.tui, cfg.tui);
    merge_if_false(&mut args.instances, cfg.instances);
    merge_if_none(&mut args.project, &cfg.project);
}

fn apply_monthly_config(_args: &mut MonthlyArgs, _cfg: &MonthlyConfig) {}

fn apply_today_config(args: &mut TodayArgs, cfg: &TodayConfig) {
    merge_if_none(&mut args.project, &cfg.project);
}

fn apply_activity_config(args: &mut ActivityArgs, cfg: &ActivityConfig) {
    merge_if_none(&mut args.project, &cfg.project);
    merge_if_u32_default(&mut args.days, cfg.days, 7);
    merge_if_usize_default(&mut args.limit, cfg.limit, 5);
}

fn apply_weekly_config(args: &mut WeeklyArgs, cfg: &WeeklyConfig) {
    merge_if_default(
        &mut args.start_of_week,
        cfg.start_of_week,
        WeekStart::Sunday,
    );
}

fn apply_session_config(args: &mut SessionArgs, cfg: &SessionConfig) {
    merge_if_none(&mut args.id, &cfg.id);
}

fn apply_blocks_config(args: &mut BlocksArgs, cfg: &BlocksConfig) {
    merge_if_false(&mut args.active, cfg.active);
    merge_if_false(&mut args.recent, cfg.recent);
    merge_if_none(&mut args.token_limit, &cfg.token_limit);
    merge_if_u32_default(&mut args.session_length, cfg.session_length, 5);
    merge_if_false(&mut args.live, cfg.live);
    merge_if_u64_default(&mut args.refresh_interval, cfg.refresh_interval, 1);
}

fn apply_live_from_blocks_config(args: &mut LiveArgs, cfg: &BlocksConfig) {
    merge_if_none(&mut args.token_limit, &cfg.token_limit);
    merge_if_u32_default(&mut args.session_length, cfg.session_length, 5);
    merge_if_u64_default(&mut args.refresh_interval, cfg.refresh_interval, 1);
}

fn apply_live_config(args: &mut LiveArgs, cfg: &LiveConfig) {
    merge_if_none(&mut args.token_limit, &cfg.token_limit);
    merge_if_u32_default(&mut args.session_length, cfg.session_length, 5);
    merge_if_u64_default(&mut args.refresh_interval, cfg.refresh_interval, 1);
}

fn apply_statusline_config(args: &mut StatuslineArgs, cfg: &StatuslineConfig) {
    if let Some(value) = cfg.cache {
        args.cache = value;
    }
    merge_if_u64_default(&mut args.refresh_interval, cfg.refresh_interval, 1);
    merge_if_default(
        &mut args.visual_burn_rate,
        cfg.visual_burn_rate,
        VisualBurnRate::Off,
    );
    merge_if_default(&mut args.cost_source, cfg.cost_source, CostSource::Auto);
    merge_if_u8_default(
        &mut args.context_low_threshold,
        cfg.context_low_threshold,
        50,
    );
    merge_if_u8_default(
        &mut args.context_medium_threshold,
        cfg.context_medium_threshold,
        80,
    );
}

fn apply_gui_config(args: &mut GuiArgs, cfg: &GuiConfig) {
    merge_if_default(&mut args.period, cfg.period, GuiPeriod::Daily);
    merge_if_false(&mut args.instances, cfg.instances);
    merge_if_none(&mut args.project, &cfg.project);
    merge_if_default(
        &mut args.start_of_week,
        cfg.start_of_week,
        WeekStart::Sunday,
    );
}

fn apply_img_config(args: &mut ImgArgs, cfg: &ImgConfig) {
    merge_if_default(&mut args.period, cfg.period, ImgPeriod::Both);
    merge_if_none(&mut args.project, &cfg.project);
    merge_if_string_default(
        &mut args.output,
        cfg.output.as_deref(),
        "tokenusage-share.png",
    );
    merge_if_u32_default(&mut args.width, cfg.width, 1600);
    merge_if_u32_default(&mut args.height, cfg.height, 900);
    merge_if_usize_default(&mut args.bars, cfg.bars, 56);
    merge_if_string_default(&mut args.brand, cfg.brand.as_deref(), "tokenusage");
    merge_if_string_default(
        &mut args.brand_url,
        cfg.brand_url.as_deref(),
        "https://github.com/hanbu97/tokenusage",
    );
    merge_if_none(&mut args.logo, &cfg.logo);
}

fn merge_if_none<T: Clone>(dst: &mut Option<T>, src: &Option<T>) {
    if dst.is_none() {
        *dst = src.clone();
    }
}

fn merge_if_false(dst: &mut bool, src: Option<bool>) {
    if !*dst && let Some(value) = src {
        *dst = value;
    }
}

fn merge_if_default<T: Copy + Eq>(dst: &mut T, src: Option<T>, default: T) {
    if *dst == default
        && let Some(value) = src
    {
        *dst = value;
    }
}

fn merge_if_usize_default(dst: &mut usize, src: Option<usize>, default: usize) {
    if *dst == default
        && let Some(value) = src
    {
        *dst = value;
    }
}

fn merge_if_u32_default(dst: &mut u32, src: Option<u32>, default: u32) {
    if *dst == default
        && let Some(value) = src
    {
        *dst = value;
    }
}

fn merge_if_u64_default(dst: &mut u64, src: Option<u64>, default: u64) {
    if *dst == default
        && let Some(value) = src
    {
        *dst = value;
    }
}

fn merge_if_u8_default(dst: &mut u8, src: Option<u8>, default: u8) {
    if *dst == default
        && let Some(value) = src
    {
        *dst = value;
    }
}

fn merge_if_string_default(dst: &mut String, src: Option<&str>, default: &str) {
    if dst == default
        && let Some(value) = src
    {
        *dst = value.to_string();
    }
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
