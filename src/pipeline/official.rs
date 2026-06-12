use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, TimeZone, Utc};
use chrono_tz::Tz;
use crossbeam_channel::{Receiver, bounded};
use serde::{Deserialize, Serialize};

use crate::types::{ActivitySummary, TokenCounts};

use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct CodexAuthFile {
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    #[serde(default)]
    tokens: Option<CodexAuthTokens>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CodexAuthTokens {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CodexUsageResponse {
    #[serde(default)]
    plan_type: Option<String>,
    #[serde(default)]
    rate_limit: Option<CodexRateLimitDetails>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CodexRateLimitDetails {
    #[serde(default)]
    primary_window: Option<CodexUsageWindow>,
    #[serde(default)]
    secondary_window: Option<CodexUsageWindow>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CodexUsageWindow {
    #[serde(default)]
    used_percent: Option<f64>,
    #[serde(default)]
    reset_at: Option<i64>,
    #[serde(default)]
    limit_window_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CodexRefreshResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Debug)]
pub(super) enum CodexOAuthFetchError {
    Unauthorized,
    Other(anyhow::Error),
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaudeCredentialsFile {
    #[serde(rename = "claudeAiOauth", alias = "claude_ai_oauth")]
    claude_ai_oauth: Option<ClaudeOAuthTokens>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ClaudeOAuthTokens {
    #[serde(rename = "accessToken", alias = "access_token")]
    access_token: Option<String>,
    #[serde(rename = "refreshToken", alias = "refresh_token")]
    refresh_token: Option<String>,
    #[serde(rename = "expiresAt", alias = "expires_at")]
    expires_at: Option<f64>,
    #[serde(rename = "rateLimitTier", alias = "rate_limit_tier")]
    rate_limit_tier: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaudeOAuthUsageResponse {
    #[serde(default)]
    five_hour: Option<ClaudeOAuthWindow>,
    #[serde(default)]
    seven_day: Option<ClaudeOAuthWindow>,
    #[serde(default)]
    seven_day_oauth_apps: Option<ClaudeOAuthWindow>,
    #[serde(default)]
    seven_day_opus: Option<ClaudeOAuthWindow>,
    #[serde(default)]
    seven_day_sonnet: Option<ClaudeOAuthWindow>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaudeOAuthWindow {
    #[serde(default)]
    utilization: Option<f64>,
    #[serde(default)]
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaudeRefreshResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Debug)]
pub(super) enum ClaudeOAuthFetchError {
    Unauthorized,
    Other(anyhow::Error),
}

#[derive(Debug, Deserialize)]
pub(super) struct RpcEnvelope {
    id: Option<serde_json::Value>,
    result: Option<serde_json::Value>,
    error: Option<RpcErrorObject>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RpcErrorObject {
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RpcRateLimitsReadResult {
    #[serde(rename = "rateLimits")]
    rate_limits: Option<RpcRateLimits>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RpcRateLimits {
    primary: Option<RpcRateLimitWindow>,
    secondary: Option<RpcRateLimitWindow>,
    #[serde(rename = "planType")]
    plan_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RpcRateLimitWindow {
    #[serde(rename = "usedPercent")]
    used_percent: Option<f64>,
    #[serde(rename = "windowDurationMins")]
    window_duration_mins: Option<i64>,
    #[serde(rename = "resetsAt")]
    resets_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RpcAccountReadResult {
    account: Option<RpcAccount>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RpcAccount {
    #[serde(rename = "planType")]
    plan_type: Option<String>,
}

pub(super) async fn fetch_codex_official_limits() -> Result<OfficialCodexSnapshot> {
    match fetch_codex_official_limits_via_oauth().await {
        Ok(snapshot) => Ok(snapshot),
        Err(oauth_error) => {
            let fallback = tokio::task::spawn_blocking(fetch_codex_official_limits_blocking)
                .await
                .context("codex app-server task join failed")?;
            fallback.with_context(|| format!("oauth failed first: {oauth_error}"))
        }
    }
}

pub(super) async fn fetch_codex_official_limits_via_oauth() -> Result<OfficialCodexSnapshot> {
    let (auth_path, mut tokens) = load_codex_auth_tokens()?;

    match fetch_codex_usage_with_access_token(&tokens.access_token, tokens.account_id.as_deref())
        .await
    {
        Ok(snapshot) => Ok(snapshot),
        Err(CodexOAuthFetchError::Unauthorized) => {
            let refresh = tokens
                .refresh_token
                .as_deref()
                .filter(|v| !v.is_empty())
                .context("Codex OAuth token unauthorized and no refresh token available")?;
            let refreshed = refresh_codex_access_token(refresh).await?;
            tokens.access_token = refreshed.0;
            tokens.refresh_token = Some(refreshed.1);
            tokens.id_token = refreshed.2;
            let _ = save_codex_auth_tokens(&auth_path, &tokens);
            fetch_codex_usage_with_access_token(&tokens.access_token, tokens.account_id.as_deref())
                .await
                .map_err(|err| match err {
                    CodexOAuthFetchError::Unauthorized => {
                        anyhow::anyhow!("Codex OAuth remained unauthorized after refresh")
                    }
                    CodexOAuthFetchError::Other(error) => error,
                })
        }
        Err(CodexOAuthFetchError::Other(error)) => Err(error),
    }
}

pub(super) fn load_codex_auth_tokens() -> Result<(PathBuf, CodexAuthTokens)> {
    let auth_path = codex_auth_path().context("Failed to resolve Codex auth path")?;
    let raw = std::fs::read(&auth_path)
        .with_context(|| format!("Failed to read Codex auth file: {}", auth_path.display()))?;
    let parsed: CodexAuthFile =
        serde_json::from_slice(&raw).context("Invalid Codex auth.json format")?;

    if let Some(api_key) = parsed.openai_api_key {
        let trimmed = api_key.trim();
        if !trimmed.is_empty() {
            return Ok((
                auth_path,
                CodexAuthTokens {
                    access_token: trimmed.to_string(),
                    refresh_token: None,
                    id_token: None,
                    account_id: None,
                },
            ));
        }
    }

    let Some(tokens) = parsed.tokens else {
        bail!("Codex auth.json missing tokens");
    };
    if tokens.access_token.trim().is_empty() {
        bail!("Codex auth.json missing access_token");
    }
    Ok((auth_path, tokens))
}

pub(super) fn save_codex_auth_tokens(path: &Path, tokens: &CodexAuthTokens) -> Result<()> {
    let existing = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let mut root = existing;
    if !root.is_object() {
        root = serde_json::json!({});
    }
    let obj = root
        .as_object_mut()
        .context("Codex auth root must be object")?;
    let tokens_value = obj
        .entry("tokens".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !tokens_value.is_object() {
        *tokens_value = serde_json::json!({});
    }
    let token_obj = tokens_value
        .as_object_mut()
        .context("Codex auth tokens must be object")?;
    token_obj.insert(
        "access_token".to_string(),
        serde_json::Value::String(tokens.access_token.clone()),
    );
    if let Some(refresh) = tokens.refresh_token.as_ref().filter(|v| !v.is_empty()) {
        token_obj.insert(
            "refresh_token".to_string(),
            serde_json::Value::String(refresh.clone()),
        );
    }
    if let Some(id_token) = tokens.id_token.as_ref().filter(|v| !v.is_empty()) {
        token_obj.insert(
            "id_token".to_string(),
            serde_json::Value::String(id_token.clone()),
        );
    }
    if let Some(account_id) = tokens.account_id.as_ref().filter(|v| !v.is_empty()) {
        token_obj.insert(
            "account_id".to_string(),
            serde_json::Value::String(account_id.clone()),
        );
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let bytes = serde_json::to_vec_pretty(&root).context("Failed to serialize Codex auth file")?;
    std::fs::write(path, bytes)
        .with_context(|| format!("Failed to write Codex auth file: {}", path.display()))?;
    Ok(())
}

pub(super) fn codex_auth_path() -> Option<PathBuf> {
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        let trimmed = codex_home.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed).join("auth.json"));
        }
    }
    Some(dirs::home_dir()?.join(".codex").join("auth.json"))
}

pub(super) fn codex_config_path() -> Option<PathBuf> {
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        let trimmed = codex_home.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed).join("config.toml"));
        }
    }
    Some(dirs::home_dir()?.join(".codex").join("config.toml"))
}

pub(super) fn resolve_codex_usage_base_url() -> String {
    let mut base = "https://chatgpt.com/backend-api".to_string();
    if let Some(config_path) = codex_config_path()
        && let Ok(contents) = std::fs::read_to_string(config_path)
        && let Some(parsed) = parse_chatgpt_base_url(&contents)
    {
        base = parsed;
    }

    while base.ends_with('/') {
        base.pop();
    }
    if (base.starts_with("https://chatgpt.com") || base.starts_with("https://chat.openai.com"))
        && !base.contains("/backend-api")
    {
        base.push_str("/backend-api");
    }
    base
}

pub(super) fn parse_chatgpt_base_url(contents: &str) -> Option<String> {
    for raw_line in contents.lines() {
        let line = raw_line
            .split('#')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '=');
        let key = parts.next()?.trim();
        let value = parts.next()?.trim();
        if key != "chatgpt_base_url" {
            continue;
        }
        let unquoted = value
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .to_string();
        if !unquoted.is_empty() {
            return Some(unquoted);
        }
    }
    None
}

pub(super) async fn fetch_codex_usage_with_access_token(
    access_token: &str,
    account_id: Option<&str>,
) -> std::result::Result<OfficialCodexSnapshot, CodexOAuthFetchError> {
    let base = resolve_codex_usage_base_url();
    let path = if base.contains("/backend-api") {
        "/wham/usage"
    } else {
        "/api/codex/usage"
    };
    let url = format!("{base}{path}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| CodexOAuthFetchError::Other(anyhow::Error::new(error)))?;

    let mut request = client
        .get(url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .header("User-Agent", "tokenusage");
    if let Some(account_id) = account_id.filter(|v| !v.is_empty()) {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    let response = request
        .send()
        .await
        .map_err(|error| CodexOAuthFetchError::Other(anyhow::Error::new(error)))?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(CodexOAuthFetchError::Unauthorized);
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(CodexOAuthFetchError::Other(anyhow::anyhow!(
            "Codex usage API returned {status}: {body}"
        )));
    }

    let usage: CodexUsageResponse = response
        .json()
        .await
        .map_err(|error| CodexOAuthFetchError::Other(anyhow::Error::new(error)))?;
    Ok(OfficialCodexSnapshot {
        plan_type: usage.plan_type,
        primary_used_percent: usage.rate_limit.as_ref().and_then(|r| {
            r.primary_window
                .as_ref()
                .and_then(|window| window.used_percent)
                .map(normalize_official_used_percent)
        }),
        secondary_used_percent: usage.rate_limit.as_ref().and_then(|r| {
            r.secondary_window
                .as_ref()
                .and_then(|window| window.used_percent)
                .map(normalize_official_used_percent)
        }),
        primary_window_mins: usage.rate_limit.as_ref().and_then(|r| {
            r.primary_window
                .as_ref()
                .and_then(|window| window.limit_window_seconds)
                .map(|secs| secs / 60)
        }),
        secondary_window_mins: usage.rate_limit.as_ref().and_then(|r| {
            r.secondary_window
                .as_ref()
                .and_then(|window| window.limit_window_seconds)
                .map(|secs| secs / 60)
        }),
        primary_resets_at: usage
            .rate_limit
            .as_ref()
            .and_then(|r| r.primary_window.as_ref().and_then(|window| window.reset_at)),
        secondary_resets_at: usage.rate_limit.as_ref().and_then(|r| {
            r.secondary_window
                .as_ref()
                .and_then(|window| window.reset_at)
        }),
    })
}

pub(super) async fn refresh_codex_access_token(
    refresh_token: &str,
) -> Result<(String, String, Option<String>)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client for Codex refresh")?;
    let response = client
        .post("https://auth.openai.com/oauth/token")
        .json(&serde_json::json!({
            "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "scope": "openid profile email"
        }))
        .send()
        .await
        .context("Codex refresh request failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Codex refresh failed ({status}): {body}");
    }
    let payload: CodexRefreshResponse = response
        .json()
        .await
        .context("Invalid Codex refresh response")?;
    Ok((
        payload.access_token,
        payload
            .refresh_token
            .unwrap_or_else(|| refresh_token.to_string()),
        payload.id_token,
    ))
}

pub(super) async fn fetch_claude_official_limits() -> Result<OfficialClaudeSnapshot> {
    // Primary approach: CLI PTY probe (run `claude`, send `/usage`, parse output).
    match fetch_claude_limits_via_cli().await {
        Ok(snapshot) => {
            save_claude_snapshot_cache(&snapshot);
            return Ok(snapshot);
        }
        Err(_cli_err) => {
            // Silently fall through to OAuth fallback.
        }
    }

    // Fallback: OAuth via ~/.claude/.credentials.json (may not exist on newer installs).
    let oauth_result: Result<OfficialClaudeSnapshot> = async {
        let (credentials_path, mut tokens) = load_claude_oauth_tokens()?;
        let current_access_token = tokens.access_token.clone().unwrap_or_default();
        match fetch_claude_usage_with_access_token(
            &current_access_token,
            tokens.rate_limit_tier.as_deref(),
        )
        .await
        {
            Ok(snapshot) => Ok(snapshot),
            Err(ClaudeOAuthFetchError::Unauthorized) => {
                let refresh = tokens
                    .refresh_token
                    .as_deref()
                    .filter(|v| !v.is_empty())
                    .context("Claude OAuth token unauthorized and no refresh token available")?;
                let refreshed = refresh_claude_access_token(refresh).await?;
                tokens.access_token = Some(refreshed.0);
                if let Some(refresh_token) = refreshed.1 {
                    tokens.refresh_token = Some(refresh_token);
                }
                if let Some(expires_at) = refreshed.2 {
                    tokens.expires_at = Some(expires_at as f64);
                }
                let _ = save_claude_oauth_tokens(&credentials_path, &tokens);
                let refreshed_access_token = tokens.access_token.clone().unwrap_or_default();
                fetch_claude_usage_with_access_token(
                    &refreshed_access_token,
                    tokens.rate_limit_tier.as_deref(),
                )
                .await
                .map_err(|err| match err {
                    ClaudeOAuthFetchError::Unauthorized => {
                        anyhow::anyhow!("Claude OAuth remained unauthorized after refresh")
                    }
                    ClaudeOAuthFetchError::Other(error) => error,
                })
            }
            Err(ClaudeOAuthFetchError::Other(error)) => Err(error),
        }
    }
    .await;

    match oauth_result {
        Ok(snapshot) => {
            save_claude_snapshot_cache(&snapshot);
            Ok(snapshot)
        }
        Err(err) => {
            // Both CLI and OAuth failed — try the local cache as last resort.
            if let Some(cached) = load_claude_snapshot_cache() {
                Ok(cached)
            } else {
                Err(err)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Claude official-limits snapshot cache (survives probe failures).
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct ClaudeSnapshotCache {
    fetched_unix: u64,
    snapshot: OfficialClaudeSnapshot,
}

/// Maximum age of the cached snapshot (10 minutes).  Beyond this the cache
/// is considered stale and will not be used as a fallback.
pub(super) const CLAUDE_SNAPSHOT_CACHE_MAX_AGE_SECS: u64 = 600;

pub(super) fn claude_snapshot_cache_path() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("tokenusage").join("claude-limits-cache.json"))
}

pub(super) fn save_claude_snapshot_cache(snapshot: &OfficialClaudeSnapshot) {
    let Some(path) = claude_snapshot_cache_path() else {
        return;
    };
    let cache = ClaudeSnapshotCache {
        fetched_unix: unix_now_secs(),
        snapshot: snapshot.clone(),
    };
    if let Ok(json) = serde_json::to_vec(&cache) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, json);
    }
}

pub(super) fn load_claude_snapshot_cache() -> Option<OfficialClaudeSnapshot> {
    let path = claude_snapshot_cache_path()?;
    let body = std::fs::read(&path).ok()?;
    let cache: ClaudeSnapshotCache = serde_json::from_slice(&body).ok()?;
    let age = unix_now_secs().saturating_sub(cache.fetched_unix);
    if age > CLAUDE_SNAPSHOT_CACHE_MAX_AGE_SECS {
        return None; // Too stale
    }
    Some(cache.snapshot)
}

// ---------------------------------------------------------------------------
// Live frame cache — persist last rendered frame data so `tu live` can start
// instantly showing the previous session's values instead of a blank skeleton.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct LiveFrameCache {
    pub(super) cached_at_unix: u64,
    /// ISO date string (YYYY-MM-DD) in user's timezone, so we can invalidate
    /// today_totals if the day has changed.
    pub(super) cached_date: String,
    pub(super) today_totals: TokenCounts,
    pub(super) last_30d_totals: TokenCounts,
    pub(super) last_30d_active_days: u32,
    pub(super) today_activity: Option<ActivitySummary>,
    pub(super) last_30d_activity: Option<ActivitySummary>,
    pub(super) official_codex: Option<OfficialCodexSnapshot>,
    pub(super) official_claude: Option<OfficialClaudeSnapshot>,
    pub(super) official_antigravity: Option<OfficialAntigravitySnapshot>,
    pub(super) official_deepseek: Option<OfficialDeepSeekSnapshot>,
    pub(super) official_openrouter: Option<OfficialOpenRouterSnapshot>,
    pub(super) official_grok: Option<OfficialGrokSnapshot>,
    pub(super) official_kimi: Option<OfficialKimiSnapshot>,
    pub(super) official_anthropic_api: Option<OfficialAnthropicApiSnapshot>,
}

pub(super) fn live_frame_cache_path() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("tokenusage").join("live-frame-cache.json"))
}

pub(super) fn save_live_frame_cache(cache: &LiveFrameCache) {
    let Some(path) = live_frame_cache_path() else {
        return;
    };
    if let Ok(json) = serde_json::to_vec(cache) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, json);
    }
}

