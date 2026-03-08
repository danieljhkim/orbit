use chrono::Utc;
use orbit_core::OrbitRuntime;
use orbit_core::agent::context::{compose_agent_context, parse_planned_tool_calls};
use orbit_core::command::task::TaskAddParams;
use orbit_types::{Role, Skill};
use tempfile::tempdir;

fn sample_skill(
    name: &str,
    tools: &[&str],
    role: Role,
    context_files: &[&str],
    instructions: &str,
) -> Skill {
    Skill {
        schema_version: 1,
        name: name.to_string(),
        description: None,
        instructions: instructions.to_string(),
        context_files: context_files.iter().map(|v| v.to_string()).collect(),
        allowed_tools: tools.iter().map(|v| v.to_string()).collect(),
        role,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn composition_merges_in_deterministic_order_and_hash_is_stable() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let task = runtime
        .add_task(TaskAddParams {
            title: "compose".to_string(),
            description: "task description".to_string(),
            plan: r#"{"tool_calls":[{"name":"fs.read","input":{"path":"README.md"}}]}"#.to_string(),
            context_files: vec!["A.md".to_string(), "B.md".to_string()],
            ..Default::default()
        })
        .expect("task");

    let task = runtime.get_task(&task.id).expect("task");
    let skills = vec![
        sample_skill(
            "s1",
            &["fs.read", "fs.write"],
            Role::Admin,
            &["B.md", "C.md"],
            "skill 1",
        ),
        sample_skill(
            "s2",
            &["fs.read"],
            Role::Agent,
            &["C.md", "D.md"],
            "skill 2",
        ),
    ];
    let c1 = compose_agent_context(&runtime, &task, &skills, Role::Admin, None).expect("compose");
    let c2 = compose_agent_context(&runtime, &task, &skills, Role::Admin, None).expect("compose");

    assert_eq!(
        c1.context_files,
        vec![
            "A.md".to_string(),
            "B.md".to_string(),
            "C.md".to_string(),
            "D.md".to_string()
        ]
    );
    assert_eq!(
        c1.allowed_tools_raw,
        Some(vec!["fs.read".to_string()]),
        "intersection should preserve deterministic order"
    );
    assert_eq!(c1.role, Role::Agent, "most restrictive role wins");
    assert_eq!(
        c1.effective_allowed_tools,
        vec!["fs.read".to_string()],
        "policy ∩ skill allowlist"
    );
    assert_eq!(c1.composed_context_hash, c2.composed_context_hash);
}

#[test]
fn parse_planned_tool_calls_from_task_plan_payload() {
    let payload = r#"{
      "tool_calls": [
        {"name":"fs.read","input":{"path":"README.md"}},
        {"name":"time.now","input":{}}
      ]
    }"#;

    let calls = parse_planned_tool_calls(payload).expect("parse");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "fs.read");
    assert_eq!(calls[1].name, "time.now");
}

#[test]
fn empty_effective_allowlist_is_rejected() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let task = runtime
        .add_task(TaskAddParams {
            title: "compose".to_string(),
            plan: r#"{"tool_calls":[{"name":"fs.read","input":{"path":"README.md"}}]}"#.to_string(),
            ..Default::default()
        })
        .expect("task");

    let task = runtime.get_task(&task.id).expect("task");
    let skills = vec![
        sample_skill("only-write", &["fs.write"], Role::Agent, &[], "only write"),
        sample_skill("only-read", &["fs.read"], Role::Agent, &[], "only read"),
    ];
    let result = compose_agent_context(&runtime, &task, &skills, Role::Agent, None);
    assert!(
        result.is_err(),
        "disjoint skill allowlists should produce empty intersection"
    );
}
