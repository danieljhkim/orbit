use std::path::Path;

use orbit_tools::ToolContext;
use orbit_types::{OrbitError, OrbitEvent, Role, StoredTool};
use serde_json::Value;

use crate::OrbitRuntime;

pub use crate::runtime::pipeline::DryRunResult;

#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub builtin: bool,
    pub parameters: Vec<orbit_types::ToolParam>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoctorStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct DoctorResult {
    pub tool_name: String,
    pub status: DoctorStatus,
    pub message: String,
}

impl OrbitRuntime {
    pub fn execute_tool_command(&self, name: &str, input: Value) -> Result<Value, OrbitError> {
        let allowed_tools = read_activity_tools_from_env();
        let (agent_name, model_name) = read_agent_identity_from_env();
        let proc_allowed_programs = read_proc_allowed_programs_from_env();
        let cwd = std::env::current_dir()
            .ok()
            .map(|path| path.to_string_lossy().into_owned());
        let tool_context = ToolContext {
            cwd,
            allowed_tools,
            agent_name,
            model_name,
            workspace_root: None,
            proc_allowed_programs,
            ..Default::default()
        };
        self.run_tool_with_context_and_role(name, input, Role::Admin, tool_context)
    }
}

fn read_agent_identity_from_env() -> (Option<String>, Option<String>) {
    let agent = std::env::var("ORBIT_AGENT_NAME")
        .ok()
        .filter(|s| !s.is_empty());
    let model = std::env::var("ORBIT_AGENT_MODEL")
        .ok()
        .filter(|s| !s.is_empty());
    (agent, model)
}

