use std::io;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color as TuiColor, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph, Wrap};

use crate::activity::{ActivityDataset, activity_enabled, fetch_activity_dataset};
use crate::cli::{BlocksArgs, CommonArgs};
use crate::types::{ActivitySummary, SourceKind, TokenCounts, UsageEvent};

use super::block_report::build_block_json_report;
use super::membership::*;
use super::official::*;
use super::parsing::load_usage;
use super::statusline::*;
use super::*;

pub(crate) async fn run_blocks(args: BlocksArgs) -> Result<()> {
    if args.session_length == 0 {
        bail!("--session-length must be greater than 0");
    }
    if args.refresh_interval == 0 {
        bail!("--refresh-interval must be greater than 0");
    }

    let use_json = should_emit_json(&args.common);
    if args.live && use_json {
        bail!("--live cannot be used together with --json/--jq");
    }
    if args.official_limits_only {
        if !use_json {
            bail!("--official-limits-only requires --json or --jq");
        }
        return emit_json(
            &serde_json::json!({
                "official_codex": fetch_codex_official_limits().await?,
            }),
            args.common.jq.as_deref(),
        );
    }

    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let window_secs = i64::from(args.session_length) * 3600;
    let token_limit_mode = parse_token_limit_mode(args.token_limit.as_deref())?;
    if args.smoke_check {
        let loaded = load_usage(&args.common, &tz).await?;
        println!(
            "live-smoke-ok: events={} files={} parsed_lines={}",
            loaded.events.len(),
            loaded.stats.files_discovered,
            loaded.stats.lines_parsed
        );
        return Ok(());
    }
    // For live mode, skip blocking initial fetch — go straight to TUI and fetch in background.
    if args.live {
        return run_blocks_live(
            &args,
            &tz,
            window_secs,
            token_limit_mode,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
    }

    let (
        official_codex,
        official_claude,
        official_antigravity,
        official_deepseek,
        official_openrouter,
        official_grok,
        official_kimi,
        official_anthropic_api,
    ) = if args.official_limits {
        let (codex, claude, antigravity, deepseek, openrouter, grok, kimi, anthropic, errors) =
            fetch_selected_official_limits(&args.common).await;
        for error in errors {
            eprintln!("{error}");
        }
        (
            codex,
            claude,
            antigravity,
            deepseek,
            openrouter,
            grok,
            kimi,
            anthropic,
        )
    } else {
        (None, None, None, None, None, None, None, None)
    };

    let loaded = load_usage(&args.common, &tz).await?;
    let now = Utc::now();
    let membership_estimate = estimate_membership_from_logs(&loaded.events, now, window_secs);
    let inferred_limit = membership_estimate
        .as_ref()
        .map(|estimate| estimate.estimated_window_tokens);
    let resolved_from_mode =
        resolve_token_limit(token_limit_mode, &loaded.events, now, window_secs);
    let resolved_limit = resolved_from_mode.or(inferred_limit);
    let token_limit_source =
        resolve_token_limit_source(token_limit_mode, resolved_from_mode, inferred_limit);
    let json_report = build_block_json_report(
        loaded,
        &tz,
        BlockReportBuildOptions {
            order: args.common.order,
            recent_only: args.recent,
            active_only: args.active,
            window_secs,
            token_limit: resolved_limit,
            token_limit_source,
            membership_estimate: membership_estimate.clone(),
            official_codex: official_codex.clone(),
            official_claude: official_claude.clone(),
            official_antigravity: official_antigravity.clone(),
            official_deepseek: official_deepseek.clone(),
            official_openrouter: official_openrouter.clone(),
            official_grok: official_grok.clone(),
            official_kimi: official_kimi.clone(),
            official_anthropic_api: official_anthropic_api.clone(),
            now,
        },
    );

    if use_json {
        emit_json(&json_report, args.common.jq.as_deref())
    } else {
        let rows = json_report
            .blocks
            .iter()
            .map(|row| DailyRow {
                date: row.start_time.clone(),
                totals: row.totals.clone(),
                models: row.models.clone(),
                sources: BTreeMap::new(),
                models_by_source: BTreeMap::new(),
                activity: None,
            })
            .collect::<Vec<_>>();

        let totals = json_report.totals.clone();
        let insights = Some(crate::insights::compute_report_insights(
            &rows, &totals, None,
        ));

        let show = DailyReport {
            daily: rows,
            totals,
            activity_totals: None,
            stats: json_report.stats,
            insights,
        };

        print_report_table_with_options(
            &show,
            args.common.compact,
            args.common.breakdown,
            args.common.brief,
        );
        print_membership_estimate(
            &json_report.membership_estimate,
            resolved_limit,
            token_limit_source,
            json_report.official_codex.as_ref(),
            json_report.official_claude.as_ref(),
            json_report.official_antigravity.as_ref(),
            &tz,
        );
        print_debug(&show.stats, &args.common);
        Ok(())
    }
}

pub(super) async fn fetch_selected_official_limits(
    common: &CommonArgs,
) -> (
    Option<OfficialCodexSnapshot>,
    Option<OfficialClaudeSnapshot>,
    Option<OfficialAntigravitySnapshot>,
    Option<OfficialDeepSeekSnapshot>,
    Option<OfficialOpenRouterSnapshot>,
    Option<OfficialGrokSnapshot>,
    Option<OfficialKimiSnapshot>,
    Option<OfficialAnthropicApiSnapshot>,
    Vec<String>,
) {
    let codex_enabled = !common.no_codex;
    let claude_enabled = !common.no_claude;
    let antigravity_enabled = !common.no_antigravity;
    let deepseek_enabled = std::env::var("DEEPSEEK_API_KEY").is_ok();
    let openrouter_enabled = std::env::var("OPENROUTER_API_KEY").is_ok();
    let grok_enabled = std::env::var("XAI_API_KEY").is_ok();
    let kimi_enabled = std::env::var("MOONSHOT_API_KEY").is_ok();
    let anthropic_api_enabled =
        std::env::var("ANTHROPIC_API_KEY").is_ok() || std::env::var("ANTHROPIC_ADMIN_KEY").is_ok();

    let (
        codex_result,
        claude_result,
        antigravity_result,
        deepseek_result,
        openrouter_result,
        grok_result,
        kimi_result,
        anthropic_result,
    ) = tokio::join!(
        async {
            if codex_enabled {
                Some(fetch_codex_official_limits().await)
            } else {
                None
            }
        },
        async {
            if claude_enabled {
                Some(fetch_claude_official_limits().await)
            } else {
                None
            }
        },
        async {
            if antigravity_enabled {
                Some(fetch_antigravity_official_limits().await)
            } else {
                None
            }
        },
        async {
            if deepseek_enabled {
                Some(fetch_deepseek_official_limits().await)
            } else {
                None
            }
        },
        async {
            if openrouter_enabled {
                Some(fetch_openrouter_account_limits().await)
            } else {
                None
            }
        },
        async {
            if grok_enabled {
                Some(fetch_grok_official_limits().await)
            } else {
                None
            }
        },
        async {
            if kimi_enabled {
                Some(fetch_kimi_official_limits().await)
            } else {
                None
            }
        },
        async {
            if anthropic_api_enabled {
                Some(fetch_anthropic_api_limits().await)
            } else {
                None
            }
        }
    );

    let mut errors = Vec::new();

    let codex = match codex_result {
        Some(Ok(snapshot)) => Some(snapshot),
        Some(Err(error)) => {
            errors.push(format!("official: failed to fetch Codex limits ({error})"));
            None
        }
        None => None,
    };

    let claude = match claude_result {
        Some(Ok(snapshot)) => Some(snapshot),
        Some(Err(error)) => {
            errors.push(format!("official: failed to fetch Claude limits ({error})"));
            None
        }
        None => None,
    };

    let antigravity = match antigravity_result {
        Some(Ok(snapshot)) => Some(snapshot),
        Some(Err(_)) => None, // Silently skip — Antigravity may not be running
        None => None,
    };

    let deepseek = match deepseek_result {
        Some(Ok(snapshot)) => Some(snapshot),
        Some(Err(error)) => {
            errors.push(format!(
                "official: failed to fetch DeepSeek limits ({error})"
            ));
            None
        }
        None => None,
    };

    let openrouter = match openrouter_result {
        Some(Ok(snapshot)) => Some(snapshot),
        Some(Err(error)) => {
            errors.push(format!(
                "official: failed to fetch OpenRouter limits ({error})"
            ));
            None
        }
        None => None,
    };

    let grok = match grok_result {
        Some(Ok(snapshot)) => Some(snapshot),
        Some(Err(error)) => {
            errors.push(format!("official: failed to fetch Grok limits ({error})"));
            None
        }
        None => None,
    };

    let kimi = match kimi_result {
        Some(Ok(snapshot)) => Some(snapshot),
        Some(Err(error)) => {
            errors.push(format!("official: failed to fetch Kimi limits ({error})"));
            None
        }
        None => None,
    };

    let anthropic = match anthropic_result {
        Some(Ok(snapshot)) => Some(snapshot),
        Some(Err(error)) => {
            errors.push(format!(
                "official: failed to fetch Anthropic API limits ({error})"
            ));
            None
        }
        None => None,
    };

    (
        codex,
        claude,
        antigravity,
        deepseek,
        openrouter,
        grok,
        kimi,
        anthropic,
        errors,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_blocks_live(
    args: &BlocksArgs,
    tz: &TimeZoneMode,
    window_secs: i64,
    token_limit_mode: Option<TokenLimitMode>,
    mut official_codex: Option<OfficialCodexSnapshot>,
    mut official_claude: Option<OfficialClaudeSnapshot>,
    mut official_antigravity: Option<OfficialAntigravitySnapshot>,
    mut official_deepseek: Option<OfficialDeepSeekSnapshot>,
    mut official_openrouter: Option<OfficialOpenRouterSnapshot>,
    mut official_grok: Option<OfficialGrokSnapshot>,
    mut official_kimi: Option<OfficialKimiSnapshot>,
    mut official_anthropic_api: Option<OfficialAnthropicApiSnapshot>,
) -> Result<()> {
    if !std::io::stdout().is_terminal() {
        bail!("--live requires an interactive terminal");
    }

    let refresh_every = args.refresh_interval.max(1);
    let mut session = BlocksLiveSession::enter()?;
    let mut active_tab = LiveTab::Overview;

    // Render an instant first frame using cached data from the previous
    // session so the TUI shows real-looking values immediately.  Falls back
    // to zeros if no cache exists.
    {
        let now = Utc::now();
        let block_start_unix = now.timestamp() - window_secs;
        let block_end_unix = now.timestamp();
        let cached = load_live_frame_cache();
        let today_str = tz.now_date().to_string();
        // Only use cached today_totals if the date matches; otherwise zero.
        let (cached_today, cached_30d, cached_30d_days, cached_today_activity, cached_30d_activity) =
            match cached.as_ref() {
                Some(c) if c.cached_date == today_str => (
                    c.today_totals.clone(),
                    c.last_30d_totals.clone(),
                    c.last_30d_active_days,
                    c.today_activity.clone(),
                    c.last_30d_activity.clone(),
                ),
                Some(c) => (
                    TokenCounts::default(),
                    c.last_30d_totals.clone(),
                    c.last_30d_active_days,
                    None,
                    c.last_30d_activity.clone(),
                ),
                None => (
                    TokenCounts::default(),
                    TokenCounts::default(),
                    0,
                    None,
                    None,
                ),
            };
        // Use cached official snapshots if the caller didn't provide them.
        if official_codex.is_none() {
            if let Some(ref c) = cached {
                official_codex = c.official_codex.clone();
            }
        }
        if official_claude.is_none() {
            if let Some(ref c) = cached {
                official_claude = c.official_claude.clone();
            }
        }
        if official_antigravity.is_none() {
            if let Some(ref c) = cached {
                official_antigravity = c.official_antigravity.clone();
            }
        }
        if official_deepseek.is_none() {
            if let Some(ref c) = cached {
                official_deepseek = c.official_deepseek.clone();
            }
        }
        if official_openrouter.is_none() {
            if let Some(ref c) = cached {
                official_openrouter = c.official_openrouter.clone();
            }
        }
        if official_grok.is_none() {
            if let Some(ref c) = cached {
                official_grok = c.official_grok.clone();
            }
        }
        if official_kimi.is_none() {
            if let Some(ref c) = cached {
                official_kimi = c.official_kimi.clone();
            }
        }
        if official_anthropic_api.is_none() {
            if let Some(ref c) = cached {
                official_anthropic_api = c.official_anthropic_api.clone();
            }
        }
        let skeleton = LiveFrameContext::new(
            now,
            tz,
            block_start_unix,
            block_end_unix,
            LimitDisplayContext {
                token_limit: None,
                token_limit_source: TokenLimitSource::Unset,
                membership_estimate: None,
            },
            official_codex.as_ref(),
            official_claude.as_ref(),
            official_antigravity.as_ref(),
            official_deepseek.as_ref(),
            official_openrouter.as_ref(),
            official_grok.as_ref(),
            official_kimi.as_ref(),
            official_anthropic_api.as_ref(),
            None,
            cached_today,
            cached_30d,
            cached_30d_days,
            if activity_enabled(&args.common) {
                cached_today_activity
            } else {
                None
            },
            if activity_enabled(&args.common) {
                cached_30d_activity
            } else {
                None
            },
            None,
            active_tab,
        );
        render_blocks_live_frame(&mut session, &skeleton)?;
    }

    // Spawn runtime initialisation in the background so the event loop
    // starts immediately and the cached frame stays responsive.
    let common_for_init = args.common.clone();
    let mut pending_runtime_task: Option<tokio::task::JoinHandle<Result<LiveUsageRuntime>>> =
        Some(tokio::spawn(async move {
            LiveUsageRuntime::new(&common_for_init, refresh_every, true).await
        }));
    let mut live_runtime: Option<LiveUsageRuntime> = None;

    let mut last_official_refresh = Instant::now();
    #[allow(unused_assignments)]
    let mut last_data_refresh = Instant::now();
    let mut first_real_frame_done = false;
    #[allow(clippy::type_complexity)]
    let mut pending_official_task: Option<
        tokio::task::JoinHandle<(
            Option<OfficialCodexSnapshot>,
            Option<OfficialClaudeSnapshot>,
            Option<OfficialAntigravitySnapshot>,
            Option<OfficialDeepSeekSnapshot>,
            Option<OfficialOpenRouterSnapshot>,
            Option<OfficialGrokSnapshot>,
            Option<OfficialKimiSnapshot>,
            Option<OfficialAnthropicApiSnapshot>,
            Vec<String>,
        )>,
    > = None;
    let mut activity_dataset: Option<ActivityDataset> = None;
    let mut last_activity_refresh = Instant::now() - Duration::from_secs(3600);
    let activity_refresh_interval = Duration::from_secs(60);
    // Cooldown for official limits refresh.  Starts at 30s, doubles on
    // failure (caps at 5 minutes) so we don't hammer Claude's /usage
    // endpoint when it is rate-limited or broken.
    let mut official_refresh_interval = Duration::from_secs(30);
    let official_refresh_interval_base = Duration::from_secs(30);
    let official_refresh_interval_max = Duration::from_secs(300);

    loop {
        let now = Utc::now();

        // Check if background runtime init has completed.
        if live_runtime.is_none() {
            if let Some(ref task) = pending_runtime_task {
                if task.is_finished() {
                    if let Some(task) = pending_runtime_task.take() {
                        match task.await {
                            Ok(Ok(rt)) => live_runtime = Some(rt),
                            Ok(Err(e)) => return Err(e),
                            Err(e) => return Err(anyhow::anyhow!("runtime init task failed: {e}")),
                        }
                    }
                }
            }
        }

        // If runtime isn't ready yet, just handle input on the cached frame.
        let Some(ref mut live_rt) = live_runtime else {
            last_data_refresh = Instant::now();
            loop {
                let poll_for = Duration::from_millis(50);
                match poll_live_input(poll_for, active_tab)? {
                    LiveInputEvent::Exit => {
                        if let Some(task) = pending_runtime_task.take() {
                            task.abort();
                        }
                        return Ok(());
                    }
                    LiveInputEvent::SwitchTab(_tab) => {
                        // Can't re-render with different source data yet,
                        // but we still accept the input so it takes effect
                        // once the runtime is ready.
                        active_tab = _tab;
                    }
                    LiveInputEvent::Tick => {}
                }
                // Break out to re-check runtime readiness.
                if last_data_refresh.elapsed() >= Duration::from_millis(100) {
                    break;
                }
            }
            continue;
        };

        // After the first (fast, Codex-only) frame has been rendered and
        // shown to the user, merge deferred Claude files.  This blocks but
        // the user already has a visible TUI with Codex data.
        if first_real_frame_done && live_rt.has_deferred_claude() {
            live_rt.merge_deferred_claude();
        }

        live_rt.maybe_refresh_sources(&args.common).await?;

        let source_hint = select_live_source(
            &args.common,
            None,
            official_codex.as_ref(),
            official_claude.as_ref(),
        );
        let (block_start_unix, block_end_unix, live_window_secs) = resolve_live_block_bounds(
            now,
            window_secs,
            source_hint,
            official_codex.as_ref(),
            official_claude.as_ref(),
        );

        // Non-blocking official limits fetch with exponential backoff on failure.
        let should_refresh_official = args.official_limits
            && pending_official_task.is_none()
            && (official_codex.is_none()
                || official_claude.is_none()
                || official_antigravity.is_none()
                || official_deepseek.is_none()
                || official_openrouter.is_none()
                || official_grok.is_none()
                || official_kimi.is_none()
                || official_anthropic_api.is_none()
                || last_official_refresh.elapsed() >= official_refresh_interval);
        if should_refresh_official {
            let common = args.common.clone();
            pending_official_task = Some(tokio::spawn(async move {
                fetch_selected_official_limits(&common).await
            }));
        }

        // Check if the background task has completed (non-blocking).
        if let Some(ref task) = pending_official_task {
            if task.is_finished() {
                if let Some(task) = pending_official_task.take() {
                    if let Ok((
                        codex,
                        claude,
                        antigravity,
                        deepseek,
                        openrouter,
                        grok,
                        kimi,
                        anthropic,
                        errors,
                    )) = task.await
                    {
                        let any_new = codex.is_some()
                            || claude.is_some()
                            || antigravity.is_some()
                            || deepseek.is_some()
                            || openrouter.is_some()
                            || grok.is_some()
                            || kimi.is_some()
                            || anthropic.is_some();
                        if codex.is_some() {
                            official_codex = codex;
                        }
                        if claude.is_some() {
                            official_claude = claude;
                        }
                        if antigravity.is_some() {
                            official_antigravity = antigravity;
                        }
                        if deepseek.is_some() {
                            official_deepseek = deepseek;
                        }
                        if openrouter.is_some() {
                            official_openrouter = openrouter;
                        }
                        if grok.is_some() {
                            official_grok = grok;
                        }
                        if kimi.is_some() {
                            official_kimi = kimi;
                        }
                        if anthropic.is_some() {
                            official_anthropic_api = anthropic;
                        }
                        last_official_refresh = Instant::now();

                        // Adjust cooldown: reset on success, backoff on failure.
                        if any_new || errors.is_empty() {
                            official_refresh_interval = official_refresh_interval_base;
                        } else {
                            official_refresh_interval =
                                (official_refresh_interval * 2).min(official_refresh_interval_max);
                        }
                    }
                }
            }
        }

        let loaded = live_rt.load(tz);
        let activity_today = local_date(now, tz);
        if activity_enabled(&args.common)
            && (activity_dataset.is_none()
                || last_activity_refresh.elapsed() >= activity_refresh_interval)
        {
            activity_dataset =
                fetch_activity_dataset(&args.common, tz, &loaded.events, None).await?;
            last_activity_refresh = Instant::now();
        }
        let membership_estimate =
            estimate_membership_from_logs(&loaded.events, now, live_window_secs);
        let inferred_limit = membership_estimate
            .as_ref()
            .map(|estimate| estimate.estimated_window_tokens);
        let resolved_from_mode =
            resolve_token_limit(token_limit_mode, &loaded.events, now, live_window_secs);
        let token_limit = resolved_from_mode.or(inferred_limit);
        let token_limit_source =
            resolve_token_limit_source(token_limit_mode, resolved_from_mode, inferred_limit);
        let active =
            active_block_summary_for_bounds(&loaded.events, now, block_start_unix, block_end_unix);
        let selected_source = select_live_source(
            &args.common,
            active.as_ref(),
            official_codex.as_ref(),
            official_claude.as_ref(),
        );
        let (today_totals, last_30d_totals, last_30d_active_days) =
            aggregate_recent_costs(&loaded.events, now, tz, selected_source);
        let (today_activity, last_30d_activity) = if activity_enabled(&args.common) {
            summarize_live_activity(activity_dataset.as_ref(), activity_today)
        } else {
            (None, None)
        };
        let mut frame_context = LiveFrameContext::new(
            now,
            tz,
            block_start_unix,
            block_end_unix,
            LimitDisplayContext {
                token_limit,
                token_limit_source,
                membership_estimate: membership_estimate.as_ref(),
            },
            official_codex.as_ref(),
            official_claude.as_ref(),
            official_antigravity.as_ref(),
            official_deepseek.as_ref(),
            official_openrouter.as_ref(),
            official_grok.as_ref(),
            official_kimi.as_ref(),
            official_anthropic_api.as_ref(),
            selected_source,
            today_totals,
            last_30d_totals,
            last_30d_active_days,
            today_activity,
            last_30d_activity,
            active.as_ref(),
            active_tab,
        );

        frame_context.active_tab = active_tab;
        render_blocks_live_frame(&mut session, &frame_context)?;
        last_data_refresh = Instant::now();
        first_real_frame_done = true;

        // Persist frame data so the next `tu live` startup is instant.
        save_live_frame_cache(&LiveFrameCache {
            cached_at_unix: unix_now_secs(),
            cached_date: tz.now_date().to_string(),
            today_totals: frame_context.today_totals.clone(),
            last_30d_totals: frame_context.last_30d_totals.clone(),
            last_30d_active_days: frame_context.last_30d_active_days,
            today_activity: frame_context.today_activity.clone(),
            last_30d_activity: frame_context.last_30d_activity.clone(),
            official_codex: official_codex.clone(),
            official_claude: official_claude.clone(),
            official_antigravity: official_antigravity.clone(),
            official_deepseek: official_deepseek.clone(),
            official_openrouter: official_openrouter.clone(),
            official_grok: official_grok.clone(),
            official_kimi: official_kimi.clone(),
            official_anthropic_api: official_anthropic_api.clone(),
        });

        // Fast inner loop: poll input at ~50ms intervals, re-render on tab switch
        // immediately. Only break out to refresh data when the refresh timer fires.
        loop {
            let until_refresh =
                Duration::from_secs(refresh_every).saturating_sub(last_data_refresh.elapsed());
            if until_refresh.is_zero() {
                break; // Time to refresh data
            }
            let poll_for = until_refresh.min(Duration::from_millis(50));
            match poll_live_input(poll_for, active_tab)? {
                LiveInputEvent::Exit => {
                    // Abort any pending background task before exiting.
                    if let Some(task) = pending_official_task.take() {
                        task.abort();
                    }
                    live_rt.flush_cache(true);
                    return Ok(());
                }
                LiveInputEvent::SwitchTab(tab) => {
                    active_tab = tab;
                    let mut ctx = frame_context.clone();
                    ctx.active_tab = active_tab;
                    render_blocks_live_frame(&mut session, &ctx)?;
                }
                LiveInputEvent::Tick => {
                    // Check non-blocking official task completion mid-frame.
                    if let Some(ref task) = pending_official_task {
                        if task.is_finished() {
                            if let Some(task) = pending_official_task.take() {
                                if let Ok((
                                    codex,
                                    claude,
                                    antigravity,
                                    deepseek,
                                    openrouter,
                                    grok,
                                    kimi,
                                    anthropic,
                                    errors,
                                )) = task.await
                                {
                                    let any_new = codex.is_some()
                                        || claude.is_some()
                                        || antigravity.is_some()
                                        || deepseek.is_some()
                                        || openrouter.is_some()
                                        || grok.is_some()
                                        || kimi.is_some()
                                        || anthropic.is_some();
                                    if codex.is_some() {
                                        official_codex = codex;
                                    }
                                    if claude.is_some() {
                                        official_claude = claude;
                                    }
                                    if antigravity.is_some() {
                                        official_antigravity = antigravity;
                                    }
                                    if deepseek.is_some() {
                                        official_deepseek = deepseek;
                                    }
                                    if openrouter.is_some() {
                                        official_openrouter = openrouter;
                                    }
                                    if grok.is_some() {
                                        official_grok = grok;
                                    }
                                    if kimi.is_some() {
                                        official_kimi = kimi;
                                    }
                                    if anthropic.is_some() {
                                        official_anthropic_api = anthropic;
                                    }
                                    last_official_refresh = Instant::now();
                                    if any_new || errors.is_empty() {
                                        official_refresh_interval = official_refresh_interval_base;
                                    } else {
                                        official_refresh_interval = (official_refresh_interval * 2)
                                            .min(official_refresh_interval_max);
                                    }
                                }
                            }
                            // Official data changed — force an immediate data refresh.
                            break;
                        }
                    }
                }
            }
        }
    }
}

pub(super) fn select_live_source(
    common: &CommonArgs,
    active: Option<&ActiveBlockSummary>,
    official_codex: Option<&OfficialCodexSnapshot>,
    official_claude: Option<&OfficialClaudeSnapshot>,
) -> Option<SourceKind> {
    if common.no_codex && !common.no_claude {
        return Some(SourceKind::Claude);
    }
    if common.no_claude && !common.no_codex {
        return Some(SourceKind::Codex);
    }
    if common.no_claude && common.no_codex && !common.no_gemini && common.no_opencode {
        return Some(SourceKind::Gemini);
    }
    if common.no_claude && common.no_codex && common.no_gemini && !common.no_opencode {
        return Some(SourceKind::OpenCode);
    }
    if let Some(source) = active.and_then(|v| v.dominant_source) {
        return Some(source);
    }
    // When both sources exist, return None to show combined data.
    // Only pick a single source when it's the sole provider.
    match (official_codex.is_some(), official_claude.is_some()) {
        (true, false) => Some(SourceKind::Codex),
        (false, true) => Some(SourceKind::Claude),
        _ => None,
    }
}

pub(super) fn resolve_live_block_bounds(
    now: DateTime<Utc>,
    default_window_secs: i64,
    source_hint: Option<SourceKind>,
    official_codex: Option<&OfficialCodexSnapshot>,
    official_claude: Option<&OfficialClaudeSnapshot>,
) -> (i64, i64, i64) {
    let fallback = {
        let now_unix = now.timestamp();
        let start = now_unix - now_unix.rem_euclid(default_window_secs.max(1));
        let end = start + default_window_secs.max(1);
        (start, end, default_window_secs.max(1))
    };

    let (reset_at, window_secs) = match source_hint {
        Some(SourceKind::Codex) => {
            let Some(snapshot) = official_codex else {
                return fallback;
            };
            let reset = snapshot.primary_resets_at;
            let window = snapshot
                .primary_window_mins
                .map(|mins| mins.saturating_mul(60))
                .unwrap_or(default_window_secs);
            (reset, window)
        }
        Some(SourceKind::Claude) => {
            let Some(snapshot) = official_claude else {
                return fallback;
            };
            let reset = snapshot.primary_resets_at;
            let window = snapshot
                .primary_window_mins
                .map(|mins| mins.saturating_mul(60))
                .unwrap_or(default_window_secs);
            (reset, window)
        }
        Some(SourceKind::Gemini) | Some(SourceKind::OpenCode) | Some(SourceKind::Grok) => {
            return fallback;
        }
        None => return fallback,
    };

    let Some(mut end_unix) = reset_at else {
        return fallback;
    };
    let window_secs = window_secs.max(1);
    let now_unix = now.timestamp();

    if end_unix <= now_unix {
        let steps = (now_unix - end_unix).div_euclid(window_secs) + 1;
        end_unix = end_unix.saturating_add(steps.saturating_mul(window_secs));
    } else if end_unix - now_unix > window_secs {
        let steps = (end_unix - now_unix - 1).div_euclid(window_secs);
        end_unix = end_unix.saturating_sub(steps.saturating_mul(window_secs));
    }

    let start_unix = end_unix.saturating_sub(window_secs);
    if now_unix < start_unix || now_unix >= end_unix {
        return fallback;
    }
    (start_unix, end_unix, window_secs)
}

pub(super) fn aggregate_recent_costs(
    events: &[UsageEvent],
    now: DateTime<Utc>,
    tz: &TimeZoneMode,
    source: Option<SourceKind>,
) -> (TokenCounts, TokenCounts, u32) {
    let today = local_date(now, tz);
    let last_30d_start = today
        .checked_sub_signed(chrono::TimeDelta::days(29))
        .unwrap_or(today);
    let mut today_totals = TokenCounts::default();
    let mut last_30d_totals = TokenCounts::default();
    let mut active_days = HashSet::new();

    for event in events {
        if source.is_some_and(|selected| event.source != selected) {
            continue;
        }
        let day = local_date(event.timestamp, tz);
        let counts = event.usage.to_counts();
        if day == today {
            today_totals.add_assign(counts.clone());
        }
        if day >= last_30d_start && day <= today {
            if counts.total_tokens > 0 {
                active_days.insert(day);
            }
            last_30d_totals.add_assign(counts);
        }
    }

    (today_totals, last_30d_totals, active_days.len() as u32)
}

pub(super) fn summarize_live_activity(
    dataset: Option<&ActivityDataset>,
    today: NaiveDate,
) -> (Option<ActivitySummary>, Option<ActivitySummary>) {
    let Some(dataset) = dataset else {
        return (None, None);
    };
    let start = today
        .checked_sub_signed(chrono::TimeDelta::days(29))
        .unwrap_or(today);
    (
        dataset.summary_for_day(today),
        dataset.summary_for_range(start, today),
    )
}

pub(super) struct BlocksLiveSession {
    pub(super) terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl BlocksLiveSession {
    pub(super) fn enter() -> Result<Self> {
        // Install a panic hook that restores the terminal so a panic during
        // TUI mode doesn't leave the user's shell in raw/alternate-screen.
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            original_hook(info);
        }));

        enable_raw_mode()?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(out);
        let mut terminal = Terminal::new(backend)?;
        terminal.hide_cursor()?;
        Ok(Self { terminal })
    }
}

impl Drop for BlocksLiveSession {
    fn drop(&mut self) {
        let _ = self.terminal.show_cursor();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

pub(super) fn render_blocks_live_frame(
    session: &mut BlocksLiveSession,
    context: &LiveFrameContext<'_>,
) -> Result<()> {
    session
        .terminal
        .draw(|frame| draw_blocks_live_tui(frame, context))?;
    Ok(())
}

pub(super) fn draw_blocks_live_tui(frame: &mut ratatui::Frame<'_>, context: &LiveFrameContext<'_>) {
    let root = frame.area();
    let preferred_official = preferred_official_for_live(context);
    let progress_height: u16 = match context.active_tab {
        LiveTab::Overview => {
            // Count how many sources have data
            let n = [
                context.official_codex.is_some(),
                context.official_claude.is_some(),
                context.official_antigravity.is_some(),
            ]
            .iter()
            .filter(|&&v| v)
            .count() as u16;
            // 2 lines per source (label + bar), plus time bar (2 lines)
            2 + n * 2
        }
        LiveTab::Codex => {
            if context
                .official_codex
                .and_then(|s| s.secondary_used_percent)
                .is_some()
            {
                6
            } else {
                4
            }
        }
        LiveTab::Claude => {
            if context
                .official_claude
                .and_then(|s| s.secondary_used_percent)
                .is_some()
            {
                6
            } else {
                4
            }
        }
        LiveTab::Gemini | LiveTab::OpenCode => 4,
        LiveTab::Antigravity
        | LiveTab::DeepSeek
        | LiveTab::OpenRouter
        | LiveTab::Grok
        | LiveTab::Kimi
        | LiveTab::AnthropicApi => 0,
    };
    let tab_bar_height = 1u16;
    let info_height = 1u16;
    let [tab_area, info_area, progress_area, body_area] = Layout::vertical([
        Constraint::Length(tab_bar_height),
        Constraint::Length(info_height),
        Constraint::Length(progress_height),
        Constraint::Min(4),
    ])
    .margin(1)
    .areas(root);

    let mode_text = if preferred_official.is_some() {
        "official"
    } else {
        "estimated"
    };
    let plan_text = preferred_official
        .and_then(LiveOfficialRef::plan_type)
        .unwrap_or("unknown");

    // Tab bar at top
    render_live_tab_bar(frame, tab_area, context.active_tab);

    // Condensed info line below tabs
    let info_line = if root.width >= 100 {
        Line::from(vec![
            Span::styled(
                context.now_text.clone(),
                Style::default().fg(TuiColor::DarkGray),
            ),
            Span::styled("  |  ", Style::default().fg(TuiColor::DarkGray)),
            Span::styled(
                format!(
                    "block {} -> {}",
                    context.block_start_text, context.block_end_text
                ),
                Style::default().fg(TuiColor::DarkGray),
            ),
            Span::styled("  |  ", Style::default().fg(TuiColor::DarkGray)),
            Span::raw(format!("{mode_text} · plan {plan_text}")),
            Span::styled("  |  ", Style::default().fg(TuiColor::DarkGray)),
            Span::styled(
                "q/Esc exit · Tab/←→ switch",
                Style::default().fg(TuiColor::DarkGray),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                context.now_text.clone(),
                Style::default().fg(TuiColor::DarkGray),
            ),
            Span::styled("  |  ", Style::default().fg(TuiColor::DarkGray)),
            Span::raw(format!("{mode_text} · {plan_text}")),
            Span::styled("  |  ", Style::default().fg(TuiColor::DarkGray)),
            Span::styled("q · Tab switch", Style::default().fg(TuiColor::DarkGray)),
        ])
    };
    frame.render_widget(Paragraph::new(info_line), info_area);

    match context.active_tab {
        LiveTab::Overview => {
            render_live_overview_bars(frame, progress_area, context);
            render_live_overview_body(frame, body_area, context);
        }
        LiveTab::Codex => {
            render_live_progress_bars_for(frame, progress_area, context, Some(SourceKind::Codex));
            render_live_source_detail(frame, body_area, context, SourceKind::Codex);
        }
        LiveTab::Claude => {
            render_live_progress_bars_for(frame, progress_area, context, Some(SourceKind::Claude));
            render_live_source_detail(frame, body_area, context, SourceKind::Claude);
        }
        LiveTab::Gemini => {
            render_live_progress_bars_for(frame, progress_area, context, Some(SourceKind::Gemini));
            render_live_source_detail(frame, body_area, context, SourceKind::Gemini);
        }
        LiveTab::OpenCode => {
            render_live_progress_bars_for(
                frame,
                progress_area,
                context,
                Some(SourceKind::OpenCode),
            );
            render_live_source_detail(frame, body_area, context, SourceKind::OpenCode);
        }
        LiveTab::Antigravity => {
            render_live_antigravity_tab(frame, body_area, context);
        }
        LiveTab::DeepSeek => {
            render_live_deepseek_tab(frame, body_area, context);
        }
        LiveTab::OpenRouter => {
            render_live_openrouter_tab(frame, body_area, context);
        }
        LiveTab::Grok => {
            render_live_grok_tab(frame, body_area, context);
        }
        LiveTab::Kimi => {
            render_live_kimi_tab(frame, body_area, context);
        }
        LiveTab::AnthropicApi => {
            render_live_anthropic_api_tab(frame, body_area, context);
        }
    }
}

pub(super) fn render_live_tab_bar(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    active: LiveTab,
) {
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(
            "tu live",
            Style::default()
                .fg(TuiColor::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
    ];
    for (i, tab) in ALL_LIVE_TABS.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        let label = format!(" {} ", tab.label());
        if *tab == active {
            spans.push(Span::styled(
                label,
                Style::default()
                    .fg(TuiColor::Black)
                    .bg(TuiColor::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(label, Style::default().fg(TuiColor::DarkGray)));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub(super) fn render_live_antigravity_tab(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    let Some(ag) = context.official_antigravity else {
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "Antigravity language server not detected",
                Style::default()
                    .fg(TuiColor::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from("Make sure Antigravity IDE is running with its language server."),
        ])
        .wrap(Wrap { trim: true });
        frame.render_widget(msg, area);
        return;
    };

    let mut lines: Vec<Line<'static>> = Vec::new();

    let plan = ag.plan_type.as_deref().unwrap_or("unknown");
    let email = ag.account_email.as_deref().unwrap_or("—");
    lines.push(Line::from(vec![
        Span::styled(
            "Antigravity",
            Style::default()
                .fg(TuiColor::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  plan {plan}  |  {email}")),
    ]));
    lines.push(Line::from(""));

    let now = Utc::now();
    let ordered = select_antigravity_models(&ag.models);
    let shown: HashSet<String> = ordered.iter().map(|m| m.label.clone()).collect();
    let rest: Vec<_> = ag
        .models
        .iter()
        .filter(|m| !shown.contains(&m.label))
        .collect();

    // Draw each model with a gauge-style bar
    for model in ordered.iter().chain(rest) {
        lines.push(Line::from(vec![Span::styled(
            format!("  {}", model.label),
            Style::default().add_modifier(Modifier::BOLD),
        )]));

        if let Some(frac) = model.remaining_fraction {
            let remaining_pct = frac * 100.0;
            let used_pct = 100.0 - remaining_pct;
            let color = if remaining_pct >= 40.0 {
                TuiColor::Green
            } else if remaining_pct >= 15.0 {
                TuiColor::Yellow
            } else {
                TuiColor::Red
            };

            let bar_width = (area.width as usize).saturating_sub(6).min(60);
            let filled = ((used_pct / 100.0) * bar_width as f64).round() as usize;
            let empty = bar_width.saturating_sub(filled);
            let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty),);

            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(bar, Style::default().fg(color)),
                Span::styled(
                    format!(" {remaining_pct:.0}% left"),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ]));

            if let Some(reset_ts) = model.reset_time {
                let eta = format_time_until_reset_short(reset_ts, now);
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("resets in {eta}"),
                        Style::default().fg(TuiColor::DarkGray),
                    ),
                ]));
            }
        } else {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    "quota not reported",
                    Style::default().fg(TuiColor::DarkGray),
                ),
            ]));
        }
        lines.push(Line::from(""));
    }

    let body = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(body, area);
}

