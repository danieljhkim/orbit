use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitAdrSupersedeTool;

impl Tool for OrbitAdrSupersedeTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "old_id".to_string(),
                description: "ADR being superseded.".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "new_id".to_string(),
                description:
                    "Replacement ADR. Must already be in the `accepted` state, or the call returns an invalid_transition error and writes nothing."
                        .to_string(),
                param_type: "string".to_string(),
                required: true,
            },
        ];
        parameters.extend(super::super::model_identity_params());
        ToolSchema {
            name: "orbit.adr.supersede".to_string(),
            description:
                "Write the bidirectional supersession edge: `old.status = superseded`, `old.superseded_by = new`, and `new.supersedes` gains `old`."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::reject_agent_field(&input, "orbit.adr.supersede")?;
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::AdrSupersede)
    }
}
