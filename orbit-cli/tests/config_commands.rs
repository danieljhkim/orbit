use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;

fn orbit_in(dir: &Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd
}

#[test]
fn config_show_json_uses_defaults_when_config_file_missing() {
    let dir = tempfile::tempdir().expect("tempdir");

    let output = orbit_in(dir.path())
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).expect("json");

    assert_eq!(value["exists"], false);
    assert_eq!(value["execution"]["env"]["inherit"], false);
    assert_eq!(
        value["execution"]["env"]["pass"],
        serde_json::json!([
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "HOME",
            "PATH",
            "CODEX_HOME"
        ])
    );
}

#[test]
fn config_show_json_reads_and_normalizes_runtime_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orbit_dir = dir.path().join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("create orbit dir");
    std::fs::write(
        orbit_dir.join("config.toml"),
        "[execution.env]\ninherit = true\npass = [\"PATH\",\"HOME\",\"PATH\"]\n",
    )
    .expect("write config");

    let output = orbit_in(dir.path())
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).expect("json");

    assert_eq!(value["exists"], true);
    assert_eq!(value["execution"]["env"]["inherit"], true);
    assert_eq!(
        value["execution"]["env"]["pass"],
        serde_json::json!(["HOME", "PATH"])
    );
}
