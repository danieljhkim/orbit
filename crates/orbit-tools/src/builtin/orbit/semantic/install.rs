use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitSemanticInstallTool;

impl Tool for OrbitSemanticInstallTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.semantic.install".to_string(),
            description: "Download the search companion and selected model.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "model".to_string(),
                    description: "Optional semantic embedding model alias, such as bge-small."
                        .to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "force".to_string(),
                    description:
                        "Replace the companion even when the installed version is current."
                            .to_string(),
                    param_type: "boolean".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::SemanticInstall)
    }
}
