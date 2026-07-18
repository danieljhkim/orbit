use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::dispatcher::DispatchError;

pub fn resolve_subprocess_cwd(
    input: &Value,
    task_ctx: Option<&Value>,
    tool_ctx_workspace_root: Option<&Path>,
) -> Result<Option<PathBuf>, DispatchError> {
    // A *declared* workspace_path must be usable. Fail closed if the key is
    // present but renders to a non-string, null, or empty value — a
    // worktree-based pipeline step whose `{{ ... workspace_path }}` failed to
    // render would otherwise fall through and silently run the agent in the
    // primary checkout, which is the ORB-10134 data-loss hazard. A genuinely
    // absent key (direct, non-worktree runs) still falls back to the tool
    // context's workspace_root below.
    if let Some(resolved) = resolve_declared_workspace_path(Some(input), "activity input")? {
        return Ok(Some(resolved));
    }
    if let Some(resolved) = resolve_declared_workspace_path(task_ctx, "task context")? {
        return Ok(Some(resolved));
    }

    let Some(path) = tool_ctx_workspace_root else {
        return Ok(None);
    };

    if path.is_dir() {
        return Ok(Some(canonicalize_dir(path)));
    }

    tracing::warn!(
        target: "orbit.engine.cli_runner",
        path = %path.display(),
        "tool_ctx workspace_root missing, child will inherit parent cwd"
    );
    Ok(None)
}

/// Resolve a `workspace_path` declared on `container`. Returns:
/// - `Ok(None)` when the key is absent or JSON `null` (caller falls back — the
///   agent envelope / task context always serialize an undeclared
///   workspace_path as `null`, so `null` means "not declared");
/// - `Ok(Some(dir))` when it is a valid, existing directory;
/// - `Err(..)` when it is present but a non-string/non-null value or empty, or
///   names a path that is not a writable directory (fail closed — never fall
///   back to the primary checkout).
fn resolve_declared_workspace_path(
    container: Option<&Value>,
    source: &str,
) -> Result<Option<PathBuf>, DispatchError> {
    let Some(value) = container.and_then(|container| container.get("workspace_path")) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(path) = value.as_str() else {
        return Err(DispatchError::CliInvocationFailed(format!(
            "{source} declared a non-string workspace_path ({value}); \
             refusing to fall back to the repository root"
        )));
    };
    validate_declared_workspace_path(path).map(Some)
}

fn validate_declared_workspace_path(path: &str) -> Result<PathBuf, DispatchError> {
    let path_buf = PathBuf::from(path);
    if path.trim().is_empty() || !path_buf.is_dir() {
        return Err(DispatchError::CliInvocationFailed(format!(
            "workspace path {} is not a writable directory",
            path_buf.display()
        )));
    }
    Ok(canonicalize_dir(&path_buf))
}

