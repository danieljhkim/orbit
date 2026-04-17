use orbit_knowledge::{Selector, TaskGraphService};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

use super::knowledge_write::{
    resolve_knowledge_dir, resolve_workspace_root_with_override, task_graph_scope,
    write_err_to_orbit,
};

pub struct OrbitKnowledgeAddTool;

impl Tool for OrbitKnowledgeAddTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.add".to_string(),
            description: "Add a new symbol to a source file, writing to disk and updating the working graph. Rejects if the symbol already exists.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Symbol selector like `symbol:path#name:kind` for the new symbol".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "source".to_string(),
                    description: "Full source code for the new symbol".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "position".to_string(),
                    description: "Optional anchor selector like `after:symbol:path#name:kind`. Inserts after the anchor. Omit to append before `#[cfg(test)]` or at end of file.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "reason".to_string(),
                    description: "Optional reason, stored in version chain".to_string(),
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
        let source = crate::require_str(&input, "source")?;
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
                "graph.add requires a symbol selector (symbol:path#name:kind)".to_string(),
            ));
        }

        let workspace_root_buf = resolve_workspace_root_with_override(ctx, &input)?;
        let workspace_root = workspace_root_buf.as_path();
        let knowledge_dir = resolve_knowledge_dir(ctx, &input)?;
        let service = TaskGraphService::new(knowledge_dir, task_graph_scope(ctx));
        let position_selector = parse_position(position.as_deref())?;

        let result = service.mutate(
            &selector,
            &[],
            reason.as_deref().unwrap_or("adding"),
            workspace_root,
            |working_graph| {
                if working_graph.has_leaf(&selector) {
                    let error = orbit_knowledge::WriteError::leaf_already_exists(&selector_str);
                    return Err(OrbitError::Execution(
                        serde_json::to_value(&error)
                            .map(|value| value.to_string())
                            .unwrap_or_else(|_| format!("{error:?}")),
                    ));
                }

                working_graph
                    .insert_leaf(
                        &selector,
                        &source,
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
