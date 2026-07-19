//! Pricing — loaded from the editable `pricing.toml` at startup (BUILD_SPEC §4.5, §0.5 #12).
//! Rust is the ONLY cost calculator; the frontend never re-prices.
//!
//! Rates are USD per 1,000,000 tokens. Matching is lowercased-substring, order-sensitive
//! (first `match` that is a substring of the model id wins), so preserve the TOML row order.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Rate {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_write: f64,
    pub cache_read: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MatchedRate {
    #[serde(rename = "match")]
    pub key: String,
    #[serde(flatten)]
    pub rate: Rate,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pricing {
    pub claude: Vec<MatchedRate>,
    pub claude_default: Rate,
    pub codex: Vec<MatchedRate>,
    pub codex_default: Rate,
}

impl Pricing {
    /// Load from pricing.toml (bundled as a resource; path resolved by the caller).
    pub fn load(toml_text: &str) -> Result<Self, String> {
        toml::from_str(toml_text).map_err(|e| format!("pricing.toml parse error: {e}"))
    }

    /// Resolve the rate for a Claude model id (lowercased-substring, order-sensitive).
    pub fn claude_rate(&self, model: &str) -> &Rate {
        let m = model.to_lowercase();
        self.claude
            .iter()
            .find(|r| m.contains(&r.key))
            .map(|r| &r.rate)
            .unwrap_or(&self.claude_default)
    }

    /// Resolve the rate for a Codex/OpenAI model id.
    pub fn codex_rate(&self, model: &str) -> &Rate {
        let m = model.to_lowercase();
        self.codex
            .iter()
            .find(|r| m.contains(&r.key))
            .map(|r| &r.rate)
            .unwrap_or(&self.codex_default)
    }
}

/// Claude cost (BUILD_SPEC §4.5).
pub fn claude_cost(r: &Rate, input: u64, output: u64, cache_write: u64, cache_read: u64) -> f64 {
    (input as f64 * r.input
        + output as f64 * r.output
        + cache_write as f64 * r.cache_write
        + cache_read as f64 * r.cache_read)
        / 1_000_000.0
}

/// Codex cost (BUILD_SPEC §4.5): OpenAI `input` already includes cached, so bill the
/// non-cached remainder at the input rate + cached at cache_read + output. Reasoning
/// tokens are inside output and NOT billed separately.
pub fn codex_cost(r: &Rate, input: u64, cached: u64, output: u64) -> f64 {
    let non_cached = input.saturating_sub(cached);
    (non_cached as f64 * r.input + cached as f64 * r.cache_read + output as f64 * r.output)
        / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pricing() -> Pricing {
        // Load the shipped defaults so the tests double as a config sanity check.
        Pricing::load(include_str!("../../pricing.toml")).expect("pricing.toml valid")
    }

    #[test]
    fn opus_4_7_rate() {
        let r = pricing().claude_rate("claude-opus-4.7");
        assert_eq!(r.input, 5.0);
        assert_eq!(r.output, 25.0);
    }

    #[test]
    fn opus_5_beats_opus_substring() {
        // order-sensitivity: "opus-5" must win before the generic "opus" row.
        let r = pricing().claude_rate("claude-opus-5");
        assert_eq!(r.output, 100.0);
    }

    #[test]
    fn codex_non_cached_billing() {
        let p = pricing();
        let r = p.codex_rate("gpt-5.4");
        // 1000 input (200 cached) + 500 output at gpt-5.4 rates.
        let cost = codex_cost(r, 1000, 200, 500);
        let expected = (800.0 * 2.50 + 200.0 * 0.25 + 500.0 * 15.00) / 1_000_000.0;
        assert!((cost - expected).abs() < 1e-12);
    }
}