// pub(crate) widened for tests/ layout under ORB-00225; test reaches via exposed surface.
pub(crate) fn canonicalize_dir(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// A pipeline-declared linked-worktree assignment that has been validated
/// against the runtime's registered primary checkout before provider setup.
#[derive(Debug, Clone)]
pub(crate) struct DeclaredWorktreePair {
    requested_workspace_path: String,
    requested_repo_root: String,
    assigned_root: PathBuf,
    primary_root: PathBuf,
}

/// Validate the two independently rendered path fields used by task shipment.
///
/// `workspace_path` controls the child cwd while `repo_root` is included in the
/// agent contract. Treating either one as advisory lets a partially rendered
/// pipeline place the provider in one checkout while telling it to edit
/// another. A declared pair therefore has to name the same exact linked
/// worktree, and that worktree has to be a non-primary checkout of the
/// registered repository. Every failure is the same typed, non-retryable
/// boundary error used for a post-spawn escape.
pub(crate) fn validate_declared_worktree_pair(
    input: &Value,
    task_ctx: Option<&Value>,
    run_id: &str,
    provider: &str,
    registered_primary_root: Option<&Path>,
) -> Result<Option<DeclaredWorktreePair>, DispatchError> {
    let Some(repo_root_value) = input.get("repo_root") else {
        return Ok(None);
    };

    let workspace_path_value = input
        .get("workspace_path")
        .filter(|value| !value.is_null())
        .or_else(|| {
            task_ctx
                .and_then(|task| task.get("workspace_path"))
                .filter(|value| !value.is_null())
        });
    let requested_workspace_path = workspace_path_value.map(declared_value_text);
    let requested_repo_root = declared_value_text(repo_root_value);
    let mismatch = |reason: String, assigned_root: Option<&Path>, primary_root: Option<&Path>| {
        worktree_mismatch_error(
            input,
            task_ctx,
            run_id,
            provider,
            requested_workspace_path.as_deref(),
            Some(&requested_repo_root),
            assigned_root,
            primary_root,
            reason,
        )
    };

    let workspace_path = workspace_path_value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            mismatch(
                "declared repo_root requires a non-empty string workspace_path".to_string(),
                None,
                registered_primary_root,
            )
        })?;
    let repo_root = repo_root_value
        .as_str()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            mismatch(
                "declared repo_root must be a non-empty string".to_string(),
                None,
                registered_primary_root,
            )
        })?;

    let workspace_path = exact_canonical_dir(Path::new(workspace_path), "workspace_path")
        .map_err(|reason| mismatch(reason, None, registered_primary_root))?;
    let repo_root = exact_canonical_dir(Path::new(repo_root), "repo_root")
        .map_err(|reason| mismatch(reason, Some(&workspace_path), registered_primary_root))?;
    let assigned_root = required_git_top_level(&workspace_path, "workspace_path")
        .map_err(|reason| mismatch(reason, Some(&workspace_path), registered_primary_root))?;
    let repo_git_root = required_git_top_level(&repo_root, "repo_root")
        .map_err(|reason| mismatch(reason, Some(&assigned_root), registered_primary_root))?;

    if workspace_path != assigned_root {
        return Err(mismatch(
            format!(
                "workspace_path must name the exact Git checkout root '{}', not '{}'",
                assigned_root.display(),
                workspace_path.display()
            ),
            Some(&assigned_root),
            registered_primary_root,
        ));
    }
    if repo_root != repo_git_root {
        return Err(mismatch(
            format!(
                "repo_root must name the exact Git checkout root '{}', not '{}'",
                repo_git_root.display(),
                repo_root.display()
            ),
            Some(&assigned_root),
            registered_primary_root,
        ));
    }
    if assigned_root != repo_git_root {
        return Err(mismatch(
            "workspace_path and repo_root resolve to different Git checkouts".to_string(),
            Some(&assigned_root),
            registered_primary_root,
        ));
    }

    let primary_path = registered_primary_root.ok_or_else(|| {
        mismatch(
            "declared worktree pair requires a registered primary checkout".to_string(),
            Some(&assigned_root),
            None,
        )
    })?;
    let primary_path = exact_canonical_dir(primary_path, "registered primary root")
        .map_err(|reason| mismatch(reason, Some(&assigned_root), Some(primary_path)))?;
    let primary_root = required_git_top_level(&primary_path, "registered primary root")
        .map_err(|reason| mismatch(reason, Some(&assigned_root), Some(&primary_path)))?;
    if primary_path != primary_root {
        return Err(mismatch(
            format!(
                "registered primary root must name the exact Git checkout root '{}', not '{}'",
                primary_root.display(),
                primary_path.display()
            ),
            Some(&assigned_root),
            Some(&primary_root),
        ));
    }
    if assigned_root == primary_root {
        return Err(mismatch(
            "assigned checkout collapses to the registered primary checkout".to_string(),
            Some(&assigned_root),
            Some(&primary_root),
        ));
    }

    let assigned_common_dir = required_git_common_dir(&assigned_root, "assigned checkout")
        .map_err(|reason| mismatch(reason, Some(&assigned_root), Some(&primary_root)))?;
    let primary_common_dir = required_git_common_dir(&primary_root, "registered primary checkout")
        .map_err(|reason| mismatch(reason, Some(&assigned_root), Some(&primary_root)))?;
    if assigned_common_dir != primary_common_dir {
        return Err(mismatch(
            format!(
                "assigned and registered primary checkouts have different Git common dirs ('{}' != '{}')",
                assigned_common_dir.display(),
                primary_common_dir.display()
            ),
            Some(&assigned_root),
            Some(&primary_root),
        ));
    }

    Ok(Some(DeclaredWorktreePair {
        requested_workspace_path: requested_workspace_path
            .unwrap_or_else(|| assigned_root.display().to_string()),
        requested_repo_root,
        assigned_root,
        primary_root,
    }))
}

