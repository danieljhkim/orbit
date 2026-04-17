use std::collections::HashSet;

use orbit_types::{PolicyDef, Role};

use crate::{PolicyDecision, evaluator};

#[derive(Debug, Clone, Default)]
pub struct PolicyContext {
    pub entrypoint: String,
    pub tool_name: Option<String>,
    pub role: Role,
}

#[derive(Debug, Clone)]
pub struct PolicyEngine {
    default_allow: bool,
    denied_tools: HashSet<String>,
    allowed_tools: HashSet<String>,
    allowed_commands: HashSet<String>,
    denied_commands: HashSet<String>,
    allow_write_paths: Vec<String>,
    deny_write_paths: Vec<String>,
}

impl PolicyEngine {
    pub fn new_local_default_allow() -> Self {
        Self {
            default_allow: true,
            denied_tools: HashSet::new(),
            allowed_tools: HashSet::new(),
            allowed_commands: HashSet::new(),
            denied_commands: HashSet::new(),
            allow_write_paths: Vec::new(),
            deny_write_paths: Vec::new(),
        }
    }

    pub fn from_def(def: &PolicyDef) -> Self {
        let mut engine = Self::new_local_default_allow();

        if let Some(tools) = &def.tools {
            engine.denied_tools = tools.deny.iter().cloned().collect();
            engine.allowed_tools = tools.allow.iter().cloned().collect();
        }

        if let Some(process) = &def.process {
            engine.allowed_commands = process.allow_commands.iter().cloned().collect();
            engine.denied_commands = process.deny_commands.iter().cloned().collect();
        }

        if let Some(fs) = &def.filesystem {
            engine.allow_write_paths = fs.allow_write.clone();
            engine.deny_write_paths = fs.deny_write.clone();
        }

        engine
    }

    pub fn deny_tool(mut self, name: impl Into<String>) -> Self {
        self.denied_tools.insert(name.into());
        self
    }

    pub fn evaluate(&self, ctx: &PolicyContext) -> PolicyDecision {
        evaluator::evaluate(ctx, &self.denied_tools, self.default_allow)
    }

    /// Checks whether a tool is allowed by the ToolPolicy allow/deny lists.
    pub fn evaluate_tool(&self, tool_name: &str) -> PolicyDecision {
        if self.denied_tools.contains(tool_name) {
            return PolicyDecision::Deny {
                reason: format!("tool `{tool_name}` denied by policy"),
            };
        }
        if !self.allowed_tools.is_empty() && !self.allowed_tools.contains(tool_name) {
            return PolicyDecision::Deny {
                reason: format!("tool `{tool_name}` not in allow list"),
            };
        }
        PolicyDecision::Allow
    }

    /// Checks whether a command is allowed by the ProcessPolicy allow/deny lists.
    pub fn evaluate_process(&self, command: &str) -> PolicyDecision {
        let base_command = command.split_whitespace().next().unwrap_or(command);

        if self.denied_commands.contains(base_command) || self.denied_commands.contains(command) {
            return PolicyDecision::Deny {
                reason: format!("command `{command}` denied by policy"),
            };
        }
        if !self.allowed_commands.is_empty()
            && !self.allowed_commands.contains(base_command)
            && !self.allowed_commands.contains(command)
        {
            return PolicyDecision::Deny {
                reason: format!("command `{command}` not in allow list"),
            };
        }
        PolicyDecision::Allow
    }

    /// Checks whether a filesystem path is allowed for read or write access.
    /// Uses simple prefix/contains matching against configured paths.
    pub fn evaluate_filesystem(&self, path: &str, write: bool) -> PolicyDecision {
        if !write {
            return PolicyDecision::Allow;
        }

        for denied in &self.deny_write_paths {
            if path.starts_with(denied) || path.contains(denied) {
                return PolicyDecision::Deny {
                    reason: format!("write to `{path}` denied by policy (matches `{denied}`)"),
                };
            }
        }

        if !self.allow_write_paths.is_empty() {
            for allowed in &self.allow_write_paths {
                if path.starts_with(allowed) || path.contains(allowed) {
                    return PolicyDecision::Allow;
                }
            }
            return PolicyDecision::Deny {
                reason: format!("write to `{path}` not in allow list"),
            };
        }

        PolicyDecision::Allow
    }
}