pub(super) fn load_live_frame_cache() -> Option<LiveFrameCache> {
    let path = live_frame_cache_path()?;
    let body = std::fs::read(&path).ok()?;
    let cache: LiveFrameCache = serde_json::from_slice(&body).ok()?;
    // Allow up to 24 hours of staleness — the cache is just for instant startup,
    // real data replaces it within seconds.
    let age = unix_now_secs().saturating_sub(cache.cached_at_unix);
    if age > 86400 {
        return None;
    }
    Some(cache)
}

// ---------------------------------------------------------------------------
// Claude CLI PTY probe — run `claude` in a pseudo-terminal, send `/usage`,
// parse the rendered TUI text to extract session/weekly usage percentages.
// ---------------------------------------------------------------------------

pub(super) async fn fetch_claude_limits_via_cli() -> Result<OfficialClaudeSnapshot> {
    let claude_bin = resolve_claude_binary()?;
    let bin = claude_bin.clone();
    let raw_output = tokio::task::spawn_blocking(move || claude_pty_capture(&bin))
        .await
        .context("claude pty task join failed")??;
    let clean = strip_ansi_codes(&raw_output);
    let debug = std::env::var("TU_DEBUG_CLI").is_ok();
    if debug {
        eprintln!("--- claude cli CLEAN output ({} bytes) ---", clean.len());
        eprintln!("{clean}");
        eprintln!("--- end claude cli output ---");
    }

    // Retry once if output doesn't look relevant (CodexBar approach).
    let looks_relevant = {
        let compact: String = clean
            .to_ascii_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        compact.contains("currentsession")
            || compact.contains("currentweek")
            || compact.contains("loadingusage")
            || compact.contains("failedtoloadusagedata")
    };
    if !looks_relevant {
        if debug {
            eprintln!("[pty] output looked like startup; retrying once...");
        }
        let bin2 = claude_bin;
        let raw2 = tokio::task::spawn_blocking(move || claude_pty_capture(&bin2))
            .await
            .context("claude pty retry join failed")??;
        let clean2 = strip_ansi_codes(&raw2);
        if debug {
            eprintln!(
                "--- claude cli RETRY CLEAN output ({} bytes) ---",
                clean2.len()
            );
            eprintln!("{clean2}");
            eprintln!("--- end claude cli retry output ---");
        }
        return parse_claude_usage_text(&clean2);
    }

    parse_claude_usage_text(&clean)
}

