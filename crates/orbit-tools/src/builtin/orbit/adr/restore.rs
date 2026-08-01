use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitAdrRestoreTool;

impl Tool for OrbitAdrRestoreTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![ToolParam {
            name: "id".to_string(),
            description: "Exact existing canonical ADR allocation to restore (e.g. `ADR-0042`)."
                .to_string(),
            param_type: "string".to_string(),
            required: true,
        }];
        parameters.extend(super::create_params());
        parameters.extend(super::super::model_identity_params());
        ToolSchema {
            name: "orbit.adr.restore".to_string(),
            description:
                "Restore an unreadable ADR at its exact existing allocation. Refuses missing allocations, readable artifacts, lifecycle collisions, and concurrent allocation changes; never allocates or overwrites."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::reject_agent_field(&input, "orbit.adr.restore")?;
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::AdrRestore)
    }
}
