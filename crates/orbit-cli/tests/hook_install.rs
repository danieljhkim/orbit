#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

// ORB-10366: `orbit workspace init` no longer wires up learning-reminder
// registrations as a side effect. `orbit hook install` remains as an
// explicit, human-invoked opt-in — see crates/orbit-cli/CLAUDE.md /
// docs/design/project-learnings/4_decisions.md for the rationale.

#[test]
fn workspace_init_rejects_hooks_flag() {
    let workspace = TestWorkspace::new();
    workspace.seed_agent_dirs(&[".claude"]);

    let output = workspace.run_raw(
        &["workspace", "init", "--name", "hooks", "--hooks"],
        "init with removed --hooks flag",
    );

    assert!(
        !output.status.success(),
        "expected --hooks to be rejected by clap, stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--hooks") || stderr.contains("unexpected argument"),
        "stderr did not mention the rejected flag: {stderr}"
    );
}

#[test]
fn workspace_init_leaves_no_learning_reminder_registrations() {
    let workspace = TestWorkspace::new();
    workspace.seed_agent_dirs(&[".claude", ".codex", ".gemini", ".grok"]);

    workspace.run(&["workspace", "init", "--name", "hooks"], "init workspace");

    for path in [
        ".claude/hooks/orbit-learning-reminder",
        ".codex/hooks/orbit-learning-reminder",
        ".gemini/hooks/orbit-learning-reminder",
        ".grok/hooks/orbit-learning-reminder",
    ] {
        assert!(
            !workspace.work.join(path).exists(),
            "fresh workspace init must not write shim {path}"
        );
    }

    for path in [".claude/settings.json", ".codex/config.toml"] {
        let full = workspace.work.join(path);
        if !full.exists() {
            continue;
        }
        let contents = fs::read_to_string(&full).expect("read config");
        assert!(
            !contents.contains("orbit-learning-reminder"),
            "{path} unexpectedly contains a learning-reminder registration:\n{contents}"
        );
    }
}

#[test]
fn hook_install_command_seeds_hooks_and_is_idempotent() {
    let workspace = TestWorkspace::new();
    workspace.seed_agent_dirs(&[".claude", ".codex", ".gemini", ".grok"]);
    workspace.run(&["workspace", "init", "--name", "hooks"], "init workspace");

    workspace.run(&["hook", "install"], "hook install");
    assert_json_hook(
        &workspace.work.join(".claude/settings.json"),
        "PreToolUse",
        ".claude/hooks/orbit-learning-reminder",
    );
    assert_toml_hook(
        &workspace.work.join(".codex/config.toml"),
        "PreToolUse",
        ".codex/hooks/orbit-learning-reminder",
    );

    let first = workspace.read_configs();
    workspace.run(&["hook", "install"], "hook install again");
    let second = workspace.read_configs();
    assert_eq!(first, second);
}

#[test]
fn workspace_teardown_removes_orbit_hooks_only() {
    let workspace = TestWorkspace::new();
    workspace.seed_agent_dirs(&[".claude"]);
    fs::write(
        workspace.work.join(".claude/settings.json"),
        serde_json::to_string_pretty(&json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Write",
                    "hooks": [{
                        "type": "command",
                        "command": ".claude/hooks/user-hook"
                    }]
                }]
            }
        }))
        .expect("serialize settings"),
    )
    .expect("write settings");

    workspace.run(&["workspace", "init", "--name", "hooks"], "init workspace");
    workspace.run(&["hook", "install"], "install hooks");
    workspace.run(&["workspace", "teardown", "--confirm"], "teardown hooks");

    assert!(
        !workspace
            .work
            .join(".claude/hooks/orbit-learning-reminder")
            .exists()
    );
    let settings: Value = serde_json::from_str(
        &fs::read_to_string(workspace.work.join(".claude/settings.json")).expect("read settings"),
    )
    .expect("parse settings");
    assert_json_value_contains_command(&settings, ".claude/hooks/user-hook");
    assert!(!json_value_contains_command(
        &settings,
        ".claude/hooks/orbit-learning-reminder"
    ));
}

fn assert_json_hook(path: &Path, event: &str, command: &str) {
    let settings: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("read JSON config"))
            .expect("parse JSON config");
    let entries = settings["hooks"][event].as_array().expect("event hooks");
    assert!(
        entries
            .iter()
            .any(|entry| json_value_contains_command(entry, command)),
        "{path:?} missing command {command}"
    );
}

fn assert_toml_hook(path: &Path, event: &str, command: &str) {
    let config: toml::Value = toml::from_str(&fs::read_to_string(path).expect("read TOML config"))
        .expect("parse TOML config");
    let entries = config["hooks"][event].as_array().expect("event hooks");
    assert!(
        entries
            .iter()
            .any(|entry| toml_value_contains_command(entry, command)),
        "{path:?} missing command {command}"
    );
}

fn assert_json_value_contains_command(value: &Value, command: &str) {
    assert!(
        json_value_contains_command(value, command),
        "missing command {command} in {value}"
    );
}

fn json_value_contains_command(value: &Value, command: &str) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (key == "command"
                && value
                    .as_str()
                    .map(|candidate| candidate.contains(command))
                    .unwrap_or(false))
                || json_value_contains_command(value, command)
        }),
        Value::Array(values) => values
            .iter()
            .any(|value| json_value_contains_command(value, command)),
        _ => false,
    }
}

fn toml_value_contains_command(value: &toml::Value, command: &str) -> bool {
    match value {
        toml::Value::Table(table) => table.iter().any(|(key, value)| {
            (key == "command"
                && value
                    .as_str()
                    .map(|candidate| candidate.contains(command))
                    .unwrap_or(false))
                || toml_value_contains_command(value, command)
        }),
        toml::Value::Array(values) => values
            .iter()
            .any(|value| toml_value_contains_command(value, command)),
        _ => false,
    }
}

struct TestWorkspace {
    _temp: TempDir,
    home: PathBuf,
    work: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let temp = tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let work = temp.path().join("work");
        fs::create_dir_all(&home).expect("create home");
        fs::create_dir_all(&work).expect("create work");
        Self {
            _temp: temp,
            home,
            work,
        }
    }

    fn seed_agent_dirs(&self, dirs: &[&str]) {
        for dir in dirs {
            fs::create_dir_all(self.work.join(dir)).expect("create agent dir");
        }
    }

    fn read_configs(&self) -> Vec<(String, String)> {
        [
            ".claude/settings.json",
            ".codex/config.toml",
            ".gemini/settings.json",
            ".grok/config.toml",
        ]
        .into_iter()
        .map(|path| {
            (
                path.to_string(),
                fs::read_to_string(self.work.join(path)).expect("read config"),
            )
        })
        .collect()
    }

    fn run_raw(&self, args: &[&str], _label: &str) -> Output {
        let mut command = cargo_bin_cmd!("orbit");
        command
            .current_dir(&self.work)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            // ORB-10453: a test binary is not a terminal, so the capability
            // chokepoint resolves it as an unidentified caller and refuses
            // `workspace teardown`. Claiming the operator capability explicitly
            // is exactly the escape hatch the denial names.
            .env("ORBIT_OPERATOR", "1")
            .env_remove("ORBIT_ROOT")
            .args(args)
            .output()
            .expect("run orbit")
    }

    fn run(&self, args: &[&str], label: &str) -> Output {
        let output = self.run_raw(args, label);
        assert!(
            output.status.success(),
            "{label} failed\nargs: {args:?}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}