pub(super) fn resolve_claude_binary() -> Result<String> {
    // Check common locations.
    if let Ok(path) = std::env::var("CLAUDE_BIN") {
        if Path::new(&path).is_file() {
            return Ok(path);
        }
    }
    // `which claude`
    let output = Command::new("which")
        .arg("claude")
        .output()
        .context("failed to run `which claude`")?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() && Path::new(&path).is_file() {
            return Ok(path);
        }
    }
    // Well-known paths.
    for p in [
        dirs::home_dir().map(|h| h.join(".bun/bin/claude")),
        dirs::home_dir().map(|h| h.join(".npm-global/bin/claude")),
        Some(PathBuf::from("/usr/local/bin/claude")),
    ]
    .into_iter()
    .flatten()
    {
        if p.is_file() {
            return Ok(p.to_string_lossy().into_owned());
        }
    }
    bail!("Claude CLI binary not found. Install it or set CLAUDE_BIN env var.")
}

/// Open a PTY via `portable-pty`, spawn `claude --allowed-tools ""`,
/// send `/usage`, and collect the rendered TUI output.
pub(super) fn claude_pty_capture(binary: &str) -> Result<String> {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let debug = std::env::var("TU_DEBUG_CLI").is_ok();

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 50,
            cols: 160,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("openpty failed")?;

    let mut cmd = CommandBuilder::new(binary);
    cmd.args(["--allowed-tools", ""]);

    // Remove all Claude-related env vars to avoid nested-session detection.
    for (key, _) in std::env::vars() {
        if key == "CLAUDECODE" || key.starts_with("ANTHROPIC_") || key.starts_with("CLAUDE_CODE") {
            cmd.env_remove(&key);
        }
    }
    cmd.env("TERM", "xterm-256color");
    if let Some(home) = dirs::home_dir() {
        cmd.cwd(&home);
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .context("failed to spawn claude CLI")?;
    // Drop slave side in parent so reads on master detect EOF properly.
    drop(pair.slave);

    let reader = pair.master.try_clone_reader().context("clone pty reader")?;
    let mut writer = pair.master.take_writer().context("take pty writer")?;

    // Spawn a background reader thread. The PTY reader can block, so we
    // funnel all data into a channel that the main thread polls.
    let (tx, rx) = bounded::<Vec<u8>>(256);
    let reader_thread = thread::spawn(move || {
        let mut reader = reader;
        let mut buf = vec![0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Helper: drain all currently available chunks from the channel.
    let drain_rx = |rx: &Receiver<Vec<u8>>| -> String {
        let mut out = Vec::new();
        while let Ok(chunk) = rx.try_recv() {
            out.extend_from_slice(&chunk);
        }
        String::from_utf8_lossy(&out).into_owned()
    };

    // Helper: write to PTY master.
    let write_pty = |writer: &mut Box<dyn Write + Send>, text: &str| -> Result<()> {
        writer
            .write_all(text.as_bytes())
            .context("write to PTY failed")?;
        writer.flush().ok();
        Ok(())
    };

    // Wait for TUI to fully initialize. The Claude TUI needs time to render
    // the initial prompt. We look for the prompt character (❯) as a signal.
    let mut startup_buf = String::new();
    let startup_deadline = Instant::now() + Duration::from_secs(15);
    let mut last_startup_data = Instant::now();
    let mut sent_trust_accept = false;
    let mut saw_prompt = false;

    while Instant::now() < startup_deadline {
        let chunk = drain_rx(&rx);
        if !chunk.is_empty() {
            startup_buf.push_str(&chunk);
            last_startup_data = Instant::now();

            let clean = strip_ansi_codes(&startup_buf);
            let clean_lower = clean.to_ascii_lowercase();
            // Also check raw text (ANSI stripped may lose spaces between words).
            let raw_lower = startup_buf.to_ascii_lowercase();

            // Handle trust/safety prompts.
            // Claude CLI asks "Is this a project you created or one you trust?"
            // ANSI stripping may remove spaces, so match generously.
            let is_trust_dialog = (clean_lower.contains("trust") || raw_lower.contains("trust"))
                && (clean_lower.contains("enter to confirm")
                    || raw_lower.contains("enter to confirm")
                    || clean_lower.contains("entertoconfirm"));
            if !sent_trust_accept && is_trust_dialog {
                if debug {
                    eprintln!("[pty] detected trust/safety prompt, accepting...");
                }
                // Send Enter to confirm the default selection (❯ 1. Yes).
                let _ = write_pty(&mut writer, "\r");
                thread::sleep(Duration::from_millis(500));
                let _ = write_pty(&mut writer, "\r");
                sent_trust_accept = true;
                startup_buf.clear();
                last_startup_data = Instant::now();
                continue;
            }

            // Respond to command palette hints (compact match — no spaces).
            let compact_lower: String =
                clean_lower.chars().filter(|c| !c.is_whitespace()).collect();
            if compact_lower.contains("showplan") {
                let _ = write_pty(&mut writer, "\r");
            }

            // The Claude TUI shows ❯ when ready for input (main prompt, not
            // inside a trust/selection dialog — those also use ❯ for selection).
            let has_dialog = clean_lower.contains("trust")
                || raw_lower.contains("trust")
                || clean_lower.contains("entertoconfirm")
                || raw_lower.contains("enter to confirm");
            let is_main_prompt = (clean.contains('❯') || clean.contains("> ")) && !has_dialog;
            if is_main_prompt {
                saw_prompt = true;
                // Give a short settle after seeing prompt.
                thread::sleep(Duration::from_millis(300));
                let extra = drain_rx(&rx);
                if !extra.is_empty() {
                    startup_buf.push_str(&extra);
                }
                break;
            }
        }

        // Extended idle: wait longer (4s) since TUI startup can have gaps.
        if !startup_buf.is_empty()
            && Instant::now().duration_since(last_startup_data) > Duration::from_secs(4)
        {
            break;
        }

        thread::sleep(Duration::from_millis(80));
    }

    if debug {
        let clean = strip_ansi_codes(&startup_buf);
        eprintln!(
            "[pty] startup done ({} bytes, prompt={saw_prompt}), clean tail: {:?}",
            startup_buf.len(),
            clean
                .chars()
                .rev()
                .take(300)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        );
    }

    // Send /usage command.
    write_pty(&mut writer, "/usage\r")?;
    if debug {
        eprintln!("[pty] sent /usage command");
    }

    // Read output until we see usage data or timeout.
    // CodexBar approach: stop on specific labels, send Enter periodically to help TUI render,
    // settle for 2s after detecting stop patterns.
    let mut buffer = String::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last_enter_at = Instant::now();
    let send_enter_every = Duration::from_millis(800);
    let mut stopped_early = false;

    while Instant::now() < deadline {
        let chunk = drain_rx(&rx);
        if !chunk.is_empty() {
            buffer.push_str(&chunk);

            // Auto-respond to command palette prompts (compact match — no spaces).
            let chunk_compact: String = strip_ansi_codes(&chunk)
                .to_ascii_lowercase()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            if chunk_compact.contains("showplan") {
                let _ = write_pty(&mut writer, "\r");
            }

            // Check stop conditions using compact text (no whitespace).
            let clean_compact: String = strip_ansi_codes(&buffer)
                .to_ascii_lowercase()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();

            // Stop patterns (inspired by CodexBar): once we see these, the panel is rendering.
            let has_stop = clean_compact.contains("currentsession")
                || clean_compact.contains("currentweek")
                || clean_compact.contains("failedtoloadusagedata");
            if has_stop {
                stopped_early = true;
                break;
            }
        }

        // Send Enter periodically to help TUI render (CodexBar does this for /usage).
        if Instant::now().duration_since(last_enter_at) >= send_enter_every {
            let _ = write_pty(&mut writer, "\r");
            last_enter_at = Instant::now();
        }

        thread::sleep(Duration::from_millis(60));
    }

    // Settle: after detecting stop patterns, wait 2s more to capture the full panel.
    if stopped_early {
        let settle_deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < settle_deadline {
            let extra = drain_rx(&rx);
            if !extra.is_empty() {
                buffer.push_str(&extra);
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    if debug {
        eprintln!("[pty] read loop done, buffer len = {}", buffer.len());
    }

    // Clean up: send /exit and kill.
    let _ = write_pty(&mut writer, "/exit\r");
    thread::sleep(Duration::from_millis(200));
    let _ = child.kill();
    let _ = child.wait();
    drop(writer);
    drop(pair.master);
    let _ = reader_thread.join();

    if buffer.is_empty() {
        bail!("Claude CLI produced no output (timed out)");
    }
    Ok(buffer)
}

/// Strip ANSI escape codes from PTY output.
pub(super) fn strip_ansi_codes(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // ESC sequence.
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                // Read until terminating letter.
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() || c == '~' {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                // OSC sequence: ESC ] ... ST (or BEL).
                chars.next();
                for c in chars.by_ref() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            } else {
                // Other ESC sequences — skip next char.
                chars.next();
            }
        } else if ch == '\r' {
            // Ignore carriage returns.
            continue;
        } else {
            result.push(ch);
        }
    }
    result
}

/// Parse the cleaned `/usage` text to extract session/weekly usage percentages.
/// NOTE: Because TUI PTY output has ANSI cursor-movement sequences, when
/// stripped the words often run together (e.g. "Currentsession38%used").
/// We handle both spaced and compact forms.
pub(super) fn parse_claude_usage_text(text: &str) -> Result<OfficialClaudeSnapshot> {
    // Step 1: Trim to latest usage panel (like CodexBar's trimToLatestUsagePanel).
    // Find the last "Settings:" header containing "Usage" to skip startup fragments.
    let panel_text = trim_to_latest_usage_panel(text).unwrap_or(text);

    // Step 2: Build line-based search context.
    // Normalize each line by collapsing whitespace to single space (CodexBar approach).
    let lines: Vec<&str> = panel_text.lines().collect();
    let normalized_lines: Vec<String> = lines
        .iter()
        .map(|l| normalize_for_label_search(l))
        .collect();

    // Compact form for quick label existence checks.
    let compact: String = panel_text
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    // Step 3: Extract percentages with label-based line search.
    let mut session_pct = extract_pct_by_label("current session", &lines, &normalized_lines);
    let mut weekly_pct = extract_pct_by_label("current week", &lines, &normalized_lines);

    // Fallback: ordered percent scraping (CodexBar does this when labels match
    // but surrounding layout moved the percentages).
    let has_weekly_label = compact.contains("currentweek");
    if session_pct.is_none() || (has_weekly_label && weekly_pct.is_none()) {
        let ordered = all_percents_from_lines(&lines);
        if session_pct.is_none() {
            session_pct = ordered.first().copied();
        }
        if has_weekly_label && weekly_pct.is_none() {
            weekly_pct = ordered.get(1).copied();
        }
    }

    if session_pct.is_none() {
        bail!("Could not find 'Current session' usage in Claude CLI output");
    }

    // Step 4: Extract reset times.
    let session_reset = extract_reset_by_label("current session", &lines, &normalized_lines);
    let weekly_reset = if has_weekly_label {
        extract_reset_by_label("current week", &lines, &normalized_lines)
    } else {
        None
    };

    // Step 5: Extract plan type.
    let plan_type = extract_claude_plan_from_compact(&compact);

    Ok(OfficialClaudeSnapshot {
        plan_type,
        primary_used_percent: session_pct,
        secondary_used_percent: weekly_pct,
        primary_window_mins: Some(5 * 60),
        secondary_window_mins: Some(7 * 24 * 60),
        primary_resets_at: session_reset
            .as_deref()
            .and_then(parse_claude_reset_to_unix),
        secondary_resets_at: weekly_reset.as_deref().and_then(parse_claude_reset_to_unix),
    })
}

/// Trim to the latest "Settings: ... Usage ..." panel in the output.
/// This skips startup fragments (status bar, logo, etc.) that may contain stray percent values.
pub(super) fn trim_to_latest_usage_panel(text: &str) -> Option<&str> {
    // Find the last "Settings:" header.
    let settings_pos = text.to_ascii_lowercase().rfind("settings:")?;
    let tail = &text[settings_pos..];
    let tail_lower = tail.to_ascii_lowercase();
    // Must contain "Usage" tab indicator.
    if !tail_lower.contains("usage") {
        return None;
    }
    // Must have percent values with usage keywords, or "loading usage".
    let has_percent = tail_lower.contains('%');
    let has_usage_words = tail_lower.contains("used")
        || tail_lower.contains("left")
        || tail_lower.contains("remaining")
        || tail_lower.contains("available");
    let has_loading = tail_lower.contains("loading usage");
    if (has_percent && has_usage_words) || has_loading {
        Some(tail)
    } else {
        None
    }
}

/// Normalize text for label search: lowercase, collapse whitespace to single space.
pub(super) fn normalize_for_label_search(text: &str) -> String {
    text.to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extract percent "remaining" from lines near a label.
/// Returns the "used" percent (if text says "XX% used", returns XX;
/// if text says "XX% remaining/left", returns 100-XX).
pub(super) fn extract_pct_by_label(
    label: &str,
    lines: &[&str],
    normalized: &[String],
) -> Option<f64> {
    let norm_label = normalize_for_label_search(label);
    for (idx, norm_line) in normalized.iter().enumerate() {
        if !norm_line.contains(&norm_label) {
            continue;
        }
        // Scan up to 12 lines from the label for a percent value.
        for candidate in lines.iter().skip(idx).take(12) {
            if let Some((pct, is_used)) = percent_from_line(candidate) {
                return if is_used {
                    Some(pct)
                } else {
                    Some(100.0 - pct)
                };
            }
        }
    }
    None
}

/// Extract a percent value from a single line.
/// Returns (value, is_used_not_remaining).
/// Skips status-bar context lines (containing | with model names).
pub(super) fn percent_from_line(line: &str) -> Option<(f64, bool)> {
    // Skip status-bar context lines (e.g. "opus | 0% | sonnet").
    if line.contains('|') {
        let lower = line.to_ascii_lowercase();
        if ["opus", "sonnet", "haiku", "default"]
            .iter()
            .any(|m| lower.contains(m))
        {
            return None;
        }
    }

    // Find XX% pattern (allow Unicode whitespace before %).
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            // Skip optional whitespace/NBSP before %.
            let mut j = i;
            while j < bytes.len() {
                if bytes[j] == b' ' {
                    j += 1;
                } else if j + 1 < bytes.len() && bytes[j] == 0xc2 && bytes[j + 1] == 0xa0 {
                    j += 2; // U+00A0 NBSP
                } else {
                    break;
                }
            }
            if j < bytes.len() && bytes[j] == b'%' {
                let num_str = &line[start..i];
                if let Ok(val) = num_str.parse::<f64>() {
                    let clamped = val.clamp(0.0, 100.0);
                    let lower = line.to_ascii_lowercase();
                    let is_used = if ["used", "spent", "consumed"]
                        .iter()
                        .any(|k| lower.contains(k))
                    {
                        true
                    } else if ["remaining", "left", "available"]
                        .iter()
                        .any(|k| lower.contains(k))
                    {
                        false
                    } else {
                        // Default: Claude CLI shows "XX% used".
                        true
                    };
                    return Some((clamped, is_used));
                }
            }
        }
        i += 1;
    }
    None
}

/// Collect all percent values from the text (ordered, for fallback).
pub(super) fn all_percents_from_lines(lines: &[&str]) -> Vec<f64> {
    lines
        .iter()
        .filter_map(|l| {
            percent_from_line(l).map(|(pct, is_used)| if is_used { pct } else { 100.0 - pct })
        })
        .collect()
}

/// Extract "Resets ..." text near a label using line-based normalized search.
pub(super) fn extract_reset_by_label(
    label: &str,
    lines: &[&str],
    normalized: &[String],
) -> Option<String> {
    let norm_label = normalize_for_label_search(label);
    for (idx, norm_line) in normalized.iter().enumerate() {
        if !norm_line.contains(&norm_label) {
            continue;
        }
        // Scan up to 14 lines from the label for "Resets".
        for scan_line in lines.iter().skip(idx).take(14) {
            let scan_norm = normalize_for_label_search(scan_line);
            // Stop if we hit another "current" section.
            if scan_norm.starts_with("current ") && !scan_norm.contains(&norm_label) {
                break;
            }
            if scan_norm.contains("reset") || scan_norm.contains("reses") {
                if let Some(time_str) = extract_time_string(scan_line) {
                    return Some(time_str);
                }
            }
        }
    }
    None
}

/// Extract a time-like string from a line (e.g. "4:59pm", "2pm", "Mar 5, 3pm").
pub(super) fn extract_time_string(line: &str) -> Option<String> {
    // Find patterns like "HH:MMam/pm" or "Ham/pm" in the raw text.
    let lower = line.to_ascii_lowercase();

    // Look for "H:MMam/pm" pattern.
    for (i, _) in lower.char_indices() {
        let rest = &lower[i..];
        if rest.starts_with(|c: char| c.is_ascii_digit()) {
            // Try to match time pattern.
            let mut end = i;
            // Digits.
            while end < lower.len() && lower.as_bytes()[end].is_ascii_digit() {
                end += 1;
            }
            // Optional colon + digits.
            if end < lower.len() && lower.as_bytes()[end] == b':' {
                end += 1;
                while end < lower.len() && lower.as_bytes()[end].is_ascii_digit() {
                    end += 1;
                }
            }
            // am/pm.
            let after = &lower[end..];
            if after.starts_with("am") || after.starts_with("pm") {
                let time_part = &line[i..end + 2];
                // Check for timezone in parens after.
                let remaining = line[end + 2..].trim();
                if remaining.starts_with('(') {
                    if let Some(close) = remaining.find(')') {
                        return Some(format!("{} {}", time_part, &remaining[..=close]));
                    }
                }
                return Some(time_part.to_string());
            }
        }
    }
    None
}

/// Try to parse "4:59pm (Asia/Shanghai)" or "2pm (Asia/Shanghai)" into a Unix timestamp.
pub(super) fn parse_claude_reset_to_unix(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    // Extract timezone if present in parens.
    let (time_part, tz_str) = if let Some(paren_start) = text.find('(') {
        let tz = text[paren_start + 1..]
            .trim_end_matches(')')
            .trim()
            .to_string();
        (text[..paren_start].trim(), Some(tz))
    } else {
        (text, None)
    };

    let now = Local::now();

    // Try parsing with timezone.
    let tz: Option<Tz> = tz_str.as_deref().and_then(|s| s.parse::<Tz>().ok());

    // Try time-only formats: "4:59pm", "4:59PM", "16:59", "2pm".
    for fmt in &[
        "%I:%M%p", "%I:%M%P", "%I:%M %p", "%H:%M", "%I%p", "%I%P", "%I %p",
    ] {
        if let Ok(time) = chrono::NaiveTime::parse_from_str(time_part, fmt) {
            let today = now.date_naive();
            let dt = today.and_time(time);
            let ts = if let Some(tz) = tz {
                tz.from_local_datetime(&dt).single()?.timestamp()
            } else {
                now.timezone()
                    .from_local_datetime(&dt)
                    .single()?
                    .timestamp()
            };
            if ts <= now.timestamp() {
                return Some(ts + 86400);
            }
            return Some(ts);
        }
    }

    // Try date+time formats.
    for fmt in &[
        "%b %d, %I:%M%p",
        "%b %d, %I:%M %p",
        "%b %d %I:%M%p",
        "%b %d, %H:%M",
    ] {
        let with_year = format!("{} {}", now.format("%Y"), time_part);
        let fmt_with_year = format!("%Y {fmt}");
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&with_year, &fmt_with_year) {
            let ts = if let Some(tz) = tz {
                tz.from_local_datetime(&dt).single()?.timestamp()
            } else {
                now.timezone()
                    .from_local_datetime(&dt)
                    .single()?
                    .timestamp()
            };
            return Some(ts);
        }
    }

    None
}

/// Extract Claude plan name from compact text.
pub(super) fn extract_claude_plan_from_compact(compact: &str) -> Option<String> {
    for (pattern, label) in &[
        ("claudemax", "Claude Max"),
        ("claudepro", "Claude Pro"),
        ("claudeteam", "Claude Team"),
        ("claudeenterprise", "Claude Enterprise"),
    ] {
        if compact.contains(*pattern) {
            return Some(label.to_string());
        }
    }
    None
}

pub(super) fn load_claude_oauth_tokens() -> Result<(PathBuf, ClaudeOAuthTokens)> {
    let path = dirs::home_dir()
        .map(|home| home.join(".claude").join(".credentials.json"))
        .context("Failed to resolve Claude credentials path")?;
    let body = std::fs::read(&path)
        .with_context(|| format!("Failed to read Claude credentials file: {}", path.display()))?;
    let parsed: ClaudeCredentialsFile =
        serde_json::from_slice(&body).context("Invalid Claude .credentials.json format")?;
    let Some(tokens) = parsed.claude_ai_oauth else {
        bail!("Claude credentials missing claudeAiOauth payload");
    };
    let access_token = tokens
        .access_token
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if access_token.is_empty() {
        bail!("Claude credentials missing access token");
    }
    Ok((
        path,
        ClaudeOAuthTokens {
            access_token: Some(access_token.to_string()),
            ..tokens
        },
    ))
}

pub(super) fn save_claude_oauth_tokens(path: &Path, tokens: &ClaudeOAuthTokens) -> Result<()> {
    let existing = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let mut root = existing;
    if !root.is_object() {
        root = serde_json::json!({});
    }
    let obj = root
        .as_object_mut()
        .context("Claude credentials root must be object")?;
    let oauth_value = obj
        .entry("claudeAiOauth".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !oauth_value.is_object() {
        *oauth_value = serde_json::json!({});
    }
    let oauth = oauth_value
        .as_object_mut()
        .context("claudeAiOauth must be object")?;

    if let Some(access) = tokens.access_token.as_ref().filter(|v| !v.is_empty()) {
        oauth.insert(
            "accessToken".to_string(),
            serde_json::Value::String(access.clone()),
        );
    }
    if let Some(refresh) = tokens.refresh_token.as_ref().filter(|v| !v.is_empty()) {
        oauth.insert(
            "refreshToken".to_string(),
            serde_json::Value::String(refresh.clone()),
        );
    }
    if let Some(expires_at) = tokens.expires_at {
        if let Some(number) = serde_json::Number::from_f64(expires_at) {
            oauth.insert("expiresAt".to_string(), serde_json::Value::Number(number));
        }
    }
    if let Some(tier) = tokens.rate_limit_tier.as_ref().filter(|v| !v.is_empty()) {
        oauth.insert(
            "rateLimitTier".to_string(),
            serde_json::Value::String(tier.clone()),
        );
    }

    let bytes =
        serde_json::to_vec_pretty(&root).context("Failed to serialize Claude credentials file")?;
    std::fs::write(path, bytes).with_context(|| {
        format!(
            "Failed to write Claude credentials file: {}",
            path.display()
        )
    })?;
    Ok(())
}

pub(super) async fn fetch_claude_usage_with_access_token(
    access_token: &str,
    rate_limit_tier: Option<&str>,
) -> std::result::Result<OfficialClaudeSnapshot, ClaudeOAuthFetchError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| ClaudeOAuthFetchError::Other(anyhow::Error::new(error)))?;
    let response = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "tokenusage")
        .send()
        .await
        .map_err(|error| ClaudeOAuthFetchError::Other(anyhow::Error::new(error)))?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(ClaudeOAuthFetchError::Unauthorized);
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ClaudeOAuthFetchError::Other(anyhow::anyhow!(
            "Claude usage API returned {status}: {body}"
        )));
    }

    let usage: ClaudeOAuthUsageResponse = response
        .json()
        .await
        .map_err(|error| ClaudeOAuthFetchError::Other(anyhow::Error::new(error)))?;

    let weekly = usage
        .seven_day
        .or(usage.seven_day_oauth_apps)
        .or(usage.seven_day_opus)
        .or(usage.seven_day_sonnet);
    Ok(OfficialClaudeSnapshot {
        plan_type: infer_claude_plan_label(rate_limit_tier),
        primary_used_percent: usage
            .five_hour
            .as_ref()
            .and_then(|window| window.utilization)
            .map(normalize_official_used_percent),
        secondary_used_percent: weekly
            .as_ref()
            .and_then(|window| window.utilization)
            .map(normalize_official_used_percent),
        primary_window_mins: Some(5 * 60),
        secondary_window_mins: Some(7 * 24 * 60),
        primary_resets_at: usage
            .five_hour
            .as_ref()
            .and_then(|window| parse_iso8601_to_unix(window.resets_at.as_deref())),
        secondary_resets_at: weekly
            .as_ref()
            .and_then(|window| parse_iso8601_to_unix(window.resets_at.as_deref())),
    })
}

