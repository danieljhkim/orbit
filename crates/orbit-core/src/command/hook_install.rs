use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use serde_json::{Map as JsonMap, Value as JsonValue};
use toml::{Table as TomlTable, Value as TomlValue};

use crate::OrbitRuntime;
use crate::redact_sensitive_env_text;

const HOOK_FILE_NAME: &str = "orbit-learning-reminder";
const SHIM_COMMENT: &str = "# Orbit project-learning PreToolUse hook.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookAgent {
    Claude,
    Codex,
    Gemini,
    Grok,
}

impl HookAgent {
    fn all() -> &'static [Self] {
        &[Self::Claude, Self::Codex, Self::Gemini, Self::Grok]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Grok => "grok",
        }
    }

    fn dir(self) -> &'static str {
        match self {
            Self::Claude => ".claude",
            Self::Codex => ".codex",
            Self::Gemini => ".gemini",
            Self::Grok => ".grok",
        }
    }

    fn config_path(self, workspace_root: &Path) -> PathBuf {
        match self {
            Self::Claude => workspace_root.join(".claude").join("settings.json"),
            Self::Codex => workspace_root.join(".codex").join("config.toml"),
            Self::Gemini => workspace_root.join(".gemini").join("settings.json"),
            Self::Grok => workspace_root.join(".grok").join("config.toml"),
        }
    }

    fn shim_path(self, workspace_root: &Path) -> PathBuf {
        workspace_root
            .join(self.dir())
            .join("hooks")
            .join(HOOK_FILE_NAME)
    }

    fn command(self) -> String {
        match self {
            Self::Claude => ".claude/hooks/orbit-learning-reminder".to_string(),
            Self::Codex => {
                "\"$(git rev-parse --show-toplevel)/.codex/hooks/orbit-learning-reminder\""
                    .to_string()
            }
            Self::Gemini => ".gemini/hooks/orbit-learning-reminder".to_string(),
            Self::Grok => ".grok/hooks/orbit-learning-reminder".to_string(),
        }
    }

    fn command_marker(self) -> &'static str {
        match self {
            Self::Claude => ".claude/hooks/orbit-learning-reminder",
            Self::Codex => ".codex/hooks/orbit-learning-reminder",
            Self::Gemini => ".gemini/hooks/orbit-learning-reminder",
            Self::Grok => ".grok/hooks/orbit-learning-reminder",
        }
    }

    fn shim_format(self) -> Option<&'static str> {
        match self {
            Self::Claude => None,
            Self::Codex => Some("codex"),
            Self::Gemini => Some("gemini"),
            Self::Grok => Some("grok"),
        }
    }
}

pub fn install_for_workspace(workspace_root: &Path) -> Result<Vec<String>, OrbitError> {
    let mut installed = Vec::new();
    for agent in HookAgent::all() {
        if !workspace_root.join(agent.dir()).is_dir() {
            continue;
        }
        match install_agent(workspace_root, *agent) {
            Ok(()) => installed.push(agent.label().to_string()),
            Err(error) => warn_agent_failure("install", *agent, &error),
        }
    }
    Ok(installed)
}

pub fn install_for_runtime(runtime: &OrbitRuntime) -> Result<Vec<String>, OrbitError> {
    install_for_workspace(&runtime.paths().repo_root)
}

pub fn uninstall_for_workspace(workspace_root: &Path) -> Result<Vec<String>, OrbitError> {
    let mut uninstalled = Vec::new();
    for agent in HookAgent::all() {
        match uninstall_agent(workspace_root, *agent) {
            Ok(removed) => {
                if removed {
                    uninstalled.push(agent.label().to_string());
                }
            }
            Err(error) => warn_agent_failure("uninstall", *agent, &error),
        }
    }
    Ok(uninstalled)
}

pub fn uninstall_for_runtime(runtime: &OrbitRuntime) -> Result<Vec<String>, OrbitError> {
    uninstall_for_workspace(&runtime.paths().repo_root)
}

fn warn_agent_failure(action: &str, agent: HookAgent, error: &OrbitError) {
    tracing::warn!(
        action,
        agent = agent.label(),
        error = %redact_sensitive_env_text(&error.to_string()),
        "workspace hook integration failed open",
    );
}

