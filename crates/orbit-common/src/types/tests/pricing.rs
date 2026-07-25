use chrono::{DateTime, Utc};

use crate::types::TokenUsage;
use crate::types::pricing::{PriceRow, cost_from_rows, derive_cost_usd, shipped_price_table};

fn dt(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .expect("valid rfc3339 fixture timestamp")
        .with_timezone(&Utc)
}

#[test]
fn shipped_price_table_parses_and_is_non_empty() {
    assert!(
        !shipped_price_table().is_empty(),
        "shipped model_prices.yaml should seed at least one row"
    );
}

#[test]
fn derives_cost_for_a_covered_model() {
    let usage = TokenUsage {
        input: 1_000_000,
        cache_read: 0,
        cache_create: 0,
        cache_create_1h: 0,
        output: 1_000_000,
    };
    // claude-opus-4-7: input $5/M + output $25/M (corrected from the stale
    // 15/75 training-prior seed).
    let cost = derive_cost_usd("claude-opus-4-7", dt("2026-06-01T00:00:00Z"), &usage)
        .expect("claude-opus-4-7 is priced in the shipped table");
    assert!((cost - 30.0).abs() < f64::EPSILON, "cost was {cost}");
}

#[test]
fn returns_none_for_an_unpriced_model() {
    assert_eq!(
        derive_cost_usd("some-unpriced-model", Utc::now(), &TokenUsage::default()),
        None
    );
}

#[test]
fn returns_none_before_the_effective_date() {
    let usage = TokenUsage {
        input: 1_000,
        ..TokenUsage::default()
    };
    assert_eq!(
        derive_cost_usd("claude-opus-4-7", dt("2020-01-01T00:00:00Z"), &usage),
        None
    );
}

// ─── Ground-truth validation (ORB-10338) ────────────────────────────────────
// Token splits and provider-reported costs are taken verbatim from real worker
// run 91d7ef01 (supervisor run record `modelUsage`). The derived cost must
// reproduce the provider figure, which is the strongest check that the shipped
// rates are correct — stronger than any pricing page.

#[test]
fn ground_truth_opus_4_8_1m_reproduces_provider_cost() {
    // claude-opus-4-8[1m]: {input 36, output 8265, cache_read 858526,
    // cache_write_1h 37795} → provider costUSD 1.0140179999999996.
    // The `[1m]` context-window suffix resolves to the base claude-opus-4-8 row.
    let usage = TokenUsage {
        input: 36,
        cache_read: 858_526,
        cache_create: 0,
        cache_create_1h: 37_795,
        output: 8_265,
    };
    let cost = derive_cost_usd("claude-opus-4-8[1m]", dt("2026-07-19T00:00:00Z"), &usage)
        .expect("claude-opus-4-8[1m] resolves to the base opus-4-8 row");
    assert!(
        (cost - 1.014_018).abs() < 1e-6,
        "derived cost was {cost}, expected ~1.014018"
    );
}

#[test]
fn ground_truth_haiku_4_5_reproduces_provider_cost() {
    // claude-haiku-4-5-20251001: {input 1338, output 21} → provider 0.001443.
    let usage = TokenUsage {
        input: 1_338,
        cache_read: 0,
        cache_create: 0,
        cache_create_1h: 0,
        output: 21,
    };
    let cost = derive_cost_usd(
        "claude-haiku-4-5-20251001",
        dt("2026-07-19T00:00:00Z"),
        &usage,
    )
    .expect("claude-haiku-4-5-20251001 is priced in the shipped table");
    assert!(
        (cost - 0.001_443).abs() < 1e-9,
        "derived cost was {cost}, expected 0.001443"
    );
}

#[test]
fn one_hour_and_five_minute_cache_writes_price_distinctly() {
    // 1M tokens of each cache-write TTL against opus-4-8: 5m = 6.25, 1h = 10.0.
    let five_minute = TokenUsage {
        cache_create: 1_000_000,
        ..TokenUsage::default()
    };
    let one_hour = TokenUsage {
        cache_create_1h: 1_000_000,
        ..TokenUsage::default()
    };
    let at = dt("2026-07-19T00:00:00Z");
    let cost_5m = derive_cost_usd("claude-opus-4-8", at, &five_minute).expect("priced");
    let cost_1h = derive_cost_usd("claude-opus-4-8", at, &one_hour).expect("priced");
    assert!((cost_5m - 6.25).abs() < f64::EPSILON, "5m was {cost_5m}");
    assert!((cost_1h - 10.0).abs() < f64::EPSILON, "1h was {cost_1h}");
}