fn declared_value_text(value: &Value) -> String {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn exact_canonical_dir(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return Err(format!("{label} '{}' is not a directory", path.display()));
    }
    path.canonicalize()
        .map_err(|error| format!("cannot canonicalize {label} '{}': {error}", path.display()))
}

fn required_git_top_level(path: &Path, label: &str) -> Result<PathBuf, String> {
    git_top_level(path)
        .map_err(|error| format!("cannot resolve Git top level for {label}: {error}"))?
        .ok_or_else(|| format!("{label} '{}' is not a Git checkout", path.display()))
}

fn required_git_common_dir(path: &Path, label: &str) -> Result<PathBuf, String> {
    git_common_dir(path)
        .map_err(|error| format!("cannot resolve Git common dir for {label}: {error}"))?
        .ok_or_else(|| {
            format!(
                "cannot resolve Git common dir for {label} '{}'",
                path.display()
            )
        })
}

#[allow(clippy::too_many_arguments)]
fn worktree_mismatch_error(
    input: &Value,
    task_ctx: Option<&Value>,
    run_id: &str,
    provider: &str,
    requested_workspace_path: Option<&str>,
    requested_repo_root: Option<&str>,
    assigned_root: Option<&Path>,
    primary_root: Option<&Path>,
    reason: String,
) -> DispatchError {
    let diagnostic = json!({
        "code": "worktree_mismatch",
        "reason": reason,
        "task_id": task_id(input, task_ctx),
        "run_id": run_id,
        "provider": provider,
        "requested_workspace_path": requested_workspace_path,
        "requested_repo_root": requested_repo_root,
        "resolved_assigned_root": assigned_root,
        "registered_primary_root": primary_root,
        "automatic_reconciliation": false,
    });
    DispatchError::WorktreeIntegrity {
        code: "worktree_mismatch",
        diagnostic: diagnostic.to_string(),
    }
}

/// Exact, read-only identity of the Git state that an agent invocation can
/// observe or mutate. Large byte streams are represented by domain-separated
/// SHA-256 identities; untracked files retain one content identity per path so
/// diagnostics can name the primary-checkout delta without staging it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct GitWorktreeFingerprint {
    head: String,
    branch: Option<String>,
    index_sha256: String,
    tracked_patch_sha256: String,
    untracked_content: BTreeMap<String, String>,
    dirty_paths: Vec<String>,
    path_states: BTreeMap<String, GitPathState>,
}

/// Per-path Git identity. Optional identities distinguish an absent index
/// entry, an empty staged/unstaged delta, a deletion from the worktree, and an
/// untracked file without reading file contents into the diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct GitPathState {
    index_entry_sha256: Option<String>,
    staged_patch_sha256: Option<String>,
    worktree_patch_sha256: Option<String>,
    worktree_present: bool,
    untracked_content_sha256: Option<String>,
}

/// Pre-spawn boundary guard for a linked-worktree provider invocation.
///
/// The registered primary checkout comes from the runtime's tool context; the
/// assigned checkout comes from the rendered `workspace_path`. The guard is
/// enabled only for a validated declared worktree pair. A direct invocation
/// may bypass the guard only when its cwd and registered root are the same
/// checkout; distinct roots without the pair fail before spawn.
pub(crate) struct WorktreeBoundaryGuard {
    task_id: String,
    run_id: String,
    provider: String,
    requested_workspace_path: String,
    requested_repo_root: Option<String>,
    assigned_root: PathBuf,
    primary_root: PathBuf,
    assigned_before: GitWorktreeFingerprint,
    primary_before: GitWorktreeFingerprint,
}

