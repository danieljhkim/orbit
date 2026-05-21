use orbit_common::friction::friction_tags_literal;
use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitFrictionAddTool;

impl Tool for OrbitFrictionAddTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.friction.add".to_string(),
            description: "Append an Orbit friction report under .orbit/frictions/".to_string(),
            parameters: vec![
                ToolParam {
                    name: "body".to_string(),
                    description:
                        "Markdown body describing what happened and why it caused friction"
                            .to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "tags".to_string(),
                    description: format!(
                        "Friction taxonomy tags as a string or array; valid tags: {}; defaults to other",
                        friction_tags_literal()
                    ),
                    param_type: "string_list".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "during_task".to_string(),
                    description: "Optional task ID being worked on when friction occurred"
                        .to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "model".to_string(),
                    description:
                        "Required agent family for attribution (`codex`, `claude`, `gemini`, or `grok`)"
                            .to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::reject_agent_field(&input, "orbit.friction.add")?;
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::FrictionAdd)
    }
}