fn install_agent(workspace_root: &Path, agent: HookAgent) -> Result<(), OrbitError> {
    write_shim(&agent.shim_path(workspace_root), agent.shim_format())?;
    match agent {
        HookAgent::Claude => merge_json_hook(
            &agent.config_path(workspace_root),
            "PreToolUse",
            "Edit|Write|Read",
            &agent.command(),
            agent.command_marker(),
            None,
        ),
        HookAgent::Gemini => merge_json_hook(
            &agent.config_path(workspace_root),
            "BeforeTool",
            "*",
            &agent.command(),
            agent.command_marker(),
            Some("orbit-learning-reminder"),
        ),
        HookAgent::Codex => merge_toml_hook(
            &agent.config_path(workspace_root),
            "PreToolUse",
            "^(Bash|apply_patch|mcp__.*|mcp\\..*)$",
            &agent.command(),
            agent.command_marker(),
        ),
        HookAgent::Grok => merge_toml_hook(
            &agent.config_path(workspace_root),
            "PreToolUse",
            "Edit|Write|Read",
            &agent.command(),
            agent.command_marker(),
        ),
    }
}

fn uninstall_agent(workspace_root: &Path, agent: HookAgent) -> Result<bool, OrbitError> {
    let mut removed = false;
    let shim_path = agent.shim_path(workspace_root);
    if shim_path.exists() {
        fs::remove_file(&shim_path).map_err(|error| {
            OrbitError::Io(format!(
                "failed to remove '{}': {error}",
                shim_path.display()
            ))
        })?;
        removed = true;
    }

    let demerged = match agent {
        HookAgent::Claude => remove_json_hook(
            &agent.config_path(workspace_root),
            "PreToolUse",
            agent.command_marker(),
        )?,
        HookAgent::Gemini => remove_json_hook(
            &agent.config_path(workspace_root),
            "BeforeTool",
            agent.command_marker(),
        )?,
        HookAgent::Codex => remove_toml_hook(
            &agent.config_path(workspace_root),
            "PreToolUse",
            agent.command_marker(),
        )?,
        HookAgent::Grok => remove_toml_hook(
            &agent.config_path(workspace_root),
            "PreToolUse",
            agent.command_marker(),
        )?,
    };
    Ok(removed || demerged)
}

fn write_shim(path: &Path, format: Option<&str>) -> Result<(), OrbitError> {
    let parent = path.parent().ok_or_else(|| {
        OrbitError::InvalidInput(format!("hook path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        OrbitError::Io(format!("failed to create '{}': {error}", parent.display()))
    })?;

    let command = match format {
        Some(format) => {
            format!("exec \"${{ORBIT_BIN:-orbit}}\" hook pretooluse --format {format} \"$@\"")
        }
        None => "exec \"${ORBIT_BIN:-orbit}\" hook pretooluse \"$@\"".to_string(),
    };
    let contents = format!("#!/bin/sh\n{SHIM_COMMENT}\n{command}\n");
    fs::write(path, contents).map_err(|error| {
        OrbitError::Io(format!("failed to write '{}': {error}", path.display()))
    })?;
    make_executable(path)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), OrbitError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|error| OrbitError::Io(format!("failed to stat '{}': {error}", path.display())))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).map_err(|error| {
        OrbitError::Io(format!(
            "failed to set executable permissions on '{}': {error}",
            path.display()
        ))
    })
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), OrbitError> {
    Ok(())
}

fn merge_json_hook(
    path: &Path,
    event: &str,
    matcher: &str,
    command: &str,
    marker: &str,
    name: Option<&str>,
) -> Result<(), OrbitError> {
    let mut root = load_json_object(path)?;
    if json_contains_command(&JsonValue::Object(root.clone()), marker) {
        return write_json_object(path, &root);
    }
    let hooks = ensure_json_object(&mut root, "hooks")?;
    let event_hooks = ensure_json_array(hooks, event)?;
    event_hooks.push(json_hook_entry(matcher, command, name));
    write_json_object(path, &root)
}