#[test]
fn context_window_suffix_falls_back_to_the_base_row() {
    let usage = TokenUsage {
        input: 1_000_000,
        ..TokenUsage::default()
    };
    let at = dt("2026-07-19T00:00:00Z");
    let base = derive_cost_usd("claude-opus-4-8", at, &usage).expect("base priced");
    let suffixed = derive_cost_usd("claude-opus-4-8[1m]", at, &usage).expect("suffix priced");
    assert_eq!(base, suffixed, "the [1m] variant prices at the base rate");
    assert!((base - 5.0).abs() < f64::EPSILON, "input rate was {base}");
}

/// Coverage guard (ORB-10338): every model string the fleet actually runs must
/// resolve to a price row, so a newly-fielded model can't silently derive
/// `None`/zero cost. Keyed by the exact strings observed in the worker ledger
/// (`modelUsage` keys) and the crew config. When the fleet adds a model, add it
/// here and to `assets/model_prices.yaml` in the same change.
///
/// This stays the cheap, curated form on purpose: it runs in CI, which has no
/// invocation ledger to scan, and it catches a missing row before the model is
/// ever fielded. The authoritative coverage signal is now the live-store scan
/// added by [ORB-10354] — `Store::list_unpriced_invocation_models`, which
/// groups the `model` strings actually observed and reports the ones no row
/// covers. That scan became usable once the store stopped recording unversioned
/// crew aliases (`opus`/`sonnet`/`fable`/`pro`) in the `model` column; aliases
/// are deliberately still NOT priced here, they resolve to an exact string at
/// ingest and are asserted against this table by
/// `types::tests::model_identity::every_alias_target_is_priced`.
#[test]
fn every_fleet_model_string_is_priced() {
    const FLEET_MODELS: &[&str] = &[
        "claude-opus-5",
        "claude-opus-4-8",
        "claude-opus-4-8[1m]",
        "claude-opus-4-7",
        "claude-sonnet-5",
        "claude-haiku-4-5-20251001",
        "claude-fable-5",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
        "gemini-3.5-flash",
        "grok-build",
    ];
    // A nonzero split so a zero-rate row would still yield Some (we assert
    // coverage, not a specific figure).
    let usage = TokenUsage {
        input: 1_000,
        output: 1_000,
        ..TokenUsage::default()
    };
    // 2026-07-24 (not 07-19): must be on/after claude-opus-5's effective_from
    // so its row is in range too, while still covering every other row below.
    let at = dt("2026-07-24T00:00:00Z");
    for model in FLEET_MODELS {
        assert!(
            derive_cost_usd(model, at, &usage).is_some(),
            "fleet model {model} has no covering price row in model_prices.yaml"
        );
    }
}

fn flat_row(model: &str, effective_from: &str, input_per_million_usd: f64) -> PriceRow {
    PriceRow {
        model: model.to_string(),
        effective_from: dt(effective_from),
        effective_until: None,
        input_per_million_usd,
        cache_read_per_million_usd: 0.0,
        cache_create_per_million_usd: 0.0,
        cache_create_1h_per_million_usd: 0.0,
        output_per_million_usd: 0.0,
    }
}

#[test]
fn picks_the_most_recent_covering_row_when_ranges_overlap() {
    let rows = vec![
        flat_row("dup-model", "2026-01-01T00:00:00Z", 1.0),
        flat_row("dup-model", "2026-06-01T00:00:00Z", 2.0),
    ];
    let usage = TokenUsage {
        input: 1_000_000,
        ..TokenUsage::default()
    };
    let cost = cost_from_rows(&rows, "dup-model", dt("2026-07-01T00:00:00Z"), &usage)
        .expect("one row covers");
    assert!((cost - 2.0).abs() < f64::EPSILON, "cost was {cost}");
}

#[test]
fn respects_an_exclusive_effective_until_boundary() {
    let mut closed = flat_row("closed-model", "2026-01-01T00:00:00Z", 1.0);
    closed.effective_until = Some(dt("2026-06-01T00:00:00Z"));
    let rows = vec![closed];
    let usage = TokenUsage {
        input: 1_000_000,
        ..TokenUsage::default()
    };

    assert!(
        cost_from_rows(&rows, "closed-model", dt("2026-06-01T00:00:00Z"), &usage).is_none(),
        "effective_until is exclusive"
    );
    assert!(cost_from_rows(&rows, "closed-model", dt("2026-05-31T23:59:59Z"), &usage).is_some());
}
