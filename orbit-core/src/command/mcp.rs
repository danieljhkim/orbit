use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map as JsonMap, Value as JsonValue, json};
use toml::Value as TomlValue;

use crate::OrbitRuntime;
use orbit_types::OrbitError;

#[derive(Debug, Clone)]
pub struct McpConfigMutation {
    pub path: PathBuf,
    pub existed: bool,
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct McpInitResult {
    pub codex: McpConfigMutation,
    pub claude: McpConfigMutation,
    pub claude_code: McpConfigMutation,
}

impl OrbitRuntime {
    pub fn init_mcp_configs(&self, dry_run: bool) -> Result<McpInitResult, OrbitError> {
        let codex_path = codex_config_path()?;
        let claude_path = claude_config_path()?;
        let claude_code_path = claude_code_config_path()?;
        let command = resolve_orbit_command()?;
        let data_root = self.data_root();
        upsert_mcp_configs(
            &codex_path,
            &claude_path,
            &claude_code_path,
            &command,
            &data_root,
            dry_run,
        )
    }
}

pub fn upsert_mcp_configs(
    codex_path: &Path,
    claude_path: &Path,
    claude_code_path: &Path,
    command: &str,
    data_root: &Path,
    dry_run: bool,
) -> Result<McpInitResult, OrbitError> {
    let codex = upsert_codex_config(codex_path, command, data_root, dry_run)?;
    let claude = upsert_claude_config(claude_path, command, data_root, dry_run)?;
    let claude_code = upsert_claude_config(claude_code_path, command, data_root, dry_run)?;
    Ok(McpInitResult {
        codex,
        claude,
        claude_code,
    })
}

fn upsert_codex_config(
    path: &Path,
    command: &str,
    data_root: &Path,
    dry_run: bool,
) -> Result<McpConfigMutation, OrbitError> {
    let existed = path.exists();

    let mut root = if existed {
        let raw = fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
        toml::from_str::<TomlValue>(&raw).map_err(|e| {
            OrbitError::InvalidInput(format!(
                "invalid codex TOML config '{}': {e}",
                path.display()
            ))
        })?
    } else {
        TomlValue::Table(Default::default())
    };

    let root_table = root.as_table_mut().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "codex config root must be a table: {}",
            path.display()
        ))
    })?;

    let mcp_servers = root_table
        .entry("mcp_servers")
        .or_insert_with(|| TomlValue::Table(Default::default()));
    let mcp_servers_table = mcp_servers.as_table_mut().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "codex config field 'mcp_servers' must be a table: {}",
            path.display()
        ))
    })?;

    let orbit_entry = mcp_servers_table
        .entry("orbit")
        .or_insert_with(|| TomlValue::Table(Default::default()));
    let orbit_table = orbit_entry.as_table_mut().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "codex config field 'mcp_servers.orbit' must be a table: {}",
            path.display()
        ))
    })?;

    let data_root_str = data_root.to_string_lossy();

    let mut changed = false;
    changed |= set_toml_string(orbit_table, "command", command);
    changed |= set_toml_string_array(orbit_table, "args", &["mcp", "start"]);
    changed |= set_toml_inline_table(orbit_table, "env", &[("ORBIT_ROOT", &data_root_str)]);

    if changed && !dry_run {
        write_text(
            path,
            toml::to_string_pretty(&root).map_err(|e| OrbitError::Execution(e.to_string()))?,
        )?;
    }

    Ok(McpConfigMutation {
        path: path.to_path_buf(),
        existed,
        changed,
    })
}

fn upsert_claude_config(
    path: &Path,
    command: &str,
    data_root: &Path,
    dry_run: bool,
) -> Result<McpConfigMutation, OrbitError> {
    let existed = path.exists();

    let mut root = if existed {
        let raw = fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
        serde_json::from_str::<JsonValue>(&raw).map_err(|e| {
            OrbitError::InvalidInput(format!(
                "invalid Claude JSON config '{}': {e}",
                path.display()
            ))
        })?
    } else {
        JsonValue::Object(JsonMap::new())
    };

    let root_obj = root.as_object_mut().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "Claude config root must be an object: {}",
            path.display()
        ))
    })?;

    let mcp_servers = root_obj
        .entry("mcpServers")
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    let mcp_servers_obj = mcp_servers.as_object_mut().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "Claude config field 'mcpServers' must be an object: {}",
            path.display()
        ))
    })?;

    let orbit_entry = mcp_servers_obj
        .entry("orbit")
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    let orbit_obj = orbit_entry.as_object_mut().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "Claude config field 'mcpServers.orbit' must be an object: {}",
            path.display()
        ))
    })?;

    let mut changed = false;
    changed |= set_json_string(orbit_obj, "command", command);
    changed |= set_json_string_array(orbit_obj, "args", &["mcp", "start"]);
    changed |= set_json_object(
        orbit_obj,
        "env",
        &[("ORBIT_ROOT", &data_root.to_string_lossy())],
    );

    if changed && !dry_run {
        let rendered = serde_json::to_string_pretty(&root)
            .map_err(|e| OrbitError::Execution(e.to_string()))?;
        write_text(path, format!("{rendered}\n"))?;
    }

    Ok(McpConfigMutation {
        path: path.to_path_buf(),
        existed,
        changed,
    })
}

