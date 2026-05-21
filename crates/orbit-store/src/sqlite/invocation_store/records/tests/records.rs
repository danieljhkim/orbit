// Migrated from sqlite/invocation_store/records.rs per ORB-00231
use orbit_common::types::{InvocationTrace, RoleSlot, TokenUsage, ToolCallTrace};

use super::super::*;

#[test]
fn invocation_records_persist_planning_duel_slot() {
    let store = Store::open_in_memory().expect("open store");

    store
        .insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: "jrun-1".to_string(),
            activity_id: "propose_duel_plan".to_string(),
            agent: "gemini".to_string(),
            model: Some("gemini-3.1-pro".to_string()),
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
            model: Some("gpt-5.5".to_string()),
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
            model: Some("gpt-5.5".to_string()),
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
            },
        })
        .expect("insert matching invocation");
    store
        .insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: "jrun-filter-other".to_string(),
            activity_id: "implement_one".to_string(),
            agent: "codex".to_string(),
            model: Some("gpt-5.5".to_string()),
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
}
