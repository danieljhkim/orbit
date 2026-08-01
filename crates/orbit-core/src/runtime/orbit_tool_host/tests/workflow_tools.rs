use std::collections::BTreeSet;

use chrono::Utc;
use orbit_common::types::{AuditEventStatus, McpCapability, Role, ToolSessionContext};
use orbit_tools::ToolContext;
use serde_json::json;

use super::super::test_support::{run_tool_as_operator, test_runtime};

#[test]
fn operator_can_observe_runs_and_agent_denial_is_audited() {
    let (_root, runtime, _repo_root) = test_runtime();
    let run = runtime
        .stores()
        .jobs()
        .insert_job_run("task_auto_pipeline", 1, Utc::now(), None, None)
        .expect("insert run");

    let shown = run_tool_as_operator(
        &runtime,
        "orbit.workflow.run.show",
        json!({"id": run.run_id}),
    )
    .expect("operator run show");
    assert_eq!(shown["run_id"], json!(run.run_id));

    let listed = run_tool_as_operator(&runtime, "orbit.workflow.run.list", json!({}))
        .expect("operator run list");
    assert_eq!(listed["items"][0]["run_id"], json!(run.run_id));

    let denied = runtime.run_tool_with_context_and_role(
        "orbit.workflow.run.show",
        json!({"id": run.run_id}),
        Role::Admin,
        ToolContext {
            session_context: ToolSessionContext {
                effective_capabilities: BTreeSet::from([McpCapability::Agent]),
                ..ToolSessionContext::default()
            },
            ..ToolContext::default()
        },
    );
    assert!(
        matches!(
            denied,
            Err(orbit_common::types::OrbitError::CapabilityDenied(_))
        ),
        "{denied:?}"
    );

    let audit = runtime
        .list_audit_events(None, None, Some(AuditEventStatus::Denied), None, 20)
        .expect("read denial audit");
    assert!(audit.iter().any(|event| {
        event.command == "authorization"
            && event.target_id.as_deref() == Some("orbit.workflow.run.show")
    }));
}