pub(super) fn infer_claude_plan_label(rate_limit_tier: Option<&str>) -> Option<String> {
    let tier = rate_limit_tier?.trim();
    if tier.is_empty() {
        return None;
    }
    let normalized = tier.to_ascii_lowercase();
    let label = if normalized.contains("enterprise") {
        "Claude Enterprise"
    } else if normalized.contains("team") {
        "Claude Team"
    } else if normalized.contains("max") {
        "Claude Max"
    } else if normalized.contains("pro") {
        "Claude Pro"
    } else {
        tier
    };
    Some(label.to_string())
}

pub(super) fn normalize_official_used_percent(raw: f64) -> f64 {
    if raw < 1.0 {
        (raw * 100.0).clamp(0.0, 100.0)
    } else {
        raw.clamp(0.0, 100.0)
    }
}

pub(super) fn parse_iso8601_to_unix(raw: Option<&str>) -> Option<i64> {
    let text = raw?.trim();
    if text.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(text)
        .ok()
        .or_else(|| DateTime::parse_from_str(text, "%+").ok())
        .map(|ts| ts.timestamp())
}

pub(super) async fn refresh_claude_access_token(
    refresh_token: &str,
) -> Result<(String, Option<String>, Option<i64>)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client for Claude refresh")?;
    let response = client
        .post(CLAUDE_OAUTH_REFRESH_URL)
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLAUDE_OAUTH_REFRESH_CLIENT_ID),
        ])
        .send()
        .await
        .context("Claude refresh request failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Claude refresh failed ({status}): {body}");
    }
    let payload: ClaudeRefreshResponse = response
        .json()
        .await
        .context("Invalid Claude refresh response")?;
    let expires_at_ms = payload
        .expires_in
        .map(|seconds| (Utc::now().timestamp() + seconds).saturating_mul(1000));
    Ok((payload.access_token, payload.refresh_token, expires_at_ms))
}

