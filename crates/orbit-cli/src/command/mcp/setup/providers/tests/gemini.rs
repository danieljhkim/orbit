use tempfile::tempdir;

use super::super::super::args::{McpAction, McpProvider, ProviderSelectionMode, ScopeArg};
use super::super::super::dispatch::run_action;

#[test]
fn gemini_workspace_scope_init_and_remove_preserve_unrelated_entries() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(repo.path().join(".gemini")).expect("create .gemini");
    std::fs::write(
        repo.path().join(".gemini").join("settings.json"),
        "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"other\": {\"command\": \"demo\"}\n  }\n}\n",
    )
    .expect("write settings");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Gemini]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init gemini");

    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.path().join(".gemini").join("settings.json"))
            .expect("read settings"),
    )
    .expect("parse settings");
    assert_eq!(settings["theme"], "dark");
    assert!(settings["mcpServers"]["orbit"].is_object());
    assert!(settings["mcpServers"]["other"].is_object());
    assert!(settings["mcpServers"]["orbit"]["cwd"].is_null());
    let args = settings["mcpServers"]["orbit"]["args"]
        .as_array()
        .expect("args array");
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].as_str(), Some("mcp"));
    assert_eq!(args[1].as_str(), Some("serve"));

    run_action(
        McpAction::Remove,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Gemini]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("remove gemini");

    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.path().join(".gemini").join("settings.json"))
            .expect("read settings"),
    )
    .expect("parse settings");
    assert_eq!(settings["theme"], "dark");
    assert!(settings["mcpServers"]["orbit"].is_null());
    assert!(settings["mcpServers"]["other"].is_object());
}

#[test]
fn workspace_scope_gemini_init_is_idempotent() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Gemini]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init gemini");
    let first = std::fs::read_to_string(repo.path().join(".gemini").join("settings.json"))
        .expect("read first settings");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Gemini]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init gemini again");
    let second = std::fs::read_to_string(repo.path().join(".gemini").join("settings.json"))
        .expect("read second settings");

    assert_eq!(first, second);
}
