use assert_cmd::Command;
use predicates::prelude::*;

fn orbit_in(dir: &std::path::Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
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
        .stdout(predicate::str::contains("claude: updated"));

    let codex_path = codex_home.path().join("config.toml");
    let claude_path = home.path().join(".claude.json");

    assert!(codex_path.exists());
    assert!(claude_path.exists());

    let codex_raw = std::fs::read_to_string(codex_path).expect("read codex");
    assert!(codex_raw.contains("[mcp_servers.orbit]"));

    let claude_raw = std::fs::read_to_string(claude_path).expect("read claude");
    let claude_json: serde_json::Value = serde_json::from_str(&claude_raw).expect("claude json");
    assert_eq!(claude_json["mcpServers"]["orbit"]["command"], "orbit");
    assert_eq!(claude_json["mcpServers"]["orbit"]["args"][0], "mcp");
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
    assert_eq!(claude_json["mcpServers"]["orbit"]["command"], "orbit");
}
