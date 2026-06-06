pub mod copy;
pub mod create;
pub mod delete;
pub mod ls;
pub mod mkdir;
pub mod move_file;
pub mod patch;
pub mod read;
pub mod write;

use std::path::{Path, PathBuf};

use orbit_common::tracing;
use orbit_common::types::{FsOperation, OrbitError};

use crate::{FsCallEvent, FsCallEventKind, ToolContext, ToolRegistry};

#[derive(Debug, Clone)]
pub(crate) struct FsPolicyAllowance {
    pub(crate) profile: String,
    pub(crate) op: FsOperation,
    pub(crate) path: String,
    pub(crate) matched_rule: String,
}

pub fn register(registry: &mut ToolRegistry) {
    registry.register(read::FsReadTool);
    registry.register(delete::FsDeleteTool);
}

#[cfg(test)]
mod tests;

/// Checks that `path` resolves inside the context workspace root.
///
/// Symlink escapes are blocked because the path is canonicalized before the check.
/// For paths that do not yet exist (e.g. `fs.write` creating a new file), the
/// nearest existing ancestor is canonicalized and the remaining components are
/// appended before the check.
///
/// Returns `Err(PolicyDenied)` when no workspace root is set (fail-closed) or
/// when the canonical path is outside the root. Returns `Ok` only when the
/// canonical path is inside the workspace root.
pub(crate) fn check_workspace_boundary(
    ctx: &ToolContext,
    path: &Path,
) -> Result<PathBuf, OrbitError> {
    let workspace_root = match &ctx.workspace_root {
        Some(root) => root,
        None => {
            return Err(OrbitError::PolicyDenied(
                "workspace_root is not set; filesystem access denied".to_string(),
            ));
        }
    };

    let canonical = canonicalize_with_missing_tail(path)?;

    // Canonicalize the workspace root so symlinks (e.g. /var -> /private/var on
    // macOS) don't cause false negatives when comparing against the canonical path.
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.clone());

    if !canonical.starts_with(&canonical_root) {
        return Err(OrbitError::PolicyDenied(format!(
            "path is outside workspace: {}",
            canonical.display()
        )));
    }

    Ok(canonical)
}

fn canonicalize_with_missing_tail(path: &Path) -> Result<PathBuf, OrbitError> {
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|e| OrbitError::Io(format!("failed to canonicalize path: {e}")));
    }

    let mut missing_components = Vec::new();
    let mut existing_ancestor = path;
    while !existing_ancestor.exists() {
        let name = existing_ancestor
            .file_name()
            .ok_or_else(|| OrbitError::InvalidInput("path has no file name".to_string()))?;
        missing_components.push(name.to_os_string());
        existing_ancestor = existing_ancestor.parent().ok_or_else(|| {
            OrbitError::InvalidInput("path has no existing parent directory".to_string())
        })?;
    }

    let mut canonical = existing_ancestor
        .canonicalize()
        .map_err(|e| OrbitError::Io(format!("failed to canonicalize parent directory: {e}")))?;
    for component in missing_components.iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

pub(crate) fn check_file_lock(
    _ctx: &ToolContext,
    _canonical_path: &Path,
) -> Result<(), OrbitError> {
    // File-level locking removed; graph-level locking is handled by
    // the shared lock store at .orbit/knowledge/graph_locks.json.
    Ok(())
}

pub(crate) fn check_read_policy(
    ctx: &ToolContext,
    canonical_path: &Path,
) -> Result<Option<FsPolicyAllowance>, OrbitError> {
    enforce_fs_policy(ctx, canonical_path, FsOperation::Read)
}

pub(crate) fn check_modify_policy(
    ctx: &ToolContext,
    canonical_path: &Path,
) -> Result<Option<FsPolicyAllowance>, OrbitError> {
    enforce_fs_policy(ctx, canonical_path, FsOperation::Modify)
}

pub(crate) fn emit_success(
    ctx: &ToolContext,
    allowance: Option<&FsPolicyAllowance>,
) -> Result<(), OrbitError> {
    let Some(allowance) = allowance else {
        return Ok(());
    };
    emit_fs_event(ctx, FsCallEventKind::Result, allowance, true)
}

fn enforce_fs_policy(
    ctx: &ToolContext,
    canonical_path: &Path,
    op: FsOperation,
) -> Result<Option<FsPolicyAllowance>, OrbitError> {
    let Some(profile) = ctx.fs_profile.as_deref() else {
        return Ok(None);
    };
    let Some(policy_engine) = ctx.policy_engine.as_ref() else {
        return Ok(None);
    };

    let path = workspace_relative_path(ctx, canonical_path)?;
    let evaluation = policy_engine.check(profile.to_string(), op, path.clone())?;
    let allowance = FsPolicyAllowance {
        profile: evaluation.profile,
        op: evaluation.operation,
        path,
        matched_rule: evaluation.matched_rule,
    };

    if evaluation.allowed {
        emit_fs_event(ctx, FsCallEventKind::Request, &allowance, true)?;
        return Ok(Some(allowance));
    }

    emit_fs_event(ctx, FsCallEventKind::Denied, &allowance, false)?;
    let tool = format!("fs.{}", allowance.op.as_str());
    tracing::warn!(
        target: "orbit.policy.deny",
        tool = tool.as_str(),
        path = allowance.path.as_str(),
        profile = allowance.profile.as_str(),
        matched_rule = allowance.matched_rule.as_str(),
    );
    Err(OrbitError::PolicyDenied(format!(
        "fs.{} denied for `{}` under fsProfile `{}` (matched rule `{}`)",
        allowance.op.as_str(),
        allowance.path,
        allowance.profile,
        allowance.matched_rule,
    )))
}

fn emit_fs_event(
    ctx: &ToolContext,
    kind: FsCallEventKind,
    allowance: &FsPolicyAllowance,
    allowed: bool,
) -> Result<(), OrbitError> {
    let Some(logger) = ctx.fs_audit.as_ref() else {
        return Ok(());
    };

    logger.emit(FsCallEvent {
        kind,
        profile: allowance.profile.clone(),
        op: allowance.op.as_str().to_string(),
        path: allowance.path.clone(),
        allowed,
        matched_rule: allowance.matched_rule.clone(),
    })
}

fn workspace_relative_path(ctx: &ToolContext, canonical_path: &Path) -> Result<String, OrbitError> {
    let workspace_root = ctx.workspace_root.as_ref().ok_or_else(|| {
        OrbitError::PolicyDenied("workspace_root is not set; filesystem access denied".to_string())
    })?;
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.clone());
    let relative = canonical_path.strip_prefix(&canonical_root).map_err(|_| {
        OrbitError::PolicyDenied(format!(
            "path is outside workspace: {}",
            canonical_path.display()
        ))
    })?;

    let rendered = relative.to_string_lossy().replace('\\', "/");
    if rendered.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(format!("./{rendered}"))
    }
}
