// Migrated from the inline `state_env_var_tests` block in src/context.rs
// when the module was decomposed (ORB-10015).
use std::collections::HashMap;
use std::path::PathBuf;

use chrono::Utc;
use orbit_common::types::Activity;
use serde_json::{Value, json};

use super::super::{ExecutionContext, ProvenanceEnv, provenance_env, state_env_vars};

fn activity_with_id(id: &str) -> Activity {
    let now = Utc::now();
    Activity {
        id: id.to_string(),
        spec_type: "agent_invoke".to_string(),
        description: String::new(),
        input_schema_json: json!({}),
        output_schema_json: json!({}),
        spec_config: json!({}),
        tools: Vec::new(),
        proc_allowed_programs: Vec::new(),
        executor: None,
        workspace_path: None,
        created_by: None,
        is_active: true,
        created_at: now,
        updated_at: now,
    }
}

fn execution_with(input: Value, run_id: Option<&str>) -> ExecutionContext {
    ExecutionContext {
        activity: activity_with_id("agent_implement"),
        job: None,
        agent_cli: "claude".to_string(),
        model: None,
        timeout_seconds: 60,
        env_extra: Vec::new(),
        env_set: HashMap::new(),
        input,
        debug: false,
        steps_outputs: HashMap::new(),
        run_id: run_id.map(ToOwned::to_owned),
        step_index: run_id.map(|_| 2),
        state_dir: run_id.map(|_| PathBuf::from("/tmp/state")),
    }
}

#[test]
fn provenance_env_preserves_both_namespace_contracts() {
    let vars = provenance_env(ProvenanceEnv {
        orbit_run_id: Some("jrun-42"),
        orbit_managed_run_context: true,
        orbit_agent_name: Some("codex"),
        orbit_agent_model: Some("gpt-5.6-sol"),
        orbit_session_id: Some("session-7"),
        orbit_task_id: Some("ORB-10344"),
        orbit_active_task: true,
        agent_run_id: Some("jrun-42"),
        agent_model: Some("gpt-5.6-sol"),
        agent_task_id: Some("ORB-10344"),
    });

    assert_eq!(
        vars,
        vec![
            ("ORBIT_RUN_ID".to_string(), "jrun-42".to_string()),
            ("ORBIT_MANAGED_RUN_CONTEXT".to_string(), "1".to_string()),
            ("ORBIT_AGENT_NAME".to_string(), "codex".to_string()),
            ("ORBIT_AGENT_MODEL".to_string(), "gpt-5.6-sol".to_string()),
            ("ORBIT_SESSION_ID".to_string(), "session-7".to_string()),
            ("ORBIT_TASK_ID".to_string(), "ORB-10344".to_string()),
            ("ORBIT_ACTIVE_TASK_ID".to_string(), "ORB-10344".to_string()),
            ("AGENT_RUN_ID".to_string(), "jrun-42".to_string()),
            ("AGENT_MODEL".to_string(), "gpt-5.6-sol".to_string()),
            ("AGENT_TASK".to_string(), "ORB-10344".to_string()),
        ]
    );
}

#[test]
fn provenance_env_omits_unknown_fields() {
    assert!(provenance_env(ProvenanceEnv::default()).is_empty());
}

#[test]
fn state_env_vars_emits_activity_and_task_ids_without_run_state() {
    let exec = execution_with(json!({ "task_id": "T20260428-7" }), None);
    let vars: HashMap<String, String> = state_env_vars(&exec).into_iter().collect();
    assert_eq!(
        vars.get("ORBIT_ACTIVITY_ID").map(String::as_str),
        Some("agent_implement")
    );
    assert_eq!(
        vars.get("ORBIT_TASK_ID").map(String::as_str),
        Some("T20260428-7")
    );
    assert_eq!(
        vars.get("ORBIT_ACTIVE_TASK_ID").map(String::as_str),
        Some("T20260428-7")
    );
    assert!(!vars.contains_key("ORBIT_RUN_ID"));
}

#[test]
fn state_env_vars_emits_full_set_inside_a_run() {
    let exec = execution_with(json!({ "task_id": "T-abc" }), Some("jrun-42"));
    let vars: HashMap<String, String> = state_env_vars(&exec).into_iter().collect();
    assert_eq!(vars.get("ORBIT_TASK_ID").map(String::as_str), Some("T-abc"));
    assert_eq!(
        vars.get("ORBIT_ACTIVE_TASK_ID").map(String::as_str),
        Some("T-abc")
    );
    assert_eq!(
        vars.get("ORBIT_RUN_ID").map(String::as_str),
        Some("jrun-42")
    );
    assert_eq!(vars.get("ORBIT_STEP_INDEX").map(String::as_str), Some("2"));
    assert_eq!(
        vars.get("ORBIT_ACTIVITY_ID").map(String::as_str),
        Some("agent_implement")
    );
}

#[test]
fn state_env_vars_omits_task_id_when_input_lacks_it() {
    let exec = execution_with(json!({}), None);
    let vars: HashMap<String, String> = state_env_vars(&exec).into_iter().collect();
    assert_eq!(
        vars.get("ORBIT_ACTIVITY_ID").map(String::as_str),
        Some("agent_implement")
    );
    assert!(!vars.contains_key("ORBIT_TASK_ID"));
    assert!(!vars.contains_key("ORBIT_ACTIVE_TASK_ID"));
}