// ---------------------------------------------------------------------------
// Antigravity local probe — detect language_server_macos process, find port,
// query the Connect-protocol gRPC endpoints for quota data.
// ---------------------------------------------------------------------------

pub(super) async fn fetch_antigravity_official_limits() -> Result<OfficialAntigravitySnapshot> {
    let (pid, csrf_token, extension_port) = detect_antigravity_process().await?;
    let ports = antigravity_listening_ports(pid).await?;
    let connect_port = antigravity_find_working_port(&ports, &csrf_token).await?;
    let ctx = AntigravityRequestContext {
        https_port: connect_port,
        http_port: extension_port,
        csrf_token,
    };

    let snapshot = match antigravity_fetch_user_status(&ctx).await {
        Ok(snap) => snap,
        Err(_) => antigravity_fetch_command_model_configs(&ctx).await?,
    };
    Ok(snapshot)
}

pub(super) struct AntigravityRequestContext {
    https_port: u16,
    http_port: Option<u16>,
    csrf_token: String,
}

pub(super) async fn detect_antigravity_process() -> Result<(u32, String, Option<u16>)> {
    if cfg!(windows) {
        bail!(
            "Antigravity detection isn't supported on Windows yet — tu uses ps/lsof (macOS/Linux). \
             On Windows the language server runs as `agy` with a different port discovery; \
             see usage-tray-windows for a working implementation."
        );
    }
    let output = tokio::task::spawn_blocking(|| {
        Command::new("/bin/ps")
            .args(["-ax", "-o", "pid=,command="])
            .output()
            .context("failed to run ps for antigravity detection")
    })
    .await
    .context("ps task join failed")??;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (pid_str, cmd) = match trimmed.split_once(|c: char| c.is_whitespace()) {
            Some((p, c)) => (p.trim(), c),
            None => continue,
        };
        let lower = cmd.to_ascii_lowercase();
        if !lower.contains("language_server_macos") {
            continue;
        }
        let is_antigravity = (lower.contains("--app_data_dir") && lower.contains("antigravity"))
            || lower.contains("/antigravity/");
        if !is_antigravity {
            continue;
        }
        let pid: u32 = match pid_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let csrf = match extract_flag_value("--csrf_token", cmd) {
            Some(v) => v,
            None => continue,
        };
        let ext_port =
            extract_flag_value("--extension_server_port", cmd).and_then(|v| v.parse::<u16>().ok());
        return Ok((pid, csrf, ext_port));
    }
    bail!("Antigravity language server not detected")
}

