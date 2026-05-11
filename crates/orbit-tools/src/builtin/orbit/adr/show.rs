use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitAdrShowTool;

impl Tool for OrbitAdrShowTool {
    fn schema(&self) -> ToolSchema {
        let parameters = vec![
            ToolParam {
                name: "id".to_string(),
                description: "Canonical ADR ID (e.g. `ADR-0042`).".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "legacy_id".to_string(),
                description:
                    "Legacy ID alias (e.g. `activity-job/ADR-039`). Either `id` or `legacy_id` must be provided; specifying both is an error."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        ToolSchema {
            name: "orbit.adr.show".to_string(),
            description:
                "Fetch a single ADR by global ID or legacy ID alias. Returns the full record JSON including envelope fields and body."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::AdrShow)
    }
}
