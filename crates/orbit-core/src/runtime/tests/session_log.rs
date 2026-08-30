//! Retained Core session-log store coverage after the public-tool withdrawal
//! [ORB-11097]. The store, persistence, and scan consumer stay; the MCP /
//! `orbit tool list` family does not.

use orbit_store::compose::workspace_session_log_store;
use orbit_store::contracts::{SessionLogAppendParams, SessionLogFilter, SessionLogKind};
use serde_json::json;

use crate::OrbitRuntime;

#[test]
fn session_log_store_round_trip_on_temp_root() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let store = workspace_session_log_store(runtime.paths().orbit_dir.clone());

    let appended = store
        .append(SessionLogAppendParams {
            kind: SessionLogKind::CheckLater,
            body: "revisit scan exclusions".to_string(),
            related_task_ids: vec!["ORB-10784".to_string()],
            related_run_ids: Vec::new(),
        })
        .expect("append");
    assert_eq!(appended.id, "SL-0001");
    assert_eq!(appended.kind, SessionLogKind::CheckLater);

    store
        .append(SessionLogAppendParams {
            kind: SessionLogKind::Status,
            body: "first fire, nothing else".to_string(),
            related_task_ids: Vec::new(),
            related_run_ids: Vec::new(),
        })
        .expect("status");

    let unresolved = store
        .list(SessionLogFilter {
            unresolved_only: true,
            ..SessionLogFilter::default()
        })
        .expect("list unresolved");
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].id, "SL-0001");

    let resolved = store.resolve("SL-0001").expect("resolve");
    assert!(resolved.resolved_at.is_some());

    let after = store
        .list(SessionLogFilter {
            unresolved_only: true,
            ..SessionLogFilter::default()
        })
        .expect("list after resolve");
    assert!(after.is_empty());
}

#[test]
fn session_log_tools_are_absent_from_the_runtime_tool_surface() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    for name in [
        "orbit.session_log.append",
        "orbit.session_log.list",
        "orbit.session_log.resolve",
    ] {
        runtime
            .run_tool(name, json!({}))
            .expect_err("withdrawn session-log tool must not dispatch");
    }
}
