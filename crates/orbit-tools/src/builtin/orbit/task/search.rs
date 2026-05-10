use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitTaskSearchTool;

impl Tool for OrbitTaskSearchTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "query".to_string(),
                description:
                    "Case-insensitive substring query matched against task title, description, and external ref IDs."
                        .to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "tag".to_string(),
                description: "Filter by tag. Repeat or pass an array for AND semantics."
                    .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::super::identity_params());
        ToolSchema {
            name: "orbit.task.search".to_string(),
            description:
                "Search Orbit tasks by case-insensitive title, description, or external ref ID match."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::TaskSearch)
    }
}