fn read_proc_allowed_programs_from_env() -> Vec<String> {
    std::env::var("ORBIT_PROC_ALLOWED_PROGRAMS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn read_activity_tools_from_env() -> Vec<String> {
    if std::env::var("ORBIT_TASK_ACTOR_KIND").ok().as_deref() != Some("agent") {
        return Vec::new();
    }
    std::env::var("ORBIT_ACTIVITY_TOOLS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

impl OrbitRuntime {
    pub fn list_tools(&self) -> Result<Vec<ToolInfo>, OrbitError> {
        let registry_schemas = self.tool_registry().schemas();
        let stored_tools = self.list_tool_records()?;

        let mut tools: Vec<ToolInfo> = registry_schemas
            .into_iter()
            .map(|schema| {
                let stored = stored_tools.iter().find(|s| s.name == schema.name);
                let enabled = stored.is_none_or(|s| s.enabled);
                ToolInfo {
                    name: schema.name.clone(),
                    description: schema.description.clone(),
                    enabled,
                    builtin: schema.builtin,
                    parameters: schema.parameters,
                }
            })
            .collect();

        // Add external tools that are in the store but not yet in the registry
        for stored in &stored_tools {
            if !stored.builtin && !tools.iter().any(|t| t.name == stored.name) {
                tools.push(ToolInfo {
                    name: stored.name.clone(),
                    description: stored.description.clone(),
                    enabled: stored.enabled,
                    builtin: false,
                    parameters: vec![],
                });
            }
        }

        tools.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tools)
    }

    pub fn show_tool(&self, name: &str) -> Result<ToolInfo, OrbitError> {
        let schema = self
            .tool_registry()
            .get_schema(name)
            .ok_or_else(|| OrbitError::ToolNotFound(name.to_string()))?;

        let stored = self.get_tool_record(name)?;
        let enabled = stored.is_none_or(|s| s.enabled);

        Ok(ToolInfo {
            name: schema.name,
            description: schema.description,
            enabled,
            builtin: schema.builtin,
            parameters: schema.parameters,
        })
    }

    pub fn add_tool(&self, name: &str, path: &str, description: &str) -> Result<(), OrbitError> {
        let p = Path::new(path);
        if !p.exists() {
            return Err(OrbitError::InvalidInput(format!(
                "path does not exist: {path}"
            )));
        }

        if let Some(schema) = self.tool_registry().get_schema(name)
            && schema.builtin
        {
            return Err(OrbitError::InvalidInput(format!(
                "cannot overwrite built-in tool '{name}'"
            )));
        }

        let tool = StoredTool {
            name: name.to_string(),
            path: path.to_string(),
            description: description.to_string(),
            enabled: true,
            builtin: false,
        };

        self.with_mutation(|| {
            self.insert_tool_record(&tool)?;
            Ok((
                (),
                OrbitEvent::ToolAdded {
                    name: name.to_string(),
                },
            ))
        })
    }

    pub fn remove_tool(&self, name: &str) -> Result<(), OrbitError> {
        if let Some(schema) = self.tool_registry().get_schema(name)
            && schema.builtin
        {
            return Err(OrbitError::InvalidInput(format!(
                "cannot remove built-in tool '{name}'; use 'orbit tool disable {name}' instead"
            )));
        }

        self.with_mutation(|| {
            let deleted = self.delete_tool_record(name)?;
            if !deleted {
                return Err(OrbitError::ToolNotFound(name.to_string()));
            }
            Ok((
                (),
                OrbitEvent::ToolRemoved {
                    name: name.to_string(),
                },
            ))
        })
    }

    pub fn doctor(&self) -> Result<Vec<DoctorResult>, OrbitError> {
        let tools = self.list_tools()?;
        let mut results = Vec::new();

        for tool in &tools {
            if !tool.enabled {
                results.push(DoctorResult {
                    tool_name: tool.name.clone(),
                    status: DoctorStatus::Warning,
                    message: "tool is disabled".to_string(),
                });
                continue;
            }

            if tool.description.is_empty() {
                results.push(DoctorResult {
                    tool_name: tool.name.clone(),
                    status: DoctorStatus::Warning,
                    message: "missing description".to_string(),
                });
                continue;
            }

            if !tool.builtin
                && let Some(stored) = self.get_tool_record(&tool.name)?
                && !stored.path.is_empty()
            {
                let path = std::path::Path::new(&stored.path);
                if !path.exists() {
                    results.push(DoctorResult {
                        tool_name: tool.name.clone(),
                        status: DoctorStatus::Error,
                        message: format!("executable not found: {}", stored.path),
                    });
                    continue;
                }
            }

            results.push(DoctorResult {
                tool_name: tool.name.clone(),
                status: DoctorStatus::Ok,
                message: String::new(),
            });
        }

        Ok(results)
    }

    pub fn enable_tool(&self, name: &str) -> Result<(), OrbitError> {
        self.set_tool_enabled_state(name, true)
    }

    pub fn disable_tool(&self, name: &str) -> Result<(), OrbitError> {
        self.set_tool_enabled_state(name, false)
    }

    fn set_tool_enabled_state(&self, name: &str, enabled: bool) -> Result<(), OrbitError> {
        if !self.tool_registry().has(name) {
            return Err(OrbitError::ToolNotFound(name.to_string()));
        }

        let existing = self.get_tool_record(name)?;
        if existing.is_none() {
            let schema = self
                .tool_registry()
                .get_schema(name)
                .ok_or_else(|| OrbitError::ToolNotFound(name.to_string()))?;
            let tool = StoredTool {
                name: name.to_string(),
                path: String::new(),
                description: schema.description.clone(),
                enabled,
                builtin: schema.builtin,
            };
            return self.with_mutation(|| {
                self.insert_tool_record(&tool)?;
                let event = if enabled {
                    OrbitEvent::ToolEnabled {
                        name: name.to_string(),
                    }
                } else {
                    OrbitEvent::ToolDisabled {
                        name: name.to_string(),
                    }
                };
                Ok(((), event))
            });
        }

        self.with_mutation(|| {
            self.set_tool_enabled_record(name, enabled)?;
            let event = if enabled {
                OrbitEvent::ToolEnabled {
                    name: name.to_string(),
                }
            } else {
                OrbitEvent::ToolDisabled {
                    name: name.to_string(),
                }
            };
            Ok(((), event))
        })
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use orbit_types::OrbitError;
    use serde_json::{Value, json};

    use super::read_activity_tools_from_env;
    use crate::OrbitRuntime;

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    const TEST_ENV_KEYS: &[&str] = &[
        "ORBIT_TASK_ACTOR_KIND",
        "ORBIT_TASK_ACTOR_LABEL",
        "ORBIT_ACTIVITY_TOOLS",
        "ORBIT_AGENT_NAME",
        "ORBIT_AGENT_MODEL",
        "ORBIT_PROC_ALLOWED_PROGRAMS",
    ];

    struct EnvGuard {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn capture() -> Self {
            Self {
                saved: TEST_ENV_KEYS
                    .iter()
                    .map(|key| (*key, std::env::var(key).ok()))
                    .collect(),
            }
        }

        fn clear_test_keys() {
            for key in TEST_ENV_KEYS {
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            Self::clear_test_keys();
            for (key, value) in self.saved.drain(..) {
                if let Some(value) = value {
                    unsafe { std::env::set_var(key, value) };
                }
            }
        }
    }

    fn with_test_env<R>(updates: &[(&'static str, Option<&str>)], f: impl FnOnce() -> R) -> R {
        let _lock = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock");
        let _guard = EnvGuard::capture();
        EnvGuard::clear_test_keys();

        for (key, value) in updates {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }

        f()
    }

    #[test]
    fn read_activity_tools_returns_allowlist_for_agent_actor() {
        with_test_env(
            &[
                ("ORBIT_TASK_ACTOR_KIND", Some("agent")),
                ("ORBIT_ACTIVITY_TOOLS", Some("fs.read, fs.write")),
            ],
            || {
                assert_eq!(
                    read_activity_tools_from_env(),
                    vec!["fs.read".to_string(), "fs.write".to_string()]
                );
            },
        );
    }

    #[test]
    fn read_activity_tools_returns_empty_without_agent_actor_kind() {
        with_test_env(
            &[("ORBIT_ACTIVITY_TOOLS", Some("fs.read,fs.write"))],
            || {
                assert!(read_activity_tools_from_env().is_empty());
            },
        );
    }

    #[test]
    fn read_activity_tools_returns_empty_when_allowlist_is_missing() {
        with_test_env(&[("ORBIT_TASK_ACTOR_KIND", Some("agent"))], || {
            assert!(read_activity_tools_from_env().is_empty());
        });
    }

    #[test]
    fn execute_tool_command_allows_allowlisted_tool_for_agent_actor() {
        with_test_env(
            &[
                ("ORBIT_TASK_ACTOR_KIND", Some("agent")),
                ("ORBIT_TASK_ACTOR_LABEL", Some("codex")),
                ("ORBIT_ACTIVITY_TOOLS", Some("time.now")),
                ("ORBIT_AGENT_NAME", Some("codex")),
                ("ORBIT_AGENT_MODEL", Some("gpt-5.4")),
            ],
            || {
                let runtime = OrbitRuntime::in_memory().expect("runtime");
                let output = runtime
                    .execute_tool_command("time.now", json!({}))
                    .expect("allowlisted tool should run");

                assert!(output.get("now").and_then(Value::as_str).is_some());
            },
        );
    }

    #[test]
    fn execute_tool_command_rejects_disallowed_tool_for_agent_actor() {
        with_test_env(
            &[
                ("ORBIT_TASK_ACTOR_KIND", Some("agent")),
                ("ORBIT_TASK_ACTOR_LABEL", Some("codex")),
                ("ORBIT_ACTIVITY_TOOLS", Some("time.now")),
                ("ORBIT_AGENT_NAME", Some("codex")),
                ("ORBIT_AGENT_MODEL", Some("gpt-5.4")),
            ],
            || {
                let runtime = OrbitRuntime::in_memory().expect("runtime");
                let error = runtime
                    .execute_tool_command("time.sleep", json!({"ms": 0}))
                    .expect_err("disallowed tool should be rejected");

                assert!(matches!(
                    error,
                    OrbitError::PolicyDenied(message)
                    if message == "tool 'time.sleep' is not in the activity allowlist"
                ));
            },
        );
    }
}
