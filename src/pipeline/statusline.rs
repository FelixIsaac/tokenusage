use std::io::{IsTerminal, Read as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};

use crate::cli::{CostSource, StatuslineAction, StatuslineArgs, StatuslineInitArgs, VisualBurnRate};
use crate::types::{SourceKind, TokenCounts, UsageEvent};

use super::live::fetch_selected_official_limits;
use super::parsing::load_usage;
use super::*;

pub(crate) async fn run_statusline(args: StatuslineArgs) -> Result<()> {
    if let Some(StatuslineAction::Init(init)) = &args.action {
        return run_statusline_init(init);
    }

    if args.context_low_threshold >= args.context_medium_threshold {
        bail!(
            "--context-low-threshold ({}) must be less than --context-medium-threshold ({})",
            args.context_low_threshold,
            args.context_medium_threshold
        );
    }
    if args.context_medium_threshold > 100 {
        bail!("--context-medium-threshold must be <= 100");
    }

    let tz = parse_timezone_mode(args.common.timezone.as_deref())?;
    let hook = read_statusline_hook_input()?;
    let session_id = hook
        .as_ref()
        .and_then(|h| h.session_id.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    // Structured modes (--json / --field / --format) render from a shared fields
    // cache so N widget calls trigger at most one parse per refresh interval.
    if args.common.json || args.field.is_some() || args.format.is_some() {
        return run_statusline_structured(&args, hook.as_ref(), session_id, &tz).await;
    }

    let cache_path = statusline_cache_path(session_id);

    if args.cache
        && let Some(cached) = read_statusline_cache(
            &cache_path,
            args.refresh_interval,
            hook.as_ref().and_then(|h| h.transcript_path.as_deref()),
        )
    {
        print!("{cached}");
        return Ok(());
    }

    let loaded = load_usage(&args.common, &tz).await?;
    let today = local_date(Utc::now(), &tz);

    let today_totals = loaded
        .events
        .iter()
        .filter(|e| local_date(e.timestamp, &tz) == today)
        .fold(TokenCounts::default(), |mut acc, e| {
            acc.add_assign(e.usage.to_counts());
            acc
        });

    let session_totals = session_id.and_then(|id| aggregate_session_totals(&loaded.events, id));
    let block_summary = active_block_summary(&loaded.events, Utc::now(), 5 * 3600);
    let (official_codex, official_claude, official_antigravity, _, _, _, _, _) = if args.official_limits {
        let (codex, claude, antigravity, deepseek, openrouter, grok, kimi, anthropic, errors) =
            fetch_selected_official_limits(&args.common).await;
        for error in errors {
            eprintln!("{error}");
        }
        (codex, claude, antigravity, deepseek, openrouter, grok, kimi, anthropic)
    } else {
        (None, None, None, None, None, None, None, None)
    };
    let line = build_statusline_line(
        &args,
        hook.as_ref(),
        session_totals.as_ref(),
        &today_totals,
        block_summary.as_ref(),
        official_codex.as_ref(),
        official_claude.as_ref(),
        official_antigravity.as_ref(),
        &tz,
    );

    println!("{line}");

    if args.cache {
        write_statusline_cache(
            &cache_path,
            &line,
            hook.as_ref().and_then(|h| h.transcript_path.as_deref()),
        );
    }

    Ok(())
}

const DEFAULT_INIT_FLAGS: &str = "--cache --refresh-interval 30";

/// What `tu statusline init` would do to an existing `settings.json`.
enum StatuslinePlan {
    /// No statusLine present — clean insert.
    Insert,
    /// statusLine already equals our target command.
    Noop,
    /// statusLine is a tu line with different flags — safe to update.
    Update(String),
    /// statusLine belongs to another tool — never overwrite without --yes.
    Conflict(String),
}

/// Classify the current `statusLine.command` against our target command.
fn classify_statusline_plan(root: &serde_json::Value, command: &str) -> StatuslinePlan {
    let current = root
        .get("statusLine")
        .and_then(|s| s.get("command"))
        .and_then(|c| c.as_str());
    match current {
        None => StatuslinePlan::Insert,
        Some(cur) if cur == command => StatuslinePlan::Noop,
        Some(cur) if is_tu_statusline(cur) => StatuslinePlan::Update(cur.to_string()),
        Some(cur) => StatuslinePlan::Conflict(cur.to_string()),
    }
}

/// `tu statusline init` — wire tu into Claude Code's status line, or print the
/// config / an integration prompt for an existing ccstatusline / custom line.
fn run_statusline_init(init: &StatuslineInitArgs) -> Result<()> {
    let flags = init.flags.as_deref().unwrap_or(DEFAULT_INIT_FLAGS).trim();
    let command = if flags.is_empty() {
        "tu statusline".to_string()
    } else {
        format!("tu statusline {flags}")
    };

    // Integration prompt path: never touches settings.json — emits text the user
    // hands to their assistant to wire tu into whatever they already run.
    if init.ccstatusline {
        print!("{}", ccstatusline_integration_prompt(&command, flags));
        return Ok(());
    }

    let settings_path = dirs::home_dir()
        .context("Failed to resolve home directory")?
        .join(".claude")
        .join("settings.json");

    let block = serde_json::json!({ "type": "command", "command": command });

    // Load existing settings (empty object if absent). A malformed file is a hard
    // stop — we never overwrite a file we couldn't parse.
    let existing_raw = std::fs::read_to_string(&settings_path).ok();
    let mut root: serde_json::Value = match existing_raw.as_deref().map(str::trim) {
        None | Some("") => serde_json::json!({}),
        Some(raw) => serde_json::from_str(raw).with_context(|| {
            format!(
                "{} isn't valid JSON — refusing to overwrite. Fix or move it, then re-run.",
                settings_path.display()
            )
        })?,
    };
    if !root.is_object() {
        bail!(
            "{} doesn't contain a JSON object — refusing to overwrite.",
            settings_path.display()
        );
    }

    // Classify the current statusLine so we know whether this is a clean insert,
    // an update of our own line, a no-op, or a conflict with someone else's.
    let plan = classify_statusline_plan(&root, &command);

    let pretty_block = serde_json::to_string_pretty(&serde_json::json!({ "statusLine": block }))
        .unwrap_or_default();

    if let StatuslinePlan::Noop = plan {
        println!(
            "Already set — {} statusLine is\n  {command}\nNothing to do.",
            settings_path.display()
        );
        return Ok(());
    }

    // --print: pure preview, write nothing — predictable for agents regardless of
    // what's already there.
    if init.print {
        println!("// {}", settings_path.display());
        println!("{pretty_block}");
        return Ok(());
    }

    // Conflict: another tool owns the line. Never clobber silently — explain and
    // point at the ccstatusline prompt, unless the user forces it with --yes.
    if let StatuslinePlan::Conflict(cur) = &plan
        && !init.yes
    {
        println!("{} already has a statusLine that isn't tu:", settings_path.display());
        println!("  current : {cur}");
        println!("  tu would : {command}");
        println!();
        println!("Not overwriting. Pick one:");
        println!("  • Keep both / compose — run `tu statusline init --ccstatusline` for a prompt to");
        println!("    hand your assistant that integrates tu into the line above.");
        println!("  • Replace it with tu — re-run `tu statusline init --yes`.");
        return Ok(());
    }

    // Interactive confirm (skipped with --yes). If we can't prompt and weren't
    // told --yes, bail rather than guess.
    if !init.yes {
        if !std::io::stdin().is_terminal() {
            bail!(
                "Refusing to edit {} non-interactively. Re-run with --yes to apply, or --print to preview.",
                settings_path.display()
            );
        }
        match &plan {
            StatuslinePlan::Update(cur) => println!("Update tu statusline in {}:\n  from : {cur}\n  to   : {command}", settings_path.display()),
            _ => println!("Add this to {}:\n{pretty_block}", settings_path.display()),
        }
        print!("Apply? [y/N] ");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("Cancelled — no changes made.");
            return Ok(());
        }
    }

    // Back up an existing file before mutating it.
    if existing_raw.is_some() {
        let backup = backup_path(&settings_path);
        std::fs::copy(&settings_path, &backup)
            .with_context(|| format!("Failed to back up {}", settings_path.display()))?;
        println!("Backed up → {}", backup.display());
    } else if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    // Merge: set only statusLine, preserve every other key.
    root["statusLine"] = block;
    let serialized = serde_json::to_string_pretty(&root)?;
    std::fs::write(&settings_path, format!("{serialized}\n"))
        .with_context(|| format!("Failed to write {}", settings_path.display()))?;

    println!("Done — {} statusLine is now\n  {command}", settings_path.display());
    println!("Restart Claude Code (or start a new session) to see it.");
    Ok(())
}

