// Migrated from sqlite/invocation_store/tests/records.rs (nested anti-pattern under
// invocation_store.rs) to sibling under `sqlite/tests/` per ORB-00247 and
// docs/design-patterns/test_layout.md.

use orbit_common::test_fixtures::{TEST_CODEX_MODEL, TEST_GEMINI_MODEL};
use orbit_common::types::{InvocationTrace, RoleSlot, TokenUsage, ToolCallTrace};

// Frozen production Claude model literal, chosen because the shipped
// `assets/model_prices.yaml` prices it (unlike the frozen test fixtures,
// which are deliberately kept out of the production price table).
const PRICED_MODEL: &str = "claude-opus-4-7";

use super::super::invocation_store::{InvocationInsertParams, InvocationQuery};
use crate::Store;

#[test]
fn invocation_records_persist_planning_duel_slot() {
    let store = Store::open_in_memory().expect("open store");

    store
        .insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: "jrun-1".to_string(),
            activity_id: "propose_duel_plan".to_string(),
            agent: "gemini".to_string(),
            model: Some(TEST_GEMINI_MODEL.to_string()),
            slot: Some(RoleSlot::PlannerA),
            task_ids: vec!["ORB-1".to_string()],
            trace: InvocationTrace::default(),
        })
        .expect("insert invocation");

    let records = store
        .list_invocation_records(&InvocationQuery {
            job_run_id: Some("jrun-1".to_string()),
            slot: Some(RoleSlot::PlannerA),
            limit: 10,
            ..InvocationQuery::default()
        })
        .expect("list records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].slot, Some(RoleSlot::PlannerA));
}

#[test]
fn invocation_records_persist_non_duel_slot_as_null() {
    let store = Store::open_in_memory().expect("open store");

    store
        .insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: "jrun-2".to_string(),
            activity_id: "implement_one".to_string(),
            agent: "codex".to_string(),
            model: Some(TEST_CODEX_MODEL.to_string()),
            slot: None,
            task_ids: vec!["ORB-2".to_string()],
            trace: InvocationTrace::default(),
        })
        .expect("insert invocation");

    let records = store
        .list_invocation_records(&InvocationQuery {
            job_run_id: Some("jrun-2".to_string()),
            limit: 10,
            ..InvocationQuery::default()
        })
        .expect("list records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].slot, None);
}

#[test]
fn invocation_records_filter_by_nested_task_and_tool() {
    let store = Store::open_in_memory().expect("open store");

    store
        .insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: "jrun-filter-match".to_string(),
            activity_id: "implement_one".to_string(),
            agent: "codex".to_string(),
            model: Some(TEST_CODEX_MODEL.to_string()),
            slot: None,
            task_ids: vec!["ORB-1".to_string()],
            trace: InvocationTrace {
                usage: TokenUsage {
                    input: 10,
                    output: 5,
                    ..Default::default()
                },
                tool_calls: vec![ToolCallTrace {
                    seq: 0,
                    tool_name: "fs.read".to_string(),
                    result_bytes: 42,
                    result_payload: None,
                }],
                duration_ms: 100,
                provider_model: None,
                provider_cost_usd: Some(0.5),
            },
        })
        .expect("insert matching invocation");
    store
        .insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: "jrun-filter-other".to_string(),
            activity_id: "implement_one".to_string(),
            agent: "codex".to_string(),
            model: Some(TEST_CODEX_MODEL.to_string()),
            slot: None,
            task_ids: vec!["ORB-2".to_string()],
            trace: InvocationTrace {
                tool_calls: vec![ToolCallTrace {
                    seq: 0,
                    tool_name: "fs.write".to_string(),
                    result_bytes: 9,
                    result_payload: None,
                }],
                ..Default::default()
            },
        })
        .expect("insert other invocation");

    let records = store
        .list_invocation_records(&InvocationQuery {
            task_id: Some("ORB-1".to_string()),
            tool_name: Some("fs.read".to_string()),
            limit: 10,
            ..Default::default()
        })
        .expect("list filtered records");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].job_run_id, "jrun-filter-match");
    assert_eq!(records[0].task_ids, vec!["ORB-1"]);
    assert_eq!(records[0].tool_calls[0].tool_name, "fs.read");
    assert_eq!(records[0].provider_cost_usd, Some(0.5));
}

