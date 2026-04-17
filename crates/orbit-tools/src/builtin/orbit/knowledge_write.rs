use std::path::{Path, PathBuf};

use orbit_knowledge::{Selector, TaskGraphScope, TaskGraphService, default_knowledge_dir};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeWriteTool;

impl Tool for OrbitKnowledgeWriteTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.write".to_string(),
            description: "Edit a symbol or file in the knowledge graph. Accepts symbol selectors (edit/insert leaf) or file selectors (rewrite file or region).".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Symbol selector (`symbol:path#symbol:kind`) to edit a leaf, or file selector (`file:path`) for file-level writes.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "new_source".to_string(),
                    description: "The source code to write".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "position".to_string(),
                    description: "For insert mode: anchor selector like `after:symbol:path#symbol:kind`. Inserts after the anchor leaf.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "start_line".to_string(),
                    description: "For file selectors: start line of region to replace (1-indexed). Requires end_line.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "end_line".to_string(),
                    description: "For file selectors: end line of region to replace (1-indexed, inclusive). Requires start_line.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "reason".to_string(),
                    description: "Optional reason for this edit, stored in the version chain".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "workspace_path".to_string(),
                    description: "Optional workspace root override for branch/worktree targeting".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "knowledge_dir".to_string(),
                    description: "Optional knowledge artifact directory; defaults to `<workspace>/.orbit/knowledge`".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let selector_str = crate::require_str(&input, "selector")?;
        let new_source = require_new_source(&input)?;
        let reason = optional_str(&input, "reason");
        let position_str = optional_str(&input, "position");

        let selector: Selector = selector_str
            .parse::<Selector>()
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;

        if matches!(selector, Selector::Dir { .. }) {
            return Err(OrbitError::InvalidInput(
                "graph.write does not accept dir selectors".to_string(),
            ));
        }

        let workspace_root_buf = resolve_workspace_root_with_override(ctx, &input)?;
        let workspace_root = workspace_root_buf.as_path();
        let knowledge_dir = resolve_knowledge_dir(ctx, &input)?;
        let service = TaskGraphService::new(knowledge_dir, task_graph_scope(ctx));
        let position_selector = parse_position_selector(position_str.as_deref())?;

        let result = service.mutate(
            &selector,
            &[],
            reason.as_deref().unwrap_or("editing"),
            workspace_root,
            |working_graph| match &selector {
                Selector::File { path } => {
                    let start_line = input.get("start_line").and_then(Value::as_u64);
                    let end_line = input.get("end_line").and_then(Value::as_u64);
                    match (start_line, end_line) {
                        (Some(start_line), Some(end_line)) => working_graph
                            .rewrite_file_region(
                                path.as_str(),
                                start_line as usize,
                                end_line as usize,
                                &new_source,
                                reason.as_deref(),
                                workspace_root,
                            )
                            .map_err(write_err_to_orbit),
                        (None, None) => working_graph
                            .rewrite_file(
                                path.as_str(),
                                &new_source,
                                reason.as_deref(),
                                workspace_root,
                            )
                            .map_err(write_err_to_orbit),
                        _ => Err(OrbitError::InvalidInput(
                            "both start_line and end_line are required for region edits"
                                .to_string(),
                        )),
                    }
                }
                Selector::Symbol { .. } => {
                    if working_graph.has_leaf(&selector) {
                        working_graph
                            .edit_leaf(&selector, &new_source, reason.as_deref(), workspace_root)
                            .map_err(write_err_to_orbit)
                    } else {
                        working_graph
                            .insert_leaf(
                                &selector,
                                &new_source,
                                position_selector.as_ref(),
                                reason.as_deref(),
                                workspace_root,
                            )
                            .map_err(write_err_to_orbit)
                    }
                }
                Selector::Dir { .. } => unreachable!(),
            },
        )?;

        serde_json::to_value(result)
            .map_err(|error| OrbitError::Execution(format!("serialize result: {error}")))
    }
}

pub(super) fn resolve_workspace_root_with_override(
    ctx: &ToolContext,
    input: &Value,
) -> Result<PathBuf, OrbitError> {
    if let Some(workspace_path) = input.get("workspace_path").and_then(Value::as_str)
        && !workspace_path.trim().is_empty()
    {
        return Ok(PathBuf::from(workspace_path));
    }
    ctx.workspace_root
        .clone()
        .ok_or_else(|| OrbitError::InvalidInput("workspace_root is required".to_string()))
}

pub(super) fn write_err_to_orbit(error: orbit_knowledge::WriteError) -> OrbitError {
    serde_json::to_value(&error)
        .map(|value| OrbitError::Execution(value.to_string()))
        .unwrap_or_else(|_| OrbitError::Execution(format!("{error:?}")))
}

fn require_new_source(input: &Value) -> Result<String, OrbitError> {
    let value = input
        .get("new_source")
        .ok_or_else(|| OrbitError::InvalidInput("missing `new_source`".to_string()))?;
    let raw = value
        .as_str()
        .ok_or_else(|| OrbitError::InvalidInput("`new_source` must be a string".to_string()))?;
    if raw.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "`new_source` must not be empty".to_string(),
        ));
    }
    Ok(raw.to_string())
}

fn optional_str(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn parse_position_selector(position: Option<&str>) -> Result<Option<Selector>, OrbitError> {
    let Some(position) = position else {
        return Ok(None);
    };

    let selector = position.strip_prefix("after:").unwrap_or(position);
    selector
        .parse()
        .map(Some)
        .map_err(|error| OrbitError::InvalidInput(format!("invalid position: {error}")))
}

pub(super) fn resolve_knowledge_dir(
    ctx: &ToolContext,
    input: &Value,
) -> Result<PathBuf, OrbitError> {
    if let Some(raw) = input.get("knowledge_dir").and_then(Value::as_str)
        && !raw.trim().is_empty()
    {
        let path = Path::new(raw);
        if path.is_absolute() {
            return Ok(path.to_path_buf());
        }
        if let Some(workspace_root) = ctx.workspace_root.as_deref() {
            return Ok(workspace_root.join(path));
        }
        return Ok(path.to_path_buf());
    }

    let task_scope = super::task_scope(ctx);
    if let Some(orbit_root) = task_scope.orbit_root.as_deref() {
        return Ok(orbit_root.join("knowledge"));
    }

    let Some(workspace_root) = ctx.workspace_root.as_deref() else {
        return Err(OrbitError::InvalidInput(
            "`knowledge_dir` is required when `workspace_root` is unavailable".to_string(),
        ));
    };
    Ok(default_knowledge_dir(
        workspace_root,
        task_scope.orbit_root.as_deref(),
    ))
}

pub(super) fn task_graph_scope(ctx: &ToolContext) -> TaskGraphScope {
    let task_scope = super::task_scope(ctx);
    let owner = task_scope
        .task_id
        .clone()
        .or_else(|| ctx.agent_name.clone())
        .unwrap_or_else(|| "unknown".to_string());
    TaskGraphScope {
        orbit_root: task_scope.orbit_root,
        task_id: task_scope.task_id,
        owner,
    }
}