pub(super) fn extract_flag_value(flag: &str, command: &str) -> Option<String> {
    let idx = command.find(flag)?;
    let after = &command[idx + flag.len()..];
    let after = after
        .strip_prefix('=')
        .unwrap_or_else(|| after.trim_start());
    let value = after.split_whitespace().next()?;
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

pub(super) async fn antigravity_listening_ports(pid: u32) -> Result<Vec<u16>> {
    let lsof = ["/usr/sbin/lsof", "/usr/bin/lsof"]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .context("lsof not available for antigravity port detection")?;

    let lsof = lsof.to_string();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(&lsof)
            .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-a", "-p", &pid.to_string()])
            .output()
            .context("failed to run lsof for antigravity")
    })
    .await
    .context("lsof task join failed")??;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut ports = std::collections::BTreeSet::new();
    for line in stdout.lines() {
        // Match lines like: ... TCP *:42150 (LISTEN)
        if let Some(listen_idx) = line.find("(LISTEN)") {
            let before = &line[..listen_idx];
            if let Some(colon_idx) = before.rfind(':') {
                let port_str = before[colon_idx + 1..].trim();
                if let Ok(port) = port_str.parse::<u16>() {
                    ports.insert(port);
                }
            }
        }
    }

    if ports.is_empty() {
        bail!("Antigravity process has no listening ports");
    }
    Ok(ports.into_iter().collect())
}

pub(super) async fn antigravity_find_working_port(ports: &[u16], csrf_token: &str) -> Result<u16> {
    let unleash_body = serde_json::json!({
        "context": {
            "properties": {
                "devMode": "false",
                "extensionVersion": "unknown",
                "hasAnthropicModelAccess": "true",
                "ide": "antigravity",
                "ideVersion": "unknown",
                "installationId": "tokenusage",
                "language": "UNSPECIFIED",
                "os": "macos",
                "requestedModelId": "MODEL_UNSPECIFIED",
            }
        }
    });
    let path = "/exa.language_server_pb.LanguageServerService/GetUnleashData";

    for &port in ports {
        let url = format!("https://127.0.0.1:{port}{path}");
        let result = antigravity_post(&url, csrf_token, &unleash_body).await;
        if result.is_ok() {
            return Ok(port);
        }
    }
    bail!(
        "no working Antigravity API port found among {} candidates",
        ports.len()
    )
}

pub(super) fn antigravity_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .no_proxy()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .context("failed to build HTTP client for antigravity")
}

pub(super) async fn antigravity_post(
    url: &str,
    csrf_token: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    let client = antigravity_http_client()?;
    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .header("X-Codeium-Csrf-Token", csrf_token)
        .json(body)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        bail!("Antigravity HTTP {status}: {text}");
    }
    response
        .json()
        .await
        .context("invalid JSON from antigravity")
}

pub(super) async fn antigravity_post_with_http_fallback(
    ctx: &AntigravityRequestContext,
    path: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    let https_url = format!("https://127.0.0.1:{}{path}", ctx.https_port);
    match antigravity_post(&https_url, &ctx.csrf_token, body).await {
        Ok(v) => Ok(v),
        Err(https_err) => {
            if let Some(http_port) = ctx.http_port {
                if http_port != ctx.https_port {
                    let http_url = format!("http://127.0.0.1:{http_port}{path}");
                    return antigravity_post(&http_url, &ctx.csrf_token, body).await;
                }
            }
            Err(https_err)
        }
    }
}

pub(super) fn antigravity_default_request_body() -> serde_json::Value {
    serde_json::json!({
        "metadata": {
            "ideName": "antigravity",
            "extensionName": "antigravity",
            "ideVersion": "unknown",
            "locale": "en",
        }
    })
}

pub(super) async fn antigravity_fetch_user_status(
    ctx: &AntigravityRequestContext,
) -> Result<OfficialAntigravitySnapshot> {
    let path = "/exa.language_server_pb.LanguageServerService/GetUserStatus";
    let body = antigravity_default_request_body();
    let resp = antigravity_post_with_http_fallback(ctx, path, &body).await?;
    parse_antigravity_user_status(&resp)
}

pub(super) async fn antigravity_fetch_command_model_configs(
    ctx: &AntigravityRequestContext,
) -> Result<OfficialAntigravitySnapshot> {
    let path = "/exa.language_server_pb.LanguageServerService/GetCommandModelConfigs";
    let body = antigravity_default_request_body();
    let resp = antigravity_post_with_http_fallback(ctx, path, &body).await?;
    parse_antigravity_command_model_configs(&resp)
}

