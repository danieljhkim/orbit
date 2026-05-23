use std::collections::HashMap;
use std::sync::Arc;

use orbit_common::types::{NotFoundKind, OrbitError, ToolSchema};
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
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, ToolEntry>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        self.register_with_availability(tool, ToolAvailability::Active);
    }

    pub fn register_inactive<T: Tool + 'static>(&mut self, tool: T) {
        self.register_with_availability(tool, ToolAvailability::Inactive);
    }

    fn register_with_availability<T: Tool + 'static>(
        &mut self,
        tool: T,
        availability: ToolAvailability,
    ) {
        let schema = tool.schema();
        self.tools.insert(
            schema.name,
            ToolEntry {
                tool: Arc::new(tool),
                availability,
            },
        );
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
}