impl WorktreeBoundaryGuard {
    pub(crate) fn capture(
        input: &Value,
        task_ctx: Option<&Value>,
        run_id: &str,
        provider: &str,
        subprocess_cwd: Option<&Path>,
        registered_primary_root: Option<&Path>,
        declared_pair: Option<&DeclaredWorktreePair>,
    ) -> Result<Option<Self>, DispatchError> {
        let Some(pair) = declared_pair else {
            let (Some(subprocess_cwd), Some(registered_primary_root)) =
                (subprocess_cwd, registered_primary_root)
            else {
                return Ok(None);
            };
            let assigned_path =
                exact_canonical_dir(subprocess_cwd, "subprocess cwd").map_err(|reason| {
                    worktree_mismatch_error(
                        input,
                        task_ctx,
                        run_id,
                        provider,
                        declared_workspace_path(input, task_ctx).as_deref(),
                        None,
                        Some(subprocess_cwd),
                        Some(registered_primary_root),
                        reason,
                    )
                })?;
            let primary_path =
                exact_canonical_dir(registered_primary_root, "registered primary root").map_err(
                    |reason| {
                        worktree_mismatch_error(
                            input,
                            task_ctx,
                            run_id,
                            provider,
                            declared_workspace_path(input, task_ctx).as_deref(),
                            None,
                            Some(&assigned_path),
                            Some(registered_primary_root),
                            reason,
                        )
                    },
                )?;
            if assigned_path == primary_path {
                return Ok(None);
            }

            let assigned_git_root = git_top_level(&assigned_path).map_err(|error| {
                worktree_mismatch_error(
                    input,
                    task_ctx,
                    run_id,
                    provider,
                    declared_workspace_path(input, task_ctx).as_deref(),
                    None,
                    Some(&assigned_path),
                    Some(&primary_path),
                    format!("cannot resolve subprocess Git checkout: {error}"),
                )
            })?;
            let primary_git_root = git_top_level(&primary_path).map_err(|error| {
                worktree_mismatch_error(
                    input,
                    task_ctx,
                    run_id,
                    provider,
                    declared_workspace_path(input, task_ctx).as_deref(),
                    None,
                    Some(&assigned_path),
                    Some(&primary_path),
                    format!("cannot resolve registered primary Git checkout: {error}"),
                )
            })?;
            if assigned_git_root.is_some() && assigned_git_root == primary_git_root {
                return Ok(None);
            }
            return Err(worktree_mismatch_error(
                input,
                task_ctx,
                run_id,
                provider,
                declared_workspace_path(input, task_ctx).as_deref(),
                None,
                assigned_git_root.as_deref().or(Some(&assigned_path)),
                primary_git_root.as_deref().or(Some(&primary_path)),
                "distinct checkouts require a declared, matching workspace_path/repo_root pair"
                    .to_string(),
            ));
        };

        let subprocess_cwd = subprocess_cwd.ok_or_else(|| {
            worktree_mismatch_error(
                input,
                task_ctx,
                run_id,
                provider,
                Some(&pair.requested_workspace_path),
                Some(&pair.requested_repo_root),
                Some(&pair.assigned_root),
                Some(&pair.primary_root),
                "validated worktree pair did not resolve a subprocess cwd".to_string(),
            )
        })?;
        let resolved_cwd =
            exact_canonical_dir(subprocess_cwd, "subprocess cwd").map_err(|reason| {
                worktree_mismatch_error(
                    input,
                    task_ctx,
                    run_id,
                    provider,
                    Some(&pair.requested_workspace_path),
                    Some(&pair.requested_repo_root),
                    Some(&pair.assigned_root),
                    Some(&pair.primary_root),
                    reason,
                )
            })?;
        if resolved_cwd != pair.assigned_root {
            return Err(worktree_mismatch_error(
                input,
                task_ctx,
                run_id,
                provider,
                Some(&pair.requested_workspace_path),
                Some(&pair.requested_repo_root),
                Some(&resolved_cwd),
                Some(&pair.primary_root),
                "resolved subprocess cwd differs from the validated assigned checkout".to_string(),
            ));
        }
        let registered_primary_root = registered_primary_root.ok_or_else(|| {
            worktree_mismatch_error(
                input,
                task_ctx,
                run_id,
                provider,
                Some(&pair.requested_workspace_path),
                Some(&pair.requested_repo_root),
                Some(&pair.assigned_root),
                None,
                "registered primary checkout disappeared after pair validation".to_string(),
            )
        })?;
        let resolved_primary =
            exact_canonical_dir(registered_primary_root, "registered primary root").map_err(
                |reason| {
                    worktree_mismatch_error(
                        input,
                        task_ctx,
                        run_id,
                        provider,
                        Some(&pair.requested_workspace_path),
                        Some(&pair.requested_repo_root),
                        Some(&pair.assigned_root),
                        Some(registered_primary_root),
                        reason,
                    )
                },
            )?;
        if resolved_primary != pair.primary_root {
            return Err(worktree_mismatch_error(
                input,
                task_ctx,
                run_id,
                provider,
                Some(&pair.requested_workspace_path),
                Some(&pair.requested_repo_root),
                Some(&pair.assigned_root),
                Some(&resolved_primary),
                "registered primary checkout changed after pair validation".to_string(),
            ));
        }

        let assigned_root = pair.assigned_root.clone();
        let primary_root = pair.primary_root.clone();
        let requested_workspace_path = pair.requested_workspace_path.clone();
        let requested_repo_root = Some(pair.requested_repo_root.clone());
        let task_id = task_id(input, task_ctx);

        Ok(Some(Self {
            task_id,
            run_id: run_id.to_string(),
            provider: provider.to_string(),
            requested_workspace_path,
            requested_repo_root,
            assigned_before: git_fingerprint(&assigned_root)?,
            primary_before: git_fingerprint(&primary_root)?,
            assigned_root,
            primary_root,
        }))
    }