#[test]
fn invocation_records_derive_cost_from_price_table_and_keep_provider_cost() {
    let store = Store::open_in_memory().expect("open store");

    store
        .insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: "jrun-priced".to_string(),
            activity_id: "implement_one".to_string(),
            agent: "claude".to_string(),
            model: Some(PRICED_MODEL.to_string()),
            slot: None,
            task_ids: vec!["ORB-3".to_string()],
            trace: InvocationTrace {
                usage: TokenUsage {
                    input: 1_000_000,
                    output: 1_000_000,
                    ..Default::default()
                },
                // Deliberately different from the derived figure so the test
                // proves the two never collapse into one number.
                provider_cost_usd: Some(123.45),
                ..Default::default()
            },
        })
        .expect("insert priced invocation");

    let records = store
        .list_invocation_records(&InvocationQuery {
            job_run_id: Some("jrun-priced".to_string()),
            limit: 10,
            ..Default::default()
        })
        .expect("list priced records");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider_cost_usd, Some(123.45));
    let derived = records[0]
        .derived_cost_usd
        .expect("claude-opus-4-7 is priced in the shipped table");
    // claude-opus-4-7: 1M input @ $5 + 1M output @ $25 = $30.
    assert!(
        (derived - 30.0).abs() < f64::EPSILON,
        "derived cost was {derived}"
    );
}

#[test]
fn invocation_records_round_trip_one_hour_cache_writes_and_derive_ground_truth() {
    // End-to-end ground truth: persist the exact token split from worker run
    // 91d7ef01 (claude-opus-4-8[1m]) and confirm the `cache_create_1h_tokens`
    // column round-trips so the read-time derivation reproduces the
    // provider-reported cost of $1.014018.
    let store = Store::open_in_memory().expect("open store");

    store
        .insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: "jrun-1h".to_string(),
            activity_id: "implement_one".to_string(),
            agent: "claude".to_string(),
            model: Some("claude-opus-4-8[1m]".to_string()),
            slot: None,
            task_ids: vec!["ORB-5".to_string()],
            trace: InvocationTrace {
                usage: TokenUsage {
                    input: 36,
                    cache_read: 858_526,
                    cache_create: 0,
                    cache_create_1h: 37_795,
                    output: 8_265,
                },
                provider_cost_usd: Some(1.014_018),
                ..Default::default()
            },
        })
        .expect("insert 1h-cache invocation");

    let records = store
        .list_invocation_records(&InvocationQuery {
            job_run_id: Some("jrun-1h".to_string()),
            limit: 10,
            ..Default::default()
        })
        .expect("list records");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].cache_create_1h_tokens, 37_795);
    let derived = records[0].derived_cost_usd.expect("priced");
    assert!(
        (derived - 1.014_018).abs() < 1e-6,
        "derived cost was {derived}, expected ~1.014018"
    );
}

#[test]
fn invocation_records_leave_derived_cost_none_for_an_unpriced_model() {
    let store = Store::open_in_memory().expect("open store");

    store
        .insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: "jrun-unpriced".to_string(),
            activity_id: "implement_one".to_string(),
            agent: "codex".to_string(),
            model: Some("some-unpriced-model".to_string()),
            slot: None,
            task_ids: vec!["ORB-4".to_string()],
            trace: InvocationTrace::default(),
        })
        .expect("insert unpriced invocation");

    let records = store
        .list_invocation_records(&InvocationQuery {
            job_run_id: Some("jrun-unpriced".to_string()),
            limit: 10,
            ..Default::default()
        })
        .expect("list unpriced records");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider_cost_usd, None);
    assert_eq!(records[0].derived_cost_usd, None);
}
