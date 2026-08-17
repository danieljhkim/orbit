//! Versioned model price table (ORB-10338, ADR-0245).
//!
//! Cost is derived from token splits at query time against a price row keyed
//! by exact model string and effective date range, rather than computed once
//! and frozen at ingest. The table itself is a data asset
//! (`assets/model_prices.yaml`), not Rust — adding or correcting a rate is a
//! YAML edit, no code change required. See ADR-0245 for the tradeoffs.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::invocation::TokenUsage;

/// How a provider reports the input-token total carried by [`TokenUsage`].
///
/// Most providers report mutually exclusive input and cache buckets. OpenAI
/// reports a gross input total that already includes cached reads and writes,
/// so those buckets must be removed before the full input rate is applied.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InputTokenBasis {
    /// `TokenUsage::input` excludes every cache bucket.
    #[default]
    Exclusive,
    /// `TokenUsage::input` includes cache reads and both cache-write buckets.
    GrossIncludesCache,
}

/// One versioned price row: USD per million tokens, by exact model string
/// and the date range this rate was in effect. `effective_until` is
/// exclusive; `None` means the rate is still current.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PriceRow {
    pub model: String,
    pub effective_from: DateTime<Utc>,
    #[serde(default)]
    pub effective_until: Option<DateTime<Utc>>,
    /// Provider reporting convention for [`TokenUsage::input`]. Existing rows
    /// default to [`InputTokenBasis::Exclusive`] for backward compatibility.
    #[serde(default)]
    pub input_token_basis: InputTokenBasis,
    pub input_per_million_usd: f64,
    pub cache_read_per_million_usd: f64,
    /// Rate for standard 5-minute-TTL cache-creation (write) tokens
    /// ([`TokenUsage::cache_create`]).
    pub cache_create_per_million_usd: f64,
    /// Rate for premium 1-hour-TTL cache-creation (write) tokens
    /// ([`TokenUsage::cache_create_1h`]). Defaults to `0.0` for providers with
    /// no 1h-TTL concept (their `cache_create_1h` count is always zero, so the
    /// rate is never applied); Anthropic rows set it to 2x the input rate.
    #[serde(default)]
    pub cache_create_1h_per_million_usd: f64,
    pub output_per_million_usd: f64,
}

impl PriceRow {
    fn covers(&self, model: &str, at: DateTime<Utc>) -> bool {
        self.model == model
            && at >= self.effective_from
            && self.effective_until.is_none_or(|until| at < until)
    }

    fn cost_usd(&self, usage: &TokenUsage) -> Option<f64> {
        let input = match self.input_token_basis {
            InputTokenBasis::Exclusive => usage.input,
            InputTokenBasis::GrossIncludesCache => {
                let cache_detail = usage
                    .cache_read
                    .checked_add(usage.cache_create)?
                    .checked_add(usage.cache_create_1h)?;
                usage.input.checked_sub(cache_detail)?
            }
        };

        Some(
            rate(input, self.input_per_million_usd)
                + rate(usage.cache_read, self.cache_read_per_million_usd)
                + rate(usage.cache_create, self.cache_create_per_million_usd)
                + rate(usage.cache_create_1h, self.cache_create_1h_per_million_usd)
                + rate(usage.output, self.output_per_million_usd),
        )
    }
}

/// Strip a trailing context-window marker like `[1m]` / `[200k]` from a model
/// string, returning the base model key. The fleet's ledger records the 1M
/// context variant of Opus as `claude-opus-4-8[1m]` (a `modelUsage` key), but
/// those runs are billed at the same per-token rates as the base model — the
/// bracket denotes the context window, not a distinct price tier. Rather than
/// duplicate every row for each suffix, [`cost_from_rows`] falls back to the
/// stripped key when no exact row matches. If a provider ever charges a
/// long-context premium, add a row keyed by the full `model[1m]` string and it
/// wins via the exact-match pass below.
fn strip_context_suffix(model: &str) -> Option<&str> {
    if model.ends_with(']') {
        model
            .rfind('[')
            .map(|open| model[..open].trim_end())
            .filter(|base| !base.is_empty())
    } else {
        None
    }
}

fn rate(tokens: u64, per_million_usd: f64) -> f64 {
    (tokens as f64 / 1_000_000.0) * per_million_usd
}

/// Pick the covering row with the latest `effective_from` and price `usage`.
pub fn cost_from_rows(
    rows: &[PriceRow],
    model: &str,
    at: DateTime<Utc>,
    usage: &TokenUsage,
) -> Option<f64> {
    let pick = |model: &str| {
        rows.iter()
            .filter(|row| row.covers(model, at))
            .max_by(|a, b| a.effective_from.cmp(&b.effective_from))
    };
    // Exact match first (so a dedicated `model[1m]` premium row would win),
    // then fall back to the base key with any context-window suffix stripped.
    pick(model)
        .or_else(|| strip_context_suffix(model).and_then(pick))
        .and_then(|row| row.cost_usd(usage))
}

/// Converts provider-reported usage into mutually-exclusive token buckets
/// using an already-selected price table.
pub fn normalize_token_usage_from_rows(
    rows: &[PriceRow],
    model: &str,
    at: DateTime<Utc>,
    usage: &TokenUsage,
) -> Option<TokenUsage> {
    let pick = |model: &str| {
        rows.iter()
            .filter(|row| row.covers(model, at))
            .max_by(|a, b| a.effective_from.cmp(&b.effective_from))
    };
    let row = pick(model).or_else(|| strip_context_suffix(model).and_then(pick))?;
    let input = match row.input_token_basis {
        InputTokenBasis::Exclusive => usage.input,
        InputTokenBasis::GrossIncludesCache => usage.input.checked_sub(
            usage
                .cache_read
                .checked_add(usage.cache_create)?
                .checked_add(usage.cache_create_1h)?,
        )?,
    };
    Some(TokenUsage {
        input,
        ..usage.clone()
    })
}
