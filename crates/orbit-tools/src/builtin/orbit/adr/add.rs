use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitAdrAddTool;

impl Tool for OrbitAdrAddTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "title".to_string(),
                description: "ADR title (short noun phrase).".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "body".to_string(),
                description:
                    "ADR body as markdown. Must include Context / Decision / Consequences sections per the ADR template; at least one consequences bullet must be a labeled `Cost:` line."
                        .to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "owner".to_string(),
                description:
                    "Agent identity that owns the ADR (e.g. `claude`, `codex`). Defaults to the calling actor."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "related_features".to_string(),
                description:
                    "Feature folder names this decision touches. Accepts a string or array of strings."
                        .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "related_tasks".to_string(),
                description:
                    "Orbit task IDs that proposed or shipped the decision. May be empty at creation per ADR-008. Accepts a string or array of strings."
                        .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::super::model_identity_params());
        ToolSchema {
            name: "orbit.adr.add".to_string(),
            description:
                "Create a Proposed Architecture Decision Record. Returns the assigned global ID and the full record JSON."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::reject_agent_field(&input, "orbit.adr.add")?;
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::AdrAdd)
    }
}
