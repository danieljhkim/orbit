use tempfile::tempdir;

use super::super::super::args::{McpAction, McpProvider, ProviderSelectionMode, ScopeArg};
use super::super::super::dispatch::run_action;
use super::super::claude::*;

#[test]
fn claude_workspace_scope_init_and_remove_preserve_unrelated_entries() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(repo.path().join(".claude")).expect("create .claude");
    std::fs::write(
        repo.path().join(".mcp.json"),
        "{\n  \"mcpServers\": {\n    \"other\": {\"command\": \"demo\"}\n  }\n}\n",
    )
    .expect("write mcp file");
    std::fs::write(
        repo.path().join(".claude").join("settings.json"),
        "{\n  \"permissions\": {\n    \"allow\": [\"OtherTool\"]\n  },\n  \"theme\": \"light\"\n}\n",
    )
    .expect("write settings");

    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    let providers = run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Claude]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init claude");
    assert_eq!(providers, vec![McpProvider::Claude]);

    let mcp: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.path().join(".mcp.json")).expect("read mcp"),
    )
    .expect("parse mcp");
    assert!(mcp["mcpServers"]["orbit"].is_object());
    assert!(mcp["mcpServers"]["other"].is_object());
    let args = mcp["mcpServers"]["orbit"]["args"]
        .as_array()
        .expect("args array");
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].as_str(), Some("mcp"));
    assert_eq!(args[1].as_str(), Some("serve"));

    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.path().join(".claude").join("settings.json"))
            .expect("read settings"),
    )
    .expect("parse settings");
    let allow = settings["permissions"]["allow"]
        .as_array()
        .expect("allow array");
    assert!(allow.iter().any(|item| item == "OtherTool"));
    assert!(
        allow
            .iter()
            .any(|item| item == &claude_permission_name("orbit.task.show"))
    );
    assert_eq!(settings["theme"], "light");

    run_action(
        McpAction::Remove,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Claude]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("remove claude");

    let mcp: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.path().join(".mcp.json")).expect("read mcp"),
    )
    .expect("parse mcp");
    assert!(mcp["mcpServers"]["orbit"].is_null());
    assert!(mcp["mcpServers"]["other"].is_object());
}
