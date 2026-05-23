use orbit_common::types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitSemanticStatsTool;

impl Tool for OrbitSemanticStatsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.semantic.stats".to_string(),
            description: "Show orbit-search index and companion status.".to_string(),
            parameters: Vec::new(),
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::SemanticStats)
    }
}
