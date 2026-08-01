use orbit_common::types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitAdrAddTool;

impl Tool for OrbitAdrAddTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::create_params();
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
