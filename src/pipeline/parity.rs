use anyhow::{Context, Result, bail};
use serde::Serialize;

use super::{ReportPeriod, collect_report, emit_json};
use crate::cli::{ParityArgs, ParityOpencodeScope, ParityPeriod, ProviderArg};
use crate::types::TokenCounts;

#[derive(Debug, Serialize)]
struct ParityOutput {
    provider: String,
    period: String,
    tu: TokenCounts,
    tu_parity: TokenCounts,
    ccusage: TokenCounts,
    delta: TokenCountsDelta,
    within_threshold: bool,
}

#[derive(Debug, Default, Serialize)]
struct TokenCountsDelta {
    input_tokens: i128,
    cache_creation_input_tokens: i128,
    cache_read_input_tokens: i128,
    output_tokens: i128,
    reasoning_output_tokens: i128,
    total_tokens: i128,
    cost_usd: f64,
}

pub(crate) async fn run_parity(args: ParityArgs) -> Result<()> {
    let mut common = args.common.clone();
    common.only = vec![args.provider];
    common.sources.clear();
    apply_opencode_scope_compat(&mut common, args.provider, args.opencode_scope);
    apply_provider_parity_compat(&mut common, args.provider, args.period);

    let period = match args.period {
        ParityPeriod::Daily => ReportPeriod::Daily,
        ParityPeriod::Weekly => ReportPeriod::Weekly,
        ParityPeriod::Monthly => ReportPeriod::Monthly,
    };

    let week_start = parity_week_start(args.provider, args.period);
    let tu_report = collect_report(common.clone(), period, false, None, week_start).await?;
    let tu_totals = tu_report.totals;
    let tu_parity_totals = normalize_tu_for_provider(args.provider, &tu_totals);
    let cc_totals = fetch_ccusage_totals(&common, args.provider, args.period)?;
    let delta = TokenCountsDelta {
        input_tokens: tu_parity_totals.input_tokens as i128 - cc_totals.input_tokens as i128,
        cache_creation_input_tokens: tu_parity_totals.cache_creation_input_tokens as i128
            - cc_totals.cache_creation_input_tokens as i128,
        cache_read_input_tokens: tu_parity_totals.cache_read_input_tokens as i128
            - cc_totals.cache_read_input_tokens as i128,
        output_tokens: tu_parity_totals.output_tokens as i128 - cc_totals.output_tokens as i128,
        reasoning_output_tokens: tu_parity_totals.reasoning_output_tokens as i128
            - cc_totals.reasoning_output_tokens as i128,
        total_tokens: tu_parity_totals.total_tokens as i128 - cc_totals.total_tokens as i128,
        cost_usd: tu_parity_totals.cost_usd - cc_totals.cost_usd,
    };

    let out = ParityOutput {
        provider: provider_name(args.provider).to_string(),
        period: match args.period {
            ParityPeriod::Daily => "daily".to_string(),
            ParityPeriod::Weekly => "weekly".to_string(),
            ParityPeriod::Monthly => "monthly".to_string(),
        },
        tu: tu_totals,
        tu_parity: tu_parity_totals,
        ccusage: cc_totals,
        delta,
        within_threshold: true,
    };
    let token_abs = out.delta.total_tokens.unsigned_abs();
    let cost_abs = out.delta.cost_usd.abs();
    let within_threshold =
        token_abs <= args.max_token_delta as u128 && cost_abs <= args.max_cost_delta;
    let out = ParityOutput {
        within_threshold,
        ..out
    };
    if args.fail_on_delta && !within_threshold {
        bail!(
            "parity delta exceeds thresholds: total_tokens={} (max {}), cost_usd={:.6} (max {:.6})",
            out.delta.total_tokens,
            args.max_token_delta,
            out.delta.cost_usd,
            args.max_cost_delta
        );
    }

    emit_json(&out, args.common.jq.as_deref())
}

