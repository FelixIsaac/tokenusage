use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::types::{ContextTierRate, PricingRate, TokenRates};

use super::*;

pub(super) fn openrouter_rate(pricing: &OpenRouterPricingEntry) -> Option<PricingRate> {
    let input_per_million = openrouter_token_price_per_million(pricing.prompt.as_ref())?;
    let output_per_million = openrouter_token_price_per_million(pricing.completion.as_ref())?;
    let cache_read_per_million =
        openrouter_token_price_per_million(pricing.input_cache_read.as_ref()).unwrap_or(0.0);
    let cache_creation_per_million =
        openrouter_token_price_per_million(pricing.input_cache_write.as_ref()).unwrap_or(0.0);

    Some(PricingRate {
        input_per_million,
        output_per_million,
        cache_creation_per_million,
        cache_read_per_million,
        // Reasoning tokens are already represented inside output tokens in our parser.
        reasoning_output_per_million: 0.0,
        ..PricingRate::default()
    })
}

pub(super) fn openrouter_token_price_per_million(value: Option<&OpenRouterNumber>) -> Option<f64> {
    let per_token = match value? {
        OpenRouterNumber::Number(value) => *value,
        OpenRouterNumber::String(raw) => f64::from_str(raw).ok()?,
    };

    if !per_token.is_finite() || per_token < 0.0 {
        return None;
    }

    Some(per_token * 1_000_000.0)
}

pub(super) fn openrouter_model_aliases(id: &str) -> Vec<String> {
    let normalized = canonical_model_name(id);
    if normalized.is_empty() {
        return Vec::new();
    }

    let mut aliases = HashSet::new();
    aliases.insert(normalized.clone());

    if let Some(tail) = normalized.rsplit('/').next() {
        aliases.insert(tail.to_string());
        if let Some(stripped) = strip_model_date_suffix(tail) {
            aliases.insert(stripped.to_string());
        }
        if tail.contains('.') {
            aliases.insert(tail.replace('.', "-"));
        }
    }

    aliases.into_iter().collect()
}

pub(super) fn strip_model_date_suffix(model: &str) -> Option<&str> {
    let (head, tail) = model.rsplit_once('-')?;
    if tail.len() == 8 && tail.chars().all(|ch| ch.is_ascii_digit()) {
        Some(head)
    } else {
        None
    }
}

pub(super) fn canonical_model_name(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

pub(super) fn openrouter_pricing_cache_path() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("tokenusage").join("openrouter-pricing-v1.json"))
}

pub(super) fn litellm_pricing_cache_path() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("tokenusage").join("litellm-pricing-v1.json"))
}

const PRICING_BINARY_MAGIC: &[u8; 4] = b"TUP1";

pub(super) fn load_openrouter_pricing_cache(path: &Path) -> Option<OpenRouterPricingCacheStore> {
    load_pricing_cache(path, OPENROUTER_PRICING_CACHE_VERSION)
}

pub(super) fn load_pricing_cache(
    path: &Path,
    expected_version: u32,
) -> Option<OpenRouterPricingCacheStore> {
    let body = std::fs::read(path).ok()?;
    if body.starts_with(PRICING_BINARY_MAGIC) {
        let store = decode_pricing_cache_binary(&body)?;
        return (store.version == expected_version).then_some(store);
    }

    // Legacy JSON cache: parse and transparently upgrade to TUP1 binary on disk
    // so subsequent CLI invocations avoid JSON parsing overhead.
    let cache: OpenRouterPricingCacheStore = serde_json::from_slice(&body).ok()?;
    if cache.version != expected_version {
        return None;
    }
    write_pricing_cache_binary(path, cache.version, cache.fetched_unix, &cache.exact);
    Some(cache)
}

pub(super) fn save_pricing_cache(
    path: &Path,
    version: u32,
    fetched_unix: u64,
    exact: &HashMap<String, PricingRate>,
) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    write_pricing_cache_binary(path, version, fetched_unix, exact);
}

fn write_pricing_cache_binary(
    path: &Path,
    version: u32,
    fetched_unix: u64,
    exact: &HashMap<String, PricingRate>,
) {
    let bytes = encode_pricing_cache_binary(version, fetched_unix, exact);
    let _ = std::fs::write(path, bytes);
}

