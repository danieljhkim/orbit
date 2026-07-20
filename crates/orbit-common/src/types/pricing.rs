//! Versioned model price table (ORB-10338, ADR-0245).
//!
//! Cost is derived from token splits at query time against a price row keyed
//! by exact model string and effective date range, rather than computed once
//! and frozen at ingest. The table itself is a data asset
//! (`assets/model_prices.yaml`), not Rust — adding or correcting a rate is a
//! YAML edit, no code change required. See ADR-0245 for the tradeoffs.

// The shipped asset is a build-time invariant (a checked-in file this crate
// controls), not user input; `shipped_price_table_parses_and_is_non_empty`
// guards it in CI. See `redaction.rs` for the same documented-invariant use
// of `expect` (ORB-00013).
#![allow(clippy::expect_used)]

use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::invocation::TokenUsage;

const MODEL_PRICES_YAML: &str = include_str!("../../assets/model_prices.yaml");

/// One versioned price row: USD per million tokens, by exact model string
/// and the date range this rate was in effect. `effective_until` is
/// exclusive; `None` means the rate is still current.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PriceRow {
    pub model: String,
    pub effective_from: DateTime<Utc>,
    #[serde(default)]
    pub effective_until: Option<DateTime<Utc>>,
    pub input_per_million_usd: f64,
    pub cache_read_per_million_usd: f64,
    pub cache_create_per_million_usd: f64,
    pub output_per_million_usd: f64,
}

impl PriceRow {
    fn covers(&self, model: &str, at: DateTime<Utc>) -> bool {
        self.model == model
            && at >= self.effective_from
            && self.effective_until.is_none_or(|until| at < until)
    }

    fn cost_usd(&self, usage: &TokenUsage) -> f64 {
        rate(usage.input, self.input_per_million_usd)
            + rate(usage.cache_read, self.cache_read_per_million_usd)
            + rate(usage.cache_create, self.cache_create_per_million_usd)
            + rate(usage.output, self.output_per_million_usd)
    }
}

fn rate(tokens: u64, per_million_usd: f64) -> f64 {
    (tokens as f64 / 1_000_000.0) * per_million_usd
}

#[derive(Debug, Deserialize)]
struct PriceTableFile {
    prices: Vec<PriceRow>,
}

fn price_table() -> &'static [PriceRow] {
    static TABLE: OnceLock<Vec<PriceRow>> = OnceLock::new();
    &TABLE.get_or_init(|| {
        let file: PriceTableFile = serde_yaml::from_str(MODEL_PRICES_YAML)
            .expect("crates/orbit-common/assets/model_prices.yaml must parse");
        file.prices
    })[..]
}

/// Derive normalized USD cost from token splits using the price row in
/// effect for `model` at `at` (an invocation's timestamp). Returns `None`
/// when no price row covers the model/date — callers keep the
/// provider-reported cost as the only figure in that case, they never
/// substitute a guess.
pub fn derive_cost_usd(model: &str, at: DateTime<Utc>, usage: &TokenUsage) -> Option<f64> {
    cost_from_rows(price_table(), model, at, usage)
}

/// Selection algorithm shared by [`derive_cost_usd`] and its tests: pick the
/// covering row with the latest `effective_from` (in case ranges ever
/// overlap) and price `usage` against it.
pub(super) fn cost_from_rows(
    rows: &[PriceRow],
    model: &str,
    at: DateTime<Utc>,
    usage: &TokenUsage,
) -> Option<f64> {
    rows.iter()
        .filter(|row| row.covers(model, at))
        .max_by(|a, b| a.effective_from.cmp(&b.effective_from))
        .map(|row| row.cost_usd(usage))
}

#[cfg(test)]
pub(super) fn shipped_price_table() -> &'static [PriceRow] {
    price_table()
}