    /// Compare both monitored checkouts after the provider reaches any
    /// terminal outcome. A primary delta always fails closed. When the
    /// assigned checkout changed too, attribution is ambiguous rather than an
    /// automatic escape/reconciliation claim.
    pub(crate) fn verify(self) -> Result<(), DispatchError> {
        let assigned_after = git_fingerprint(&self.assigned_root)?;
        let primary_after = git_fingerprint(&self.primary_root)?;
        if primary_after == self.primary_before {
            return Ok(());
        }

        let assigned_changed = assigned_after != self.assigned_before;
        let code = if assigned_changed {
            "worktree_integrity_ambiguous"
        } else {
            "worktree_escape"
        };
        let changed_paths = changed_paths(&self.primary_root, &self.primary_before, &primary_after);
        let diagnostic = json!({
            "code": code,
            "task_id": self.task_id,
            "run_id": self.run_id,
            "provider": self.provider,
            "requested_workspace_path": self.requested_workspace_path,
            "requested_repo_root": self.requested_repo_root,
            "resolved_assigned_root": self.assigned_root,
            "registered_primary_root": self.primary_root,
            "changed_paths": changed_paths,
            "assigned_changed": assigned_changed,
            "assigned_before": self.assigned_before,
            "assigned_after": assigned_after,
            "primary_before": self.primary_before,
            "primary_after": primary_after,
            "automatic_reconciliation": false,
        });
        Err(DispatchError::WorktreeIntegrity {
            code,
            diagnostic: diagnostic.to_string(),
        })
    }
}

