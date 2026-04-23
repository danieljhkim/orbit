use std::path::{Path, PathBuf};

use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::{Selector, TaskGraphScope, TaskGraphService, default_knowledge_dir};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeWriteTool;

impl Tool for OrbitKnowledgeWriteTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.write".to_string(),
            description: "Use when you need a graph-aware edit. Prefer over grep when text search cannot safely target the node to change.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "File or symbol selector.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "new_source".to_string(),
                    description: "Replacement source.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "position".to_string(),
                    description: "Insert after this selector.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "start_line".to_string(),
                    description: "File-write start line.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "end_line".to_string(),
                    description: "File-write end line.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
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
                ToolParam {
                    name: "knowledge_dir".to_string(),
                    description: "Override knowledge dir.".to_string(),
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
    let workspace_root = ctx
        .workspace_root
        .as_deref()
        .ok_or_else(|| OrbitError::InvalidInput("workspace_root is required".to_string()))?;
    let canonical_workspace_root = canonicalize_existing_dir(workspace_root, "workspace_root")?;

    if let Some(workspace_path) = input.get("workspace_path").and_then(Value::as_str)
        && !workspace_path.trim().is_empty()
    {
        let candidate = resolve_candidate_path(workspace_path, workspace_root);
        let canonical_candidate = canonicalize_existing_dir(&candidate, "workspace_path")?;
        ensure_path_within_boundary(
            &canonical_candidate,
            &canonical_workspace_root,
            "workspace_path",
            "workspace_root",
        )?;
        return Ok(canonical_candidate);
    }

    Ok(canonical_workspace_root)
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
    let task_scope = super::task_scope(ctx);
    let workspace_root = ctx.workspace_root.as_deref();
    let boundary_root = knowledge_boundary_root(workspace_root, task_scope.orbit_root.as_deref())?;

    if let Some(raw) = input.get("knowledge_dir").and_then(Value::as_str)
        && !raw.trim().is_empty()
    {
        let base = workspace_root
            .or(task_scope.orbit_root.as_deref())
            .ok_or_else(|| {
                OrbitError::InvalidInput(
                    "`knowledge_dir` requires workspace_root or orbit_root".to_string(),
                )
            })?;
        let candidate = resolve_candidate_path(raw, base);
        return canonicalize_knowledge_dir(&candidate, &boundary_root);
    }

    if let Some(orbit_root) = task_scope.orbit_root.as_deref() {
        return canonicalize_knowledge_dir(&orbit_root.join("knowledge"), &boundary_root);
    }

    let Some(workspace_root) = ctx.workspace_root.as_deref() else {
        return Err(OrbitError::InvalidInput(
            "`knowledge_dir` is required when `workspace_root` is unavailable".to_string(),
        ));
    };
    canonicalize_knowledge_dir(
        &default_knowledge_dir(workspace_root, task_scope.orbit_root.as_deref()),
        &boundary_root,
    )
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

fn resolve_candidate_path(raw: &str, base: &Path) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn knowledge_boundary_root(
    workspace_root: Option<&Path>,
    orbit_root: Option<&Path>,
) -> Result<PathBuf, OrbitError> {
    if let Some(orbit_root) = orbit_root {
        return canonicalize_with_missing_tail(orbit_root, "orbit_root");
    }

    let Some(workspace_root) = workspace_root else {
        return Err(OrbitError::InvalidInput(
            "`knowledge_dir` requires workspace_root or orbit_root".to_string(),
        ));
    };
    canonicalize_with_missing_tail(&workspace_root.join(".orbit"), "workspace data root")
}

fn canonicalize_knowledge_dir(path: &Path, boundary_root: &Path) -> Result<PathBuf, OrbitError> {
    let canonical = canonicalize_with_missing_tail(path, "knowledge_dir")?;
    ensure_path_within_boundary(&canonical, boundary_root, "knowledge_dir", "data root")?;
    if canonical.exists() && !canonical.is_dir() {
        return Err(OrbitError::InvalidInput(format!(
            "`knowledge_dir` must be a directory: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn canonicalize_existing_dir(path: &Path, label: &str) -> Result<PathBuf, OrbitError> {
    let canonical = canonicalize_with_missing_tail(path, label)?;
    if !canonical.is_dir() {
        return Err(OrbitError::InvalidInput(format!(
            "`{label}` must reference an existing directory: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn canonicalize_with_missing_tail(path: &Path, label: &str) -> Result<PathBuf, OrbitError> {
    if path.exists() {
        return path.canonicalize().map_err(|error| {
            OrbitError::InvalidInput(format!("failed to canonicalize `{label}`: {error}"))
        });
    }

    let mut missing_components = Vec::new();
    let mut existing_ancestor = path;
    while !existing_ancestor.exists() {
        let name = existing_ancestor
            .file_name()
            .ok_or_else(|| OrbitError::InvalidInput(format!("`{label}` has no file name")))?;
        missing_components.push(name.to_os_string());
        existing_ancestor = existing_ancestor.parent().ok_or_else(|| {
            OrbitError::InvalidInput(format!("`{label}` has no existing parent directory"))
        })?;
    }

    let mut canonical = existing_ancestor.canonicalize().map_err(|error| {
        OrbitError::InvalidInput(format!(
            "failed to canonicalize `{label}` parent directory: {error}"
        ))
    })?;
    for component in missing_components.iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

fn ensure_path_within_boundary(
    path: &Path,
    boundary: &Path,
    path_label: &str,
    boundary_label: &str,
) -> Result<(), OrbitError> {
    if !path.starts_with(boundary) {
        return Err(OrbitError::InvalidInput(format!(
            "`{path_label}` must stay within {boundary_label}: {}",
            path.display()
        )));
    }
    Ok(())
}