fn normalize_tu_for_provider(provider: ProviderArg, raw: &TokenCounts) -> TokenCounts {
    match provider {
        ProviderArg::Codex => {
            // @ccusage/codex reports "inputTokens" as raw input (including cached input),
            // and totalTokens as inputTokens + outputTokens (reasoning tracked separately).
            let input_tokens = raw.input_tokens + raw.cache_read_input_tokens;
            let output_tokens = raw.output_tokens;
            TokenCounts {
                input_tokens,
                cache_creation_input_tokens: raw.cache_creation_input_tokens,
                cache_read_input_tokens: raw.cache_read_input_tokens,
                output_tokens,
                reasoning_output_tokens: raw.reasoning_output_tokens,
                total_tokens: input_tokens + output_tokens,
                cost_usd: raw.cost_usd,
            }
        }
        ProviderArg::Claude | ProviderArg::Gemini | ProviderArg::Opencode => {
            // ccusage-family totals generally exclude reasoning as a separate additive bucket.
            TokenCounts {
                input_tokens: raw.input_tokens,
                cache_creation_input_tokens: raw.cache_creation_input_tokens,
                cache_read_input_tokens: raw.cache_read_input_tokens,
                output_tokens: raw.output_tokens,
                reasoning_output_tokens: raw.reasoning_output_tokens,
                total_tokens: raw.input_tokens
                    + raw.cache_creation_input_tokens
                    + raw.cache_read_input_tokens
                    + raw.output_tokens,
                cost_usd: raw.cost_usd,
            }
        }
    }
}

fn parity_week_start(provider: ProviderArg, period: ParityPeriod) -> crate::cli::WeekStart {
    if provider == ProviderArg::Opencode && period == ParityPeriod::Weekly {
        // ccusage-opencode weekly buckets use ISO week semantics (Monday-start).
        crate::cli::WeekStart::Monday
    } else {
        crate::cli::WeekStart::Sunday
    }
}

fn apply_provider_parity_compat(
    common: &mut crate::cli::CommonArgs,
    provider: ProviderArg,
    period: ParityPeriod,
) {
    // Keep normal reports untouched; only parity mode emulates upstream quirks.
    if provider == ProviderArg::Opencode
        && matches!(period, ParityPeriod::Daily | ParityPeriod::Monthly)
        && common.timezone.is_none()
    {
        // ccusage-opencode daily/monthly grouping currently keys by UTC date strings.
        common.timezone = Some("UTC".to_string());
    }
}

fn apply_opencode_scope_compat(
    common: &mut crate::cli::CommonArgs,
    provider: ProviderArg,
    scope: ParityOpencodeScope,
) {
    if provider == ProviderArg::Opencode && scope == ParityOpencodeScope::LegacyOnly {
        common.ignore_path.push("opencode.db".to_string());
    }
}

fn provider_name(provider: ProviderArg) -> &'static str {
    match provider {
        ProviderArg::Claude => "claude",
        ProviderArg::Codex => "codex",
        ProviderArg::Gemini => "gemini",
        ProviderArg::Opencode => "opencode",
    }
}