fn write_text(path: &Path, content: String) -> Result<(), OrbitError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
    }
    fs::write(path, content).map_err(|e| OrbitError::Io(e.to_string()))
}

fn set_toml_string(table: &mut toml::map::Map<String, TomlValue>, key: &str, value: &str) -> bool {
    match table.get(key) {
        Some(TomlValue::String(current)) if current == value => false,
        _ => {
            table.insert(key.to_string(), TomlValue::String(value.to_string()));
            true
        }
    }
}

fn set_toml_string_array(
    table: &mut toml::map::Map<String, TomlValue>,
    key: &str,
    values: &[&str],
) -> bool {
    let next = TomlValue::Array(
        values
            .iter()
            .map(|value| TomlValue::String((*value).to_string()))
            .collect(),
    );

    if table.get(key) == Some(&next) {
        return false;
    }

    table.insert(key.to_string(), next);
    true
}

fn set_json_string(obj: &mut JsonMap<String, JsonValue>, key: &str, value: &str) -> bool {
    match obj.get(key) {
        Some(JsonValue::String(current)) if current == value => false,
        _ => {
            obj.insert(key.to_string(), JsonValue::String(value.to_string()));
            true
        }
    }
}

fn set_json_object(
    obj: &mut JsonMap<String, JsonValue>,
    key: &str,
    entries: &[(&str, &str)],
) -> bool {
    let mut next = JsonMap::new();
    for (k, v) in entries {
        next.insert((*k).to_string(), JsonValue::String((*v).to_string()));
    }
    let next_value = JsonValue::Object(next);
    if obj.get(key) == Some(&next_value) {
        return false;
    }
    obj.insert(key.to_string(), next_value);
    true
}

fn set_json_string_array(obj: &mut JsonMap<String, JsonValue>, key: &str, values: &[&str]) -> bool {
    let next = json!(values);
    if obj.get(key) == Some(&next) {
        return false;
    }

    obj.insert(key.to_string(), next);
    true
}

fn set_toml_inline_table(
    table: &mut toml::map::Map<String, TomlValue>,
    key: &str,
    entries: &[(&str, &str)],
) -> bool {
    let mut next = toml::map::Map::new();
    for (k, v) in entries {
        next.insert((*k).to_string(), TomlValue::String((*v).to_string()));
    }
    let next_value = TomlValue::Table(next);
    if table.get(key) == Some(&next_value) {
        return false;
    }
    table.insert(key.to_string(), next_value);
    true
}

fn resolve_orbit_command() -> Result<String, OrbitError> {
    let current_exe = std::env::current_exe().map_err(|e| {
        OrbitError::Execution(format!("failed to resolve current executable path: {e}"))
    })?;
    Ok(current_exe.to_string_lossy().into_owned())
}

fn codex_config_path() -> Result<PathBuf, OrbitError> {
    if let Ok(codex_home) = std::env::var("CODEX_HOME")
        && !codex_home.trim().is_empty()
    {
        return Ok(PathBuf::from(codex_home).join("config.toml"));
    }

    Ok(home_dir()?.join(".codex").join("config.toml"))
}

fn claude_code_config_path() -> Result<PathBuf, OrbitError> {
    Ok(home_dir()?.join(".claude").join(".mcp.json"))
}

fn claude_config_path() -> Result<PathBuf, OrbitError> {
    #[cfg(windows)]
    {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            if !profile.trim().is_empty() {
                return Ok(PathBuf::from(profile).join(".claude.json"));
            }
        }
    }

    Ok(home_dir()?.join(".claude.json"))
}

