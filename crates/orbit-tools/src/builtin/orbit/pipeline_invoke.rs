use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitPipelineInvokeTool;

impl Tool for OrbitPipelineInvokeTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "job_name".to_string(),
                description: "Registered v2 job name to invoke.".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "input".to_string(),
                description: "JSON object forwarded as the run input payload.".to_string(),
                param_type: "object".to_string(),
                required: true,
            },
            ToolParam {
                name: "priority".to_string(),
                description: "Optional queue priority (`low`, `medium`, `high`, or `critical`)."
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.pipeline.invoke".to_string(),
            description: "Submit a durable pipeline run and return immediately with its run ID."
                .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::PipelineInvoke)
    }
}
