//! Sibling tests for `envelope.rs` (migrated per ORB-00246 / docs/design-patterns/test_layout.md).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::Activity;
use orbit_engine::ExecutionContext;
use serde_json::{Value, json};
use tempfile::tempdir;

use crate::OrbitRuntime;

use super::super::envelope::build_agent_stdin_envelope_payload;

#[test]
fn agent_envelope_loads_planning_duel_skills_from_global_root() {
    let (_root, runtime, global_root, workspace_root) = test_runtime();
    write_skill(&global_root.join("skills"), "orbit", "global orbit skill");
    write_skill(
        &global_root.join("skills"),
        "orbit-graph",
        "global graph skill",
    );

    let envelope = build_test_envelope(&runtime, &["orbit", "orbit-graph"]);
    let skills = envelope_skills(&envelope);

    assert_eq!(skill_content(&skills, "orbit"), "global orbit skill");
    assert_eq!(skill_content(&skills, "orbit-graph"), "global graph skill");
    assert!(!workspace_root.join("skills").join("orbit").exists());
    assert!(
        !workspace_root
            .join("resources")
            .join("skills")
            .join("orbit")
            .join("SKILL.md")
            .exists()
    );
}

#[test]
fn agent_envelope_preserves_workspace_skill_override_precedence() {
    let (_root, runtime, global_root, workspace_root) = test_runtime();
    write_skill(&global_root.join("skills"), "orbit", "global orbit skill");
    write_skill(
        &global_root.join("skills"),
        "orbit-graph",
        "global graph skill",
    );
    write_skill(
        &workspace_root.join("resources").join("skills"),
        "orbit",
        "workspace orbit override",
    );

    let envelope = build_test_envelope(&runtime, &["orbit", "orbit-graph"]);
    let skills = envelope_skills(&envelope);

    assert_eq!(skill_content(&skills, "orbit"), "workspace orbit override");
    assert_eq!(skill_content(&skills, "orbit-graph"), "global graph skill");
}

fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf, PathBuf) {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    fs::create_dir_all(&global_root).expect("create global root");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build runtime");
    (root, runtime, global_root, workspace_root)
}

fn build_test_envelope(runtime: &OrbitRuntime, skill_refs: &[&str]) -> Value {
    let execution = ExecutionContext {
        activity: test_activity(skill_refs),
        job: None,
        agent_cli: "codex".to_string(),
        model: Some("test-model".to_string()),
        timeout_seconds: 5,
        env_extra: Vec::new(),
        env_set: HashMap::new(),
        input: json!({}),
        debug: false,
        steps_outputs: HashMap::new(),
        run_id: None,
        step_index: None,
        state_dir: None,
    };
    let payload = build_agent_stdin_envelope_payload(runtime, &execution)
        .expect("build agent stdin envelope");
    serde_json::from_slice(&payload).expect("parse envelope json")
}

fn test_activity(skill_refs: &[&str]) -> Activity {
    let now = Utc::now();
    Activity {
        id: "propose_duel_plan".to_string(),
        spec_type: "agent_invoke".to_string(),
        description: "test planning duel activity".to_string(),
        input_schema_json: json!({}),
        output_schema_json: json!({}),
        spec_config: json!({
            "instruction": "Use the injected skills.",
            "skill_refs": skill_refs,
        }),
        tools: Vec::new(),
        proc_allowed_programs: Vec::new(),
        executor: None,
        workspace_path: None,
        created_by: Some("test".to_string()),
        is_active: true,
        created_at: now,
        updated_at: now,
    }
}

fn envelope_skills(envelope: &Value) -> Vec<Value> {
    envelope
        .get("skills")
        .and_then(Value::as_array)
        .expect("skills array")
        .clone()
}

fn skill_content(skills: &[Value], id: &str) -> String {
    skills
        .iter()
        .find(|skill| skill.get("id").and_then(Value::as_str) == Some(id))
        .and_then(|skill| skill.get("content").and_then(Value::as_str))
        .expect("skill content")
        .lines()
        .skip_while(|line| line.trim() != "# Purpose")
        .skip(1)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn write_skill(root: &Path, id: &str, purpose: &str) {
    let dir = root.join(id);
    fs::create_dir_all(&dir).expect("create skill dir");
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: test skill\n---\n\n# Purpose\n\n{purpose}\n"),
    )
    .expect("write skill");
}
