use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::super::hook_install::{install_for_workspace, uninstall_for_workspace};

#[test]
fn install_writes_expected_shims_and_config_entries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    seed_agent_dirs(root, &[".claude", ".codex", ".gemini", ".grok"]);

    let providers = install_for_workspace(root).expect("install hooks");
    assert_eq!(providers, ["claude", "codex", "gemini", "grok"]);

    assert_eq!(
        fs::read_to_string(root.join(".claude/hooks/orbit-learning-reminder"))
            .expect("read claude shim"),
        "#!/bin/sh\n# Orbit project-learning PreToolUse hook.\nexec \"${ORBIT_BIN:-orbit}\" hook pretooluse \"$@\"\n"
    );
    assert_eq!(
        fs::read_to_string(root.join(".codex/hooks/orbit-learning-reminder"))
            .expect("read codex shim"),
        "#!/bin/sh\n# Orbit project-learning PreToolUse hook.\nexec \"${ORBIT_BIN:-orbit}\" hook pretooluse --format codex \"$@\"\n"
    );
    assert_eq!(
        fs::read_to_string(root.join(".gemini/hooks/orbit-learning-reminder"))
            .expect("read gemini shim"),
        "#!/bin/sh\n# Orbit project-learning PreToolUse hook.\nexec \"${ORBIT_BIN:-orbit}\" hook pretooluse --format gemini \"$@\"\n"
    );
    assert_eq!(
        fs::read_to_string(root.join(".grok/hooks/orbit-learning-reminder"))
            .expect("read grok shim"),
        "#!/bin/sh\n# Orbit project-learning PreToolUse hook.\nexec \"${ORBIT_BIN:-orbit}\" hook pretooluse --format grok \"$@\"\n"
    );

    assert_json_hook(
        &root.join(".claude/settings.json"),
        "PreToolUse",
        ".claude/hooks/orbit-learning-reminder",
    );
    assert_json_hook(
        &root.join(".gemini/settings.json"),
        "BeforeTool",
        ".gemini/hooks/orbit-learning-reminder",
    );
    assert_toml_hook(
        &root.join(".codex/config.toml"),
        "PreToolUse",
        ".codex/hooks/orbit-learning-reminder",
    );
    assert_toml_hook(
        &root.join(".grok/config.toml"),
        "PreToolUse",
        ".grok/hooks/orbit-learning-reminder",
    );
}

#[test]
fn install_is_idempotent_for_managed_config_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    seed_agent_dirs(root, &[".claude", ".codex", ".gemini", ".grok"]);

    install_for_workspace(root).expect("install hooks");
    let first = read_configs(root);
    install_for_workspace(root).expect("install hooks again");
    let second = read_configs(root);

    assert_eq!(first, second);
}

#[test]
fn install_preserves_user_json_entries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    seed_agent_dirs(root, &[".claude"]);
    fs::write(
        root.join(".claude/settings.json"),
        serde_json::to_string_pretty(&json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Write",
                    "hooks": [{
                        "type": "command",
                        "command": ".claude/hooks/user-hook"
                    }]
                }]
            },
            "theme": "dark"
        }))
        .expect("serialize settings"),
    )
    .expect("write settings");

    install_for_workspace(root).expect("install hooks");

    let settings: Value = serde_json::from_str(
        &fs::read_to_string(root.join(".claude/settings.json")).expect("read"),
    )
    .expect("parse settings");
    assert_eq!(settings["theme"], "dark");
    assert_json_value_contains_command(&settings, ".claude/hooks/user-hook");
    assert_json_value_contains_command(&settings, ".claude/hooks/orbit-learning-reminder");
}

#[test]
fn uninstall_removes_orbit_hooks_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    seed_agent_dirs(root, &[".claude"]);
    fs::write(
        root.join(".claude/settings.json"),
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

    install_for_workspace(root).expect("install hooks");
    let removed = uninstall_for_workspace(root).expect("uninstall hooks");

    assert_eq!(removed, ["claude"]);
    assert!(!root.join(".claude/hooks/orbit-learning-reminder").exists());
    let settings: Value = serde_json::from_str(
        &fs::read_to_string(root.join(".claude/settings.json")).expect("read"),
    )
    .expect("parse settings");
    assert_json_value_contains_command(&settings, ".claude/hooks/user-hook");
    assert!(!json_value_contains_command(
        &settings,
        ".claude/hooks/orbit-learning-reminder"
    ));
}

fn seed_agent_dirs(root: &Path, dirs: &[&str]) {
    for dir in dirs {
        fs::create_dir_all(root.join(dir)).expect("create agent dir");
    }
}

fn read_configs(root: &Path) -> Vec<(String, String)> {
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
            fs::read_to_string(root.join(path)).expect("read config"),
        )
    })
    .collect()
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
