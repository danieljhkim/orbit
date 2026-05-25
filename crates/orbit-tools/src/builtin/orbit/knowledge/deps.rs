use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::commands::deps::{self, DepsInput};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeDepsTool;

impl Tool for OrbitKnowledgeDepsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.deps".to_string(),
            description: "Use when you need workspace crate edges. Prefer over grep when Cargo text must be grouped into links.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "crate".to_string(),
                    description: "Only this crate.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "workspace_path".to_string(),
                    description: "Override workspace root.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let crate_filter = super::super::optional_string(&input, "crate")?;
        let workspace_root = super::resolve_workspace_root_with_override(ctx, &input)?;

        let result = deps::run(DepsInput {
            workspace_root,
            crate_filter,
        })
        .map_err(super::knowledge_error_to_orbit)?;

        Ok(json!({
            "workspace": result.workspace.display().to_string(),
            "crates": result.crates,
        }))
    }
}
