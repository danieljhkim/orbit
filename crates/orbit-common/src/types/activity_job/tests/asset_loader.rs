use super::super::asset_loader::*;

fn agent_loop_activity_yaml(name: &str, tools: &str) -> String {
    format!(
        r#"schemaVersion: 2
kind: Activity
metadata:
  name: {name}
spec:
  type: agent_loop
  description: Test agent loop.
  instruction: Test.
  tools:
{tools}"#
    )
}

#[test]
fn load_activity_asset_accepts_task_wildcard_tool_allowlist() {
    let yaml = agent_loop_activity_yaml("task_tools", "    - orbit.task.*\n");

    let asset = load_activity_asset(&yaml).expect("activity should load");

    assert_eq!(asset.name, "task_tools");
}

#[test]
fn load_activity_asset_rejects_top_level_orbit_wildcard() {
    let yaml = agent_loop_activity_yaml("broad_tools", "    - orbit.*\n");

    let err = load_activity_asset(&yaml).expect_err("broad wildcard should fail");
    let message = err.to_string();

    assert!(message.contains("orbit.*"), "{message}");
    assert!(message.contains("wildcard root not permitted"), "{message}");
}

#[test]
fn load_activity_asset_rejects_empty_tool_name() {
    let yaml = agent_loop_activity_yaml("empty_tools", "    - \"\"\n");

    let err = load_activity_asset(&yaml).expect_err("empty tool should fail");
    let message = err.to_string();

    assert!(message.contains("empty tool name"), "{message}");
    assert!(message.contains("index 0"), "{message}");
}