fn fetch_ccusage_totals(
    common: &crate::cli::CommonArgs,
    provider: ProviderArg,
    period: ParityPeriod,
) -> Result<TokenCounts> {
    if provider == ProviderArg::Codex && period == ParityPeriod::Weekly {
        bail!(
            "codex parity does not support weekly in @ccusage/codex; use --period daily or --period monthly"
        );
    }
    let package = match provider {
        ProviderArg::Claude => "ccusage@latest",
        ProviderArg::Codex => "@ccusage/codex@latest",
        ProviderArg::Gemini => "@ccusage/gemini@latest",
        ProviderArg::Opencode => "@ccusage/opencode@latest",
    };
    let report = match period {
        ParityPeriod::Daily => "daily",
        ParityPeriod::Weekly => "weekly",
        ParityPeriod::Monthly => "monthly",
    };

    let mut args = Vec::<String>::new();
    args.push(report.to_string());
    args.push("--json".to_string());
    if let Some(since) = common.since.as_deref() {
        args.push("--since".to_string());
        args.push(to_ccusage_date(since));
    }
    if let Some(until) = common.until.as_deref() {
        args.push("--until".to_string());
        args.push(to_ccusage_date(until));
    }
    if common.offline {
        args.push("--offline".to_string());
    }
    if let Some(tz) = common.timezone.as_deref() {
        args.push("--timezone".to_string());
        args.push(tz.to_string());
    }

    let output = run_ccusage_command(package, &args)?;
    if !output.status.success() {
        bail!(
            "ccusage parity command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_ccusage_totals(&output.stdout)
}

fn to_ccusage_date(input: &str) -> String {
    input.replace('-', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_payload_finds_embedded_object() {
        let text = "WARN x\n{\"totals\":{\"input_tokens\":1}}\n";
        let json = extract_json_payload(text).expect("json payload");
        assert_eq!(json, "{\"totals\":{\"input_tokens\":1}}");
    }

    #[test]
    fn parse_ccusage_totals_supports_camel_case() {
        let raw =
            br#"{"totals":{"inputTokens":2,"outputTokens":3,"totalTokens":5,"costUSD":1.25}}"#;
        let totals = parse_ccusage_totals(raw).expect("parsed");
        assert_eq!(totals.input_tokens, 2);
        assert_eq!(totals.output_tokens, 3);
        assert_eq!(totals.total_tokens, 5);
        assert!((totals.cost_usd - 1.25).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_ccusage_totals_supports_opencode_shape() {
        let raw = br#"{"totals":{"inputTokens":10,"outputTokens":20,"cacheCreationTokens":30,"cacheReadTokens":40,"totalTokens":100,"totalCost":12.5}}"#;
        let totals = parse_ccusage_totals(raw).expect("parsed");
        assert_eq!(totals.input_tokens, 10);
        assert_eq!(totals.output_tokens, 20);
        assert_eq!(totals.cache_creation_input_tokens, 30);
        assert_eq!(totals.cache_read_input_tokens, 40);
        assert_eq!(totals.total_tokens, 100);
        assert!((totals.cost_usd - 12.5).abs() < f64::EPSILON);
    }
}

fn parse_ccusage_totals(raw: &[u8]) -> Result<TokenCounts> {
    let text = String::from_utf8_lossy(raw);
    let json_slice = extract_json_payload(&text).unwrap_or(text.as_ref());
    let value: serde_json::Value =
        serde_json::from_str(json_slice).context("Invalid ccusage JSON output")?;
    let totals = value
        .get("totals")
        .cloned()
        .or_else(|| value.get("total").cloned())
        .context("ccusage JSON does not include totals")?;

    let input_tokens = extract_u64(&totals, &["input_tokens", "inputTokens"]).unwrap_or(0);
    let cache_creation_input_tokens = extract_u64(
        &totals,
        &[
            "cache_creation_input_tokens",
            "cacheCreationInputTokens",
            "cache_write_input_tokens",
            "cacheCreationTokens",
        ],
    )
    .unwrap_or(0);
    let cache_read_input_tokens = extract_u64(
        &totals,
        &[
            "cache_read_input_tokens",
            "cacheReadInputTokens",
            "cached_input_tokens",
            "cachedInputTokens",
            "cacheReadTokens",
        ],
    )
    .unwrap_or(0);
    let output_tokens = extract_u64(&totals, &["output_tokens", "outputTokens"]).unwrap_or(0);
    let reasoning_output_tokens = extract_u64(
        &totals,
        &["reasoning_output_tokens", "reasoningOutputTokens"],
    )
    .unwrap_or(0);
    let total_tokens =
        extract_u64(&totals, &["total_tokens", "totalTokens"]).unwrap_or_else(|| {
            input_tokens
                + cache_creation_input_tokens
                + cache_read_input_tokens
                + output_tokens
                + reasoning_output_tokens
        });
    let cost_usd = extract_f64(
        &totals,
        &[
            "cost_usd",
            "costUSD",
            "totalCostUSD",
            "totalCost",
            "cost",
        ],
    )
    .unwrap_or(0.0);

    Ok(TokenCounts {
        input_tokens,
        cache_creation_input_tokens,
        cache_read_input_tokens,
        output_tokens,
        reasoning_output_tokens,
        total_tokens,
        cost_usd,
    })
}

fn run_ccusage_command(package: &str, args: &[String]) -> Result<std::process::Output> {
    let npx_bin = if cfg!(target_os = "windows") {
        "npx.cmd"
    } else {
        "npx"
    };
    let mut npx = std::process::Command::new(npx_bin);
    npx.arg(package);
    for arg in args {
        npx.arg(arg);
    }
    if let Ok(output) = npx.output() {
        return Ok(output);
    }

    let npm_bin = if cfg!(target_os = "windows") {
        "npm.cmd"
    } else {
        "npm"
    };
    let mut npm = std::process::Command::new(npm_bin);
    npm.arg("exec")
        .arg("--package")
        .arg(package)
        .arg("--")
        .arg("ccusage");
    for arg in args {
        npm.arg(arg);
    }
    npm.output()
        .with_context(|| format!("Failed to run npx or npm exec for package {package}"))
}

fn extract_json_payload(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        return None;
    }
    Some(&text[start..=end])
}

fn extract_u64(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        value.get(key).and_then(|v| match v {
            serde_json::Value::Number(n) => {
                n.as_u64().or_else(|| n.as_i64().map(|x| x.max(0) as u64))
            }
            serde_json::Value::String(s) => s.parse::<u64>().ok(),
            _ => None,
        })
    })
}

fn extract_f64(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value.get(key).and_then(|v| match v {
            serde_json::Value::Number(n) => n.as_f64(),
            serde_json::Value::String(s) => s.parse::<f64>().ok(),
            _ => None,
        })
    })
}
