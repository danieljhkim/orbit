use std::path::Path;

use orbit_tools::ToolContext;
use orbit_types::{
    OrbitError, OrbitEvent, Role, StoredTool, ToolParam, normalize_optional_attribution_label,
};
use serde_json::Value;

use crate::{OrbitRuntime, context::ActorIdentity};

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
    pub fn execute_tool_command(
        &self,
        name: &str,
        input: Value,
        agent_override: Option<String>,
        model_override: Option<String>,
    ) -> Result<Value, OrbitError> {
        let allowed_tools = read_activity_tools_from_env();
        let (actor, agent_name, model_name) = resolve_tool_identity(agent_override, model_override);
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
        self.clone()
            .with_actor(actor)
            .run_tool_with_context_and_role(name, input, Role::Admin, tool_context)
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

fn resolve_agent_identity(
    agent_override: Option<String>,
    model_override: Option<String>,
) -> (Option<String>, Option<String>) {
    let (env_agent_name, env_model_name) = read_agent_identity_from_env();
    (
        agent_override.or(env_agent_name),
        model_override.or(env_model_name),
    )
}

fn resolve_tool_identity(
    agent_override: Option<String>,
    model_override: Option<String>,
) -> (ActorIdentity, Option<String>, Option<String>) {
    let (agent_name, model_name) = resolve_agent_identity(agent_override, model_override);
    let actor_label = normalize_optional_attribution_label(
        model_name.as_deref().or(agent_name.as_deref()),
        model_name.as_deref(),
    )
    .unwrap_or_else(|| "agent".to_string());
    (ActorIdentity::agent(actor_label), agent_name, model_name)
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
        let stored_tools = self.stores().tools().list()?;

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
                    parameters: stored.parameters.clone(),
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

        let stored = self.stores().tools().get(name)?;
        let enabled = stored.is_none_or(|s| s.enabled);

        Ok(ToolInfo {
            name: schema.name,
            description: schema.description,
            enabled,
            builtin: schema.builtin,
            parameters: schema.parameters,
        })
    }

    pub fn add_tool(
        &self,
        name: &str,
        path: &str,
        description: &str,
        parameters: Vec<ToolParam>,
    ) -> Result<(), OrbitError> {
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
            parameters,
        };

        self.with_mutation(|| {
            self.stores().tools().insert(&tool)?;
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
            let deleted = self.stores().tools().delete(name)?;
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
                && let Some(stored) = self.stores().tools().get(&tool.name)?
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

        let existing = self.stores().tools().get(name)?;
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
                parameters: schema.parameters.clone(),
            };
            return self.with_mutation(|| {
                self.stores().tools().insert(&tool)?;
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
            self.stores().tools().set_enabled(name, enabled)?;
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
