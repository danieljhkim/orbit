//! Tests for runtime identity resolution in orbit tools.

use serde_json::json;

use super::super::*;

#[test]
fn runtime_identity_overwrites_self_reported_model_at_tool_boundary() {
    let ctx = tool_context("claude", "claude-opus-4-7");

    let identity =
        resolve_identity(&ctx, &json!({ "model": "opus-4.7" })).expect("identity resolves");

    assert_eq!(identity.agent.as_deref(), Some("claude"));
    assert_eq!(identity.model.as_deref(), Some("claude"));
    assert_eq!(identity.actor_label.as_deref(), Some("claude"));
}

fn tool_context(agent: &str, model: &str) -> ToolContext {
    ToolContext {
        cwd: None,
        session_context: Default::default(),
        allowed_tools: Vec::new(),
        workspace_root: None,
        agent_name: Some(agent.to_string()),
        model_name: Some(model.to_string()),
        role_slot: None,
        proc_allowed_programs: Vec::new(),
        proc_spawn_activity_scoped: false,
        policy_engine: None,
        fs_profile: None,
        fs_audit: None,
        reservation_owner: None,
        orbit_host: None,
        groundhog_host: None,
    }
}