fn write_token_rates_bin(out: &mut Vec<u8>, r: &TokenRates) {
    out.extend_from_slice(&r.input_per_million.to_le_bytes());
    out.extend_from_slice(&r.output_per_million.to_le_bytes());
    out.extend_from_slice(&r.cache_creation_per_million.to_le_bytes());
    out.extend_from_slice(&r.cache_creation_1h_per_million.to_le_bytes());
    out.extend_from_slice(&r.cache_read_per_million.to_le_bytes());
    out.extend_from_slice(&r.reasoning_output_per_million.to_le_bytes());
}

fn read_token_rates_bin(data: &[u8], pos: &mut usize) -> Option<TokenRates> {
    if pos.checked_add(48)? > data.len() {
        return None;
    }
    let read_f64 =
        |offset: usize| -> f64 { f64::from_le_bytes(data[offset..offset + 8].try_into().unwrap()) };
    let rates = TokenRates {
        input_per_million: read_f64(*pos),
        output_per_million: read_f64(*pos + 8),
        cache_creation_per_million: read_f64(*pos + 16),
        cache_creation_1h_per_million: read_f64(*pos + 24),
        cache_read_per_million: read_f64(*pos + 32),
        reasoning_output_per_million: read_f64(*pos + 40),
    };
    *pos += 48;
    Some(rates)
}