fn home_dir() -> Result<PathBuf, OrbitError> {
    if let Ok(home) = std::env::var("HOME")
        && !home.trim().is_empty()
    {
        return Ok(PathBuf::from(home));
    }
    if let Ok(profile) = std::env::var("USERPROFILE")
        && !profile.trim().is_empty()
    {
        return Ok(PathBuf::from(profile));
    }
    Err(OrbitError::InvalidInput(
        "HOME/USERPROFILE is not set; cannot resolve config paths".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use super::upsert_mcp_configs;

    const TEST_COMMAND: &str = "/usr/local/bin/orbit";

    fn test_data_root() -> std::path::PathBuf {
        std::path::PathBuf::from("/home/test/.orbit")
    }

    fn test_paths(
        dir: &tempfile::TempDir,
    ) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        (
            dir.path().join("codex.toml"),
            dir.path().join("claude.json"),
            dir.path().join("claude-code.json"),
        )
    }

    #[test]
    fn preserves_unrelated_settings_when_upserting() {
        let dir = tempdir().expect("tempdir");
        let (codex_path, claude_path, claude_code_path) = test_paths(&dir);
        let data_root = test_data_root();

        fs::write(
            &codex_path,
            "[profile]\nname=\"dev\"\n[mcp_servers.other]\ncommand=\"x\"\nargs=[\"y\"]\n",
        )
        .expect("write codex");
        fs::write(
            &claude_path,
            json!({
                "theme": "dark",
                "mcpServers": { "other": { "command": "x", "args": ["y"] } }
            })
            .to_string(),
        )
        .expect("write claude");

        let result = upsert_mcp_configs(
            &codex_path,
            &claude_path,
            &claude_code_path,
            TEST_COMMAND,
            &data_root,
            false,
        )
        .expect("upsert");
        assert!(result.codex.changed);
        assert!(result.claude.changed);
        assert!(result.claude_code.changed);

        let codex = fs::read_to_string(&codex_path).expect("read codex");
        assert!(codex.contains("[profile]"));
        assert!(codex.contains("[mcp_servers.other]"));
        assert!(codex.contains("[mcp_servers.orbit]"));

        let claude_raw = fs::read_to_string(&claude_path).expect("read claude");
        let claude: serde_json::Value = serde_json::from_str(&claude_raw).expect("parse claude");
        assert_eq!(claude["theme"], "dark");
        assert_eq!(claude["mcpServers"]["other"]["command"], "x");
        assert_eq!(claude["mcpServers"]["orbit"]["command"], TEST_COMMAND);
    }

    #[test]
    fn dry_run_does_not_write() {
        let dir = tempdir().expect("tempdir");
        let (codex_path, claude_path, claude_code_path) = test_paths(&dir);
        let data_root = test_data_root();

        let result = upsert_mcp_configs(
            &codex_path,
            &claude_path,
            &claude_code_path,
            TEST_COMMAND,
            &data_root,
            true,
        )
        .expect("dry run");
        assert!(result.codex.changed);
        assert!(result.claude.changed);
        assert!(result.claude_code.changed);
        assert!(!codex_path.exists());
        assert!(!claude_path.exists());
        assert!(!claude_code_path.exists());
    }

    #[test]
    fn writes_absolute_command_and_env() {
        let dir = tempdir().expect("tempdir");
        let (codex_path, claude_path, claude_code_path) = test_paths(&dir);
        let data_root = test_data_root();

        let result = upsert_mcp_configs(
            &codex_path,
            &claude_path,
            &claude_code_path,
            TEST_COMMAND,
            &data_root,
            false,
        )
        .expect("upsert");
        assert!(result.codex.changed);
        assert!(result.claude.changed);
        assert!(result.claude_code.changed);

        // Verify codex config has absolute command and ORBIT_ROOT env
        let codex = fs::read_to_string(&codex_path).expect("read codex");
        assert!(
            codex.contains(TEST_COMMAND),
            "codex config must contain absolute command path"
        );
        assert!(
            codex.contains("ORBIT_ROOT"),
            "codex config must contain ORBIT_ROOT env"
        );

        // Verify claude config has absolute command and env
        let claude_raw = fs::read_to_string(&claude_path).expect("read claude");
        let claude: serde_json::Value = serde_json::from_str(&claude_raw).expect("parse claude");
        assert_eq!(claude["mcpServers"]["orbit"]["command"], TEST_COMMAND);
        assert_eq!(
            claude["mcpServers"]["orbit"]["env"]["ORBIT_ROOT"],
            data_root.to_string_lossy().as_ref()
        );

        // Verify claude code config has absolute command and env
        let cc_raw = fs::read_to_string(&claude_code_path).expect("read claude-code");
        let cc: serde_json::Value = serde_json::from_str(&cc_raw).expect("parse claude-code");
        assert_eq!(cc["mcpServers"]["orbit"]["command"], TEST_COMMAND);
        assert_eq!(
            cc["mcpServers"]["orbit"]["env"]["ORBIT_ROOT"],
            data_root.to_string_lossy().as_ref()
        );
    }
}
