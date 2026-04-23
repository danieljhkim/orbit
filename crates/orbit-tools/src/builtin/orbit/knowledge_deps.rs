use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::service::deps::crate_dependencies;
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

use super::knowledge_write::resolve_workspace_root_with_override;

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
        let crate_filter = super::optional_string(&input, "crate")?;
        let workspace_root = resolve_workspace_root_with_override(ctx, &input)?;

        let deps = crate_dependencies(&workspace_root, crate_filter.as_deref())
            .map_err(|e| OrbitError::Execution(format!("crate_dependencies: {e}")))?;

        Ok(json!({
            "workspace": workspace_root.display().to_string(),
            "crates": deps,
        }))
    }
}
