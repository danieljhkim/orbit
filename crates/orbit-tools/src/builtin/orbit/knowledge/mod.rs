#[allow(dead_code)]
mod add;
#[allow(dead_code)]
mod delete;
#[allow(dead_code)]
mod move_;
#[allow(dead_code)]
mod write;

use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use orbit_knowledge::commands::{GraphCommandContext, TaskGraphScope, default_knowledge_dir};
pub(super) use orbit_knowledge::knowledge_error_to_orbit;
use serde_json::Value;

use crate::ToolContext;

pub(super) fn has_explicit_knowledge_dir(input: &Value) -> bool {
    input
        .get("knowledge_dir")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
}

pub(super) fn command_context(
    ctx: &ToolContext,
    input: &Value,
) -> Result<GraphCommandContext, OrbitError> {
    let knowledge_dir = resolve_knowledge_dir(ctx, input)?;
    let explicit_ref = super::optional_string(input, "ref")?;
    Ok(GraphCommandContext {
        knowledge_dir,
        workspace_root: ctx.workspace_root.clone(),
        explicit_ref,
        explicit_knowledge_dir: has_explicit_knowledge_dir(input),
        task_scope: task_graph_scope(ctx),
    })
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
