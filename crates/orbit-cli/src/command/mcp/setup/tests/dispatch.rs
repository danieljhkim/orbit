use tempfile::tempdir;

use super::super::args::{McpAction, McpProvider, ProviderSelectionMode, ScopeArg};
use super::super::dispatch::{auto_detected_providers, run_action, vscode_home_user_dir};
use super::super::test_support::ENV_LOCK;

#[test]
fn auto_detects_expected_providers() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(repo.path().join(".claude")).expect("create .claude");
    std::fs::create_dir_all(repo.path().join(".gemini")).expect("create .gemini");
    std::fs::create_dir_all(repo.path().join(".grok")).expect("create .grok");
    std::fs::create_dir_all(home.path().join(".codex")).expect("create codex dir");
    std::fs::write(
        home.path().join(".codex").join("config.toml"),
        "model = \"gpt-5.4\"\n",
    )
    .expect("write global codex config");

    let providers = auto_detected_providers(repo.path(), Some(home.path()));
    assert_eq!(
        providers,
        vec![
            McpProvider::Claude,
            McpProvider::Codex,
            McpProvider::Gemini,
            McpProvider::Grok,
        ]
    );
}

#[test]
fn auto_detects_gemini_from_home_when_repo_lacks_dotgemini() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(home.path().join(".gemini")).expect("create gemini home dir");
    std::fs::write(home.path().join(".gemini").join("settings.json"), "{}\n")
        .expect("write global gemini settings");

    let providers = auto_detected_providers(repo.path(), Some(home.path()));
    assert_eq!(providers, vec![McpProvider::Gemini]);
}

#[test]
fn auto_detects_grok_from_home_when_repo_lacks_dotgrok() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(home.path().join(".grok")).expect("create grok home dir");
    std::fs::write(home.path().join(".grok").join("config.toml"), "\n")
        .expect("write global grok config");

    let providers = auto_detected_providers(repo.path(), Some(home.path()));
    assert_eq!(providers, vec![McpProvider::Grok]);
}

#[test]
fn home_scope_writes_to_home_paths_and_skips_repo_files() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![
            McpProvider::Claude,
            McpProvider::Codex,
            McpProvider::Gemini,
            McpProvider::Grok,
        ]),
        Some(home.path().to_path_buf()),
        ScopeArg::Home,
    )
    .expect("init home scope");

    let claude_mcp: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join(".claude").join(".mcp.json"))
            .expect("read claude home mcp"),
    )
    .expect("parse claude mcp");
    let claude_args = claude_mcp["mcpServers"]["orbit"]["args"]
        .as_array()
        .expect("claude args");
    assert_eq!(claude_args.len(), 2);
    assert_eq!(claude_args[0].as_str(), Some("mcp"));
    assert_eq!(claude_args[1].as_str(), Some("serve"));
    assert!(claude_mcp["mcpServers"]["orbit"]["cwd"].is_null());

    let claude_settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join(".claude").join("settings.json"))
            .expect("read claude home settings"),
    )
    .expect("parse claude settings");
    let allow = claude_settings["permissions"]["allow"]
        .as_array()
        .expect("allow array");
    assert!(
        allow
            .iter()
            .any(|item| item == "mcp__orbit__orbit_task_show")
    );
    assert!(
        !allow
            .iter()
            .any(|item| item.as_str().is_some_and(|s| s.starts_with("mcp__plugin_"))),
        "CLI init must not emit Claude Code plugin-scoped permission names; \
         that shape is synthesized by Claude itself for plugin installs",
    );

    let codex_config = std::fs::read_to_string(home.path().join(".codex").join("config.toml"))
        .expect("read codex home config");
    let codex_parsed: toml::Value = toml::from_str(&codex_config).expect("parse codex");
    let codex_args = codex_parsed["mcp_servers"]["orbit"]["args"]
        .as_array()
        .expect("codex args");
    assert_eq!(codex_args.len(), 2);
    assert_eq!(codex_args[0].as_str(), Some("mcp"));
    assert_eq!(codex_args[1].as_str(), Some("serve"));
    assert!(codex_parsed["mcp_servers"]["orbit"].get("cwd").is_none());

    let gemini_settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join(".gemini").join("settings.json"))
            .expect("read gemini home settings"),
    )
    .expect("parse gemini");
    let gemini_args = gemini_settings["mcpServers"]["orbit"]["args"]
        .as_array()
        .expect("gemini args");
    assert_eq!(gemini_args.len(), 2);
    assert!(gemini_settings["mcpServers"]["orbit"]["cwd"].is_null());

    let grok_config = std::fs::read_to_string(home.path().join(".grok").join("config.toml"))
        .expect("read grok home config");
    let grok_parsed: toml::Value = toml::from_str(&grok_config).expect("parse grok");
    let grok_args = grok_parsed["mcp_servers"]["orbit"]["args"]
        .as_array()
        .expect("grok args");
    assert_eq!(grok_args.len(), 2);
    assert_eq!(grok_args[0].as_str(), Some("mcp"));
    assert_eq!(grok_args[1].as_str(), Some("serve"));
    assert_eq!(
        grok_parsed["mcp_servers"]["orbit"]["enabled"].as_bool(),
        Some(true)
    );
    assert!(grok_parsed["mcp_servers"]["orbit"].get("cwd").is_none());

    // Repo-local files should not have been touched.
    assert!(!repo.path().join(".claude.json").exists());
    assert!(!repo.path().join(".codex").join("config.toml").exists());
    assert!(!repo.path().join(".gemini").join("settings.json").exists());
    assert!(!repo.path().join(".grok").join("config.toml").exists());
    assert!(!repo.path().join(".claude").join("settings.json").exists());
}

