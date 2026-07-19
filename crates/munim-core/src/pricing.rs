//! Pricing — loaded from the editable `pricing.toml` (BUILD_SPEC §4.5, §0.5 #12).
//! munim-core is the ONLY cost calculator; the frontend never re-prices.
//!
//! Rates are USD per 1,000,000 tokens. Matching is substring-based and ORDER-SENSITIVE
//! (first `match` that is a substring of the model id wins) — preserve TOML row order.
//! Both the model id and each `match` key are normalized (lowercased, `.`/`_` → `-`)
//! before comparison, so config written as `opus-4.5` matches real ids like
//! `claude-opus-4-5-20250514`.

use serde::Deserialize;

/// The default pricing table, embedded so the app always has rates even if the external
/// `pricing.toml` resource is missing. Kept in sync with the repo-root file.
pub const EMBEDDED_PRICING_TOML: &str = include_str!("../../../pricing.toml");

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

fn norm(s: &str) -> String {
    s.to_lowercase().replace(['.', '_'], "-")
}

impl Pricing {
    /// Parse a pricing TOML document.
    pub fn load(toml_text: &str) -> Result<Self, String> {
        toml::from_str(toml_text).map_err(|e| format!("pricing.toml parse error: {e}"))
    }

    /// The embedded default table (never fails in practice).
    pub fn embedded_default() -> Self {
        Self::load(EMBEDDED_PRICING_TOML).expect("embedded pricing.toml must be valid")
    }

    fn find<'a>(list: &'a [MatchedRate], default: &'a Rate, model: &str) -> &'a Rate {
        let m = norm(model);
        list.iter()
            .find(|r| m.contains(&norm(&r.key)))
            .map(|r| &r.rate)
            .unwrap_or(default)
    }

    /// Rate for a Claude model id.
    pub fn claude_rate(&self, model: &str) -> &Rate {
        Self::find(&self.claude, &self.claude_default, model)
    }

    /// Rate for a Codex / OpenAI model id.
    pub fn codex_rate(&self, model: &str) -> &Rate {
        Self::find(&self.codex, &self.codex_default, model)
    }
}

/// Claude cost (BUILD_SPEC §4.5): every token class billed at its rate.
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
        Pricing::embedded_default()
    }

    #[test]
    fn real_dash_ids_match() {
        // Real Claude model ids use dashes; config uses dots — normalization bridges them.
        assert_eq!(
            pricing().claude_rate("claude-opus-4-1-20250805").input,
            15.0
        );
        assert_eq!(pricing().claude_rate("claude-opus-4-5-20250929").input, 5.0);
        assert_eq!(
            pricing().claude_rate("claude-sonnet-4-5-20250929").output,
            15.0
        );
        assert_eq!(
            pricing().claude_rate("claude-3-5-haiku-20241022").input,
            0.25
        );
        assert_eq!(
            pricing().claude_rate("claude-haiku-4-5-20251001").input,
            1.0
        );
    }

    #[test]
    fn opus_5_beats_generic_opus() {
        // order-sensitivity: "opus-5" row must win before the generic "opus" row.
        assert_eq!(pricing().claude_rate("claude-opus-5").output, 100.0);
    }

    #[test]
    fn unknown_defaults_to_sonnet() {
        let p = pricing();
        let r = p.claude_rate("some-future-model");
        assert_eq!((r.input, r.output), (3.0, 15.0));
    }

    #[test]
    fn claude_cost_formula() {
        let p = pricing();
        let r = p.claude_rate("claude-sonnet-4-5");
        // 1000 in, 500 out, 200 cache_write, 300 cache_read at sonnet rates.
        let got = claude_cost(r, 1000, 500, 200, 300);
        let want = (1000.0 * 3.0 + 500.0 * 15.0 + 200.0 * 3.75 + 300.0 * 0.30) / 1_000_000.0;
        assert!((got - want).abs() < 1e-12);
    }

    #[test]
    fn codex_non_cached_billing() {
        let p = pricing();
        let r = p.codex_rate("gpt-5.4");
        let got = codex_cost(r, 1000, 200, 500);
        let want = (800.0 * 2.50 + 200.0 * 0.25 + 500.0 * 15.00) / 1_000_000.0;
        assert!((got - want).abs() < 1e-12);
    }

    #[test]
    fn codex_mini_before_base() {
        // "gpt-5.4-mini" must match before "gpt-5.4".
        assert_eq!(pricing().codex_rate("gpt-5-4-mini").input, 0.75);
        assert_eq!(pricing().codex_rate("gpt-5-4").input, 2.50);
    }
}
