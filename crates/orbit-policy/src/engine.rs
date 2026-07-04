use std::path::{Path, PathBuf};

use orbit_common::types::{FsOperation, OrbitError, PolicyDef};

use crate::evaluator;

/// `matched_rule` recorded when a resolved path escapes the workspace root.
const OUTSIDE_WORKSPACE: &str = "<outside workspace>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsPolicyEvaluation {
    pub profile: String,
    pub operation: FsOperation,
    pub path: String,
    pub allowed: bool,
    pub matched_rule: String,
}

#[derive(Debug, Clone)]
pub struct PolicyEngine {
    def: PolicyDef,
}

impl PolicyEngine {
    pub fn from_def(def: &PolicyDef) -> Result<Self, OrbitError> {
        def.validate()?;
        Ok(Self { def: def.clone() })
    }

    pub fn check(
        &self,
        profile: impl Into<String>,
        operation: FsOperation,
        path: impl Into<String>,
    ) -> Result<FsPolicyEvaluation, OrbitError> {
        let profile = profile.into();
        let path = path.into();
        let result = evaluator::evaluate(&self.def, &profile, operation, &path)?;
        Ok(FsPolicyEvaluation {
            profile,
            operation,
            path,
            allowed: result.allowed,
            matched_rule: result.matched_rule,
        })
    }

    /// Symlink-safe policy evaluation. [ORB-00418]
    ///
    /// Resolves `requested_path` — following symlinks — against
    /// `workspace_root` before matching rules, so a symlink inside an allowed
    /// subtree that points into a denied subtree is DENIED: rules match the
    /// *real* filesystem location, not the link path. Existing paths are
    /// canonicalized directly; for a not-yet-existing target (a write/create)
    /// the nearest existing ancestor is canonicalized (resolving symlinks in
    /// the ancestor) and the remaining components are rejoined.
    ///
    /// A resolved path that escapes the workspace root is denied outright.
    ///
    /// Unlike [`Self::check`] (which matches the caller-supplied path string as
    /// given), this method owns symlink resolution so the guarantee does not
    /// depend on every caller canonicalizing first. On non-sandboxed platforms
    /// (Linux) the policy check is the only enforcement layer, so resolving
    /// here is load-bearing. TOCTOU between this check and the actual
    /// filesystem access is an OS-level concern and out of scope.
    pub fn check_resolved(
        &self,
        workspace_root: &Path,
        profile: impl Into<String>,
        operation: FsOperation,
        requested_path: &Path,
    ) -> Result<FsPolicyEvaluation, OrbitError> {
        let profile = profile.into();
        let resolved = resolve_symlinks(requested_path)?;
        let canonical_root = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf());

        let Ok(relative) = resolved.strip_prefix(&canonical_root) else {
            // The resolved path (e.g. a symlink target) landed outside the
            // workspace root entirely — deny, don't fall through to rules that
            // only govern in-workspace paths.
            return Ok(FsPolicyEvaluation {
                profile,
                operation,
                path: resolved.to_string_lossy().replace('\\', "/"),
                allowed: false,
                matched_rule: OUTSIDE_WORKSPACE.to_string(),
            });
        };

        let relative = workspace_relative_string(relative);
        let result = evaluator::evaluate(&self.def, &profile, operation, &relative)?;
        Ok(FsPolicyEvaluation {
            profile,
            operation,
            path: relative,
            allowed: result.allowed,
            matched_rule: result.matched_rule,
        })
    }

    pub fn def(&self) -> &PolicyDef {
        &self.def
    }
}

/// Cap on symlink traversals while resolving a not-yet-existing tail. Mirrors
/// the kernel's `ELOOP` limit; a cycle of dangling links fails closed here
/// instead of looping.
const MAX_SYMLINK_FOLLOWS: usize = 40;

/// Resolve symlinks in `path`, tolerating a not-yet-existing tail (writes /
/// creates): canonicalize the nearest existing ancestor and rejoin the missing
/// components. [ORB-00418]
///
/// A *dangling* symlink reports `exists() == false` (`exists()` follows the
/// link), but an `O_CREAT` open through it creates the link's **target** — so a
/// dangling component is resolved via `read_link` rather than treated as a
/// missing tail. Without this, a dangling link inside an allowed subtree
/// pointing into a denied subtree would pass rule matching while the actual
/// write landed at the denied target.
///
/// Shared with `orbit-tools`' workspace-boundary check so the two enforcement
/// layers cannot drift.
pub fn resolve_symlinks(path: &Path) -> Result<PathBuf, OrbitError> {
    resolve_symlinks_bounded(path.to_path_buf(), 0)
}

fn resolve_symlinks_bounded(path: PathBuf, follows: usize) -> Result<PathBuf, OrbitError> {
    if follows > MAX_SYMLINK_FOLLOWS {
        return Err(OrbitError::InvalidInput(format!(
            "too many levels of symbolic links resolving {}",
            path.display()
        )));
    }
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|error| OrbitError::Io(format!("canonicalize {}: {error}", path.display())));
    }

    let mut missing = Vec::new();
    let mut ancestor = path.as_path();
    while !ancestor.exists() {
        // Dangling symlink: follow it (fail closed if the link is unreadable)
        // and restart resolution at the rejoined target.
        let is_symlink = ancestor
            .symlink_metadata()
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false);
        if is_symlink {
            let target = ancestor.read_link().map_err(|error| {
                OrbitError::Io(format!("read_link {}: {error}", ancestor.display()))
            })?;
            let mut rejoined = if target.is_absolute() {
                target
            } else {
                ancestor
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default()
                    .join(target)
            };
            for name in missing.iter().rev() {
                rejoined.push(name);
            }
            return resolve_symlinks_bounded(rejoined, follows + 1);
        }

        let name = ancestor.file_name().ok_or_else(|| {
            OrbitError::InvalidInput(format!("path has no file name: {}", path.display()))
        })?;
        missing.push(name.to_os_string());
        ancestor = ancestor.parent().ok_or_else(|| {
            OrbitError::InvalidInput(format!("path has no existing parent: {}", path.display()))
        })?;
    }

    let mut resolved = ancestor
        .canonicalize()
        .map_err(|error| OrbitError::Io(format!("canonicalize {}: {error}", ancestor.display())))?;
    for name in missing.iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

/// Render a workspace-relative path as the `./…` form the rule matcher expects
/// (mirrors the projection used by callers that pre-relativize).
fn workspace_relative_string(relative: &Path) -> String {
    let rendered = relative.to_string_lossy().replace('\\', "/");
    if rendered.is_empty() {
        ".".to_string()
    } else {
        format!("./{rendered}")
    }
}
