use orbit_common::OrbitError;
use orbit_types::tool::ToolSchema;
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitTaskLintTool;

impl Tool for OrbitTaskLintTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::super::orbit_id_params("task");
        parameters.extend(super::super::identity_params());
        ToolSchema {
            name: "orbit.task.lint".to_string(),
            description:
                "Lint an Orbit task for stale paths, vague acceptance criteria, and stale repository names."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::TaskLint)
    }
}
