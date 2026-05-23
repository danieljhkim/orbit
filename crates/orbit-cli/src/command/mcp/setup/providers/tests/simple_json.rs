use tempfile::tempdir;

use super::super::super::args::{McpAction, McpProvider, ProviderSelectionMode, ScopeArg};
use super::super::super::dispatch::{auto_detected_providers, run_action, vscode_home_user_dir};
use super::super::claude::claude_mcp_server_value;
use super::super::codex::codex_mcp_server_table;
use super::super::simple_json::simple_mcp_server_value;

#[test]
fn server_value_builders_emit_mcp_serve_only() {
    let claude = claude_mcp_server_value();
    let claude_args = claude["args"].as_array().expect("claude args");
    assert_eq!(claude_args.len(), 2);
    assert_eq!(claude_args[0].as_str(), Some("mcp"));
    assert_eq!(claude_args[1].as_str(), Some("serve"));

    let gemini = simple_mcp_server_value();
    let gemini_args = gemini["args"].as_array().expect("gemini args");
    assert_eq!(gemini_args.len(), 2);
    assert!(gemini.get("cwd").is_none());

    let codex = codex_mcp_server_table();
    let codex_args = codex["args"].as_array().expect("codex args");
    assert_eq!(codex_args.len(), 2);
    assert!(codex.get("cwd").is_none());
    assert_eq!(codex["enabled"].as_bool(), Some(true));
}

#[test]
fn cursor_workspace_scope_init_and_remove_preserve_unrelated_entries() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(repo.path().join(".cursor")).expect("create .cursor");
    std::fs::write(
        repo.path().join(".cursor").join("mcp.json"),
        "{\n  \"mcpServers\": {\n    \"other\": {\"command\": \"demo\"}\n  }\n}\n",
    )
    .expect("write cursor mcp file");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Cursor]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init cursor");

    let mcp: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.path().join(".cursor").join("mcp.json"))
            .expect("read cursor mcp"),
    )
    .expect("parse cursor mcp");
    assert!(mcp["mcpServers"]["orbit"].is_object());
    assert!(mcp["mcpServers"]["other"].is_object());
    assert!(mcp["mcpServers"]["orbit"]["cwd"].is_null());
    let args = mcp["mcpServers"]["orbit"]["args"]
        .as_array()
        .expect("args array");
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].as_str(), Some("mcp"));
    assert_eq!(args[1].as_str(), Some("serve"));

    run_action(
        McpAction::Remove,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Cursor]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("remove cursor");

    let mcp: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.path().join(".cursor").join("mcp.json"))
            .expect("read cursor mcp"),
    )
    .expect("parse cursor mcp");
    assert!(mcp["mcpServers"]["orbit"].is_null());
    assert!(mcp["mcpServers"]["other"].is_object());
}

#[test]
fn cursor_home_scope_init_writes_resolved_home_path() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Cursor]),
        Some(home.path().to_path_buf()),
        ScopeArg::Home,
    )
    .expect("init cursor home");

    let cursor_path = home.path().join(".cursor").join("mcp.json");
    let mcp: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cursor_path).expect("read cursor home mcp"))
            .expect("parse cursor home mcp");
    let args = mcp["mcpServers"]["orbit"]["args"]
        .as_array()
        .expect("cursor args");
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].as_str(), Some("mcp"));
    assert_eq!(args[1].as_str(), Some("serve"));
    assert!(mcp["mcpServers"]["orbit"]["cwd"].is_null());
    assert!(!repo.path().join(".cursor").join("mcp.json").exists());
}

#[test]
fn auto_detects_cursor_from_repo_marker() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(repo.path().join(".cursor")).expect("create .cursor");

    let providers = auto_detected_providers(repo.path(), Some(home.path()));
    assert!(providers.contains(&McpProvider::Cursor));
}

#[test]
fn auto_detects_cursor_from_home_marker() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(home.path().join(".cursor")).expect("create cursor home dir");
    std::fs::write(home.path().join(".cursor").join("mcp.json"), "{}\n")
        .expect("write cursor home file");

    let providers = auto_detected_providers(repo.path(), Some(home.path()));
    assert!(providers.contains(&McpProvider::Cursor));
}

#[test]
fn workspace_scope_cursor_init_is_idempotent() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Cursor]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init cursor");
    let first = std::fs::read_to_string(repo.path().join(".cursor").join("mcp.json"))
        .expect("read first cursor");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Cursor]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init cursor again");
    let second = std::fs::read_to_string(repo.path().join(".cursor").join("mcp.json"))
        .expect("read second cursor");

    assert_eq!(first, second);
}

#[test]
fn vscode_workspace_scope_init_and_remove_preserve_unrelated_entries() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(repo.path().join(".vscode")).expect("create .vscode");
    std::fs::write(
        repo.path().join(".vscode").join("mcp.json"),
        "{\n  \"servers\": {\n    \"other\": {\"command\": \"demo\"}\n  }\n}\n",
    )
    .expect("write vscode mcp file");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Vscode]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init vscode");

    let mcp: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.path().join(".vscode").join("mcp.json"))
            .expect("read vscode mcp"),
    )
    .expect("parse vscode mcp");
    assert!(mcp["servers"]["orbit"].is_object());
    assert!(mcp["servers"]["other"].is_object());
    assert!(
        mcp.get("mcpServers").is_none(),
        "vscode must not write a mcpServers key"
    );
    assert!(mcp["servers"]["orbit"]["cwd"].is_null());
    let args = mcp["servers"]["orbit"]["args"]
        .as_array()
        .expect("args array");
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].as_str(), Some("mcp"));
    assert_eq!(args[1].as_str(), Some("serve"));

    run_action(
        McpAction::Remove,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Vscode]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("remove vscode");

    let mcp: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.path().join(".vscode").join("mcp.json"))
            .expect("read vscode mcp"),
    )
    .expect("parse vscode mcp");
    assert!(mcp["servers"]["orbit"].is_null());
    assert!(mcp["servers"]["other"].is_object());
}