fn remove_json_hook(path: &Path, event: &str, marker: &str) -> Result<bool, OrbitError> {
    if !path.exists() {
        return Ok(false);
    }
    let mut root = load_json_object(path)?;
    let Some(hooks) = root.get_mut("hooks").and_then(JsonValue::as_object_mut) else {
        return Ok(false);
    };
    let Some(event_hooks) = hooks.get_mut(event).and_then(JsonValue::as_array_mut) else {
        return Ok(false);
    };

    let mut removed = false;
    for entry in event_hooks.iter_mut() {
        if let Some(commands) = entry.get_mut("hooks").and_then(JsonValue::as_array_mut) {
            let command_count = commands.len();
            commands.retain(|hook| !json_contains_command(hook, marker));
            removed |= commands.len() != command_count;
        }
    }
    let before = event_hooks.len();
    event_hooks.retain(|entry| {
        entry
            .get("hooks")
            .and_then(JsonValue::as_array)
            .map(|commands| !commands.is_empty())
            .unwrap_or(true)
    });
    removed |= event_hooks.len() != before;

    if event_hooks.is_empty() {
        hooks.remove(event);
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    write_or_remove_json_object(path, &root)?;
    Ok(removed)
}

fn json_hook_entry(matcher: &str, command: &str, name: Option<&str>) -> JsonValue {
    let mut hook = JsonMap::from_iter([
        ("type".to_string(), JsonValue::String("command".to_string())),
        (
            "command".to_string(),
            JsonValue::String(command.to_string()),
        ),
    ]);
    if let Some(name) = name {
        hook.insert("name".to_string(), JsonValue::String(name.to_string()));
    }
    JsonValue::Object(JsonMap::from_iter([
        (
            "matcher".to_string(),
            JsonValue::String(matcher.to_string()),
        ),
        (
            "hooks".to_string(),
            JsonValue::Array(vec![JsonValue::Object(hook)]),
        ),
    ]))
}

fn json_contains_command(value: &JsonValue, marker: &str) -> bool {
    match value {
        JsonValue::Object(object) => object.iter().any(|(key, value)| {
            (key == "command"
                && value
                    .as_str()
                    .map(|command| command.contains(marker))
                    .unwrap_or(false))
                || json_contains_command(value, marker)
        }),
        JsonValue::Array(values) => values
            .iter()
            .any(|value| json_contains_command(value, marker)),
        _ => false,
    }
}

fn load_json_object(path: &Path) -> Result<JsonMap<String, JsonValue>, OrbitError> {
    if !path.exists() {
        return Ok(JsonMap::new());
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| OrbitError::Io(format!("failed to read '{}': {error}", path.display())))?;
    if raw.trim().is_empty() {
        return Ok(JsonMap::new());
    }
    let value: JsonValue = serde_json::from_str(&raw).map_err(|error| {
        OrbitError::InvalidInput(format!("invalid JSON '{}': {error}", path.display()))
    })?;
    value.as_object().cloned().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "expected top-level JSON object in '{}'",
            path.display()
        ))
    })
}

fn write_json_object(path: &Path, root: &JsonMap<String, JsonValue>) -> Result<(), OrbitError> {
    let parent = path.parent().ok_or_else(|| {
        OrbitError::InvalidInput(format!("path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        OrbitError::Io(format!("failed to create '{}': {error}", parent.display()))
    })?;
    let mut rendered =
        serde_json::to_string_pretty(&JsonValue::Object(root.clone())).map_err(|error| {
            OrbitError::Execution(format!("serialize JSON '{}': {error}", path.display()))
        })?;
    rendered.push('\n');
    fs::write(path, rendered)
        .map_err(|error| OrbitError::Io(format!("failed to write '{}': {error}", path.display())))
}

fn write_or_remove_json_object(
    path: &Path,
    root: &JsonMap<String, JsonValue>,
) -> Result<(), OrbitError> {
    if root.is_empty() {
        if path.exists() {
            fs::remove_file(path).map_err(|error| {
                OrbitError::Io(format!("failed to remove '{}': {error}", path.display()))
            })?;
        }
        return Ok(());
    }
    write_json_object(path, root)
}

fn ensure_json_object<'a>(
    root: &'a mut JsonMap<String, JsonValue>,
    key: &str,
) -> Result<&'a mut JsonMap<String, JsonValue>, OrbitError> {
    let value = root
        .entry(key.to_string())
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    value
        .as_object_mut()
        .ok_or_else(|| OrbitError::InvalidInput(format!("expected '{key}' to be a JSON object")))
}

fn ensure_json_array<'a>(
    root: &'a mut JsonMap<String, JsonValue>,
    key: &str,
) -> Result<&'a mut Vec<JsonValue>, OrbitError> {
    let value = root
        .entry(key.to_string())
        .or_insert_with(|| JsonValue::Array(Vec::new()));
    value
        .as_array_mut()
        .ok_or_else(|| OrbitError::InvalidInput(format!("expected '{key}' to be a JSON array")))
}

fn merge_toml_hook(
    path: &Path,
    event: &str,
    matcher: &str,
    command: &str,
    marker: &str,
) -> Result<(), OrbitError> {
    let mut root = load_toml_table(path)?;
    if toml_contains_command_in_table(&root, marker) {
        return write_toml_table(path, &root);
    }
    let hooks = ensure_toml_table(&mut root, "hooks")?;
    let event_hooks = ensure_toml_array(hooks, event)?;
    event_hooks.push(TomlValue::Table(toml_hook_entry(matcher, command)));
    write_toml_table(path, &root)
}

