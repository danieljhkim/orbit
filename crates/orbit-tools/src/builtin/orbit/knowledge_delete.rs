use orbit_knowledge::{Selector, load_task_working_graph, save_task_working_graph};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

use super::knowledge_write::{
    initialize_working_graph, lock_targets_for_mutation, resolve_knowledge_dir,
    resolve_workspace_root_with_override, with_graph_locks, write_err_to_orbit,
};

pub struct OrbitKnowledgeDeleteTool;

impl Tool for OrbitKnowledgeDeleteTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.delete".to_string(),
            description: "Delete a symbol from the source file and the working graph".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Symbol selector like `symbol:path#name:kind`".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "reason".to_string(),
                    description: "Optional reason for deletion, stored in version chain"
                        .to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "workspace_path".to_string(),
                    description: "Optional workspace root override for branch/worktree targeting"
                        .to_string(),
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
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        let selector: Selector = selector_str
            .parse()
            .map_err(|e| OrbitError::InvalidInput(format!("{e}")))?;

        if !matches!(selector, Selector::Symbol { .. }) {
            return Err(OrbitError::InvalidInput(
                "graph.delete requires a symbol selector (symbol:path#name:kind)".to_string(),
            ));
        }

        let workspace_root_buf = resolve_workspace_root_with_override(ctx, &input)?;
        let workspace_root = workspace_root_buf.as_path();

        let knowledge_dir = resolve_knowledge_dir(ctx, &input)?;
        let mut working_graph =
            match load_task_working_graph(ctx.orbit_root.as_deref(), ctx.task_id.as_deref())? {
                Some(graph) => graph,
                None => initialize_working_graph(&knowledge_dir, &selector, workspace_root)?,
            };

        let lock_owner = ctx
            .agent_name
            .as_deref()
            .or(ctx.task_id.as_deref())
            .unwrap_or("unknown");
        let lock_targets = lock_targets_for_mutation(&selector, &[]);

        let result = with_graph_locks(
            &knowledge_dir,
            lock_owner,
            ctx.task_id.as_deref(),
            reason.as_deref().unwrap_or("deleting"),
            &lock_targets,
            || {
                let result = working_graph
                    .delete_leaf(&selector, reason.as_deref(), workspace_root)
                    .map_err(write_err_to_orbit)?;

                save_task_working_graph(
                    ctx.orbit_root.as_deref(),
                    ctx.task_id.as_deref(),
                    &working_graph,
                )?;
                Ok(result)
            },
        )?;

        serde_json::to_value(result)
            .map_err(|e| OrbitError::Execution(format!("serialize result: {e}")))
    }
}