pub(super) fn parse_antigravity_user_status(
    resp: &serde_json::Value,
) -> Result<OfficialAntigravitySnapshot> {
    // Check for error code
    if let Some(code) = resp.get("code") {
        let is_ok = match code {
            serde_json::Value::Number(n) => n.as_i64() == Some(0),
            serde_json::Value::String(s) => {
                let l = s.to_ascii_lowercase();
                l == "ok" || l == "success" || l == "0"
            }
            _ => true,
        };
        if !is_ok {
            bail!("Antigravity API error code: {code}");
        }
    }

    let user_status = resp
        .get("userStatus")
        .context("Antigravity response missing userStatus")?;

    let email = user_status
        .get("email")
        .and_then(|v| v.as_str())
        .map(String::from);

    let plan_name = user_status
        .pointer("/planStatus/planInfo")
        .and_then(|info| {
            let candidates = [
                "planDisplayName",
                "displayName",
                "productName",
                "planName",
                "planShortName",
            ];
            candidates
                .iter()
                .filter_map(|key| info.get(key)?.as_str())
                .find(|s| !s.trim().is_empty())
                .map(String::from)
        });

    let model_configs = user_status
        .pointer("/cascadeModelConfigData/clientModelConfigs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let models = parse_antigravity_model_quotas(&model_configs);
    Ok(build_antigravity_snapshot(plan_name, email, models))
}

pub(super) fn parse_antigravity_command_model_configs(
    resp: &serde_json::Value,
) -> Result<OfficialAntigravitySnapshot> {
    if let Some(code) = resp.get("code") {
        let is_ok = match code {
            serde_json::Value::Number(n) => n.as_i64() == Some(0),
            serde_json::Value::String(s) => {
                let l = s.to_ascii_lowercase();
                l == "ok" || l == "success" || l == "0"
            }
            _ => true,
        };
        if !is_ok {
            bail!("Antigravity API error code: {code}");
        }
    }

    let model_configs = resp
        .get("clientModelConfigs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let models = parse_antigravity_model_quotas(&model_configs);
    Ok(build_antigravity_snapshot(None, None, models))
}

pub(super) fn parse_antigravity_model_quotas(
    configs: &[serde_json::Value],
) -> Vec<AntigravityModelQuotaSnapshot> {
    configs
        .iter()
        .filter_map(|config| {
            let label = config.get("label")?.as_str()?.to_string();
            let model_id = config
                .pointer("/modelOrAlias/model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let quota = config.get("quotaInfo")?;
            let remaining_fraction = quota.get("remainingFraction").and_then(|v| v.as_f64());
            let reset_time = quota
                .get("resetTime")
                .and_then(|v| v.as_str())
                .and_then(parse_antigravity_reset_time);
            Some(AntigravityModelQuotaSnapshot {
                label,
                model_id,
                remaining_fraction,
                reset_time,
            })
        })
        .collect()
}

pub(super) fn parse_antigravity_reset_time(value: &str) -> Option<i64> {
    // Try as ISO 8601 first
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(dt.timestamp());
    }
    // Try as seconds since epoch
    if let Ok(secs) = value.parse::<f64>() {
        return Some(secs as i64);
    }
    None
}

pub(super) fn build_antigravity_snapshot(
    plan_type: Option<String>,
    account_email: Option<String>,
    models: Vec<AntigravityModelQuotaSnapshot>,
) -> OfficialAntigravitySnapshot {
    let ordered = select_antigravity_models(&models);

    let (primary_used, primary_label, primary_resets) = ordered
        .first()
        .map(|m| {
            let used = m.remaining_fraction.map(|f| (1.0 - f) * 100.0);
            (used, Some(m.label.clone()), m.reset_time)
        })
        .unwrap_or((None, None, None));

    let (secondary_used, secondary_label, secondary_resets) = ordered
        .get(1)
        .map(|m| {
            let used = m.remaining_fraction.map(|f| (1.0 - f) * 100.0);
            (used, Some(m.label.clone()), m.reset_time)
        })
        .unwrap_or((None, None, None));

    let (tertiary_used, tertiary_label, tertiary_resets) = ordered
        .get(2)
        .map(|m| {
            let used = m.remaining_fraction.map(|f| (1.0 - f) * 100.0);
            (used, Some(m.label.clone()), m.reset_time)
        })
        .unwrap_or((None, None, None));

    OfficialAntigravitySnapshot {
        plan_type,
        account_email,
        models,
        primary_used_percent: primary_used,
        secondary_used_percent: secondary_used,
        tertiary_used_percent: tertiary_used,
        primary_label,
        secondary_label,
        tertiary_label,
        primary_resets_at: primary_resets,
        secondary_resets_at: secondary_resets,
        tertiary_resets_at: tertiary_resets,
    }
}

/// Select and prioritise models the same way CodexBar does:
/// 1. Claude (non-thinking)  2. Gemini Pro Low  3. Gemini Flash.
///
/// Fallback: all models sorted by remaining % ascending.
pub(super) fn select_antigravity_models(
    models: &[AntigravityModelQuotaSnapshot],
) -> Vec<AntigravityModelQuotaSnapshot> {
    let mut ordered = Vec::new();

    if let Some(m) = models.iter().find(|m| {
        let l = m.label.to_ascii_lowercase();
        l.contains("claude") && !l.contains("thinking")
    }) {
        ordered.push(m.clone());
    }
    if let Some(m) = models.iter().find(|m| {
        let l = m.label.to_ascii_lowercase();
        l.contains("pro") && l.contains("low")
    }) {
        if !ordered.iter().any(|o| o.label == m.label) {
            ordered.push(m.clone());
        }
    }
    if let Some(m) = models.iter().find(|m| {
        let l = m.label.to_ascii_lowercase();
        l.contains("gemini") && l.contains("flash")
    }) {
        if !ordered.iter().any(|o| o.label == m.label) {
            ordered.push(m.clone());
        }
    }

    if ordered.is_empty() {
        let mut all: Vec<_> = models.to_vec();
        all.sort_by(|a, b| {
            let ra = a.remaining_fraction.unwrap_or(0.0);
            let rb = b.remaining_fraction.unwrap_or(0.0);
            ra.partial_cmp(&rb).unwrap_or(std::cmp::Ordering::Equal)
        });
        return all;
    }
    ordered
}

pub(super) fn fetch_codex_official_limits_blocking() -> Result<OfficialCodexSnapshot> {
    let rpc_script = r#"(
exec </dev/null;
printf '{"id":1,"method":"initialize","params":{"clientInfo":{"name":"tu","version":"official-limits"}}}\n';
sleep 0.3;
printf '{"method":"initialized","params":{}}\n';
sleep 0.3;
printf '{"id":2,"method":"account/rateLimits/read","params":{}}\n';
sleep 1.2;
printf '{"id":4,"method":"account/rateLimits/read","params":{}}\n';
sleep 1.2;
printf '{"id":3,"method":"account/read","params":{}}\n';
sleep 1.8;
) | script -q /dev/null codex -s read-only -a untrusted app-server"#;

    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 0..3 {
        let output = match Command::new("/bin/sh")
            .arg("-lc")
            .arg(rpc_script)
            .output()
            .context("failed to run codex app-server probe via script")
        {
            Ok(output) => output,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };

        let mut raw = String::new();
        raw.push_str(&String::from_utf8_lossy(&output.stdout));
        raw.push('\n');
        raw.push_str(&String::from_utf8_lossy(&output.stderr));

        match parse_codex_official_snapshot(&raw) {
            Ok(snapshot) => return Ok(snapshot),
            Err(error) => {
                last_error = Some(error);
                if attempt < 2 {
                    thread::sleep(Duration::from_millis(250));
                }
            }
        }
    }

    if let Some(error) = last_error {
        return Err(error);
    }
    bail!("codex app-server probe failed with unknown error")
}

pub(super) fn parse_codex_official_snapshot(raw: &str) -> Result<OfficialCodexSnapshot> {
    let mut rate_limits_value: Option<serde_json::Value> = None;
    let mut account_value: Option<serde_json::Value> = None;

    for chunk in extract_json_objects(raw) {
        let Ok(envelope) = serde_json::from_str::<RpcEnvelope>(&chunk) else {
            continue;
        };

        if let Some(error) = envelope.error {
            if let Some(message) = error.message {
                bail!("codex app-server error: {message}");
            }
            bail!("codex app-server returned error");
        }

        match rpc_envelope_id(&envelope) {
            Some(2) | Some(4) => rate_limits_value = envelope.result,
            Some(3) => account_value = envelope.result,
            _ => {}
        }
    }

    let rate_limits_value =
        rate_limits_value.context("codex app-server missing rateLimits response")?;
    let rate_limits: RpcRateLimitsReadResult = serde_json::from_value(rate_limits_value)
        .context("invalid account/rateLimits/read response")?;
    let account = account_value
        .and_then(|value| serde_json::from_value::<RpcAccountReadResult>(value).ok())
        .and_then(|res| res.account);

    let limits = rate_limits
        .rate_limits
        .context("rateLimits missing from Codex response")?;
    let primary_used_percent = limits
        .primary
        .as_ref()
        .and_then(|window| window.used_percent)
        .map(normalize_official_used_percent);
    let secondary_used_percent = limits
        .secondary
        .as_ref()
        .and_then(|window| window.used_percent)
        .map(normalize_official_used_percent);
    let primary_window_mins = limits
        .primary
        .as_ref()
        .and_then(|window| window.window_duration_mins);
    let secondary_window_mins = limits
        .secondary
        .as_ref()
        .and_then(|window| window.window_duration_mins);
    let primary_resets_at = limits.primary.as_ref().and_then(|window| window.resets_at);
    let secondary_resets_at = limits
        .secondary
        .as_ref()
        .and_then(|window| window.resets_at);

    Ok(OfficialCodexSnapshot {
        plan_type: limits
            .plan_type
            .or_else(|| account.and_then(|acc| acc.plan_type)),
        primary_used_percent,
        secondary_used_percent,
        primary_window_mins,
        secondary_window_mins,
        primary_resets_at,
        secondary_resets_at,
    })
}

pub(super) fn extract_json_objects(raw: &str) -> Vec<String> {
    let mut objects = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;

    for ch in raw.chars() {
        if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
            continue;
        }

        if depth == 0 {
            if ch == '{' {
                current.clear();
                current.push(ch);
                depth = 1;
                in_string = false;
                escape = false;
            }
            continue;
        }

        current.push(ch);

        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    objects.push(current.clone());
                    current.clear();
                }
            }
            _ => {}
        }
    }

    objects
}