#[test]
fn home_scope_remove_strips_only_orbit_entries() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(home.path().join(".codex")).expect("create codex home");
    std::fs::write(
        home.path().join(".codex").join("config.toml"),
        "model = \"gpt-5.4\"\n[mcp_servers.other]\ncommand = \"demo\"\n",
    )
    .expect("write codex config");
    std::fs::create_dir_all(home.path().join(".gemini")).expect("create gemini home");
    std::fs::write(
        home.path().join(".gemini").join("settings.json"),
        "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"other\": {\"command\": \"demo\"}\n  }\n}\n",
    )
    .expect("write gemini settings");
    std::fs::create_dir_all(home.path().join(".grok")).expect("create grok home");
    std::fs::write(
        home.path().join(".grok").join("config.toml"),
        "model = \"grok-4\"\n[mcp_servers.other]\ncommand = \"demo\"\n",
    )
    .expect("write grok config");

    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![
            McpProvider::Codex,
            McpProvider::Gemini,
            McpProvider::Grok,
        ]),
        Some(home.path().to_path_buf()),
        ScopeArg::Home,
    )
    .expect("init home scope");

    run_action(
        McpAction::Remove,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![
            McpProvider::Codex,
            McpProvider::Gemini,
            McpProvider::Grok,
        ]),
        Some(home.path().to_path_buf()),
        ScopeArg::Home,
    )
    .expect("remove home scope");

    let codex_config = std::fs::read_to_string(home.path().join(".codex").join("config.toml"))
        .expect("read codex");
    let codex_parsed: toml::Value = toml::from_str(&codex_config).expect("parse codex");
    assert_eq!(codex_parsed["model"].as_str(), Some("gpt-5.4"));
    assert_eq!(
        codex_parsed["mcp_servers"]["other"]["command"].as_str(),
        Some("demo")
    );
    assert!(
        codex_parsed["mcp_servers"]
            .as_table()
            .and_then(|t| t.get("orbit"))
            .is_none()
    );

    let gemini_settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join(".gemini").join("settings.json"))
            .expect("read gemini"),
    )
    .expect("parse gemini");
    assert_eq!(gemini_settings["theme"], "dark");
    assert!(gemini_settings["mcpServers"]["orbit"].is_null());
    assert!(gemini_settings["mcpServers"]["other"].is_object());

    let grok_config =
        std::fs::read_to_string(home.path().join(".grok").join("config.toml")).expect("read grok");
    let grok_parsed: toml::Value = toml::from_str(&grok_config).expect("parse grok");
    assert_eq!(grok_parsed["model"].as_str(), Some("grok-4"));
    assert_eq!(
        grok_parsed["mcp_servers"]["other"]["command"].as_str(),
        Some("demo")
    );
    assert!(
        grok_parsed["mcp_servers"]
            .as_table()
            .and_then(|t| t.get("orbit"))
            .is_none()
    );
}

#[test]
fn home_scope_without_home_dir_errors() {
    let repo = tempdir().expect("repo tempdir");
    let orbit_root = repo.path().join(".orbit");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");

    let err = run_action(
        McpAction::Init,
        repo.path(),
        &orbit_root,
        ProviderSelectionMode::Explicit(vec![McpProvider::Claude]),
        None,
        ScopeArg::Home,
    )
    .expect_err("home scope without home dir should fail");

    assert!(matches!(
        err,
        orbit_core::OrbitError::InvalidInput(message) if message.contains("HOME")
    ));
}

#[test]
fn env_lock_smoke() {
    let _guard = ENV_LOCK.lock().expect("lock env");
}

#[test]
fn vscode_home_user_dir_resolves_for_host_platform() {
    let home = std::path::PathBuf::from("/tmp/orbit-test-home");
    let resolved = vscode_home_user_dir(&home);
    // Tail must always be `Code/User`; the rest is platform-specific.
    let mut components = resolved
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let user = components.pop().expect("user segment");
    let code = components.pop().expect("code segment");
    assert_eq!(user, "User");
    assert_eq!(code, "Code");
    assert!(
        resolved.starts_with(&home),
        "resolved path {} should start with home dir {}",
        resolved.display(),
        home.display(),
    );
}
