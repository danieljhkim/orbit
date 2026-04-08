use std::collections::HashSet;

use orbit_types::Role;

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
}

impl PolicyEngine {
    pub fn new_local_default_allow() -> Self {
        Self {
            default_allow: true,
            denied_tools: HashSet::new(),
        }
    }

    pub fn deny_tool(mut self, name: impl Into<String>) -> Self {
        self.denied_tools.insert(name.into());
        self
    }

    pub fn evaluate(&self, ctx: &PolicyContext) -> PolicyDecision {
        evaluator::evaluate(ctx, &self.denied_tools, self.default_allow)
    }
}