fn remove_toml_hook(path: &Path, event: &str, marker: &str) -> Result<bool, OrbitError> {
    if !path.exists() {
        return Ok(false);
    }
    let mut root = load_toml_table(path)?;
    let Some(hooks) = root.get_mut("hooks").and_then(TomlValue::as_table_mut) else {
        return Ok(false);
    };
    let Some(event_hooks) = hooks.get_mut(event).and_then(TomlValue::as_array_mut) else {
        return Ok(false);
    };

    let mut removed = false;
    for entry in event_hooks.iter_mut().filter_map(TomlValue::as_table_mut) {
        if let Some(commands) = entry.get_mut("hooks").and_then(TomlValue::as_array_mut) {
            let command_count = commands.len();
            commands.retain(|hook| !toml_contains_command(hook, marker));
            removed |= commands.len() != command_count;
        }
    }
    let before = event_hooks.len();
    event_hooks.retain(|entry| {
        entry
            .as_table()
            .and_then(|table| table.get("hooks"))
            .and_then(TomlValue::as_array)
            .map(|commands| !commands.is_empty())
            .unwrap_or(true)
    });
    removed |= event_hooks.len() != before;
    if event_hooks.is_empty() {
        hooks.remove(event);
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    write_or_remove_toml_table(path, &root)?;
    Ok(removed)
}

fn toml_hook_entry(matcher: &str, command: &str) -> TomlTable {
    TomlTable::from_iter([
        (
            "matcher".to_string(),
            TomlValue::String(matcher.to_string()),
        ),
        (
            "hooks".to_string(),
            TomlValue::Array(vec![TomlValue::Table(TomlTable::from_iter([
                ("type".to_string(), TomlValue::String("command".to_string())),
                (
                    "command".to_string(),
                    TomlValue::String(command.to_string()),
                ),
                (
                    "statusMessage".to_string(),
                    TomlValue::String("Loading Orbit learnings".to_string()),
                ),
            ]))]),
        ),
    ])
}

fn toml_contains_command_in_table(table: &TomlTable, marker: &str) -> bool {
    table
        .values()
        .any(|value| toml_contains_command(value, marker))
}

fn toml_contains_command(value: &TomlValue, marker: &str) -> bool {
    match value {
        TomlValue::Table(table) => table.iter().any(|(key, value)| {
            (key == "command"
                && value
                    .as_str()
                    .map(|command| command.contains(marker))
                    .unwrap_or(false))
                || toml_contains_command(value, marker)
        }),
        TomlValue::Array(values) => values
            .iter()
            .any(|value| toml_contains_command(value, marker)),
        _ => false,
    }
}

fn load_toml_table(path: &Path) -> Result<TomlTable, OrbitError> {
    if !path.exists() {
        return Ok(TomlTable::new());
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| OrbitError::Io(format!("failed to read '{}': {error}", path.display())))?;
    if raw.trim().is_empty() {
        return Ok(TomlTable::new());
    }
    let value: TomlValue = toml::from_str(&raw).map_err(|error| {
        OrbitError::InvalidInput(format!("invalid TOML '{}': {error}", path.display()))
    })?;
    value.as_table().cloned().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "expected top-level TOML table in '{}'",
            path.display()
        ))
    })
}

fn write_toml_table(path: &Path, root: &TomlTable) -> Result<(), OrbitError> {
    let parent = path.parent().ok_or_else(|| {
        OrbitError::InvalidInput(format!("path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        OrbitError::Io(format!("failed to create '{}': {error}", parent.display()))
    })?;
    let rendered = toml::to_string_pretty(&TomlValue::Table(root.clone())).map_err(|error| {
        OrbitError::Execution(format!("serialize TOML '{}': {error}", path.display()))
    })?;
    fs::write(path, rendered)
        .map_err(|error| OrbitError::Io(format!("failed to write '{}': {error}", path.display())))
}

fn write_or_remove_toml_table(path: &Path, root: &TomlTable) -> Result<(), OrbitError> {
    if root.is_empty() {
        if path.exists() {
            fs::remove_file(path).map_err(|error| {
                OrbitError::Io(format!("failed to remove '{}': {error}", path.display()))
            })?;
        }
        return Ok(());
    }
    write_toml_table(path, root)
}

fn ensure_toml_table<'a>(
    root: &'a mut TomlTable,
    key: &str,
) -> Result<&'a mut TomlTable, OrbitError> {
    let value = root
        .entry(key.to_string())
        .or_insert_with(|| TomlValue::Table(TomlTable::new()));
    value
        .as_table_mut()
        .ok_or_else(|| OrbitError::InvalidInput(format!("expected '{key}' to be a TOML table")))
}

fn ensure_toml_array<'a>(
    root: &'a mut TomlTable,
    key: &str,
) -> Result<&'a mut Vec<TomlValue>, OrbitError> {
    let value = root
        .entry(key.to_string())
        .or_insert_with(|| TomlValue::Array(Vec::new()));
    value
        .as_array_mut()
        .ok_or_else(|| OrbitError::InvalidInput(format!("expected '{key}' to be a TOML array")))
}