pub(super) fn render_live_overview_bars(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    let mut constraints: Vec<Constraint> = Vec::new();
    // Time bar: label + gauge
    constraints.push(Constraint::Length(1));
    constraints.push(Constraint::Length(1));

    struct SourceBar {
        label: String,
        used: f64,
        reset_text: Option<String>,
    }
    let mut source_bars: Vec<SourceBar> = Vec::new();

    if let Some(codex) = context.official_codex {
        if let Some(used) = codex.primary_used_percent {
            let reset = codex
                .primary_resets_at
                .map(|r| format_time_until_reset_short(r, context.now));
            source_bars.push(SourceBar {
                label: "Codex".to_string(),
                used,
                reset_text: reset,
            });
            constraints.push(Constraint::Length(1));
            constraints.push(Constraint::Length(1));
        }
    }
    if let Some(claude) = context.official_claude {
        if let Some(used) = claude.primary_used_percent {
            let reset = claude
                .primary_resets_at
                .map(|r| format_time_until_reset_short(r, context.now));
            source_bars.push(SourceBar {
                label: "Claude".to_string(),
                used,
                reset_text: reset,
            });
            constraints.push(Constraint::Length(1));
            constraints.push(Constraint::Length(1));
        }
    }
    if let Some(ag) = context.official_antigravity {
        if let Some(used) = ag.primary_used_percent {
            let reset = ag
                .primary_resets_at
                .map(|r| format_time_until_reset_short(r, context.now));
            source_bars.push(SourceBar {
                label: ag
                    .primary_label
                    .as_deref()
                    .unwrap_or("Antigravity")
                    .to_string(),
                used,
                reset_text: reset,
            });
            constraints.push(Constraint::Length(1));
            constraints.push(Constraint::Length(1));
        }
    }

    let rows = Layout::vertical(constraints).split(area);

    // Time bar
    let time_ratio = if context.window_secs > 0 {
        (context.elapsed_secs as f64 / context.window_secs as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let time_title = Paragraph::new(Line::from(vec![
        Span::styled("Time ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(
            "{} / {}",
            format_hours_minutes(context.elapsed_secs / 60),
            format_hours_minutes(context.window_secs / 60)
        )),
    ]));
    frame.render_widget(time_title, rows[0]);
    let time_gauge = Gauge::default()
        .style(live_gauge_track_style())
        .gauge_style(
            Style::default()
                .fg(TuiColor::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(time_ratio)
        .label(format!("{:.1}%", time_ratio * 100.0));
    frame.render_widget(time_gauge, rows[1]);

    // Source bars
    for (i, bar) in source_bars.iter().enumerate() {
        let row_base = 2 + i * 2;
        let title = Paragraph::new(Line::from(vec![Span::styled(
            &bar.label,
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        frame.render_widget(title, rows[row_base]);

        let mut label = format!("{:.1}% used", bar.used);
        if let Some(reset) = &bar.reset_text {
            label.push_str(&format!(" | resets in {reset}"));
        }
        let gauge = Gauge::default()
            .style(live_gauge_track_style())
            .gauge_style(
                Style::default()
                    .fg(used_gauge_color(bar.used))
                    .add_modifier(Modifier::BOLD),
            )
            .ratio((bar.used / 100.0).clamp(0.0, 1.0))
            .label(label);
        frame.render_widget(gauge, rows[row_base + 1]);
    }
}

pub(super) fn render_live_overview_body(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Today summary
    lines.push(Line::from(vec![Span::styled(
        "Today (all sources)",
        Style::default()
            .fg(TuiColor::Cyan)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(vec![
        Span::raw(live_key_label("Tokens")),
        Span::styled(
            format_u64(context.today_totals.total_tokens),
            Style::default()
                .fg(TuiColor::LightCyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw(live_key_label("Cost")),
        Span::styled(
            format_usd(context.today_totals.cost_usd),
            Style::default()
                .fg(TuiColor::Green)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // 30-day avg
    if context.last_30d_active_days > 0 {
        let avg_cost = context.last_30d_totals.cost_usd / context.last_30d_active_days as f64;
        let avg_tokens = context.last_30d_totals.total_tokens / context.last_30d_active_days as u64;
        lines.push(Line::from(vec![
            Span::raw(live_key_label("30d avg/day")),
            Span::raw(format!(
                "{} · {} tokens",
                format_usd(avg_cost),
                format_u64(avg_tokens)
            )),
        ]));
    }

    lines.push(Line::from(""));

    // Per-source summaries
    if let Some(codex) = context.official_codex {
        let plan = codex.plan_type.as_deref().unwrap_or("?");
        lines.push(Line::from(vec![Span::styled(
            format!("Codex  (plan {plan})"),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        if let Some(primary) = codex.primary_used_percent {
            lines.push(live_key_value_line(
                "  Session",
                format!("{primary:.1}% used"),
                used_gauge_color(primary),
            ));
        }
        if let Some(weekly) = codex.secondary_used_percent {
            lines.push(live_key_value_line(
                "  Weekly",
                format!("{weekly:.1}% used"),
                used_gauge_color(weekly),
            ));
        }
        lines.push(Line::from(""));
    }

    if let Some(claude) = context.official_claude {
        let plan = claude.plan_type.as_deref().unwrap_or("?");
        lines.push(Line::from(vec![Span::styled(
            format!("Claude  (plan {plan})"),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        if let Some(primary) = claude.primary_used_percent {
            lines.push(live_key_value_line(
                "  Session",
                format!("{primary:.1}% used"),
                used_gauge_color(primary),
            ));
        }
        if let Some(weekly) = claude.secondary_used_percent {
            lines.push(live_key_value_line(
                "  Weekly",
                format!("{weekly:.1}% used"),
                used_gauge_color(weekly),
            ));
        }
        lines.push(Line::from(""));
    }

    if let Some(ag) = context.official_antigravity {
        let plan = ag.plan_type.as_deref().unwrap_or("?");
        lines.push(Line::from(vec![Span::styled(
            format!("Antigravity  (plan {plan})"),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        let ordered = select_antigravity_models(&ag.models);
        for model in ordered.iter().take(3) {
            if let Some(frac) = model.remaining_fraction {
                let remaining = frac * 100.0;
                let color = if remaining >= 40.0 {
                    TuiColor::Green
                } else if remaining >= 15.0 {
                    TuiColor::Yellow
                } else {
                    TuiColor::Red
                };
                lines.push(live_key_value_line(
                    format!("  {}", model.label),
                    format!("{remaining:.0}% left"),
                    color,
                ));
            }
        }
        lines.push(Line::from(""));
    }

    if context.official_codex.is_none()
        && context.official_claude.is_none()
        && context.official_antigravity.is_none()
    {
        lines.push(Line::from(vec![Span::styled(
            "No official limits available",
            Style::default().fg(TuiColor::Yellow),
        )]));
    }

    let body = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(body, area);
}

pub(super) fn render_live_source_detail(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
    source: SourceKind,
) {
    let has_official =
        source.supports_official_limits() && official_for_source(context, source).is_some();

    if !has_official {
        let label = source.as_str();
        let lower = label.to_lowercase();
        let mut lines = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                format!("{label} official limits not available"),
                Style::default()
                    .fg(TuiColor::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
        ];
        match source {
            SourceKind::Claude => {
                lines.push(Line::from(
                    "Fetching via Claude CLI probe... (may take ~15s on first load)",
                ));
                lines.push(Line::from(
                    "If this persists, ensure `claude` CLI is installed and accessible.",
                ));
            }
            SourceKind::Codex => {
                lines.push(Line::from(format!(
                    "Run `tu live {lower}` after using {label} to fetch limits.",
                )));
            }
            SourceKind::Gemini | SourceKind::OpenCode | SourceKind::Grok => {
                lines.push(Line::from(
                    "Official limits not implemented for this source.",
                ));
            }
        }

        // Still show today's usage from logs below the warning
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Usage from logs",
            Style::default()
                .fg(TuiColor::Cyan)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(vec![
            Span::raw(live_key_label("Today tokens")),
            Span::styled(
                format_u64(context.today_totals.total_tokens),
                Style::default()
                    .fg(TuiColor::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw(live_key_label("Today cost")),
            Span::styled(
                format_usd(context.today_totals.cost_usd),
                Style::default()
                    .fg(TuiColor::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        let msg = Paragraph::new(lines).wrap(Wrap { trim: true });
        frame.render_widget(msg, area);
        return;
    }

    render_live_body(frame, area, context);
}

pub(super) fn render_live_progress_bars_for(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
    source_override: Option<SourceKind>,
) {
    let preferred_official = match source_override {
        Some(source) => official_for_source(context, source),
        None => preferred_official_for_live(context),
    };
    let show_weekly = preferred_official
        .and_then(LiveOfficialRef::secondary_used_percent)
        .is_some();
    let constraints = if show_weekly {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]
    } else {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]
    };
    let rows = Layout::vertical(constraints).split(area);
    let time_label_area = rows[0];
    let time_area = rows[1];

    let time_ratio = if context.window_secs > 0 {
        (context.elapsed_secs as f64 / context.window_secs as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let time_pct = time_ratio * 100.0;
    let time_label = if area.width >= 96 {
        format!(
            "{} / {} ({time_pct:.1}%)",
            format_hours_minutes(context.elapsed_secs / 60),
            format_hours_minutes(context.window_secs / 60)
        )
    } else {
        format!("{time_pct:.1}%")
    };
    let time_title = Paragraph::new(Line::from(vec![
        Span::styled("Time ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(
            "{} / {}",
            format_hours_minutes(context.elapsed_secs / 60),
            format_hours_minutes(context.window_secs / 60)
        )),
    ]));
    frame.render_widget(time_title, time_label_area);
    let time_gauge = Gauge::default()
        .style(live_gauge_track_style())
        .gauge_style(
            Style::default()
                .fg(TuiColor::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(time_ratio)
        .label(time_label);
    frame.render_widget(time_gauge, time_area);

    if let Some(official) = preferred_official
        && let Some(primary_used) = official.primary_used_percent()
    {
        let primary_title = Paragraph::new(Line::from(vec![Span::styled(
            format!("Session ({})", official.provider_label()),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        frame.render_widget(primary_title, rows[2]);

        let mut primary_label = format!("{primary_used:.1}% used");
        if let Some(resets_at) = official.primary_resets_at() {
            let eta_text = format_time_until_reset_short(resets_at, Utc::now());
            let local_reset = format_reset_timestamp(resets_at, context.tz);
            primary_label.push_str(&format!(" | resets in {eta_text} ({local_reset})"));
        }
        let primary_gauge = Gauge::default()
            .style(live_gauge_track_style())
            .gauge_style(
                Style::default()
                    .fg(used_gauge_color(primary_used))
                    .add_modifier(Modifier::BOLD),
            )
            .ratio((primary_used / 100.0).clamp(0.0, 1.0))
            .label(primary_label);
        frame.render_widget(primary_gauge, rows[3]);

        if show_weekly && let Some(weekly_used) = official.secondary_used_percent() {
            let weekly_title = Paragraph::new(Line::from(vec![Span::styled(
                format!("Weekly ({})", official.provider_label()),
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            frame.render_widget(weekly_title, rows[4]);

            let mut weekly_label = format!("{weekly_used:.1}% used");
            if let Some(resets_at) = official.secondary_resets_at() {
                let eta_text = format_time_until_reset_short(resets_at, Utc::now());
                let local_reset = format_reset_timestamp(resets_at, context.tz);
                weekly_label.push_str(&format!(" | resets in {eta_text} ({local_reset})"));
            }
            let weekly_gauge = Gauge::default()
                .style(live_gauge_track_style())
                .gauge_style(
                    Style::default()
                        .fg(used_gauge_color(weekly_used))
                        .add_modifier(Modifier::BOLD),
                )
                .ratio((weekly_used / 100.0).clamp(0.0, 1.0))
                .label(weekly_label);
            frame.render_widget(weekly_gauge, rows[5]);
        }
        return;
    }

    let blended = blended_projection(context);
    let current_tokens = context
        .active
        .map(|active_block| active_block.totals.total_tokens)
        .unwrap_or(0);
    let projected_tokens = blended
        .map(|projection| projection.projected_tokens_end)
        .or_else(|| {
            context
                .active
                .map(|active_block| projected_end(active_block).0)
        })
        .unwrap_or(0);

    let (limit_ratio, limit_label, limit_color, promoted) = match context.limit.token_limit {
        Some(0) => (0.0, "disabled (0)".to_string(), TuiColor::DarkGray, false),
        Some(limit) => {
            let (effective_limit, promotions) = resolve_display_limit(
                limit,
                projected_tokens,
                context.limit.token_limit_source,
                context.limit.membership_estimate,
            );
            let current_pct = (current_tokens as f64 / effective_limit as f64) * 100.0;
            let projected_pct = (projected_tokens as f64 / effective_limit as f64) * 100.0;
            let (status, status_color) = limit_status(projected_pct);
            let ratio = (projected_pct / 100.0).clamp(0.0, 1.0);
            let limit_prefix = match context.limit.token_limit_source {
                TokenLimitSource::EstimatedFromLogs => "est limit",
                _ => "limit",
            };
            let promoted = !promotions.is_empty();
            let label = if area.width >= 120 && promoted {
                format!(
                    "{limit_prefix} {} (auto from {}) | current {:.1}% | projected {:.1}% ({status})",
                    format_u64(effective_limit),
                    format_u64(limit),
                    current_pct,
                    projected_pct
                )
            } else if area.width >= 120 {
                format!(
                    "{limit_prefix} {} | current {:.1}% | projected {:.1}% ({status})",
                    format_u64(effective_limit),
                    current_pct,
                    projected_pct
                )
            } else {
                format!(
                    "cur {:.1}% proj {:.1}% ({status})",
                    current_pct, projected_pct
                )
            };
            (ratio, label, status_color, promoted)
        }
        None => (
            0.0,
            "not set (--token-limit <n|max>)".to_string(),
            TuiColor::DarkGray,
            false,
        ),
    };
    let limit_title_text = match context.limit.token_limit_source {
        TokenLimitSource::EstimatedFromLogs if promoted => "Estimated limit (tiered)",
        TokenLimitSource::EstimatedFromLogs => "Estimated limit (from logs)",
        TokenLimitSource::HistoricalMax => "Limit (historical max)",
        TokenLimitSource::Explicit => "Limit (explicit)",
        TokenLimitSource::Unset => "Limit",
    };
    let limit_title = Paragraph::new(Line::from(vec![Span::styled(
        limit_title_text,
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    frame.render_widget(limit_title, rows[2]);

    let limit_gauge = Gauge::default()
        .style(live_gauge_track_style())
        .gauge_style(
            Style::default()
                .fg(limit_color)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(limit_ratio)
        .label(limit_label);
    frame.render_widget(limit_gauge, rows[3]);
}

#[derive(Clone, Copy)]
pub(super) enum LiveOfficialRef<'a> {
    Codex(&'a OfficialCodexSnapshot),
    Claude(&'a OfficialClaudeSnapshot),
    Antigravity(&'a OfficialAntigravitySnapshot),
}

impl<'a> LiveOfficialRef<'a> {
    pub(super) fn provider_label(self) -> &'static str {
        match self {
            LiveOfficialRef::Codex(_) => "Codex",
            LiveOfficialRef::Claude(_) => "Claude",
            LiveOfficialRef::Antigravity(_) => "Antigravity",
        }
    }

    pub(super) fn plan_type(self) -> Option<&'a str> {
        match self {
            LiveOfficialRef::Codex(snapshot) => snapshot.plan_type.as_deref(),
            LiveOfficialRef::Claude(snapshot) => snapshot.plan_type.as_deref(),
            LiveOfficialRef::Antigravity(snapshot) => snapshot.plan_type.as_deref(),
        }
    }

    pub(super) fn primary_used_percent(self) -> Option<f64> {
        match self {
            LiveOfficialRef::Codex(snapshot) => snapshot.primary_used_percent,
            LiveOfficialRef::Claude(snapshot) => snapshot.primary_used_percent,
            LiveOfficialRef::Antigravity(snapshot) => snapshot.primary_used_percent,
        }
    }

    pub(super) fn secondary_used_percent(self) -> Option<f64> {
        match self {
            LiveOfficialRef::Codex(snapshot) => snapshot.secondary_used_percent,
            LiveOfficialRef::Claude(snapshot) => snapshot.secondary_used_percent,
            LiveOfficialRef::Antigravity(snapshot) => snapshot.secondary_used_percent,
        }
    }

    pub(super) fn secondary_window_mins(self) -> Option<i64> {
        match self {
            LiveOfficialRef::Codex(snapshot) => snapshot.secondary_window_mins,
            LiveOfficialRef::Claude(snapshot) => snapshot.secondary_window_mins,
            LiveOfficialRef::Antigravity(_) => None,
        }
    }

    pub(super) fn primary_resets_at(self) -> Option<i64> {
        match self {
            LiveOfficialRef::Codex(snapshot) => snapshot.primary_resets_at,
            LiveOfficialRef::Claude(snapshot) => snapshot.primary_resets_at,
            LiveOfficialRef::Antigravity(snapshot) => snapshot.primary_resets_at,
        }
    }

    pub(super) fn secondary_resets_at(self) -> Option<i64> {
        match self {
            LiveOfficialRef::Codex(snapshot) => snapshot.secondary_resets_at,
            LiveOfficialRef::Claude(snapshot) => snapshot.secondary_resets_at,
            LiveOfficialRef::Antigravity(snapshot) => snapshot.secondary_resets_at,
        }
    }
}

pub(super) fn preferred_official_for_live<'a>(
    context: &'a LiveFrameContext<'a>,
) -> Option<LiveOfficialRef<'a>> {
    if let Some(source) = context.selected_source {
        return official_for_source(context, source);
    }

    // Antigravity is shown when it's the only available provider or alongside others
    if context.official_codex.is_none()
        && context.official_claude.is_none()
        && let Some(ag) = context.official_antigravity
    {
        return Some(LiveOfficialRef::Antigravity(ag));
    }

    match (context.official_codex, context.official_claude) {
        (Some(codex), None) => Some(LiveOfficialRef::Codex(codex)),
        (None, Some(claude)) => Some(LiveOfficialRef::Claude(claude)),
        (Some(codex), Some(claude)) => {
            match context.active.and_then(|active| active.dominant_source) {
                Some(SourceKind::Claude) => Some(LiveOfficialRef::Claude(claude)),
                Some(SourceKind::Codex) => Some(LiveOfficialRef::Codex(codex)),
                Some(SourceKind::Gemini)
                | Some(SourceKind::OpenCode)
                | Some(SourceKind::Grok)
                | None => Some(LiveOfficialRef::Codex(codex)),
            }
        }
        (None, None) => None,
    }
}

fn official_for_source<'a>(
    context: &'a LiveFrameContext<'a>,
    source: SourceKind,
) -> Option<LiveOfficialRef<'a>> {
    match source {
        SourceKind::Codex => context.official_codex.map(LiveOfficialRef::Codex),
        SourceKind::Claude => context.official_claude.map(LiveOfficialRef::Claude),
        SourceKind::Gemini | SourceKind::OpenCode | SourceKind::Grok => None,
    }
}

pub(super) fn used_gauge_color(used_percent: f64) -> TuiColor {
    if used_percent >= 85.0 {
        TuiColor::Red
    } else if used_percent >= 60.0 {
        TuiColor::Yellow
    } else {
        TuiColor::Green
    }
}

pub(super) fn live_gauge_track_style() -> Style {
    Style::default().bg(TuiColor::Rgb(42, 46, 64))
}

pub(super) fn render_live_body(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    if area.width >= 128 {
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)])
                .spacing(2)
                .areas(area);
        let left = Paragraph::new(live_current_lines(context)).wrap(Wrap { trim: true });
        let right = Paragraph::new(live_limit_lines(context)).wrap(Wrap { trim: true });
        frame.render_widget(left, left_area);
        frame.render_widget(right, right_area);
        return;
    }

    let mut lines = live_current_lines(context);
    lines.push(Line::from(""));
    lines.extend(live_limit_lines(context));
    let body = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(body, area);
}

#[derive(Clone, Copy)]
pub(super) struct TodayProjection {
    tokens_per_hour: f64,
    cost_per_hour: f64,
    projected_tokens_end_of_day: u64,
    projected_cost_end_of_day: f64,
}

pub(super) fn day_elapsed_seconds(now: DateTime<Utc>, tz: &TimeZoneMode) -> f64 {
    match tz {
        TimeZoneMode::Local => {
            let local = now.with_timezone(&Local);
            (i64::from(local.hour()) * 3600
                + i64::from(local.minute()) * 60
                + i64::from(local.second()))
            .max(1) as f64
        }
        TimeZoneMode::Utc => {
            (i64::from(now.hour()) * 3600 + i64::from(now.minute()) * 60 + i64::from(now.second()))
                .max(1) as f64
        }
        TimeZoneMode::Named(zone) => {
            let zoned = now.with_timezone(zone);
            (i64::from(zoned.hour()) * 3600
                + i64::from(zoned.minute()) * 60
                + i64::from(zoned.second()))
            .max(1) as f64
        }
    }
}

pub(super) fn day_progress_ratio(now: DateTime<Utc>, tz: &TimeZoneMode) -> f64 {
    (day_elapsed_seconds(now, tz) / (24.0 * 3600.0)).clamp(0.0, 1.0)
}

pub(super) const LIVE_KEY_COL_WIDTH: usize = 18;

pub(super) fn live_key_label(key: &str) -> String {
    format!("{:<width$}", format!("{key}:"), width = LIVE_KEY_COL_WIDTH)
}

pub(super) fn today_projection(context: &LiveFrameContext<'_>) -> Option<TodayProjection> {
    let elapsed_secs = day_elapsed_seconds(context.now, context.tz);
    if elapsed_secs < 10.0 * 60.0 {
        return None;
    }

    let tokens_per_sec = context.today_totals.total_tokens as f64 / elapsed_secs;
    let cost_per_sec = context.today_totals.cost_usd / elapsed_secs;
    let full_day_secs = 24.0 * 3600.0;
    Some(TodayProjection {
        tokens_per_hour: tokens_per_sec * 3600.0,
        cost_per_hour: cost_per_sec * 3600.0,
        projected_tokens_end_of_day: (tokens_per_sec * full_day_secs)
            .round()
            .max(context.today_totals.total_tokens as f64)
            as u64,
        projected_cost_end_of_day: (cost_per_sec * full_day_secs)
            .max(context.today_totals.cost_usd),
    })
}

#[derive(Clone, Copy)]
pub(super) struct BlendedProjection {
    tokens_per_minute: f64,
    cost_per_hour: f64,
    projected_tokens_end: u64,
    projected_cost_end: f64,
    short_weight: f64,
    today_weight: f64,
    long_weight: f64,
}

#[derive(Clone, Copy)]
pub(super) struct RateComponent {
    tokens_per_minute: f64,
    cost_per_minute: f64,
}

pub(super) fn blended_projection(context: &LiveFrameContext<'_>) -> Option<BlendedProjection> {
    let block_ratio = if context.window_secs > 0 {
        (context.elapsed_secs as f64 / context.window_secs as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let day_ratio = day_progress_ratio(context.now, context.tz);

    let short_component = context.active.and_then(|active| {
        active.burn.as_ref().map(|burn| RateComponent {
            tokens_per_minute: burn.tokens_per_minute.max(0.0),
            cost_per_minute: (burn.cost_per_hour / 60.0).max(0.0),
        })
    });
    let short_score = if short_component.is_some() {
        0.15 + 1.35 * block_ratio
    } else {
        0.0
    };

    let today_component = today_projection(context).map(|projection| RateComponent {
        tokens_per_minute: (projection.tokens_per_hour / 60.0).max(0.0),
        cost_per_minute: (projection.cost_per_hour / 60.0).max(0.0),
    });
    let today_score = if today_component.is_some() {
        0.25 + 0.85 * day_ratio.sqrt()
    } else {
        0.0
    };

    let active_days = context.last_30d_active_days.max(1) as f64;
    let long_tokens_per_day = context.last_30d_totals.total_tokens as f64 / active_days;
    let long_cost_per_day = context.last_30d_totals.cost_usd / active_days;
    let long_component = if long_tokens_per_day > 0.0 || long_cost_per_day > 0.0 {
        Some(RateComponent {
            tokens_per_minute: (long_tokens_per_day / 1440.0).max(0.0),
            cost_per_minute: (long_cost_per_day / 1440.0).max(0.0),
        })
    } else {
        None
    };
    let long_score = if long_component.is_some() {
        (1.20 - 0.95 * block_ratio).clamp(0.10, 2.0)
    } else {
        0.0
    };

    let total_score = short_score + today_score + long_score;
    if total_score <= f64::EPSILON {
        return None;
    }

    let short_weight = short_score / total_score;
    let today_weight = today_score / total_score;
    let long_weight = long_score / total_score;

    let (short_tokens, short_cost) = short_component
        .map(|component| (component.tokens_per_minute, component.cost_per_minute))
        .unwrap_or((0.0, 0.0));
    let (today_tokens, today_cost) = today_component
        .map(|component| (component.tokens_per_minute, component.cost_per_minute))
        .unwrap_or((0.0, 0.0));
    let (long_tokens, long_cost) = long_component
        .map(|component| (component.tokens_per_minute, component.cost_per_minute))
        .unwrap_or((0.0, 0.0));

    let blended_tokens_per_minute =
        short_tokens * short_weight + today_tokens * today_weight + long_tokens * long_weight;
    let blended_cost_per_minute =
        short_cost * short_weight + today_cost * today_weight + long_cost * long_weight;

    let (current_tokens, current_cost, remaining_minutes) = context
        .active
        .map(|active| {
            (
                active.totals.total_tokens,
                active.totals.cost_usd,
                active.remaining_minutes.max(0),
            )
        })
        .unwrap_or_else(|| {
            (
                0,
                0.0,
                ((context.window_secs - context.elapsed_secs).max(0) / 60),
            )
        });

    let projected_tokens_end = (current_tokens as f64
        + blended_tokens_per_minute * remaining_minutes as f64)
        .round()
        .max(current_tokens as f64) as u64;
    let projected_cost_end =
        (current_cost + blended_cost_per_minute * remaining_minutes as f64).max(current_cost);

    Some(BlendedProjection {
        tokens_per_minute: blended_tokens_per_minute,
        cost_per_hour: blended_cost_per_minute * 60.0,
        projected_tokens_end,
        projected_cost_end,
        short_weight,
        today_weight,
        long_weight,
    })
}

pub(super) fn live_key_value_line(
    key: impl AsRef<str>,
    value: impl Into<String>,
    value_color: TuiColor,
) -> Line<'static> {
    Line::from(vec![
        Span::raw(live_key_label(key.as_ref())),
        Span::styled(
            value.into(),
            Style::default()
                .fg(value_color)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

pub(super) fn activity_hourly_values(
    summary: &ActivitySummary,
    totals: &TokenCounts,
) -> Option<(u64, f64)> {
    if summary.total_seconds == 0 {
        return None;
    }
    let hours = summary.total_seconds as f64 / 3600.0;
    if hours <= f64::EPSILON {
        return None;
    }
    Some((
        (totals.total_tokens as f64 / hours).round().max(0.0) as u64,
        totals.cost_usd / hours,
    ))
}

pub(super) fn append_activity_lines(
    lines: &mut Vec<Line<'static>>,
    key_prefix: &str,
    summary: Option<&ActivitySummary>,
    totals: &TokenCounts,
) {
    let Some(summary) = summary else {
        return;
    };

    let label = if key_prefix.is_empty() {
        "Coding".to_string()
    } else {
        format!("{key_prefix} coding")
    };
    lines.push(live_key_value_line(
        label,
        summary.text.clone(),
        TuiColor::LightGreen,
    ));

    if let Some((tokens_per_hour, cost_per_hour)) = activity_hourly_values(summary, totals) {
        let rate_label = if key_prefix.is_empty() {
            "Tok / coding hr".to_string()
        } else {
            format!("{key_prefix} tok/hr")
        };
        lines.push(Line::from(vec![
            Span::raw(live_key_label(&rate_label)),
            Span::styled(
                format!(
                    "{} | {}/hr",
                    format_u64(tokens_per_hour),
                    format_usd(cost_per_hour)
                ),
                Style::default()
                    .fg(TuiColor::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    if key_prefix.is_empty() {
        if let Some(project) = summary.top_project.as_deref() {
            lines.push(live_key_value_line("Top project", project, TuiColor::White));
        }
        if let Some(language) = summary.top_language.as_deref() {
            lines.push(live_key_value_line("Top lang", language, TuiColor::Gray));
        }
    }
}

pub(super) fn live_current_lines(context: &LiveFrameContext<'_>) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![Span::styled(
        "Current",
        Style::default()
            .fg(TuiColor::Cyan)
            .add_modifier(Modifier::BOLD),
    )])];
    let source_label = context
        .selected_source
        .map(SourceKind::as_str)
        .unwrap_or("all");
    lines.push(live_key_value_line(
        "Source",
        source_label,
        TuiColor::LightCyan,
    ));

    let Some(active_block) = context.active else {
        lines.push(Line::from("No active usage in this 5h window yet."));
        lines.push(live_key_value_line(
            "Today",
            format!(
                "{} · {} tokens",
                format_usd(context.today_totals.cost_usd),
                format_u64(context.today_totals.total_tokens)
            ),
            TuiColor::Green,
        ));
        append_activity_lines(
            &mut lines,
            "",
            context.today_activity.as_ref(),
            &context.today_totals,
        );
        append_activity_lines(
            &mut lines,
            "30d",
            context.last_30d_activity.as_ref(),
            &context.last_30d_totals,
        );
        return lines;
    };

    lines.push(live_key_value_line(
        "5h now",
        format!(
            "{} tokens | {}",
            format_u64(active_block.totals.total_tokens),
            format_usd(active_block.totals.cost_usd)
        ),
        TuiColor::LightCyan,
    ));

    if let Some(burn) = active_block.burn.as_ref() {
        lines.push(Line::from(vec![
            Span::raw(live_key_label("Burn avg")),
            Span::styled(
                format!(
                    "{} tokens/min | {}/hr",
                    format_u64(burn.tokens_per_minute.round().max(0.0) as u64),
                    format_usd(burn.cost_per_hour)
                ),
                Style::default()
                    .fg(TuiColor::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" | "),
            Span::styled(
                burn_status_text(burn.status),
                Style::default()
                    .fg(burn_status_color(burn.status))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    if let Some(projection) = today_projection(context) {
        lines.push(live_key_value_line(
            "Today EOD(avg)",
            format!(
                "{} tokens | {}",
                format_u64(projection.projected_tokens_end_of_day),
                format_usd(projection.projected_cost_end_of_day)
            ),
            TuiColor::LightCyan,
        ));
        lines.push(Line::from(vec![
            Span::raw(live_key_label("Today avg rate")),
            Span::styled(
                format!(
                    "{}/hr | {}/hr",
                    format_u64(projection.tokens_per_hour.round().max(0.0) as u64),
                    format_usd(projection.cost_per_hour)
                ),
                Style::default()
                    .fg(TuiColor::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    lines.push(live_key_value_line(
        "Today",
        format!(
            "{} · {} tokens",
            format_usd(context.today_totals.cost_usd),
            format_u64(context.today_totals.total_tokens)
        ),
        TuiColor::Green,
    ));
    append_activity_lines(
        &mut lines,
        "",
        context.today_activity.as_ref(),
        &context.today_totals,
    );
    lines.push(live_key_value_line(
        "Last 30d",
        format!(
            "{} · {} tokens",
            format_usd(context.last_30d_totals.cost_usd),
            format_u64(context.last_30d_totals.total_tokens)
        ),
        TuiColor::Green,
    ));
    append_activity_lines(
        &mut lines,
        "30d",
        context.last_30d_activity.as_ref(),
        &context.last_30d_totals,
    );

    lines
}

pub(super) fn live_limit_lines(context: &LiveFrameContext<'_>) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![Span::styled(
        "Forecast",
        Style::default()
            .fg(TuiColor::Cyan)
            .add_modifier(Modifier::BOLD),
    )])];

    if let Some(official) = preferred_official_for_live(context) {
        if let Some(secondary_used) = official.secondary_used_percent() {
            if let Some((pace_line, pace_color)) = weekly_pace_line(
                secondary_used,
                official.secondary_window_mins(),
                official.secondary_resets_at(),
                context.now,
            ) {
                lines.push(live_key_value_line("Pace", pace_line, pace_color));
            }

            if let Some((runout_line, runout_color)) = weekly_runout_local_line(
                secondary_used,
                official.secondary_window_mins(),
                official.secondary_resets_at(),
                context.now,
                context.tz,
            ) {
                lines.push(live_key_value_line(
                    "Weekly runout",
                    runout_line,
                    runout_color,
                ));
            }
        } else if let Some(primary_used) = official.primary_used_percent() {
            let session_color = used_gauge_color(primary_used);
            lines.push(live_key_value_line(
                "Session trend",
                format!("{:.1}% used", primary_used),
                session_color,
            ));
        }
    } else {
        lines.push(live_key_value_line(
            "Official",
            "limits unavailable",
            TuiColor::Yellow,
        ));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "Projection",
        Style::default()
            .fg(TuiColor::DarkGray)
            .add_modifier(Modifier::BOLD),
    )]));

    let limit_ctx = LimitDisplayContext {
        token_limit: context.limit.token_limit,
        token_limit_source: context.limit.token_limit_source,
        membership_estimate: context.limit.membership_estimate,
    };
    let blended = blended_projection(context).unwrap_or_else(|| {
        let (current_tokens, current_cost) = context
            .active
            .map(|active| (active.totals.total_tokens, active.totals.cost_usd))
            .unwrap_or((0, 0.0));
        BlendedProjection {
            tokens_per_minute: 0.0,
            cost_per_hour: 0.0,
            projected_tokens_end: current_tokens,
            projected_cost_end: current_cost,
            short_weight: 0.0,
            today_weight: 0.0,
            long_weight: 1.0,
        }
    });

    lines.push(Line::from(vec![
        Span::raw(live_key_label("Rate blend")),
        Span::styled(
            format!(
                "{} tokens/min | {}/hr",
                format_u64(blended.tokens_per_minute.round().max(0.0) as u64),
                format_usd(blended.cost_per_hour)
            ),
            Style::default()
                .fg(TuiColor::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw(live_key_label("Weights")),
        Span::raw("short "),
        Span::styled(
            format!("{:.0}%", blended.short_weight * 100.0),
            Style::default()
                .fg(TuiColor::LightCyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" · today "),
        Span::styled(
            format!("{:.0}%", blended.today_weight * 100.0),
            Style::default()
                .fg(TuiColor::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" · 30d "),
        Span::styled(
            format!("{:.0}%", blended.long_weight * 100.0),
            Style::default()
                .fg(TuiColor::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(live_key_value_line(
        "5h projected end",
        format!(
            "{} | {}",
            format_u64(blended.projected_tokens_end),
            format_usd(blended.projected_cost_end)
        ),
        TuiColor::LightCyan,
    ));

    let current_tokens = context
        .active
        .map(|active_block| active_block.totals.total_tokens)
        .unwrap_or(0);
    if let Some(limit) = limit_ctx.token_limit {
        if limit > 0 {
            let (effective_limit, _promotions) = resolve_display_limit(
                limit,
                blended.projected_tokens_end,
                limit_ctx.token_limit_source,
                limit_ctx.membership_estimate,
            );
            let current_pct = (current_tokens as f64 / effective_limit as f64) * 100.0;
            let projected_pct =
                (blended.projected_tokens_end as f64 / effective_limit as f64) * 100.0;
            let (status, status_color) = limit_status(projected_pct);
            lines.push(Line::from(vec![
                Span::raw(live_key_label("5h limit")),
                Span::styled(
                    format_u64(effective_limit),
                    Style::default()
                        .fg(TuiColor::LightCyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" | current "),
                Span::styled(
                    format!("{current_pct:.1}%"),
                    Style::default()
                        .fg(used_gauge_color(current_pct))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" | projected "),
                Span::styled(
                    format!("{projected_pct:.1}%"),
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" | "),
                Span::styled(
                    status,
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    } else if let Some(estimate) = limit_ctx.membership_estimate {
        lines.push(Line::from(vec![
            Span::raw(live_key_label("Est. 5h limit")),
            Span::styled(
                format_u64(estimate.estimated_window_tokens),
                Style::default()
                    .fg(TuiColor::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ("),
            Span::styled(
                format!("{:.0}%", estimate.confidence * 100.0),
                Style::default()
                    .fg(TuiColor::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" confidence)"),
        ]));
    }

    if let Some(ag) = context.official_antigravity {
        lines.push(Line::from(""));
        let plan = ag.plan_type.as_deref().unwrap_or("unknown");
        lines.push(Line::from(vec![Span::styled(
            format!("Antigravity ({plan})"),
            Style::default()
                .fg(TuiColor::Green)
                .add_modifier(Modifier::BOLD),
        )]));
        let now = Utc::now();
        let ordered = select_antigravity_models(&ag.models);
        let shown: HashSet<String> = ordered.iter().map(|m| m.label.clone()).collect();
        let rest: Vec<_> = ag
            .models
            .iter()
            .filter(|m| !shown.contains(&m.label))
            .collect();
        for model in ordered.iter().chain(rest) {
            let (pct_text, color) = if let Some(frac) = model.remaining_fraction {
                let remaining = frac * 100.0;
                let used = 100.0 - remaining;
                let color = if remaining >= 40.0 {
                    TuiColor::Green
                } else if remaining >= 15.0 {
                    TuiColor::Yellow
                } else {
                    TuiColor::Red
                };
                (format!("{remaining:.0}% left ({used:.0}% used)"), color)
            } else {
                ("quota n/a".to_string(), TuiColor::DarkGray)
            };
            let mut spans = vec![
                Span::raw(format!("  {:<26} ", model.label)),
                Span::styled(
                    pct_text,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ];
            if let Some(reset_ts) = model.reset_time {
                let eta = format_time_until_reset_short(reset_ts, now);
                spans.push(Span::raw(format!(" ({eta})")));
            }
            lines.push(Line::from(spans));
        }
    }

    lines
}

pub(super) fn weekly_pace_line(
    used_percent: f64,
    window_mins: Option<i64>,
    resets_at: Option<i64>,
    now: DateTime<Utc>,
) -> Option<(String, TuiColor)> {
    let window_secs = window_mins?.max(1) * 60;
    let reset_unix = resets_at?;
    let reset_dt = DateTime::from_timestamp(reset_unix, 0)?;
    let remaining_secs = (reset_dt - now).num_seconds().clamp(0, window_secs);
    let elapsed_secs = (window_secs - remaining_secs).clamp(0, window_secs);
    let elapsed_secs_f = elapsed_secs.max(1) as f64;
    let elapsed_pct = (elapsed_secs_f / window_secs as f64) * 100.0;
    let delta = used_percent - elapsed_pct;
    if delta.abs() < 3.0 {
        return Some(("On pace · Lasts to reset".to_string(), TuiColor::Green));
    }

    if delta > 0.0 {
        let mut suffix = String::new();
        if used_percent > 0.0 {
            let used_per_sec = used_percent / elapsed_secs as f64;
            if used_per_sec.is_finite() && used_per_sec > 0.0 {
                let secs_to_full = ((100.0 - used_percent).max(0.0) / used_per_sec).round() as i64;
                if secs_to_full > 0 && secs_to_full < remaining_secs {
                    suffix = format!(" · Runs out in {}", format_hours_minutes(secs_to_full / 60));
                } else if remaining_secs > 0 {
                    suffix = " · Lasts to reset".to_string();
                }
            }
        }
        let color = if delta >= 20.0 {
            TuiColor::Red
        } else {
            TuiColor::Yellow
        };
        Some((format!("Behind (-{:.1}%){}", delta.abs(), suffix), color))
    } else {
        Some((
            format!("Ahead (+{:.1}%) · Lasts to reset", delta.abs(),),
            TuiColor::Green,
        ))
    }
}

pub(super) fn weekly_runout_local_line(
    used_percent: f64,
    window_mins: Option<i64>,
    resets_at: Option<i64>,
    now: DateTime<Utc>,
    tz: &TimeZoneMode,
) -> Option<(String, TuiColor)> {
    let window_secs = window_mins?.max(1) * 60;
    let reset_unix = resets_at?;
    let reset_dt = DateTime::from_timestamp(reset_unix, 0)?;
    let remaining_secs = (reset_dt - now).num_seconds().clamp(0, window_secs);
    let elapsed_secs = (window_secs - remaining_secs).clamp(0, window_secs);
    let elapsed_secs_f = elapsed_secs.max(1) as f64;
    let observed_used_per_sec = (used_percent / elapsed_secs_f).max(0.0);
    let baseline_used_per_sec = 100.0 / window_secs as f64;
    let observed_weight = (elapsed_secs_f / (6.0 * 3600.0)).clamp(0.15, 0.9);
    let blended_used_per_sec =
        baseline_used_per_sec * (1.0 - observed_weight) + observed_used_per_sec * observed_weight;
    if !blended_used_per_sec.is_finite() || blended_used_per_sec <= 0.0 {
        return Some(("Lasts to reset".to_string(), TuiColor::Green));
    }
    let secs_to_full = ((100.0 - used_percent).max(0.0) / blended_used_per_sec).round() as i64;
    if secs_to_full <= 0 {
        return Some(("Now".to_string(), TuiColor::Red));
    }

    let predicted = now + chrono::TimeDelta::seconds(secs_to_full);
    if predicted < reset_dt {
        let local = format_display_datetime(predicted, tz);
        let eta = format_hours_minutes((secs_to_full / 60).max(0));
        let color = if secs_to_full <= 24 * 3600 {
            TuiColor::Red
        } else {
            TuiColor::Yellow
        };
        return Some((format!("{local} (in {eta})"), color));
    }

    Some((
        format!(
            "Lasts to reset ({})",
            format_reset_timestamp(reset_unix, tz)
        ),
        TuiColor::Green,
    ))
}

pub(super) fn projected_end(active_block: &ActiveBlockSummary) -> (u64, f64) {
    let current_tokens = active_block.totals.total_tokens;
    let current_cost = active_block.totals.cost_usd;
    let Some(burn) = active_block.burn.as_ref() else {
        return (current_tokens, current_cost);
    };

    let projected_tokens = (current_tokens as f64
        + burn.tokens_per_minute * active_block.remaining_minutes.max(0) as f64)
        .round()
        .max(current_tokens as f64) as u64;
    let projected_cost = (current_cost
        + (burn.cost_per_hour / 60.0) * active_block.remaining_minutes.max(0) as f64)
        .max(current_cost);
    (projected_tokens, projected_cost)
}

pub(super) fn limit_status(projected_pct: f64) -> (&'static str, TuiColor) {
    if projected_pct >= 100.0 {
        ("EXCEEDS", TuiColor::Red)
    } else if projected_pct >= 80.0 {
        ("WARNING", TuiColor::Yellow)
    } else {
        ("OK", TuiColor::Green)
    }
}

pub(super) fn burn_status_text(status: BurnStatus) -> &'static str {
    match status {
        BurnStatus::Normal => "Normal",
        BurnStatus::Moderate => "Moderate",
        BurnStatus::High => "High",
    }
}

pub(super) fn burn_status_color(status: BurnStatus) -> TuiColor {
    match status {
        BurnStatus::Normal => TuiColor::Green,
        BurnStatus::Moderate => TuiColor::Yellow,
        BurnStatus::High => TuiColor::Red,
    }
}

pub(super) fn poll_live_input(timeout: Duration, current_tab: LiveTab) -> Result<LiveInputEvent> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let poll_for = remaining.min(Duration::from_millis(50));
        if !event::poll(poll_for)? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Ok(LiveInputEvent::Exit),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(LiveInputEvent::Exit);
            }
            KeyCode::Tab | KeyCode::Right => {
                return Ok(LiveInputEvent::SwitchTab(current_tab.next()));
            }
            KeyCode::BackTab | KeyCode::Left => {
                return Ok(LiveInputEvent::SwitchTab(current_tab.prev()));
            }
            _ => {}
        }
    }

    Ok(LiveInputEvent::Tick)
}

pub(super) fn render_live_deepseek_tab(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    let mut lines = vec![
        Line::from(vec![Span::styled(
            "DeepSeek API Balance",
            Style::default()
                .fg(TuiColor::Green)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];
    if let Some(ds) = context.official_deepseek {
        if !ds.is_available {
            lines.push(Line::from("  Service reported as unavailable."));
        } else {
            let currency = ds.currency.as_deref().unwrap_or("USD");
            if let Some(total) = ds.total_balance {
                lines.push(Line::from(format!(
                    "  Total Balance:     {total:.4} {currency}"
                )));
            }
            if let Some(granted) = ds.granted_balance {
                lines.push(Line::from(format!(
                    "  Granted Credits:   {granted:.4} {currency}"
                )));
            }
            if let Some(topped) = ds.topped_up_balance {
                lines.push(Line::from(format!(
                    "  Topped Up Balance: {topped:.4} {currency}"
                )));
            }
        }
    } else {
        lines.push(Line::from(
            "  No data available. DEEPSEEK_API_KEY env var may not be set.",
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

pub(super) fn render_live_openrouter_tab(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    let mut lines = vec![
        Line::from(vec![Span::styled(
            "OpenRouter Key Usage / Limits",
            Style::default()
                .fg(TuiColor::Green)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];
    if let Some(or) = context.official_openrouter {
        if let Some(ref label) = or.label {
            lines.push(Line::from(format!("  Key Label:    {label}")));
        }
        lines.push(Line::from(format!(
            "  Free Tier:    {}",
            if or.is_free_tier { "Yes" } else { "No" }
        )));
        if let Some(used) = or.credits_used {
            lines.push(Line::from(format!("  Credits Used: ${used:.4}")));
        }
        if let Some(limit) = or.credits_limit {
            lines.push(Line::from(format!("  Credit Limit: ${limit:.4}")));
        }
        if let Some(pct) = or.used_percent {
            lines.push(Line::from(format!("  Used Percent: {pct:.2}%")));
        }
    } else {
        lines.push(Line::from(
            "  No data available. OPENROUTER_API_KEY env var may not be set.",
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

pub(super) fn render_live_grok_tab(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    let mut lines = vec![
        Line::from(vec![Span::styled(
            "Grok (xAI) Billing Quotas",
            Style::default()
                .fg(TuiColor::Green)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];
    if let Some(grok) = context.official_grok {
        let currency = grok.currency.as_deref().unwrap_or("USD");
        if let Some(granted) = grok.total_granted {
            lines.push(Line::from(format!(
                "  Total Granted Credits:   {granted:.4} {currency}"
            )));
        }
        if let Some(used) = grok.total_used {
            lines.push(Line::from(format!(
                "  Total Used Credits:      {used:.4} {currency}"
            )));
        }
        if let Some(rem) = grok.total_remaining {
            lines.push(Line::from(format!(
                "  Total Remaining Credits: {rem:.4} {currency}"
            )));
        }
        if let Some(pct) = grok.used_percent {
            lines.push(Line::from(format!("  Used Percent:            {pct:.2}%")));
        }
    } else {
        lines.push(Line::from(
            "  No data available. XAI_API_KEY env var may not be set.",
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

pub(super) fn render_live_kimi_tab(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    let mut lines = vec![
        Line::from(vec![Span::styled(
            "Kimi (Moonshot AI) Balance",
            Style::default()
                .fg(TuiColor::Green)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];
    if let Some(kimi) = context.official_kimi {
        let currency = kimi.currency.as_deref().unwrap_or("CNY");
        if let Some(avail) = kimi.available_balance {
            lines.push(Line::from(format!(
                "  Available Balance: {avail:.4} {currency}"
            )));
        }
        if let Some(cash) = kimi.cash_balance {
            lines.push(Line::from(format!(
                "  Cash Balance:      {cash:.4} {currency}"
            )));
        }
        if let Some(voucher) = kimi.voucher_balance {
            lines.push(Line::from(format!(
                "  Voucher Balance:   {voucher:.4} {currency}"
            )));
        }
    } else {
        lines.push(Line::from(
            "  No data available. MOONSHOT_API_KEY env var may not be set.",
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

pub(super) fn render_live_anthropic_api_tab(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    context: &LiveFrameContext<'_>,
) {
    let mut lines = vec![
        Line::from(vec![Span::styled(
            "Anthropic Developer API Usage (Today)",
            Style::default()
                .fg(TuiColor::Green)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];
    if let Some(anth) = context.official_anthropic_api {
        if let Some(cost) = anth.cost_usd_today {
            lines.push(Line::from(format!("  Usage Cost today:  ${cost:.4}")));
        } else {
            lines.push(Line::from("  Usage Cost today:  $0.00"));
        }
        if let Some(input) = anth.input_tokens_today {
            lines.push(Line::from(format!("  Input Tokens:      {input}")));
        }
        if let Some(output) = anth.output_tokens_today {
            lines.push(Line::from(format!("  Output Tokens:     {output}")));
        }
        if let Some(cached) = anth.cache_read_tokens_today {
            lines.push(Line::from(format!("  Cache Read Tokens: {cached}")));
        }
    } else {
        lines.push(Line::from(
            "  No data available. ANTHROPIC_API_KEY or ANTHROPIC_ADMIN_KEY env var may not be set.",
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}
