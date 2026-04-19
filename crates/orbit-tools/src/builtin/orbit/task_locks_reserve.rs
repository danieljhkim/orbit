use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitTaskLocksReserveTool;

impl Tool for OrbitTaskLocksReserveTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "task_ids".to_string(),
                description: "Task IDs in the bundle as an array of strings.".to_string(),
                param_type: "array".to_string(),
                required: true,
            },
            ToolParam {
                name: "ttl_seconds".to_string(),
                description: "Optional reservation TTL in seconds. Defaults to 1800; max 7200."
                    .to_string(),
                param_type: "u64".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.task.locks.reserve".to_string(),
            description:
                "Attempt to atomically reserve the bundle's context files and return either a \
                 reservation grant or a conflict list."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::TaskLocksReserve)
    }
}