/// True if a status-line command already runs tu/tokenusage's statusline.
fn is_tu_statusline(cmd: &str) -> bool {
    let c = cmd.to_lowercase();
    if !c.contains("statusline") {
        return false;
    }
    c.contains("tokenusage")
        || c.split_whitespace().next() == Some("tu")
        || c.contains("/tu ")
        || c.contains("\\tu ")
        || c.contains(" tu ")
}

/// `settings.json.bak`, or a numbered variant if that already exists.
fn backup_path(settings_path: &Path) -> PathBuf {
    let base = settings_path.with_extension("json.bak");
    if !base.exists() {
        return base;
    }
    for n in 1..1000 {
        let candidate = settings_path.with_extension(format!("json.bak{n}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    base
}

/// Copy-paste prompt the user hands to an AI assistant to integrate tu into an
/// existing ccstatusline config or custom status-line script.
fn ccstatusline_integration_prompt(command: &str, flags: &str) -> String {
    let fields = STATUSLINE_FIELD_NAMES.join(", ");
    format!(
        "Copy everything below this line and paste it to your AI coding assistant \
(Claude Code, etc.). It will wire `tu` into your existing status line.\n\
─────────────────────────────────────────────────────────────────────────────\n\
I use a custom status line (ccstatusline or my own script). Please integrate the \
`tu` CLI as a *data source* into it — don't replace what I have, feed values into it.\n\
\n\
`tu statusline` reads the Claude Code status-line hook JSON on stdin and can emit \
structured output instead of its default line:\n\
  • `tu statusline --json {flags}`        → one JSON object of all values\n\
  • `tu statusline --field <name> {flags}`  → a single formatted value\n\
  • `tu statusline --format \"<tmpl>\" {flags}` → a line with {{field}} placeholders\n\
\n\
Available field names: {fields}.\n\
`--json` keys are snake_case raw numbers (e.g. today_cost_usd) so a widget can \
style them itself.\n\
\n\
PERFORMANCE — important: don't spawn `tu` once per widget per repaint. All modes \
share one per-session cache when given `{flags}`, so the expensive parse runs at \
most once per interval. Prefer ONE call that reads `--json` (then style its fields \
in my existing widgets), or at most one `--field` call per widget.\n\
\n\
For ccstatusline specifically: add a Custom Command widget that runs \
`tu statusline --field today-cost {flags}` (and/or block-left, burn-status, \
ctx-pct), or a pre-render step that runs `tu statusline --json {flags}` once and \
have widgets read the cached JSON.\n\
\n\
The standalone equivalent (if I just want tu to own the whole line) would be: \
`{command}`. Please show me the exact edit for MY current setup and explain what \
you changed.\n\
─────────────────────────────────────────────────────────────────────────────\n"
    )
}

async fn run_statusline_structured(
    args: &StatuslineArgs,
    hook: Option<&StatuslineHookInput>,
    session_id: Option<&str>,
    tz: &TimeZoneMode,
) -> Result<()> {
    let cache_path = statusline_fields_cache_path(session_id);
    let transcript = hook.and_then(|h| h.transcript_path.as_deref());

    let fields = match args
        .cache
        .then(|| read_statusline_fields_cache(&cache_path, args.refresh_interval, transcript))
        .flatten()
    {
        Some(fields) => fields,
        None => {
            let fields = compute_statusline_fields(args, hook, session_id, tz).await?;
            if args.cache {
                write_statusline_fields_cache(&cache_path, &fields, transcript);
            }
            fields
        }
    };

    if let Some(name) = &args.field {
        match statusline_field_value(&fields, name) {
            Some(value) => println!("{value}"),
            None => bail!(
                "unknown --field '{name}'. Available: {}",
                STATUSLINE_FIELD_NAMES.join(", ")
            ),
        }
    } else if let Some(template) = &args.format {
        println!("{}", render_statusline_format(&fields, template));
    } else {
        println!("{}", serde_json::to_string(&fields)?);
    }
    Ok(())
}

async fn compute_statusline_fields(
    args: &StatuslineArgs,
    hook: Option<&StatuslineHookInput>,
    session_id: Option<&str>,
    tz: &TimeZoneMode,
) -> Result<StatuslineFields> {
    let loaded = load_usage(&args.common, tz).await?;
    let today = local_date(Utc::now(), tz);
    let today_totals = loaded
        .events
        .iter()
        .filter(|e| local_date(e.timestamp, tz) == today)
        .fold(TokenCounts::default(), |mut acc, e| {
            acc.add_assign(e.usage.to_counts());
            acc
        });
    let session_totals = session_id.and_then(|id| aggregate_session_totals(&loaded.events, id));
    let block = active_block_summary(&loaded.events, Utc::now(), 5 * 3600);

    let model = hook
        .and_then(|h| h.model.as_ref())
        .and_then(|m| m.display_name.as_deref().or(m.id.as_deref()))
        .unwrap_or("unknown")
        .to_string();

    let cc_cost = hook
        .and_then(|h| h.cost.as_ref())
        .and_then(|c| c.total_cost_usd);
    let derived_cost = session_totals.as_ref().map(|t| t.cost_usd);
    let session_cost_usd = match args.cost_source {
        CostSource::Auto | CostSource::Both => cc_cost.or(derived_cost).unwrap_or(0.0),
        CostSource::Derived => derived_cost.unwrap_or(0.0),
        CostSource::Cc => cc_cost.unwrap_or(0.0),
    };

    let (block_cost_usd, block_remaining_min) = match &block {
        Some(b) => (Some(b.totals.cost_usd), Some(b.remaining_minutes)),
        None => (None, None),
    };
    let (burn_cost_per_hour, burn_tokens_per_min, burn_status) =
        match block.as_ref().and_then(|b| b.burn.as_ref()) {
            Some(burn) => (
                Some(burn.cost_per_hour),
                Some(burn.tokens_per_minute),
                Some(
                    match burn.status {
                        BurnStatus::Normal => "normal",
                        BurnStatus::Moderate => "moderate",
                        BurnStatus::High => "high",
                    }
                    .to_string(),
                ),
            ),
            None => (None, None, None),
        };

    let (context_pct, context_level) = match hook.and_then(|h| h.context_window.as_ref()) {
        Some(ctx) => {
            let input = ctx.total_input_tokens.unwrap_or(0);
            let limit = ctx.context_window_size.unwrap_or(0);
            if limit > 0 {
                let pct = (input as f64 / limit as f64) * 100.0;
                let level = if pct < f64::from(args.context_low_threshold) {
                    "low"
                } else if pct < f64::from(args.context_medium_threshold) {
                    "medium"
                } else {
                    "high"
                };
                (Some(pct), Some(level.to_string()))
            } else {
                (None, None)
            }
        }
        None => (None, None),
    };

    Ok(StatuslineFields {
        model,
        session_cost_usd,
        today_cost_usd: today_totals.cost_usd,
        block_cost_usd,
        block_remaining_min,
        burn_cost_per_hour,
        burn_tokens_per_min,
        burn_status,
        context_pct,
        context_level,
    })
}

pub(super) fn read_statusline_hook_input() -> Result<Option<StatuslineHookInput>> {
    if std::io::stdin().is_terminal() {
        return Ok(None);
    }

    let mut stdin = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin)
        .context("Failed to read statusline stdin")?;

    let raw = stdin.trim();
    if raw.is_empty() {
        return Ok(None);
    }

    let hook = serde_json::from_str::<StatuslineHookInput>(raw)
        .context("Invalid statusline stdin JSON payload")?;
    Ok(Some(hook))
}

pub(super) fn statusline_cache_path(session_id: Option<&str>) -> PathBuf {
    let suffix = session_id
        .map(sanitize_cache_key)
        .unwrap_or_else(|| "global".to_string());
    std::env::temp_dir().join(format!("tu_statusline_cache_{suffix}.json"))
}

pub(super) fn sanitize_cache_key(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
        if out.len() >= 96 {
            break;
        }
    }
    if out.is_empty() {
        "session".to_string()
    } else {
        out
    }
}

pub(super) fn read_statusline_cache(
    cache_path: &Path,
    refresh_interval: u64,
    transcript_path: Option<&str>,
) -> Option<String> {
    let raw = std::fs::read_to_string(cache_path).ok()?;
    let entry = serde_json::from_str::<StatuslineCacheEntry>(&raw).ok()?;

    let now = unix_now_secs();
    if now.saturating_sub(entry.updated_unix) >= refresh_interval {
        return None;
    }

    if let Some(path) = transcript_path {
        let current_mtime = file_mtime_unix(path);
        if entry.transcript_path.as_deref() != Some(path)
            || entry.transcript_mtime_unix != current_mtime
        {
            return None;
        }
    }

    Some(format!("{}\n", entry.line))
}

pub(super) fn write_statusline_cache(cache_path: &Path, line: &str, transcript_path: Option<&str>) {
    let entry = StatuslineCacheEntry {
        updated_unix: unix_now_secs(),
        transcript_path: transcript_path.map(ToString::to_string),
        transcript_mtime_unix: transcript_path.and_then(file_mtime_unix),
        line: line.to_string(),
    };

    if let Ok(serialized) = serde_json::to_string(&entry) {
        let _ = std::fs::write(cache_path, serialized);
    }
}

/// Structured statusline values — the source of truth that `--json`, `--field`
/// and `--format` all render from. Cached once per refresh interval (shared by
/// every mode/widget), so N widget calls trigger at most one parse.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct StatuslineFields {
    pub model: String,
    pub session_cost_usd: f64,
    pub today_cost_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_remaining_min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub burn_cost_per_hour: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub burn_tokens_per_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub burn_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_level: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StatuslineFieldsCacheEntry {
    updated_unix: u64,
    #[serde(default)]
    transcript_path: Option<String>,
    #[serde(default)]
    transcript_mtime_unix: Option<u64>,
    fields: StatuslineFields,
}

pub(super) fn statusline_fields_cache_path(session_id: Option<&str>) -> PathBuf {
    let suffix = session_id
        .map(sanitize_cache_key)
        .unwrap_or_else(|| "global".to_string());
    std::env::temp_dir().join(format!("tu_statusline_fields_{suffix}.json"))
}

pub(super) fn read_statusline_fields_cache(
    cache_path: &Path,
    refresh_interval: u64,
    transcript_path: Option<&str>,
) -> Option<StatuslineFields> {
    let raw = std::fs::read_to_string(cache_path).ok()?;
    let entry = serde_json::from_str::<StatuslineFieldsCacheEntry>(&raw).ok()?;
    if unix_now_secs().saturating_sub(entry.updated_unix) >= refresh_interval {
        return None;
    }
    if let Some(path) = transcript_path
        && (entry.transcript_path.as_deref() != Some(path)
            || entry.transcript_mtime_unix != file_mtime_unix(path))
    {
        return None;
    }
    Some(entry.fields)
}

pub(super) fn write_statusline_fields_cache(
    cache_path: &Path,
    fields: &StatuslineFields,
    transcript_path: Option<&str>,
) {
    let entry = StatuslineFieldsCacheEntry {
        updated_unix: unix_now_secs(),
        transcript_path: transcript_path.map(ToString::to_string),
        transcript_mtime_unix: transcript_path.and_then(file_mtime_unix),
        fields: fields.clone(),
    };
    if let Ok(serialized) = serde_json::to_string(&entry) {
        let _ = std::fs::write(cache_path, serialized);
    }
}

/// Formatted value for a single field name (kebab-case), for `--field`/`--format`.
pub(super) fn statusline_field_value(fields: &StatuslineFields, name: &str) -> Option<String> {
    let v = match name {
        "model" => fields.model.clone(),
        "session-cost" => format_usd(fields.session_cost_usd),
        "today-cost" => format_usd(fields.today_cost_usd),
        "block-cost" => fields.block_cost_usd.map(format_usd)?,
        "block-left" => fields.block_remaining_min.map(format_remaining_minutes)?,
        "burn-hourly" => fields.burn_cost_per_hour.map(format_usd)?,
        "burn-per-min" => fields
            .burn_tokens_per_min
            .map(|t| format_u64(t.round() as u64))?,
        "burn-status" => fields.burn_status.clone()?,
        "ctx-pct" => fields.context_pct.map(|p| format!("{p:.0}%"))?,
        "ctx-level" => fields.context_level.clone()?,
        _ => return None,
    };
    Some(v)
}

pub(super) const STATUSLINE_FIELD_NAMES: &[&str] = &[
    "model",
    "session-cost",
    "today-cost",
    "block-cost",
    "block-left",
    "burn-hourly",
    "burn-per-min",
    "burn-status",
    "ctx-pct",
    "ctx-level",
];

/// Substitute every `{field-name}` in `template` with its value (empty if absent).
pub(super) fn render_statusline_format(fields: &StatuslineFields, template: &str) -> String {
    let mut out = template.to_string();
    for name in STATUSLINE_FIELD_NAMES {
        let token = format!("{{{name}}}");
        if out.contains(&token) {
            let value = statusline_field_value(fields, name).unwrap_or_default();
            out = out.replace(&token, &value);
        }
    }
    out
}

pub(super) fn file_mtime_unix(path: &str) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

pub(super) fn aggregate_session_totals(
    events: &[UsageEvent],
    session_id: &str,
) -> Option<TokenCounts> {
    let totals = events
        .iter()
        .filter(|event| session_id_matches(&event.session, session_id))
        .fold(TokenCounts::default(), |mut acc, event| {
            acc.add_assign(event.usage.to_counts());
            acc
        });

    if totals.total_tokens == 0 && totals.cost_usd <= 0.0 {
        None
    } else {
        Some(totals)
    }
}

pub(super) fn session_id_matches(candidate: &str, query: &str) -> bool {
    candidate == query || candidate.ends_with(query) || candidate.contains(query)
}

pub(super) fn active_block_summary(
    events: &[UsageEvent],
    now: DateTime<Utc>,
    block_window_secs: i64,
) -> Option<ActiveBlockSummary> {
    if block_window_secs <= 0 {
        return None;
    }

    let now_unix = now.timestamp();
    let block_start_unix = now_unix - now_unix.rem_euclid(block_window_secs);
    let block_end_unix = block_start_unix + block_window_secs;

    active_block_summary_for_bounds(events, now, block_start_unix, block_end_unix)
}

pub(super) fn active_block_summary_for_bounds(
    events: &[UsageEvent],
    now: DateTime<Utc>,
    block_start_unix: i64,
    block_end_unix: i64,
) -> Option<ActiveBlockSummary> {
    if block_end_unix <= block_start_unix {
        return None;
    }
    let now_unix = now.timestamp();

    let mut selected = events
        .iter()
        .filter(|event| {
            let ts = event.timestamp.timestamp();
            ts >= block_start_unix && ts < block_end_unix
        })
        .collect::<Vec<_>>();

    if selected.is_empty() {
        return None;
    }

    selected.sort_by_key(|event| event.timestamp);

    let totals = selected
        .iter()
        .fold(TokenCounts::default(), |mut acc, event| {
            acc.add_assign(event.usage.to_counts());
            acc
        });

    let mut source_totals: HashMap<SourceKind, u64> = HashMap::new();
    for event in &selected {
        let entry = source_totals.entry(event.source).or_insert(0);
        *entry = entry.saturating_add(event.usage.to_counts().total_tokens);
    }
    let dominant_source = source_totals
        .into_iter()
        .max_by_key(|(_, tokens)| *tokens)
        .map(|(source, _)| source);

    let burn = {
        let first = selected.first().map(|event| event.timestamp);
        let last = selected.last().map(|event| event.timestamp);

        match (first, last) {
            (Some(first_ts), Some(last_ts)) => {
                let minutes = (last_ts - first_ts).num_minutes();
                if minutes > 0 {
                    let tokens_per_minute = totals.total_tokens as f64 / minutes as f64;
                    let non_cache_tokens = totals.input_tokens.saturating_add(totals.output_tokens);
                    let indicator = non_cache_tokens as f64 / minutes as f64;
                    let status = if indicator < 2000.0 {
                        BurnStatus::Normal
                    } else if indicator < 5000.0 {
                        BurnStatus::Moderate
                    } else {
                        BurnStatus::High
                    };

                    Some(BurnRateSummary {
                        cost_per_hour: totals.cost_usd / minutes as f64 * 60.0,
                        tokens_per_minute,
                        status,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    };

    Some(ActiveBlockSummary {
        totals,
        remaining_minutes: ((block_end_unix - now_unix) / 60).max(0),
        burn,
        dominant_source,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_statusline_line(
    args: &StatuslineArgs,
    hook: Option<&StatuslineHookInput>,
    session_totals: Option<&TokenCounts>,
    today_totals: &TokenCounts,
    block: Option<&ActiveBlockSummary>,
    official_codex: Option<&OfficialCodexSnapshot>,
    official_claude: Option<&OfficialClaudeSnapshot>,
    official_antigravity: Option<&OfficialAntigravitySnapshot>,
    tz: &TimeZoneMode,
) -> String {
    let model_name = hook
        .and_then(|h| h.model.as_ref())
        .and_then(|m| m.display_name.as_deref().or(m.id.as_deref()))
        .unwrap_or("unknown");

    let cc_cost = hook
        .and_then(|h| h.cost.as_ref())
        .and_then(|c| c.total_cost_usd);
    let derived_cost = session_totals.map(|t| t.cost_usd);
    let session_text = match args.cost_source {
        CostSource::Auto => format_usd(cc_cost.or(derived_cost).unwrap_or(0.0)),
        CostSource::Derived => format_usd(derived_cost.unwrap_or(0.0)),
        CostSource::Cc => format_usd(cc_cost.unwrap_or(0.0)),
        CostSource::Both => format!(
            "{} hook / {} derived",
            format_usd(cc_cost.unwrap_or(0.0)),
            format_usd(derived_cost.unwrap_or(0.0))
        ),
    };

    let block_text = if let Some(block) = block {
        format!(
            "{} ({})",
            format_usd(block.totals.cost_usd),
            format_remaining_minutes(block.remaining_minutes)
        )
    } else {
        "n/a".to_string()
    };

    let mut parts = vec![
        format!("model {model_name}"),
        format!(
            "session {} | today {} | block {}",
            session_text,
            format_usd(today_totals.cost_usd),
            block_text
        ),
    ];

    if let Some(burn) = block.and_then(|b| b.burn.as_ref())
        && args.visual_burn_rate != VisualBurnRate::Off
    {
        let emoji = match burn.status {
            BurnStatus::Normal => "🟢",
            BurnStatus::Moderate => "⚠️",
            BurnStatus::High => "🚨",
        };
        let label = match burn.status {
            BurnStatus::Normal => "Normal",
            BurnStatus::Moderate => "Moderate",
            BurnStatus::High => "High",
        };

        let extra = match args.visual_burn_rate {
            VisualBurnRate::Off => String::new(),
            VisualBurnRate::Emoji => format!(" {emoji}"),
            VisualBurnRate::Text => format!(" ({label})"),
            VisualBurnRate::EmojiText => format!(" {emoji} ({label})"),
        };
        parts.push(format!(
            "burn {}/hr, {}/min{}",
            format_usd(burn.cost_per_hour),
            format_u64(burn.tokens_per_minute.round() as u64),
            extra
        ));
    }

    if let Some(context) = hook.and_then(|h| h.context_window.as_ref()) {
        let input = context.total_input_tokens.unwrap_or(0);
        let limit = context.context_window_size.unwrap_or(0);
        if limit > 0 {
            let pct = (input as f64 / limit as f64) * 100.0;
            let level = if pct < f64::from(args.context_low_threshold) {
                "low"
            } else if pct < f64::from(args.context_medium_threshold) {
                "medium"
            } else {
                "high"
            };
            parts.push(format!("ctx {} ({pct:.0}%, {level})", format_u64(input)));
        }
    }

    if let Some(official) = official_codex {
        parts.push(build_statusline_official_codex_segment(official, tz));
    }
    if let Some(official) = official_claude {
        parts.push(build_statusline_official_claude_segment(official, tz));
    }
    if let Some(official) = official_antigravity {
        parts.push(build_statusline_official_antigravity_segment(official, tz));
    }

    parts.join(" | ")
}

pub(super) fn build_statusline_official_codex_segment(
    official: &OfficialCodexSnapshot,
    tz: &TimeZoneMode,
) -> String {
    let plan = official.plan_type.as_deref().unwrap_or("unknown");
    let now = Utc::now();
    let mut parts = vec![format!("official {plan}")];

    if let Some(primary_used) = official.primary_used_percent {
        let remaining = (100.0 - primary_used).clamp(0.0, 100.0);
        let mut entry = format!("5h {:.1}% left", remaining);
        if let Some(resets_at) = official.primary_resets_at {
            let reset_text = format_reset_timestamp(resets_at, tz);
            let eta_text = format_time_until_reset_short(resets_at, now);
            entry.push_str(&format!(" (reset {reset_text}, in {eta_text})"));
        }
        parts.push(entry);
    }

    if let Some(secondary_used) = official.secondary_used_percent {
        let remaining = (100.0 - secondary_used).clamp(0.0, 100.0);
        let mut entry = format!("wk {:.1}% left", remaining);
        if let Some(resets_at) = official.secondary_resets_at {
            let reset_text = format_reset_timestamp(resets_at, tz);
            let eta_text = format_time_until_reset_short(resets_at, now);
            entry.push_str(&format!(" (reset {reset_text}, in {eta_text})"));
        }
        parts.push(entry);
    }

    parts.join(" ")
}

pub(super) fn build_statusline_official_claude_segment(
    official: &OfficialClaudeSnapshot,
    tz: &TimeZoneMode,
) -> String {
    let plan = official.plan_type.as_deref().unwrap_or("unknown");
    let now = Utc::now();
    let mut parts = vec![format!("official claude {plan}")];

    if let Some(primary_used) = official.primary_used_percent {
        let remaining = (100.0 - primary_used).clamp(0.0, 100.0);
        let mut entry = format!("5h {:.1}% left", remaining);
        if let Some(resets_at) = official.primary_resets_at {
            let reset_text = format_reset_timestamp(resets_at, tz);
            let eta_text = format_time_until_reset_short(resets_at, now);
            entry.push_str(&format!(" (reset {reset_text}, in {eta_text})"));
        }
        parts.push(entry);
    }

    if let Some(secondary_used) = official.secondary_used_percent {
        let remaining = (100.0 - secondary_used).clamp(0.0, 100.0);
        let mut entry = format!("wk {:.1}% left", remaining);
        if let Some(resets_at) = official.secondary_resets_at {
            let reset_text = format_reset_timestamp(resets_at, tz);
            let eta_text = format_time_until_reset_short(resets_at, now);
            entry.push_str(&format!(" (reset {reset_text}, in {eta_text})"));
        }
        parts.push(entry);
    }

    parts.join(" ")
}

pub(super) fn build_statusline_official_antigravity_segment(
    official: &OfficialAntigravitySnapshot,
    tz: &TimeZoneMode,
) -> String {
    let plan = official.plan_type.as_deref().unwrap_or("unknown");
    let now = Utc::now();
    let mut parts = vec![format!("antigravity {plan}")];

    let slots: &[(Option<f64>, Option<&str>, Option<i64>)] = &[
        (
            official.primary_used_percent,
            official.primary_label.as_deref(),
            official.primary_resets_at,
        ),
        (
            official.secondary_used_percent,
            official.secondary_label.as_deref(),
            official.secondary_resets_at,
        ),
        (
            official.tertiary_used_percent,
            official.tertiary_label.as_deref(),
            official.tertiary_resets_at,
        ),
    ];

    for (used_opt, label, resets_at) in slots {
        if let Some(used) = used_opt {
            let tag = label.unwrap_or("model");
            let remaining = (100.0 - used).clamp(0.0, 100.0);
            let mut entry = format!("{tag} {remaining:.1}% left");
            if let Some(resets_at) = resets_at {
                let reset_text = format_reset_timestamp(*resets_at, tz);
                let eta_text = format_time_until_reset_short(*resets_at, now);
                entry.push_str(&format!(" (reset {reset_text}, in {eta_text})"));
            }
            parts.push(entry);
        }
    }

    parts.join(" ")
}

pub(super) fn format_remaining_minutes(minutes: i64) -> String {
    format!("{} left", format_hours_minutes(minutes))
}

pub(super) fn format_hours_minutes(minutes: i64) -> String {
    let safe = minutes.max(0);
    let hrs = safe / 60;
    let mins = safe % 60;
    if hrs > 0 {
        format!("{hrs}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

pub(super) fn official_window_details(
    window_mins: Option<i64>,
    resets_at: Option<i64>,
    tz: &TimeZoneMode,
) -> Vec<String> {
    let mut details = Vec::new();
    if let Some(mins) = window_mins {
        details.push(format!("window={mins}m"));
    }
    if let Some(resets_at) = resets_at {
        details.push(format!("resets={}", format_reset_timestamp(resets_at, tz)));
    }
    details
}

pub(super) fn format_reset_timestamp(unix_secs: i64, tz: &TimeZoneMode) -> String {
    DateTime::from_timestamp(unix_secs, 0)
        .map(|ts| format_display_datetime(ts, tz))
        .unwrap_or_else(|| format!("unix:{unix_secs}"))
}

pub(super) fn format_time_until_reset_short(resets_at: i64, now: DateTime<Utc>) -> String {
    let delta_secs = (resets_at - now.timestamp()).max(0);
    let minutes = delta_secs / 60;
    let days = minutes / (24 * 60);
    if days > 0 {
        let hours = (minutes % (24 * 60)) / 60;
        if hours > 0 {
            format!("{days}d {hours}h")
        } else {
            format!("{days}d")
        }
    } else {
        format_hours_minutes(minutes)
    }
}

#[cfg(test)]
mod statusline_field_tests {
    use super::*;

    fn sample() -> StatuslineFields {
        StatuslineFields {
            model: "claude-opus-4-8".to_string(),
            session_cost_usd: 12.34,
            today_cost_usd: 22.18,
            block_cost_usd: Some(6.71),
            block_remaining_min: Some(70),
            burn_cost_per_hour: Some(134.18),
            burn_tokens_per_min: Some(1_611_590.0),
            burn_status: Some("moderate".to_string()),
            context_pct: Some(45.0),
            context_level: Some("low".to_string()),
        }
    }

    #[test]
    fn field_values_are_formatted() {
        let f = sample();
        assert_eq!(statusline_field_value(&f, "model").unwrap(), "claude-opus-4-8");
        assert_eq!(statusline_field_value(&f, "today-cost").unwrap(), "$22.18");
        assert_eq!(statusline_field_value(&f, "burn-status").unwrap(), "moderate");
        assert!(statusline_field_value(&f, "block-left").unwrap().contains("1h 10m"));
        assert!(statusline_field_value(&f, "bogus").is_none());
    }

    #[test]
    fn format_substitutes_known_tokens_only() {
        let f = sample();
        assert_eq!(
            render_statusline_format(&f, "{model} {today-cost}"),
            "claude-opus-4-8 $22.18"
        );
        // Unknown tokens are left untouched.
        assert_eq!(render_statusline_format(&f, "{model} {bogus}"), "claude-opus-4-8 {bogus}");
    }
}

#[cfg(test)]
mod statusline_init_tests {
    use super::*;
    use serde_json::json;

    const CMD: &str = "tu statusline --cache --refresh-interval 30";

    fn plan(root: serde_json::Value) -> StatuslinePlan {
        classify_statusline_plan(&root, CMD)
    }

    #[test]
    fn no_statusline_is_insert() {
        assert!(matches!(plan(json!({})), StatuslinePlan::Insert));
        assert!(matches!(
            plan(json!({ "model": "opus" })),
            StatuslinePlan::Insert
        ));
    }

    #[test]
    fn identical_command_is_noop() {
        let root = json!({ "statusLine": { "type": "command", "command": CMD } });
        assert!(matches!(plan(root), StatuslinePlan::Noop));
    }

    #[test]
    fn other_tu_flags_is_update() {
        let root = json!({ "statusLine": { "command": "tu statusline -B emoji" } });
        match plan(root) {
            StatuslinePlan::Update(cur) => assert_eq!(cur, "tu statusline -B emoji"),
            _ => panic!("expected Update"),
        }
    }

    #[test]
    fn foreign_command_is_conflict() {
        for foreign in ["ccstatusline", "starship prompt", "my-script.sh"] {
            let root = json!({ "statusLine": { "command": foreign } });
            match plan(root) {
                StatuslinePlan::Conflict(cur) => assert_eq!(cur, foreign),
                _ => panic!("expected Conflict for {foreign}"),
            }
        }
    }

    #[test]
    fn is_tu_statusline_detects_ours_only() {
        assert!(is_tu_statusline("tu statusline --cache"));
        assert!(is_tu_statusline("/usr/local/bin/tu statusline"));
        assert!(is_tu_statusline("tokenusage statusline --json"));
        assert!(!is_tu_statusline("ccstatusline"));
        assert!(!is_tu_statusline("tu daily")); // tu, but not statusline
        assert!(!is_tu_statusline("status-line.sh"));
    }

    #[test]
    fn merge_preserves_other_keys() {
        let mut root = json!({
            "model": "opus",
            "permissions": { "allow": ["Bash"] },
            "statusLine": { "command": "old-tool" }
        });
        root["statusLine"] = json!({ "type": "command", "command": CMD });
        assert_eq!(root["model"], json!("opus"));
        assert_eq!(root["permissions"]["allow"], json!(["Bash"]));
        assert_eq!(root["statusLine"]["command"], json!(CMD));
        assert_eq!(root["statusLine"]["type"], json!("command"));
    }
}
