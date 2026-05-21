use tempfile::tempdir;

use super::super::super::args::{McpAction, McpProvider, ProviderSelectionMode, ScopeArg};
use super::super::super::dispatch::run_action;

#[test]
fn codex_workspace_scope_init_and_remove_preserve_unrelated_entries() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(repo.path().join(".codex")).expect("create .codex");
    std::fs::write(
        repo.path().join(".codex").join("config.toml"),
        "model = \"gpt-5.4\"\n[mcp_servers.other]\ncommand = \"demo\"\n",
    )
    .expect("write config");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Codex]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init codex");

    let config = std::fs::read_to_string(repo.path().join(".codex").join("config.toml"))
        .expect("read config");
    let parsed: toml::Value = toml::from_str(&config).expect("parse config");
    assert_eq!(parsed["model"].as_str(), Some("gpt-5.4"));
    assert_eq!(
        parsed["mcp_servers"]["orbit"]["command"].as_str(),
        Some("orbit")
    );
    let args = parsed["mcp_servers"]["orbit"]["args"]
        .as_array()
        .expect("args array");
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].as_str(), Some("mcp"));
    assert_eq!(args[1].as_str(), Some("serve"));
    assert!(parsed["mcp_servers"]["orbit"].get("cwd").is_none());
    assert_eq!(
        parsed["mcp_servers"]["other"]["command"].as_str(),
        Some("demo")
    );

    run_action(
        McpAction::Remove,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Codex]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("remove codex");

    let config = std::fs::read_to_string(repo.path().join(".codex").join("config.toml"))
        .expect("read config");
    let parsed: toml::Value = toml::from_str(&config).expect("parse config");
    assert!(
        parsed
            .get("mcp_servers")
            .and_then(toml::Value::as_table)
            .and_then(|table| table.get("orbit"))
            .is_none()
    );
    assert_eq!(
        parsed["mcp_servers"]["other"]["command"].as_str(),
        Some("demo")
    );
}

#[test]
fn workspace_scope_codex_init_is_idempotent() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Codex]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init codex");
    let first = std::fs::read_to_string(repo.path().join(".codex").join("config.toml"))
        .expect("read first config");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Codex]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init codex again");
    let second = std::fs::read_to_string(repo.path().join(".codex").join("config.toml"))
        .expect("read second config");

    assert_eq!(first, second);
}