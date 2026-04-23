use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::{Selector, TaskGraphService};
use serde_json::Value;

use crate::{Tool, ToolContext};

use super::knowledge_write::{
    resolve_knowledge_dir, resolve_workspace_root_with_override, task_graph_scope,
    write_err_to_orbit,
};

pub struct OrbitKnowledgeDeleteTool;

impl Tool for OrbitKnowledgeDeleteTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.delete".to_string(),
            description: "Use when you need to delete a symbol safely. Prefer over grep when text delete could hit the wrong node.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Symbol selector.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "reason".to_string(),
                    description: "Optional change note.".to_string(),
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
        let selector_str = crate::require_str(&input, "selector")?;
        let reason = input
            .get("reason")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned);

        let selector: Selector = selector_str
            .parse::<Selector>()
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
        if !matches!(selector, Selector::Symbol { .. }) {
            return Err(OrbitError::InvalidInput(
                "graph.delete requires a symbol selector (symbol:path#name:kind)".to_string(),
            ));
        }

        let workspace_root_buf = resolve_workspace_root_with_override(ctx, &input)?;
        let workspace_root = workspace_root_buf.as_path();
        let knowledge_dir = resolve_knowledge_dir(ctx, &input)?;
        let service = TaskGraphService::new(knowledge_dir, task_graph_scope(ctx));

        let result = service.mutate(
            &selector,
            &[],
            reason.as_deref().unwrap_or("deleting"),
            workspace_root,
            |working_graph| {
                working_graph
                    .delete_leaf(&selector, reason.as_deref(), workspace_root)
                    .map_err(write_err_to_orbit)
            },
        )?;

        serde_json::to_value(result)
            .map_err(|error| OrbitError::Execution(format!("serialize result: {error}")))
    }
}
