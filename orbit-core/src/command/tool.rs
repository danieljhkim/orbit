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
        let tool_context = ToolContext {
            cwd: None,
            allowed_tools,
        };
        self.run_tool_with_context_and_role(name, input, Role::Admin, tool_context)
    }
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
        let registry_schemas = self.context.registry.schemas();
        let stored_tools = self.context.tool_store.list_tools()?;

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
            .context
            .registry
            .get_schema(name)
            .ok_or_else(|| OrbitError::ToolNotFound(name.to_string()))?;

        let stored = self.context.tool_store.get_tool(name)?;
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

        if let Some(schema) = self.context.registry.get_schema(name)
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
            self.context.tool_store.insert_tool(&tool)?;
            Ok((
                (),
                OrbitEvent::ToolAdded {
                    name: name.to_string(),
                },
            ))
        })
    }

    pub fn remove_tool(&self, name: &str) -> Result<(), OrbitError> {
        if let Some(schema) = self.context.registry.get_schema(name)
            && schema.builtin
        {
            return Err(OrbitError::InvalidInput(format!(
                "cannot remove built-in tool '{name}'; use 'orbit tool disable {name}' instead"
            )));
        }

        self.with_mutation(|| {
            let deleted = self.context.tool_store.delete_tool(name)?;
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
                && let Some(stored) = self.context.tool_store.get_tool(&tool.name)?
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
        if !self.context.registry.has(name) {
            return Err(OrbitError::ToolNotFound(name.to_string()));
        }

        let existing = self.context.tool_store.get_tool(name)?;
        if existing.is_none() {
            let schema = self
                .context
                .registry
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
                self.context.tool_store.insert_tool(&tool)?;
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
            self.context.tool_store.set_tool_enabled(name, enabled)?;
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
