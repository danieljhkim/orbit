use std::collections::HashMap;
use std::sync::Arc;

use orbit_common::types::{
    McpToolDefinition, McpToolPolicy, McpToolPolicyError, NotFoundKind, OrbitError, ToolSchema,
    mcp_advertised_tool_name, validate_mcp_tool_definitions,
};
use serde_json::Value;

use crate::{Tool, ToolContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAvailability {
    Active,
    Inactive,
}

impl ToolAvailability {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }
}

struct ToolEntry {
    tool: Arc<dyn Tool>,
    availability: ToolAvailability,
    mcp_policy: Option<McpToolPolicy>,
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, ToolEntry>,
    mcp_registration_error: Option<McpToolPolicyError>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            mcp_registration_error: None,
        }
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        self.register_with_availability(tool, ToolAvailability::Active, None);
    }

    /// Register one active builtin with its inseparable MCP exposure policy.
    pub fn register_mcp<T: Tool + 'static>(&mut self, tool: T, policy: McpToolPolicy) {
        let policy = match policy.validate() {
            Ok(()) => Some(policy),
            Err(error) => {
                self.record_mcp_error(error);
                None
            }
        };
        self.register_with_availability(tool, ToolAvailability::Active, policy);
    }

    pub fn register_inactive<T: Tool + 'static>(&mut self, tool: T) {
        self.register_with_availability(tool, ToolAvailability::Inactive, None);
    }

    fn register_with_availability<T: Tool + 'static>(
        &mut self,
        tool: T,
        availability: ToolAvailability,
        mcp_policy: Option<McpToolPolicy>,
    ) {
        let schema = tool.schema();
        if let Some(existing) = self.tools.get(&schema.name)
            && (existing.mcp_policy.is_some() || mcp_policy.is_some())
        {
            self.record_mcp_error(McpToolPolicyError::DuplicateCanonicalName(
                schema.name.clone(),
            ));
        }
        if mcp_policy.is_some() {
            let advertised_name = mcp_advertised_tool_name(&schema.name);
            if self.tools.iter().any(|(name, entry)| {
                entry.mcp_policy.is_some()
                    && name != &schema.name
                    && mcp_advertised_tool_name(name) == advertised_name
            }) {
                self.record_mcp_error(McpToolPolicyError::DuplicateAdvertisedName(advertised_name));
            }
        }
        self.tools.insert(
            schema.name,
            ToolEntry {
                tool: Arc::new(tool),
                availability,
                mcp_policy,
            },
        );
    }

    fn record_mcp_error(&mut self, error: McpToolPolicyError) {
        if self.mcp_registration_error.is_none() {
            self.mcp_registration_error = Some(error);
        }
    }

    pub fn register_builtins(&mut self) {
        crate::builtin::register_builtins(self);
    }

    pub fn execute(
        &self,
        name: &str,
        ctx: &ToolContext,
        input: Value,
    ) -> Result<Value, OrbitError> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Tool, name.to_string()))?;
        tool.tool.execute(ctx, input)
    }

    pub fn get_schema(&self, name: &str) -> Option<ToolSchema> {
        self.tools.get(name).map(|entry| entry.tool.schema())
    }

    pub fn get_active_schema(&self, name: &str) -> Option<ToolSchema> {
        self.tools
            .get(name)
            .filter(|entry| entry.availability.is_active())
            .map(|entry| entry.tool.schema())
    }

    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn availability(&self, name: &str) -> Option<ToolAvailability> {
        self.tools.get(name).map(|entry| entry.availability)
    }

    pub fn is_active(&self, name: &str) -> bool {
        self.availability(name)
            .is_some_and(ToolAvailability::is_active)
    }

    pub fn unregister(&mut self, name: &str) -> bool {
        self.tools.remove(name).is_some()
    }

    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools
            .values()
            .filter(|entry| entry.availability.is_active())
            .map(|entry| entry.tool.schema())
            .collect()
    }

    pub fn all_schemas(&self) -> Vec<ToolSchema> {
        self.tools
            .values()
            .map(|entry| entry.tool.schema())
            .collect()
    }

    /// Enumerate validated, active builtin MCP definitions without runtime or workspace state.
    pub fn mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, McpToolPolicyError> {
        if let Some(error) = &self.mcp_registration_error {
            return Err(error.clone());
        }
        let mut definitions = self
            .tools
            .values()
            .filter(|entry| entry.availability.is_active())
            .filter_map(|entry| {
                entry
                    .mcp_policy
                    .clone()
                    .map(|policy| McpToolDefinition::new(entry.tool.schema(), policy))
            })
            .collect::<Result<Vec<_>, _>>()?;
        definitions.sort_by(|left, right| left.schema.name.cmp(&right.schema.name));
        validate_mcp_tool_definitions(&definitions)?;
        Ok(definitions)
    }
}

/// Workspace-independent source for every canonical registry-backed MCP definition.
pub fn canonical_builtin_mcp_tool_definitions() -> Result<Vec<McpToolDefinition>, McpToolPolicyError>
{
    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    registry.mcp_tool_definitions()
}
