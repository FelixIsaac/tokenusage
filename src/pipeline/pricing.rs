use std::collections::HashMap;


use crate::types::PricingRate;

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

pub(super) fn load_openrouter_pricing_cache(path: &Path) -> Option<OpenRouterPricingCacheStore> {
    let body = std::fs::read(path).ok()?;
    let cache: OpenRouterPricingCacheStore = serde_json::from_slice(&body).ok()?;
    (cache.version == OPENROUTER_PRICING_CACHE_VERSION).then_some(cache)
}

pub(super) fn save_openrouter_pricing_cache(
    path: &Path,
    fetched_unix: u64,
    exact: &HashMap<String, PricingRate>,
) {
    let cache = OpenRouterPricingCacheStore {
        version: OPENROUTER_PRICING_CACHE_VERSION,
        fetched_unix,
        exact: exact.clone(),
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec(&cache) {
        let _ = std::fs::write(path, bytes);
    }
}
