// ORB-10354: the live-store price-coverage scan that replaces the curated
// fleet-model list as the authoritative coverage signal.

use orbit_common::types::{InvocationTrace, TokenUsage};

use crate::Store;
use crate::sqlite::invocation_store::InvocationInsertParams;

const PRICED_MODEL: &str = "claude-opus-4-7";

fn insert(store: &Store, agent: &str, model: &str) {
    store
        .insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: "jrun-coverage".to_string(),
            activity_id: "implement_one".to_string(),
            agent: agent.to_string(),
            model: Some(model.to_string()),
            slot: None,
            task_ids: Vec::new(),
            trace: InvocationTrace {
                usage: TokenUsage {
                    input: 1_000,
                    output: 1_000,
                    ..TokenUsage::default()
                },
                ..InvocationTrace::default()
            },
        })
        .expect("insert invocation");
}

#[test]
fn an_empty_store_reports_no_unpriced_models() {
    let store = Store::open_in_memory().expect("open store");
    assert!(
        store
            .list_unpriced_invocation_models()
            .expect("scan")
            .is_empty()
    );
}

#[test]
fn the_scan_reports_only_models_with_no_covering_price_row() {
    let store = Store::open_in_memory().expect("open store");
    insert(&store, "claude", PRICED_MODEL);
    // A retired exact model string nobody added a price row for — the class of
    // gap the curated fleet list cannot notice.
    insert(&store, "codex", "gpt-5.5");
    insert(&store, "codex", "gpt-5.5");

    let unpriced = store.list_unpriced_invocation_models().expect("scan");
    assert_eq!(
        unpriced
            .iter()
            .map(|row| row.model.as_str())
            .collect::<Vec<_>>(),
        vec!["gpt-5.5"],
    );
    assert_eq!(unpriced[0].invocation_count, 2);
    assert!(!unpriced[0].first_seen.is_empty());
    assert!(!unpriced[0].last_seen.is_empty());
}

/// The scan is only usable because aliases no longer reach the `model` column:
/// alias rows are deliberately unpriced, so they would otherwise dominate the
/// report as permanent false positives.
#[test]
fn alias_rows_do_not_register_as_unpriced_models() {
    let store = Store::open_in_memory().expect("open store");
    // Resolves to an exact priced string.
    insert(&store, "claude", "opus");
    // Unresolvable: recorded as `model_alias` with a NULL model, which the scan
    // skips because there is no model string to price.
    insert(&store, "gemini", "pro");

    assert!(
        store
            .list_unpriced_invocation_models()
            .expect("scan")
            .is_empty(),
        "crew aliases must not surface as pricing gaps",
    );
}

/// A context-window suffix prices at the base model's rates, and the scan uses
/// the same lookup as cost derivation, so it must not report one.
#[test]
fn a_context_window_suffix_is_not_reported_as_unpriced() {
    let store = Store::open_in_memory().expect("open store");
    insert(&store, "claude", "claude-opus-4-8[1m]");

    assert!(
        store
            .list_unpriced_invocation_models()
            .expect("scan")
            .is_empty()
    );
}
