use orbit_knowledge::{Selector, TaskGraphService};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

use super::knowledge_write::{
    resolve_knowledge_dir, resolve_workspace_root_with_override, task_graph_scope,
    write_err_to_orbit,
};

pub struct OrbitKnowledgeMoveTool;

impl Tool for OrbitKnowledgeMoveTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.move".to_string(),
            description: "Move a symbol from one file to another, updating both source files and the working graph".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Source symbol selector like `symbol:path#name:kind`".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "target_file".to_string(),
                    description: "Destination file path (relative to workspace root)".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "position".to_string(),
                    description: "Optional anchor selector like `after:symbol:path#name:kind` in the target file. Omit to append before `#[cfg(test)]` or at end of file.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "reason".to_string(),
                    description: "Optional reason for the move, stored in version chain".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "workspace_path".to_string(),
                    description: "Optional workspace root override for branch/worktree targeting".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let selector_str = crate::require_str(&input, "selector")?;
        let target_file = crate::require_str(&input, "target_file")?;
        let reason = input
            .get("reason")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned);
        let position = input
            .get("position")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned);

        let selector: Selector = selector_str
            .parse::<Selector>()
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
        if !matches!(selector, Selector::Symbol { .. }) {
            return Err(OrbitError::InvalidInput(
                "graph.move requires a symbol selector (symbol:path#name:kind)".to_string(),
            ));
        }

        let workspace_root_buf = resolve_workspace_root_with_override(ctx, &input)?;
        let workspace_root = workspace_root_buf.as_path();
        let knowledge_dir = resolve_knowledge_dir(ctx, &input)?;
        let service = TaskGraphService::new(knowledge_dir, task_graph_scope(ctx));
        let position_selector = parse_position(position.as_deref())?;

        let result = service.mutate(
            &selector,
            &[target_file.as_str()],
            reason.as_deref().unwrap_or("moving"),
            workspace_root,
            |working_graph| {
                working_graph
                    .move_leaf(
                        &selector,
                        &target_file,
                        position_selector.as_ref(),
                        reason.as_deref(),
                        workspace_root,
                    )
                    .map_err(write_err_to_orbit)
            },
        )?;

        serde_json::to_value(result)
            .map_err(|error| OrbitError::Execution(format!("serialize result: {error}")))
    }
}

fn parse_position(position: Option<&str>) -> Result<Option<Selector>, OrbitError> {
    let Some(position) = position else {
        return Ok(None);
    };
    position
        .strip_prefix("after:")
        .unwrap_or(position)
        .parse()
        .map(Some)
        .map_err(|error| OrbitError::InvalidInput(format!("invalid position: {error}")))
}
