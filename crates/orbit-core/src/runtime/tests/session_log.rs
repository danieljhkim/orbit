//! Tool-surface coverage for `orbit.session_log.*` [ORB-10784].

use serde_json::json;

use crate::OrbitRuntime;

#[test]
fn session_log_tools_round_trip_on_temp_root() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");

    let appended = runtime
        .run_tool(
            "orbit.session_log.append",
            json!({
                "kind": "check_later",
                "body": "revisit scan exclusions",
                "related_task_ids": ["ORB-10784"]
            }),
        )
        .expect("append");
    assert_eq!(appended["id"], "SL-0001");
    assert_eq!(appended["kind"], "check_later");

    runtime
        .run_tool(
            "orbit.session_log.append",
            json!({ "kind": "status", "body": "first fire, nothing else" }),
        )
        .expect("status");

    let unresolved = runtime
        .run_tool("orbit.session_log.list", json!({ "unresolved_only": true }))
        .expect("list unresolved");
    assert_eq!(unresolved["count"], 1);
    assert_eq!(unresolved["entries"][0]["id"], "SL-0001");

    let resolved = runtime
        .run_tool("orbit.session_log.resolve", json!({ "id": "SL-0001" }))
        .expect("resolve");
    assert!(resolved["resolved_at"].as_str().is_some());

    let after = runtime
        .run_tool("orbit.session_log.list", json!({ "unresolved_only": true }))
        .expect("list after resolve");
    assert_eq!(after["count"], 0);
}