fn declared_workspace_path(input: &Value, task_ctx: Option<&Value>) -> Option<String> {
    input
        .get("workspace_path")
        .and_then(Value::as_str)
        .or_else(|| {
            task_ctx
                .and_then(|task| task.get("workspace_path"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
}

fn task_id(input: &Value, task_ctx: Option<&Value>) -> String {
    input
        .get("task_id")
        .and_then(Value::as_str)
        .or_else(|| {
            task_ctx
                .and_then(|task| task.get("id"))
                .and_then(Value::as_str)
        })
        .unwrap_or("unknown")
        .to_string()
}

fn git_top_level(path: &Path) -> Result<Option<PathBuf>, DispatchError> {
    let output = git_output_raw(path, &["rev-parse", "--show-toplevel"])?;
    if !output.status.success() {
        return Ok(None);
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        return Ok(None);
    }
    Ok(Some(canonicalize_dir(Path::new(&root))))
}

fn git_common_dir(path: &Path) -> Result<Option<PathBuf>, DispatchError> {
    let output = git_output_raw(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    let common = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if common.is_empty() {
        return Ok(None);
    }
    Ok(Some(canonicalize_dir(Path::new(&common))))
}

fn git_fingerprint(root: &Path) -> Result<GitWorktreeFingerprint, DispatchError> {
    let head = git_stdout(root, &["rev-parse", "--verify", "HEAD"])?;
    let branch_output = git_output_raw(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let branch = branch_output
        .status
        .success()
        .then(|| {
            String::from_utf8_lossy(&branch_output.stdout)
                .trim()
                .to_string()
        })
        .filter(|branch| !branch.is_empty());

    let index = git_stdout_bytes(root, &["ls-files", "--stage", "-z", "--"])?;
    let tracked_patch = git_stdout_bytes(
        root,
        &[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "HEAD",
            "--",
        ],
    )?;
    let untracked_paths = nul_paths(&git_stdout_bytes(
        root,
        &["ls-files", "--others", "--exclude-standard", "-z", "--"],
    )?);
    let mut untracked_content = BTreeMap::new();
    for path in &untracked_paths {
        let identity = git_stdout(root, &["hash-object", "--no-filters", "--", path])?;
        untracked_content.insert(path.clone(), format!("git-blob:{identity}"));
    }

    let mut dirty_paths = nul_paths(&git_stdout_bytes(
        root,
        &["diff", "--name-only", "-z", "--no-renames", "HEAD", "--"],
    )?);
    dirty_paths.extend(untracked_paths);
    dirty_paths.sort();
    dirty_paths.dedup();

    let mut path_states = BTreeMap::new();
    for path in &dirty_paths {
        let index_entry = git_stdout_bytes(root, &["ls-files", "--stage", "-z", "--", path])?;
        let staged_patch = git_stdout_bytes(
            root,
            &[
                "diff",
                "--cached",
                "--binary",
                "--full-index",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "HEAD",
                "--",
                path,
            ],
        )?;
        let worktree_patch = git_stdout_bytes(
            root,
            &[
                "diff",
                "--binary",
                "--full-index",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--",
                path,
            ],
        )?;
        path_states.insert(
            path.clone(),
            GitPathState {
                index_entry_sha256: optional_sha256_identity("git-index-entry-v1", &index_entry),
                staged_patch_sha256: optional_sha256_identity(
                    "git-staged-path-patch-v1",
                    &staged_patch,
                ),
                worktree_patch_sha256: optional_sha256_identity(
                    "git-worktree-path-patch-v1",
                    &worktree_patch,
                ),
                worktree_present: fs::symlink_metadata(root.join(path)).is_ok(),
                untracked_content_sha256: untracked_content.get(path).cloned(),
            },
        );
    }

    Ok(GitWorktreeFingerprint {
        head,
        branch,
        index_sha256: sha256_identity("git-index-v1", &index),
        tracked_patch_sha256: sha256_identity("git-tracked-patch-v1", &tracked_patch),
        untracked_content,
        dirty_paths,
        path_states,
    })
}

fn changed_paths(
    root: &Path,
    before: &GitWorktreeFingerprint,
    after: &GitWorktreeFingerprint,
) -> Vec<String> {
    let all_state_paths = before
        .path_states
        .keys()
        .chain(after.path_states.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut paths = BTreeSet::new();
    for path in all_state_paths {
        if before.path_states.get(&path) != after.path_states.get(&path) {
            paths.insert(path);
        }
    }

    if before.head != after.head
        && let Ok(bytes) = git_stdout_bytes(
            root,
            &[
                "diff",
                "--name-only",
                "-z",
                "--no-renames",
                &before.head,
                &after.head,
                "--",
            ],
        )
    {
        paths.extend(nul_paths(&bytes));
    }
    if paths.is_empty() {
        if before.head != after.head {
            paths.insert("<head>".to_string());
        }
        if before.branch != after.branch {
            paths.insert("<branch-ref>".to_string());
        }
        if before.index_sha256 != after.index_sha256 {
            paths.insert("<index>".to_string());
        }
        if before.tracked_patch_sha256 != after.tracked_patch_sha256 {
            paths.insert("<tracked-patch>".to_string());
        }
    }
    paths.into_iter().collect()
}

fn nul_paths(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect()
}

fn sha256_identity(domain: &str, bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn optional_sha256_identity(domain: &str, bytes: &[u8]) -> Option<String> {
    (!bytes.is_empty()).then(|| sha256_identity(domain, bytes))
}

fn git_stdout(root: &Path, args: &[&str]) -> Result<String, DispatchError> {
    let bytes = git_stdout_bytes(root, args)?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}

fn git_stdout_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, DispatchError> {
    let output = git_output_raw(root, args)?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    Err(git_command_error(root, args, &output))
}

fn git_output_raw(root: &Path, args: &[&str]) -> Result<Output, DispatchError> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|error| {
            DispatchError::CliInvocationPermanent(format!(
                "snapshot Git state in '{}': {error}",
                root.display()
            ))
        })
}

fn git_command_error(root: &Path, args: &[&str], output: &Output) -> DispatchError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    DispatchError::CliInvocationPermanent(format!(
        "snapshot Git state in '{}' with `git {}` failed (status {}): {}",
        root.display(),
        args.join(" "),
        output.status,
        stderr.trim()
    ))
}
