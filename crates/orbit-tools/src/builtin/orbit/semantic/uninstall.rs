use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitSemanticUninstallTool;

impl Tool for OrbitSemanticUninstallTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.semantic.uninstall".to_string(),
            description: "Remove installed orbit-search companion and/or models.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "model".to_string(),
                    description: "Optional semantic embedding model alias to remove.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "all".to_string(),
                    description: "Remove all installed models in addition to the companion."
                        .to_string(),
                    param_type: "boolean".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::SemanticUninstall)
    }
}