pub(super) fn rpc_envelope_id(envelope: &RpcEnvelope) -> Option<i64> {
    envelope.id.as_ref().and_then(|id| {
        if let Some(v) = id.as_i64() {
            Some(v)
        } else {
            id.as_u64().map(|v| v as i64)
        }
    })
}

// ---------------------------------------------------------------------------
// DeepSeek API — credit balance probe.
// Reads DEEPSEEK_API_KEY from env and hits /user/balance.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OfficialDeepSeekSnapshot {
    pub is_available: bool,
    pub currency: Option<String>,
    pub total_balance: Option<f64>,
    pub granted_balance: Option<f64>,
    pub topped_up_balance: Option<f64>,
}

pub(super) async fn fetch_deepseek_official_limits() -> Result<OfficialDeepSeekSnapshot> {
    let api_key = std::env::var("DEEPSEEK_API_KEY")
        .context("DEEPSEEK_API_KEY environment variable not set")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("failed to build HTTP client for DeepSeek")?;
    let response = client
        .get("https://api.deepseek.com/user/balance")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .send()
        .await
        .context("DeepSeek balance request failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("DeepSeek balance API returned {status}: {body}");
    }
    let body: serde_json::Value = response
        .json()
        .await
        .context("Invalid DeepSeek response JSON")?;
    let is_available = body
        .get("is_available")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let infos = body.get("balance_infos").and_then(|v| v.as_array());
    let first = infos.and_then(|arr| arr.first());
    let parse_balance = |key: &str| -> Option<f64> {
        first?
            .get(key)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
    };
    Ok(OfficialDeepSeekSnapshot {
        is_available,
        currency: first
            .and_then(|o| o.get("currency"))
            .and_then(|v| v.as_str())
            .map(String::from),
        total_balance: parse_balance("total_balance"),
        granted_balance: parse_balance("granted_balance"),
        topped_up_balance: parse_balance("topped_up_balance"),
    })
}

// ---------------------------------------------------------------------------
// OpenRouter — API credit balance probe.
// Reads OPENROUTER_API_KEY from env and hits /api/v1/auth/key.
// Note: OpenRouter model pricing is already used for token cost calculation.
// This adds account credit balance tracking on top.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OfficialOpenRouterSnapshot {
    pub label: Option<String>,
    pub credits_used: Option<f64>,
    pub credits_limit: Option<f64>,
    pub used_percent: Option<f64>,
    pub is_free_tier: bool,
}

pub(super) async fn fetch_openrouter_account_limits() -> Result<OfficialOpenRouterSnapshot> {
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .context("OPENROUTER_API_KEY environment variable not set")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("failed to build HTTP client for OpenRouter")?;
    let response = client
        .get("https://openrouter.ai/api/v1/auth/key")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .send()
        .await
        .context("OpenRouter key info request failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("OpenRouter API returned {status}: {body}");
    }
    let body: serde_json::Value = response
        .json()
        .await
        .context("Invalid OpenRouter response JSON")?;
    let data = body
        .get("data")
        .context("missing 'data' field in OpenRouter /auth/key response")?;
    let used = data.get("usage").and_then(|v| v.as_f64());
    let limit = data.get("limit").and_then(|v| v.as_f64());
    let used_percent = used.zip(limit).and_then(|(u, l)| {
        if l > 0.0 {
            Some((u / l * 100.0).clamp(0.0, 100.0))
        } else {
            None
        }
    });
    Ok(OfficialOpenRouterSnapshot {
        label: data
            .get("label")
            .and_then(|v| v.as_str())
            .map(String::from),
        credits_used: used,
        credits_limit: limit,
        used_percent,
        is_free_tier: data
            .get("is_free_tier")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

// ---------------------------------------------------------------------------
// Grok (xAI) — credit balance probe.
// Reads XAI_API_KEY from env and hits /v1/dashboard/billing/credit_grants.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OfficialGrokSnapshot {
    pub total_granted: Option<f64>,
    pub total_used: Option<f64>,
    pub total_remaining: Option<f64>,
    pub used_percent: Option<f64>,
    pub currency: Option<String>,
}

pub(super) async fn fetch_grok_official_limits() -> Result<OfficialGrokSnapshot> {
    let api_key = std::env::var("XAI_API_KEY")
        .context("XAI_API_KEY environment variable not set")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("failed to build HTTP client for Grok")?;
    let response = client
        .get("https://api.x.ai/v1/dashboard/billing/credit_grants")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .send()
        .await
        .context("Grok billing request failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Grok billing API returned {status}: {body}");
    }
    let body: serde_json::Value = response
        .json()
        .await
        .context("Invalid Grok response JSON")?;
    let total_granted = body
        .get("total_granted_credits")
        .and_then(|v| v.as_f64());
    let total_used = body.get("total_used_credits").and_then(|v| v.as_f64());
    let total_remaining = body
        .get("total_remaining_credits")
        .and_then(|v| v.as_f64());
    let used_percent = total_used.zip(total_granted).and_then(|(u, g)| {
        if g > 0.0 {
            Some((u / g * 100.0).clamp(0.0, 100.0))
        } else {
            None
        }
    });
    Ok(OfficialGrokSnapshot {
        total_granted,
        total_used,
        total_remaining,
        used_percent,
        currency: Some("USD".to_string()),
    })
}

// ---------------------------------------------------------------------------
// Kimi (Moonshot AI) — credit balance probe.
// Reads MOONSHOT_API_KEY from env and hits /v1/users/me/balance.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OfficialKimiSnapshot {
    pub available_balance: Option<f64>,
    pub voucher_balance: Option<f64>,
    pub cash_balance: Option<f64>,
    pub currency: Option<String>,
}

pub(super) async fn fetch_kimi_official_limits() -> Result<OfficialKimiSnapshot> {
    let api_key = std::env::var("MOONSHOT_API_KEY")
        .context("MOONSHOT_API_KEY environment variable not set")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("failed to build HTTP client for Kimi")?;
    let response = client
        .get("https://api.moonshot.cn/v1/users/me/balance")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .send()
        .await
        .context("Kimi balance request failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Kimi balance API returned {status}: {body}");
    }
    let body: serde_json::Value = response
        .json()
        .await
        .context("Invalid Kimi response JSON")?;
    // Response: {"data":{"available_balance":100.0,"voucher_balance":20.0,"cash_balance":80.0}}
    let data = body.get("data").unwrap_or(&body);
    Ok(OfficialKimiSnapshot {
        available_balance: data
            .get("available_balance")
            .and_then(|v| v.as_f64()),
        voucher_balance: data.get("voucher_balance").and_then(|v| v.as_f64()),
        cash_balance: data.get("cash_balance").and_then(|v| v.as_f64()),
        currency: Some("CNY".to_string()),
    })
}

// ---------------------------------------------------------------------------
// Anthropic Developer API — usage probe.
// Distinct from the Claude consumer OAuth (already supported in this file).
// Reads ANTHROPIC_API_KEY from env and hits the organization usage endpoint.
// Note: Full cost reports require an ANTHROPIC_ADMIN_KEY (sk-ant-admin...).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OfficialAnthropicApiSnapshot {
    pub input_tokens_today: Option<u64>,
    pub output_tokens_today: Option<u64>,
    pub cache_read_tokens_today: Option<u64>,
    pub cost_usd_today: Option<f64>,
}

pub(super) async fn fetch_anthropic_api_limits() -> Result<OfficialAnthropicApiSnapshot> {
    // Prefer admin key for org-level cost reports; fall back to standard key.
    let api_key = std::env::var("ANTHROPIC_ADMIN_KEY")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .context("ANTHROPIC_API_KEY or ANTHROPIC_ADMIN_KEY environment variable not set")?;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("failed to build HTTP client for Anthropic API")?;
    let response = client
        .get("https://api.anthropic.com/v1/organizations/usage")
        .query(&[("start_date", today.as_str()), ("end_date", today.as_str())])
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Accept", "application/json")
        .send()
        .await
        .context("Anthropic API usage request failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Anthropic usage API returned {status}: {body}");
    }
    let body: serde_json::Value = response
        .json()
        .await
        .context("Invalid Anthropic usage response JSON")?;
    // Sum up across all models in the response data array.
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut cache_read_tokens: u64 = 0;
    let mut cost_usd: f64 = 0.0;
    if let Some(data) = body.get("data").and_then(|v| v.as_array()) {
        for entry in data {
            input_tokens += entry
                .get("input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            output_tokens += entry
                .get("output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            cache_read_tokens += entry
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            cost_usd += entry
                .get("cost_usd")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
        }
    }
    Ok(OfficialAnthropicApiSnapshot {
        input_tokens_today: if input_tokens > 0 {
            Some(input_tokens)
        } else {
            None
        },
        output_tokens_today: if output_tokens > 0 {
            Some(output_tokens)
        } else {
            None
        },
        cache_read_tokens_today: if cache_read_tokens > 0 {
            Some(cache_read_tokens)
        } else {
            None
        },
        cost_usd_today: if cost_usd > 0.0 { Some(cost_usd) } else { None },
    })
}
