// Migrated from sqlite/invocation_store/tests/records.rs (nested anti-pattern under
// invocation_store.rs) to sibling under `sqlite/tests/` per ORB-00247 and
// docs/design-patterns/test_layout.md.

use chrono::{TimeZone, Utc};
use orbit_common::test_fixtures::TEST_CODEX_MODEL;
use orbit_types::telemetry::{InvocationTrace, TokenUsage, ToolCallTrace};

// Frozen production Claude model literal, chosen because the shipped
// `assets/model_prices.yaml` prices it (unlike the frozen test fixtures,
// which are deliberately kept out of the production price table).
const PRICED_MODEL: &str = "claude-opus-4-7";

use crate::Store;
use crate::contracts::{InvocationAccountingQuery, InvocationInsertParams, InvocationQuery};

#[test]
fn invocation_records_filter_by_nested_task_and_tool() {
    let store = Store::open_in_memory().expect("open store");

    store
        .insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: "jrun-filter-match".to_string(),
            activity_id: "implement_one".to_string(),
            agent: "codex".to_string(),
            model: Some(TEST_CODEX_MODEL.to_string()),
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
fn historical_invocation_is_repriced_at_query_time_without_a_migration() {
    let store = Store::open_in_memory().expect("open store");

    store
        .insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: "jrun-historical-gpt".to_string(),
            activity_id: "implement_one".to_string(),
            agent: "codex".to_string(),
            model: Some("gpt-5.6-terra".to_string()),
            task_ids: vec!["ORB-10579".to_string()],
            trace: InvocationTrace {
                usage: TokenUsage {
                    // OpenAI input is gross: 500k uncached + 200k read + 300k write.
                    input: 1_000_000,
                    cache_read: 200_000,
                    cache_create: 300_000,
                    output: 1_000_000,
                    ..TokenUsage::default()
                },
                ..InvocationTrace::default()
            },
        })
        .expect("insert invocation");

    // Simulate a row stored before the July 30 price change. No schema or row
    // migration is involved; the query-time lookup uses this persisted date.
    store
        .with_transaction(|tx| {
            tx.connection()
                .execute(
                    "UPDATE invocations SET ts = ?1 WHERE job_run_id = ?2",
                    ["2026-07-29T23:59:59+00:00", "jrun-historical-gpt"],
                )
                .map_err(|error| orbit_common::OrbitError::Store(error.to_string()))?;
            Ok(())
        })
        .expect("set historical timestamp");

    let records = store
        .list_invocation_records(&InvocationQuery {
            job_run_id: Some("jrun-historical-gpt".to_string()),
            limit: 10,
            ..InvocationQuery::default()
        })
        .expect("list historical invocation");

    let derived = records[0]
        .derived_cost_usd
        .expect("historical row is priced");
    // Historical Terra: 0.5M*$2.50 + 0.2M*$0.25 + 0.3M*$3.125 + 1M*$15.
    assert!(
        (derived - 17.2375).abs() < 1e-12,
        "derived cost was {derived}"
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

#[test]
fn accounting_facts_are_unbounded_distinct_and_half_open_without_tool_hydration() {
    let store = Store::open_in_memory().expect("open store");
    let lower = Utc
        .with_ymd_and_hms(2026, 8, 1, 0, 0, 0)
        .single()
        .expect("lower bound");
    let upper = Utc
        .with_ymd_and_hms(2026, 8, 2, 0, 0, 0)
        .single()
        .expect("upper bound");

    for index in 0..125 {
        store
            .insert_invocation_trace_record(&InvocationInsertParams {
                job_run_id: format!("jrun-accounting-{index}"),
                activity_id: "implement".to_string(),
                agent: "codex".to_string(),
                model: Some(PRICED_MODEL.to_string()),
                task_ids: vec![
                    "ORB-DUPLICATE".to_string(),
                    "ORB-DUPLICATE".to_string(),
                    format!("ORB-{index}"),
                ],
                trace: InvocationTrace {
                    usage: TokenUsage {
                        input: 10,
                        cache_read: 2,
                        cache_create: 3,
                        cache_create_1h: 4,
                        output: 5,
                    },
                    tool_calls: vec![ToolCallTrace {
                        seq: 0,
                        tool_name: "fs.read".to_string(),
                        result_bytes: 99,
                        result_payload: None,
                    }],
                    duration_ms: 1,
                    provider_model: None,
                    provider_cost_usd: Some(0.25),
                },
            })
            .expect("insert accounting invocation");
    }

    let connection = store.connection();
    let conn = connection.lock().expect("lock store");
    conn.execute(
        "UPDATE invocations SET ts = ?1",
        [(lower + chrono::Duration::hours(1)).to_rfc3339()],
    )
    .expect("place rows inside window");
    conn.execute(
        "UPDATE invocations SET ts = ?1 WHERE id = 1",
        [lower.to_rfc3339()],
    )
    .expect("place lower boundary");
    conn.execute(
        "UPDATE invocations SET ts = ?1 WHERE id = 125",
        [upper.to_rfc3339()],
    )
    .expect("place upper boundary");
    drop(conn);

    let facts = store
        .list_invocation_accounting_facts(&InvocationAccountingQuery {
            since: Some(lower),
            until: upper,
        })
        .expect("load accounting facts");

    assert_eq!(facts.len(), 124, "the loader has no detailed-list row cap");
    assert_eq!(facts[0].task_ids.len(), 2, "duplicate task ids collapse");
    assert_eq!(facts[0].cache_create_1h_tokens, 4);
    assert_eq!(facts[0].provider_cost_usd, Some(0.25));
    assert!(facts[0].derived_cost_usd.is_some());
    assert!(
        facts.iter().all(|fact| fact.id != 125),
        "until is exclusive"
    );
}

/// The accounting read hydrates every invocation in its window in one go,
/// so its `IN (...)` lists must be chunked below SQLite's bound-parameter
/// cap. 1,100 rows crosses several chunks; every row must still get its own
/// task and tool-call linkage back.
#[test]
fn hydration_spans_several_in_list_chunks_without_losing_linkage() {
    let store = Store::open_in_memory().expect("open store");
    let total = 1_100;
    for index in 0..total {
        store
            .insert_invocation_trace_record(&InvocationInsertParams {
                job_run_id: format!("jrun-chunk-{index}"),
                activity_id: "implement_one".to_string(),
                agent: "codex".to_string(),
                model: Some(TEST_CODEX_MODEL.to_string()),
                task_ids: vec![format!("ORB-{index}")],
                trace: InvocationTrace {
                    tool_calls: vec![ToolCallTrace {
                        seq: 0,
                        tool_name: format!("tool-{index}"),
                        result_bytes: 1,
                        result_payload: None,
                    }],
                    ..Default::default()
                },
            })
            .expect("insert invocation");
    }

    let facts = store
        .list_invocation_accounting_facts(&InvocationAccountingQuery {
            since: None,
            until: Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap(),
        })
        .expect("accounting facts");
    assert_eq!(facts.len(), total);
    assert!(
        facts.iter().all(|fact| fact.task_ids.len() == 1),
        "every fact keeps its task linkage across chunks"
    );

    let records = store
        .list_invocation_records(&InvocationQuery {
            limit: total + 10,
            ..Default::default()
        })
        .expect("detailed records");
    assert_eq!(records.len(), total);
    for record in &records {
        let index = record
            .job_run_id
            .strip_prefix("jrun-chunk-")
            .expect("fixture run id");
        assert_eq!(record.task_ids, vec![format!("ORB-{index}")]);
        assert_eq!(record.tool_calls.len(), 1, "{}", record.job_run_id);
        assert_eq!(record.tool_calls[0].tool_name, format!("tool-{index}"));
    }
}

/// The newest-first listing and the accounting window both order and filter
/// on `ts`; after v20 the planner must satisfy them from the index rather
/// than scanning and sorting the table.
#[test]
fn ts_ordered_reads_use_the_invocations_ts_index() {
    let store = Store::open_in_memory().expect("open store");
    let plan = store
        .with_read_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "EXPLAIN QUERY PLAN SELECT id FROM invocations \
                     WHERE ts >= '2026-01-01' AND ts < '2026-02-01' ORDER BY ts DESC, id DESC LIMIT 10",
                )
                .map_err(|e| orbit_common::OrbitError::Store(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(3))
                .map_err(|e| orbit_common::OrbitError::Store(e.to_string()))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| orbit_common::OrbitError::Store(e.to_string()))
        })
        .expect("query plan");
    let plan = plan.join("\n");
    assert!(plan.contains("idx_invocations_ts"), "{plan}");
    assert!(!plan.contains("USE TEMP B-TREE"), "{plan}");
}
