use assert_cmd::Command;
use predicates::prelude::*;

fn orbit_in(dir: &std::path::Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd.env("HOME", dir);
    cmd.env("USERPROFILE", dir);
    cmd
}

#[test]
fn mcp_init_creates_codex_and_claude_configs() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");
    let codex_home = tempfile::tempdir().expect("codex home");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .env("CODEX_HOME", codex_home.path())
        .args(["mcp", "init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("codex: updated"))
        .stdout(predicate::str::contains("claude: updated"))
        .stdout(predicate::str::contains("claude-code: updated"));

    let codex_path = codex_home.path().join("config.toml");
    let claude_path = home.path().join(".claude.json");
    let claude_code_path = home.path().join(".claude").join(".mcp.json");

    assert!(codex_path.exists());
    assert!(claude_path.exists());
    assert!(claude_code_path.exists());

    let codex_raw = std::fs::read_to_string(codex_path).expect("read codex");
    assert!(codex_raw.contains("[mcp_servers.orbit]"));
    // Command must be an absolute path, not a bare name
    assert!(
        !codex_raw.contains("command = \"orbit\""),
        "codex config must use absolute path, not bare 'orbit'"
    );
    assert!(
        codex_raw.contains("ORBIT_ROOT"),
        "codex config must set ORBIT_ROOT env"
    );

    let claude_raw = std::fs::read_to_string(claude_path).expect("read claude");
    let claude_json: serde_json::Value = serde_json::from_str(&claude_raw).expect("claude json");
    let claude_command = claude_json["mcpServers"]["orbit"]["command"]
        .as_str()
        .expect("command must be a string");
    assert!(
        std::path::Path::new(claude_command).is_absolute(),
        "claude config command must be absolute path, got: {claude_command}"
    );
    assert_eq!(claude_json["mcpServers"]["orbit"]["args"][0], "mcp");
    assert!(
        claude_json["mcpServers"]["orbit"]["env"]["ORBIT_ROOT"]
            .as_str()
            .is_some(),
        "claude config must include ORBIT_ROOT in env"
    );

    // Verify Claude Code config
    let cc_raw = std::fs::read_to_string(&claude_code_path).expect("read claude-code");
    let cc_json: serde_json::Value = serde_json::from_str(&cc_raw).expect("claude-code json");
    let cc_command = cc_json["mcpServers"]["orbit"]["command"]
        .as_str()
        .expect("command must be a string");
    assert!(
        std::path::Path::new(cc_command).is_absolute(),
        "claude-code config command must be absolute path, got: {cc_command}"
    );
    assert!(
        cc_json["mcpServers"]["orbit"]["env"]["ORBIT_ROOT"]
            .as_str()
            .is_some(),
        "claude-code config must include ORBIT_ROOT in env"
    );
}

#[test]
fn mcp_init_dry_run_does_not_write_files() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");
    let codex_home = tempfile::tempdir().expect("codex home");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .env("CODEX_HOME", codex_home.path())
        .args(["mcp", "init", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dry-run"));

    assert!(!codex_home.path().join("config.toml").exists());
    assert!(!home.path().join(".claude.json").exists());
    assert!(!home.path().join(".claude").join(".mcp.json").exists());
}

#[test]
fn mcp_init_preserves_existing_keys() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");
    let codex_home = tempfile::tempdir().expect("codex home");

    std::fs::create_dir_all(codex_home.path()).expect("codex mkdir");
    std::fs::write(
        codex_home.path().join("config.toml"),
        "[profile]\nname = \"dev\"\n[mcp_servers.other]\ncommand=\"x\"\nargs=[\"y\"]\n",
    )
    .expect("write codex");

    std::fs::write(
        home.path().join(".claude.json"),
        "{\"theme\":\"dark\",\"mcpServers\":{\"other\":{\"command\":\"x\",\"args\":[\"y\"]}}}",
    )
    .expect("write claude");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .env("CODEX_HOME", codex_home.path())
        .args(["mcp", "init"])
        .assert()
        .success();

    let codex_raw =
        std::fs::read_to_string(codex_home.path().join("config.toml")).expect("read codex");
    assert!(codex_raw.contains("[profile]"));
    assert!(codex_raw.contains("[mcp_servers.other]"));
    assert!(codex_raw.contains("[mcp_servers.orbit]"));

    let claude_raw =
        std::fs::read_to_string(home.path().join(".claude.json")).expect("read claude");
    let claude_json: serde_json::Value = serde_json::from_str(&claude_raw).expect("claude json");
    assert_eq!(claude_json["theme"], "dark");
    assert_eq!(claude_json["mcpServers"]["other"]["command"], "x");
    let orbit_command = claude_json["mcpServers"]["orbit"]["command"]
        .as_str()
        .expect("command string");
    assert!(
        std::path::Path::new(orbit_command).is_absolute(),
        "orbit command must be absolute path, got: {orbit_command}"
    );
}
