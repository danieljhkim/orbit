use std::collections::{BTreeMap, BTreeSet};
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
}

/// Pre-spawn boundary guard for a linked-worktree provider invocation.
///
/// The registered primary checkout comes from the runtime's tool context; the
/// assigned checkout comes from the rendered `workspace_path`. The guard is
/// enabled only when both roots are distinct linked worktrees of the same Git
/// repository, preserving direct/non-worktree CLI activities.
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
    ) -> Result<Option<Self>, DispatchError> {
        let (Some(subprocess_cwd), Some(registered_primary_root)) =
            (subprocess_cwd, registered_primary_root)
        else {
            return Ok(None);
        };

        let Some(assigned_root) = git_top_level(subprocess_cwd)? else {
            return Ok(None);
        };
        let Some(primary_root) = git_top_level(registered_primary_root)? else {
            return Ok(None);
        };
        if assigned_root == primary_root {
            return Ok(None);
        }

        let Some(assigned_common_dir) = git_common_dir(&assigned_root)? else {
            return Ok(None);
        };
        let Some(primary_common_dir) = git_common_dir(&primary_root)? else {
            return Ok(None);
        };
        if assigned_common_dir != primary_common_dir {
            return Ok(None);
        }

        let requested_workspace_path = declared_workspace_path(input, task_ctx)
            .unwrap_or_else(|| subprocess_cwd.display().to_string());
        let requested_repo_root = input
            .get("repo_root")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(ToOwned::to_owned);
        let task_id = input
            .get("task_id")
            .and_then(Value::as_str)
            .or_else(|| {
                task_ctx
                    .and_then(|task| task.get("id"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("unknown")
            .to_string();

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
        &["diff", "--name-only", "-z", "HEAD", "--"],
    )?);
    dirty_paths.extend(untracked_paths);
    dirty_paths.sort();
    dirty_paths.dedup();

    Ok(GitWorktreeFingerprint {
        head,
        branch,
        index_sha256: sha256_identity("git-index-v1", &index),
        tracked_patch_sha256: sha256_identity("git-tracked-patch-v1", &tracked_patch),
        untracked_content,
        dirty_paths,
    })
}

fn changed_paths(
    root: &Path,
    before: &GitWorktreeFingerprint,
    after: &GitWorktreeFingerprint,
) -> Vec<String> {
    let mut paths = before
        .dirty_paths
        .iter()
        .chain(after.dirty_paths.iter())
        .cloned()
        .collect::<BTreeSet<_>>();

    if before.head != after.head
        && let Ok(bytes) = git_stdout_bytes(
            root,
            &["diff", "--name-only", "-z", &before.head, &after.head, "--"],
        )
    {
        paths.extend(nul_paths(&bytes));
    }
    if before.branch != after.branch && paths.is_empty() {
        paths.insert("<branch-ref>".to_string());
    }
    if before.index_sha256 != after.index_sha256 && paths.is_empty() {
        paths.insert("<index>".to_string());
    }
    if before.tracked_patch_sha256 != after.tracked_patch_sha256 && paths.is_empty() {
        paths.insert("<tracked-patch>".to_string());
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