#[test]
fn vscode_home_scope_init_writes_resolved_home_path() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Vscode]),
        Some(home.path().to_path_buf()),
        ScopeArg::Home,
    )
    .expect("init vscode home");

    let resolved = vscode_home_user_dir(home.path()).join("mcp.json");
    assert!(
        resolved.is_file(),
        "expected vscode home mcp at {}",
        resolved.display()
    );
    let mcp: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&resolved).expect("read vscode home"))
            .expect("parse vscode home");
    let args = mcp["servers"]["orbit"]["args"]
        .as_array()
        .expect("vscode args");
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].as_str(), Some("mcp"));
    assert_eq!(args[1].as_str(), Some("serve"));
    assert!(mcp.get("mcpServers").is_none());
    assert!(mcp["servers"]["orbit"]["cwd"].is_null());
    assert!(!repo.path().join(".vscode").join("mcp.json").exists());
}

#[test]
fn auto_detects_vscode_from_repo_marker() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(repo.path().join(".vscode")).expect("create .vscode");

    let providers = auto_detected_providers(repo.path(), Some(home.path()));
    assert!(providers.contains(&McpProvider::Vscode));
}

#[test]
fn auto_detects_vscode_from_home_marker() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let user_dir = vscode_home_user_dir(home.path());
    std::fs::create_dir_all(&user_dir).expect("create vscode user dir");
    std::fs::write(user_dir.join("mcp.json"), "{}\n").expect("write vscode home mcp");

    let providers = auto_detected_providers(repo.path(), Some(home.path()));
    assert!(providers.contains(&McpProvider::Vscode));
}

#[test]
fn workspace_scope_vscode_init_is_idempotent() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Vscode]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init vscode");
    let first = std::fs::read_to_string(repo.path().join(".vscode").join("mcp.json"))
        .expect("read first vscode");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Vscode]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init vscode again");
    let second = std::fs::read_to_string(repo.path().join(".vscode").join("mcp.json"))
        .expect("read second vscode");

    assert_eq!(first, second);
}

#[test]
fn windsurf_workspace_scope_init_and_remove_preserve_unrelated_entries() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let windsurf_dir = repo.path().join(".codeium").join("windsurf");
    std::fs::create_dir_all(&windsurf_dir).expect("create windsurf dir");
    std::fs::write(
        windsurf_dir.join("mcp_config.json"),
        "{\n  \"mcpServers\": {\n    \"other\": {\"command\": \"demo\"}\n  }\n}\n",
    )
    .expect("write windsurf config");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Windsurf]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init windsurf");

    let mcp: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(windsurf_dir.join("mcp_config.json"))
            .expect("read windsurf config"),
    )
    .expect("parse windsurf config");
    assert!(mcp["mcpServers"]["orbit"].is_object());
    assert!(mcp["mcpServers"]["other"].is_object());
    assert!(mcp["mcpServers"]["orbit"]["cwd"].is_null());
    let args = mcp["mcpServers"]["orbit"]["args"]
        .as_array()
        .expect("args array");
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].as_str(), Some("mcp"));
    assert_eq!(args[1].as_str(), Some("serve"));

    run_action(
        McpAction::Remove,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Windsurf]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("remove windsurf");

    let mcp: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(windsurf_dir.join("mcp_config.json"))
            .expect("read windsurf config"),
    )
    .expect("parse windsurf config");
    assert!(mcp["mcpServers"]["orbit"].is_null());
    assert!(mcp["mcpServers"]["other"].is_object());
}

#[test]
fn windsurf_home_scope_init_writes_resolved_home_path() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Windsurf]),
        Some(home.path().to_path_buf()),
        ScopeArg::Home,
    )
    .expect("init windsurf home");

    let path = home
        .path()
        .join(".codeium")
        .join("windsurf")
        .join("mcp_config.json");
    let mcp: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read windsurf home"))
            .expect("parse windsurf home");
    let args = mcp["mcpServers"]["orbit"]["args"]
        .as_array()
        .expect("windsurf args");
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].as_str(), Some("mcp"));
    assert_eq!(args[1].as_str(), Some("serve"));
    assert!(mcp["mcpServers"]["orbit"]["cwd"].is_null());
    assert!(
        !repo
            .path()
            .join(".codeium")
            .join("windsurf")
            .join("mcp_config.json")
            .exists()
    );
}

#[test]
fn auto_detects_windsurf_from_home_marker() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let windsurf_dir = home.path().join(".codeium").join("windsurf");
    std::fs::create_dir_all(&windsurf_dir).expect("create windsurf home dir");
    std::fs::write(windsurf_dir.join("mcp_config.json"), "{}\n").expect("write windsurf home file");

    let providers = auto_detected_providers(repo.path(), Some(home.path()));
    assert!(providers.contains(&McpProvider::Windsurf));
}

#[test]
fn workspace_scope_windsurf_init_is_idempotent() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Windsurf]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init windsurf");
    let path = repo
        .path()
        .join(".codeium")
        .join("windsurf")
        .join("mcp_config.json");
    let first = std::fs::read_to_string(&path).expect("read first windsurf");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Windsurf]),
        Some(home.path().to_path_buf()),
        ScopeArg::Workspace,
    )
    .expect("init windsurf again");
    let second = std::fs::read_to_string(&path).expect("read second windsurf");

    assert_eq!(first, second);
}
