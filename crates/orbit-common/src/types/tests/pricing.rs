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
        output: 1_000_000,
    };
    let cost = derive_cost_usd("claude-opus-4-7", dt("2026-06-01T00:00:00Z"), &usage)
        .expect("claude-opus-4-7 is priced in the shipped table");
    assert!((cost - 90.0).abs() < f64::EPSILON, "cost was {cost}");
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

fn flat_row(model: &str, effective_from: &str, input_per_million_usd: f64) -> PriceRow {
    PriceRow {
        model: model.to_string(),
        effective_from: dt(effective_from),
        effective_until: None,
        input_per_million_usd,
        cache_read_per_million_usd: 0.0,
        cache_create_per_million_usd: 0.0,
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