fn encode_pricing_cache_binary(
    version: u32,
    fetched_unix: u64,
    exact: &HashMap<String, PricingRate>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + exact.len() * 72);
    out.extend_from_slice(PRICING_BINARY_MAGIC);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&fetched_unix.to_le_bytes());
    let count = u32::try_from(exact.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&count.to_le_bytes());

    for (key, rate) in exact.iter().take(count as usize) {
        let key_bytes = key.as_bytes();
        let key_len = u32::try_from(key_bytes.len()).unwrap_or(0);
        out.extend_from_slice(&key_len.to_le_bytes());
        out.extend_from_slice(&key_bytes[..key_len as usize]);

        out.extend_from_slice(&rate.input_per_million.to_le_bytes());
        out.extend_from_slice(&rate.output_per_million.to_le_bytes());
        out.extend_from_slice(&rate.cache_creation_per_million.to_le_bytes());
        out.extend_from_slice(&rate.cache_read_per_million.to_le_bytes());
        out.extend_from_slice(&rate.reasoning_output_per_million.to_le_bytes());

        let mut flags = 0u8;
        if rate.cache_creation_1h_per_million.is_some() {
            flags |= 0x01;
        }
        if rate.fast.is_some() {
            flags |= 0x02;
        }
        if rate.ultrafast.is_some() {
            flags |= 0x04;
        }
        if !rate.context_tiers.is_empty() {
            flags |= 0x08;
        }
        let has_legacy_above = rate.tier_threshold_tokens.is_some()
            || rate.input_above_per_million.is_some()
            || rate.output_above_per_million.is_some()
            || rate.cache_creation_above_per_million.is_some()
            || rate.cache_read_above_per_million.is_some()
            || rate.reasoning_output_above_per_million.is_some();
        if has_legacy_above {
            flags |= 0x10;
        }
        out.push(flags);

        if let Some(c1h) = rate.cache_creation_1h_per_million {
            out.extend_from_slice(&c1h.to_le_bytes());
        }
        if let Some(fast) = rate.fast.as_deref() {
            write_token_rates_bin(&mut out, fast);
        }
        if let Some(ultra) = rate.ultrafast.as_deref() {
            write_token_rates_bin(&mut out, ultra);
        }
        if !rate.context_tiers.is_empty() {
            let tier_count = u16::try_from(rate.context_tiers.len()).unwrap_or(u16::MAX);
            out.extend_from_slice(&tier_count.to_le_bytes());
            for tier in rate.context_tiers.iter().take(tier_count as usize) {
                out.extend_from_slice(&tier.threshold_tokens.to_le_bytes());
                write_token_rates_bin(&mut out, &tier.standard);
                let mut tflags = 0u8;
                if tier.fast.is_some() {
                    tflags |= 0x01;
                }
                if tier.ultrafast.is_some() {
                    tflags |= 0x02;
                }
                out.push(tflags);
                if let Some(f) = tier.fast.as_ref() {
                    write_token_rates_bin(&mut out, f);
                }
                if let Some(u) = tier.ultrafast.as_ref() {
                    write_token_rates_bin(&mut out, u);
                }
            }
        }
        if has_legacy_above {
            let opt_u64 = rate.tier_threshold_tokens;
            out.push(u8::from(opt_u64.is_some()));
            if let Some(v) = opt_u64 {
                out.extend_from_slice(&v.to_le_bytes());
            }
            for opt_f64 in [
                rate.input_above_per_million,
                rate.output_above_per_million,
                rate.cache_creation_above_per_million,
                rate.cache_read_above_per_million,
                rate.reasoning_output_above_per_million,
            ] {
                out.push(u8::from(opt_f64.is_some()));
                if let Some(v) = opt_f64 {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
    }

    out
}

fn decode_pricing_cache_binary(data: &[u8]) -> Option<OpenRouterPricingCacheStore> {
    if data.len() < 20 || &data[0..4] != PRICING_BINARY_MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(data[4..8].try_into().ok()?);
    let fetched_unix = u64::from_le_bytes(data[8..16].try_into().ok()?);
    let count = u32::from_le_bytes(data[16..20].try_into().ok()?) as usize;

    let mut pos = 20usize;
    let read_u8 = |pos: &mut usize| -> Option<u8> {
        let b = *data.get(*pos)?;
        *pos += 1;
        Some(b)
    };
    let read_u16 = |pos: &mut usize| -> Option<u16> {
        if pos.checked_add(2)? > data.len() {
            return None;
        }
        let v = u16::from_le_bytes(data[*pos..*pos + 2].try_into().ok()?);
        *pos += 2;
        Some(v)
    };
    let read_u32 = |pos: &mut usize| -> Option<u32> {
        if pos.checked_add(4)? > data.len() {
            return None;
        }
        let v = u32::from_le_bytes(data[*pos..*pos + 4].try_into().ok()?);
        *pos += 4;
        Some(v)
    };
    let read_u64 = |pos: &mut usize| -> Option<u64> {
        if pos.checked_add(8)? > data.len() {
            return None;
        }
        let v = u64::from_le_bytes(data[*pos..*pos + 8].try_into().ok()?);
        *pos += 8;
        Some(v)
    };
    let read_f64 = |pos: &mut usize| -> Option<f64> {
        if pos.checked_add(8)? > data.len() {
            return None;
        }
        let v = f64::from_le_bytes(data[*pos..*pos + 8].try_into().ok()?);
        *pos += 8;
        Some(v)
    };

    let mut exact = HashMap::with_capacity(count.min(262_144));
    for _ in 0..count {
        let key_len = read_u32(&mut pos)? as usize;
        if pos.checked_add(key_len)? > data.len() {
            return None;
        }
        let key = std::str::from_utf8(&data[pos..pos + key_len])
            .ok()?
            .to_string();
        pos += key_len;

        let input_per_million = read_f64(&mut pos)?;
        let output_per_million = read_f64(&mut pos)?;
        let cache_creation_per_million = read_f64(&mut pos)?;
        let cache_read_per_million = read_f64(&mut pos)?;
        let reasoning_output_per_million = read_f64(&mut pos)?;
        let flags = read_u8(&mut pos)?;

        let cache_creation_1h_per_million = if (flags & 0x01) != 0 {
            Some(read_f64(&mut pos)?)
        } else {
            None
        };
        let fast = if (flags & 0x02) != 0 {
            Some(Box::new(read_token_rates_bin(data, &mut pos)?))
        } else {
            None
        };
        let ultrafast = if (flags & 0x04) != 0 {
            Some(Box::new(read_token_rates_bin(data, &mut pos)?))
        } else {
            None
        };
        let context_tiers = if (flags & 0x08) != 0 {
            let tier_count = read_u16(&mut pos)? as usize;
            let mut tiers = Vec::with_capacity(tier_count.min(32));
            for _ in 0..tier_count {
                let threshold_tokens = read_u64(&mut pos)?;
                let standard = read_token_rates_bin(data, &mut pos)?;
                let tflags = read_u8(&mut pos)?;
                let fast = if (tflags & 0x01) != 0 {
                    Some(read_token_rates_bin(data, &mut pos)?)
                } else {
                    None
                };
                let ultrafast = if (tflags & 0x02) != 0 {
                    Some(read_token_rates_bin(data, &mut pos)?)
                } else {
                    None
                };
                tiers.push(ContextTierRate {
                    threshold_tokens,
                    standard,
                    fast,
                    ultrafast,
                });
            }
            tiers
        } else {
            Vec::new()
        };

        let mut tier_threshold_tokens = None;
        let mut input_above_per_million = None;
        let mut output_above_per_million = None;
        let mut cache_creation_above_per_million = None;
        let mut cache_read_above_per_million = None;
        let mut reasoning_output_above_per_million = None;
        if (flags & 0x10) != 0 {
            if read_u8(&mut pos)? != 0 {
                tier_threshold_tokens = Some(read_u64(&mut pos)?);
            }
            let read_opt_f64 = |pos: &mut usize| -> Option<Option<f64>> {
                if read_u8(pos)? != 0 {
                    Some(Some(read_f64(pos)?))
                } else {
                    Some(None)
                }
            };
            input_above_per_million = read_opt_f64(&mut pos)?;
            output_above_per_million = read_opt_f64(&mut pos)?;
            cache_creation_above_per_million = read_opt_f64(&mut pos)?;
            cache_read_above_per_million = read_opt_f64(&mut pos)?;
            reasoning_output_above_per_million = read_opt_f64(&mut pos)?;
        }

        exact.insert(
            key,
            PricingRate {
                input_per_million,
                output_per_million,
                cache_creation_per_million,
                cache_creation_1h_per_million,
                cache_read_per_million,
                reasoning_output_per_million,
                tier_threshold_tokens,
                input_above_per_million,
                output_above_per_million,
                cache_creation_above_per_million,
                cache_read_above_per_million,
                reasoning_output_above_per_million,
                fast,
                ultrafast,
                context_tiers,
            },
        );
    }

    Some(OpenRouterPricingCacheStore {
        version,
        fetched_unix,
        exact,
    })
}

fn finite_per_million(entry: &Map<String, Value>, key: &str) -> Option<f64> {
    let v = entry.get(key)?.as_f64()?;
    (v.is_finite() && v >= 0.0).then_some(v * 1_000_000.0)
}

fn read_litellm_token_rates(
    entry: &Map<String, Value>,
    suffix: &str,
    standard: Option<&TokenRates>,
    output_fallback: Option<f64>,
) -> Option<TokenRates> {
    let input = finite_per_million(entry, &format!("input_cost_per_token{suffix}"))?;
    let output =
        finite_per_million(entry, &format!("output_cost_per_token{suffix}")).or(output_fallback)?;

    let cache_rate = |name: &str, std_val: Option<f64>, fallback: f64| -> f64 {
        if let Some(v) = finite_per_million(entry, &format!("{name}{suffix}")) {
            return v;
        }
        if let (Some(std), Some(sv)) = (standard, std_val)
            && std.input_per_million > 0.0
        {
            return (sv / std.input_per_million) * input;
        }
        fallback
    };

    let cache_creation = cache_rate(
        "cache_creation_input_token_cost",
        standard.map(|s| s.cache_creation_per_million),
        input,
    );
    let cache_creation_1h = cache_rate(
        "cache_creation_input_token_cost_above_1hr",
        standard.map(|s| s.cache_creation_1h_per_million),
        cache_creation,
    );
    let cache_read = cache_rate(
        "cache_read_input_token_cost",
        standard.map(|s| s.cache_read_per_million),
        input,
    );

    // Output tokens already include reasoning tokens across parsers; only record a separate
    // reasoning rate when it genuinely differs from the regular output rate.
    let reasoning_output =
        finite_per_million(entry, &format!("output_cost_per_reasoning_token{suffix}"))
            .filter(|&r| (r - output).abs() > 1e-6)
            .unwrap_or(0.0);

    Some(TokenRates {
        input_per_million: input,
        output_per_million: output,
        cache_creation_per_million: cache_creation,
        cache_creation_1h_per_million: cache_creation_1h,
        cache_read_per_million: cache_read,
        reasoning_output_per_million: reasoning_output,
    })
}

fn scale_token_rates(rates: TokenRates, multiple: f64) -> TokenRates {
    TokenRates {
        input_per_million: rates.input_per_million * multiple,
        output_per_million: rates.output_per_million * multiple,
        cache_creation_per_million: rates.cache_creation_per_million * multiple,
        cache_creation_1h_per_million: rates.cache_creation_1h_per_million * multiple,
        cache_read_per_million: rates.cache_read_per_million * multiple,
        reasoning_output_per_million: rates.reasoning_output_per_million * multiple,
    }
}

fn litellm_fast_multiplier(entry: &Map<String, Value>) -> Option<f64> {
    let fast = entry
        .get("provider_specific_entry")
        .and_then(Value::as_object)
        .and_then(|spec| spec.get("fast"))
        .and_then(Value::as_f64)?;
    (fast.is_finite() && fast > 0.0).then_some(fast)
}

fn read_litellm_speed_rates(
    entry: &Map<String, Value>,
    standard: TokenRates,
    suffix: &str,
) -> (Option<TokenRates>, Option<TokenRates>) {
    let fast = match litellm_fast_multiplier(entry) {
        Some(mult) => Some(scale_token_rates(standard, mult)),
        None => {
            read_litellm_token_rates(entry, &format!("{suffix}_priority"), Some(&standard), None)
        }
    };
    let ultrafast =
        read_litellm_token_rates(entry, &format!("{suffix}_ultrafast"), Some(&standard), None);
    (fast, ultrafast)
}

fn parse_context_threshold_k(key: &str) -> Option<u64> {
    let rest = key.strip_prefix("input_cost_per_token_above_")?;
    let trimmed = rest
        .strip_suffix("_priority")
        .or_else(|| rest.strip_suffix("_ultrafast"))
        .unwrap_or(rest);
    let num_str = trimmed.strip_suffix("k_tokens")?;
    if num_str.is_empty() || num_str.len() > 7 || !num_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    num_str.parse::<u64>().ok()
}

fn read_litellm_context_tiers(
    entry: &Map<String, Value>,
    base: TokenRates,
) -> Vec<ContextTierRate> {
    let mut thresholds = HashSet::new();
    for key in entry.keys() {
        if let Some(k) = parse_context_threshold_k(key) {
            thresholds.insert(k);
        }
    }
    let mut tiers: Vec<ContextTierRate> = thresholds
        .into_iter()
        .map(|k| {
            let suffix = format!("_above_{k}k_tokens");
            let standard = read_litellm_token_rates(
                entry,
                &suffix,
                Some(&base),
                Some(base.output_per_million),
            )
            .unwrap_or(base);
            let (fast, ultrafast) = read_litellm_speed_rates(entry, standard, &suffix);
            ContextTierRate {
                threshold_tokens: k * 1_000,
                standard,
                fast,
                ultrafast,
            }
        })
        .collect();
    tiers.sort_by_key(|t| std::cmp::Reverse(t.threshold_tokens));
    tiers
}

fn is_first_party_provider(prefix: &str) -> bool {
    matches!(
        prefix,
        "anthropic" | "deepseek" | "gemini" | "meta" | "mistral" | "openai" | "xai"
    )
}

pub(super) fn parse_litellm_rate_table(
    document: &Map<String, Value>,
) -> HashMap<String, PricingRate> {
    let mut table = HashMap::new();
    let mut first_party_aliases: HashMap<String, PricingRate> = HashMap::new();
    let mut fallback_aliases: HashMap<String, PricingRate> = HashMap::new();

    for (name, raw) in document {
        let Some(entry) = raw.as_object() else {
            continue;
        };
        let Some(standard) = read_litellm_token_rates(entry, "", None, None) else {
            continue;
        };
        let key = canonical_model_name(name);
        if key.is_empty() {
            continue;
        }
        let (fast, ultrafast) = read_litellm_speed_rates(entry, standard, "");
        let context_tiers = read_litellm_context_tiers(entry, standard);

        let rate = PricingRate {
            input_per_million: standard.input_per_million,
            output_per_million: standard.output_per_million,
            cache_creation_per_million: standard.cache_creation_per_million,
            cache_creation_1h_per_million: Some(standard.cache_creation_1h_per_million),
            cache_read_per_million: standard.cache_read_per_million,
            reasoning_output_per_million: standard.reasoning_output_per_million,
            fast: fast.map(Box::new),
            ultrafast: ultrafast.map(Box::new),
            context_tiers,
            ..PricingRate::default()
        };

        if let Some((provider, bare)) = key.split_once('/') {
            let bare_tail = bare.rsplit('/').next().unwrap_or(bare);
            if !bare_tail.is_empty() {
                if is_first_party_provider(provider) {
                    first_party_aliases
                        .entry(bare_tail.to_string())
                        .or_insert_with(|| rate.clone());
                } else if !provider.contains('.') {
                    // Only allow non-regional bare fallbacks as last resort.
                    fallback_aliases
                        .entry(bare_tail.to_string())
                        .or_insert_with(|| rate.clone());
                }
            }
        } else if let Some(bedrock_bare) = key.strip_prefix("anthropic.") {
            fallback_aliases
                .entry(bedrock_bare.to_string())
                .or_insert_with(|| rate.clone());
        }

        table.insert(key, rate);
    }

    for (alias, rate) in first_party_aliases {
        table.entry(alias).or_insert(rate);
    }
    for (alias, rate) in fallback_aliases {
        table.entry(alias).or_insert(rate);
    }

    // Add dot/hyphen normalized keys without overwriting direct LiteLLM entries.
    let existing_keys: Vec<String> = table.keys().cloned().collect();
    for key in existing_keys {
        if !key.contains('/')
            && key.contains('.')
            && let Some(rate) = table.get(&key).cloned()
        {
            table.entry(key.replace('.', "-")).or_insert(rate);
        }
    }

    table
}

#[cfg(test)]
mod litellm_tests {
    use super::*;
    use crate::types::{PricingContext, ServiceSpeed, UsageAccumulator};

    #[test]
    fn parse_litellm_extracts_1h_tiers_and_fast_multiplier_without_regional_overwrite() {
        let raw: Map<String, Value> = serde_json::from_str(
            r#"{
                "sample_spec": {"max_tokens": 100},
                "claude-opus-5-5": {
                    "input_cost_per_token": 4e-6,
                    "output_cost_per_token": 2e-5,
                    "cache_read_input_token_cost": 2e-7,
                    "cache_creation_input_token_cost": 5e-6,
                    "cache_creation_input_token_cost_above_1hr": 8e-6,
                    "provider_specific_entry": {"fast": 2}
                },
                "us.anthropic.claude-opus-5-5": {
                    "input_cost_per_token": 4.4e-6,
                    "output_cost_per_token": 2.2e-5,
                    "cache_read_input_token_cost": 2.2e-7,
                    "cache_creation_input_token_cost": 5.5e-6,
                    "cache_creation_input_token_cost_above_1hr": 8.8e-6
                },
                "openai/gpt-6.1-sol": {
                    "input_cost_per_token": 2e-6,
                    "output_cost_per_token": 1e-5,
                    "cache_read_input_token_cost": 1e-7,
                    "cache_creation_input_token_cost": 2.5e-6,
                    "input_cost_per_token_above_272k_tokens": 4e-6,
                    "output_cost_per_token_above_272k_tokens": 1.5e-5,
                    "cache_read_input_token_cost_above_272k_tokens": 2e-7,
                    "input_cost_per_token_above_272k_tokens_priority": 8e-6,
                    "output_cost_per_token_above_272k_tokens_priority": 3e-5,
                    "cache_read_input_token_cost_above_272k_tokens_priority": 4e-7
                }
            }"#,
        )
        .unwrap();

        let exact = parse_litellm_rate_table(&raw);
        assert!(!exact.contains_key("sample_spec"));

        let opus = exact.get("claude-opus-5-5").unwrap();
        assert!((opus.input_per_million - 4.0).abs() < 1e-9);
        assert!((opus.cache_creation_1h_per_million.unwrap() - 8.0).abs() < 1e-9);
        assert!((opus.fast.as_ref().unwrap().output_per_million - 40.0).abs() < 1e-9);

        // Bare alias from first-party openai/gpt-6.1-sol
        let gpt = exact.get("gpt-6.1-sol").unwrap();
        assert_eq!(gpt.context_tiers.len(), 1);
        assert_eq!(gpt.context_tiers[0].threshold_tokens, 272_000);
        assert!((gpt.context_tiers[0].standard.input_per_million - 4.0).abs() < 1e-9);
        assert!((gpt.context_tiers[0].fast.unwrap().input_per_million - 8.0).abs() < 1e-9);

        let mut table = PricingTable::default();
        table.merge_exact(exact);
        let fast_cost = table
            .estimate_cost_with_context(
                "claude-opus-5-5",
                UsageAccumulator {
                    input_tokens: 1_000_000,
                    cache_creation_input_tokens: 1_000_000,
                    ..UsageAccumulator::default()
                },
                PricingContext {
                    cache_creation_1h_tokens: 1_000_000,
                    speed: ServiceSpeed::Fast,
                    ..PricingContext::default()
                },
            )
            .unwrap();
        // Fast 2x: input $8 + 1h cache $16 = $24
        assert!((fast_cost - 24.0).abs() < 1e-9, "got {fast_cost}");
    }
}
