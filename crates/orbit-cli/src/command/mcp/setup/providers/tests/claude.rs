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
    // The AC names the exact post-fix shape literally; pin it here so a
    // regression in `claude_permission_name` cannot pass the test above.
    assert!(
        allow
            .iter()
            .any(|item| item == "mcp__orbit__orbit_task_show"),
        "Claude allowlist must contain literal `mcp__orbit__orbit_task_show` \
         (server-id-derived name for the CLI-registered `orbit` MCP server)",
    );
    assert!(
        !allow
            .iter()
            .any(|item| item.as_str().is_some_and(|s| s.starts_with("mcp__plugin_"))),
        "CLI init must not emit Claude Code plugin-scoped permission names; \
         that shape is synthesized by Claude itself for plugin installs",
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

#[test]
fn claude_remove_strips_legacy_plugin_prefixed_entries() {
    // Pre-ORB-00286 the CLI wrote `mcp__plugin_orbit_orbit__*` entries
    // into Claude settings. After the fix `init` no longer emits them,
    // but existing user settings still carry them. `remove --claude`
    // must strip the legacy entries so an upgrade leaves a clean file,
    // while preserving unrelated `permissions.allow` entries.
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(repo.path().join(".claude")).expect("create .claude");
    std::fs::write(
        repo.path().join(".mcp.json"),
        "{\n  \"mcpServers\": {\n    \"orbit\": {\"command\": \"orbit\", \"args\": [\"mcp\", \"serve\"]}\n  }\n}\n",
    )
    .expect("write mcp file");
    std::fs::write(
        repo.path().join(".claude").join("settings.json"),
        "{\n  \"permissions\": {\n    \"allow\": [\n      \"OtherTool\",\n      \"mcp__plugin_orbit_orbit__orbit_task_show\",\n      \"mcp__plugin_orbit_orbit__orbit_search\"\n    ]\n  }\n}\n",
    )
    .expect("write settings");

    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Remove,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Claude]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("remove claude");

    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.path().join(".claude").join("settings.json"))
            .expect("read settings"),
    )
    .expect("parse settings");
    let allow = settings["permissions"]["allow"]
        .as_array()
        .expect("allow array");
    assert!(
        allow.iter().any(|item| item == "OtherTool"),
        "unrelated permission entries must survive remove",
    );
    assert!(
        !allow
            .iter()
            .any(|item| item.as_str().is_some_and(|s| s.starts_with("mcp__plugin_"))),
        "legacy plugin-prefixed entries must be stripped by remove",
    );
}
