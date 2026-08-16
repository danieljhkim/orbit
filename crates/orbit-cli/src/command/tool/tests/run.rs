use serde_json::json;

use super::super::run::{
    LOCAL_MACHINE_ID_FALLBACK, local_machine_identity, local_tool_session_context,
    shape_tool_output,
};

#[test]
fn list_output_uses_minimal_task_projection() {
    let shaped = shape_tool_output(
        "orbit.task.list",
        &json!({ "status": "backlog" }),
        json!([{
            "id": "T20260422-0001",
            "title": "Backlog task",
            "status": "backlog",
            "priority": "medium",
            "type": "feature",
            "dependencies": [],
            "resolved_dependencies": [],
            "implemented_by": null,
            "created_at": "2026-04-22T00:00:00Z",
            "updated_at": "2026-04-22T00:00:00Z",
            "description": "should be filtered out"
        }]),
        false,
        &[],
    );

    assert_eq!(
        shaped,
        json!([{
            "id": "T20260422-0001",
            "title": "Backlog task",
            "status": "backlog",
            "priority": "medium",
            "type": "feature",
            "dependencies": [],
            "resolved_dependencies": [],
            "implemented_by": null,
            "created_at": "2026-04-22T00:00:00Z",
            "updated_at": "2026-04-22T00:00:00Z"
        }])
    );
}

#[test]
fn local_invocation_context_has_trace_and_explicit_identity_fallback() {
    let runtime = orbit_core::OrbitRuntime::in_memory().expect("in-memory runtime");

    let context = local_tool_session_context(&runtime).expect("local invocation context");

    assert!(
        context
            .trace_id
            .as_deref()
            .is_some_and(|trace| trace.starts_with("trace-"))
    );
    assert_eq!(
        context.caller_machine_id.as_deref(),
        Some(LOCAL_MACHINE_ID_FALLBACK)
    );
    assert_eq!(
        context.process_machine_id.as_deref(),
        Some(LOCAL_MACHINE_ID_FALLBACK)
    );
    assert_eq!(context.caller_ip, None);
    assert!(context.effective_capabilities.is_empty());
}

#[test]
fn local_machine_identity_prefers_persisted_host_identity() {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        root.path().join("host.toml"),
        "schema_version = 2\nmachine_id = \"hm_cli\"\nhost_id = \"cli-host\"\ntask_prefix = \"CLI\"\n",
    )
    .expect("write host identity");

    let identity = local_machine_identity(root.path()).expect("load local machine identity");

    assert_eq!(
        identity,
        ("hm_cli".to_string(), Some("cli-host".to_string()))
    );
}
