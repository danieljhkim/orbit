use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitTaskLocksReleaseTool;

impl Tool for OrbitTaskLocksReleaseTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![ToolParam {
            name: "reservation_id".to_string(),
            description: "Reservation ID to release.".to_string(),
            param_type: "string".to_string(),
            required: true,
        }];
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.task.locks.release".to_string(),
            description: "Release a previously granted task lock reservation.".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::TaskLocksRelease)
    }
}
