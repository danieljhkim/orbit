use orbit_common::friction::friction_tags_literal;
use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitFrictionUpdateTool;

impl Tool for OrbitFrictionUpdateTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.friction.update".to_string(),
            description: "Update triage metadata for an Orbit friction record".to_string(),
            parameters: vec![
                ToolParam {
                    name: "id".to_string(),
                    description: "Friction record id, e.g. F2026-05-001".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "status".to_string(),
                    description: "Optional status: open, triaged, or resolved".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "tags".to_string(),
                    description: format!(
                        "Optional replacement taxonomy tags as a string or array; valid tags: {}",
                        friction_tags_literal()
                    ),
                    param_type: "string_list".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "body".to_string(),
                    description: "Optional replacement markdown body".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::FrictionUpdate)
    }
}
