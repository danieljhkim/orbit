use std::path::{Path, PathBuf};

use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::OrbitError;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BaseSyncMode {
    Local,
    Remote,
}

pub(super) fn base_sync_mode_from_input(input: &Value) -> Result<BaseSyncMode, OrbitError> {
    match input
        .as_object()
        .and_then(|map| map.get("base_sync"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None | Some("remote") => Ok(BaseSyncMode::Remote),
        Some("local") => Ok(BaseSyncMode::Local),
        Some(other) => Err(OrbitError::InvalidInput(format!(
            "input.base_sync must be 'local' or 'remote', got '{other}'"
        ))),
    }
}

pub(super) fn git_output_paths(
    current_dir: &Path,
    args: &[&str],
) -> Result<Vec<String>, OrbitError> {
    let raw = git_output_raw(current_dir, args)?;
    Ok(raw
        .split('\0')
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

pub(super) fn git_output(current_dir: &Path, args: &[&str]) -> Result<String, OrbitError> {
    Ok(git_output_raw(current_dir, args)?.trim().to_string())
}

pub(super) fn git_output_raw(current_dir: &Path, args: &[&str]) -> Result<String, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            current_dir: Some(current_dir.to_string_lossy().to_string()),
            timeout_ms: Some(30_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "git {} failed in '{}': {}",
            args.join(" "),
            current_dir.display(),
            result.stderr.trim()
        )));
    }

    Ok(result.stdout)
}

pub(super) fn git_success(current_dir: &Path, args: &[&str]) -> Result<(), OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            current_dir: Some(current_dir.to_string_lossy().to_string()),
            timeout_ms: Some(30_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "git {} failed in '{}': {}",
            args.join(" "),
            current_dir.display(),
            result.stderr.trim()
        )));
    }

    Ok(())
}

pub(super) fn git_command_success(current_dir: &Path, args: &[&str]) -> Result<bool, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            current_dir: Some(current_dir.to_string_lossy().to_string()),
            timeout_ms: Some(30_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;
    Ok(result.success)
}

pub(super) fn fetch_remote_base(repo_root: &Path, base: &str) {
    let _ = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: vec!["fetch".to_string(), "origin".to_string(), base.to_string()],
            current_dir: Some(repo_root.to_string_lossy().to_string()),
            timeout_ms: Some(60_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    );
}

pub(super) fn refresh_local_base_branch(repo_root: &Path, base: &str, sync_mode: BaseSyncMode) {
    match sync_mode {
        BaseSyncMode::Local => {}
        BaseSyncMode::Remote => {
            // Best-effort: if pull fails (e.g. no remote, offline, fresh branch),
            // we continue with whatever the local branch has. The push step will
            // catch actual divergence later.
            let _ = run_process(
                &ExecRequest {
                    program: "git".to_string(),
                    args: vec![
                        "pull".to_string(),
                        "--rebase".to_string(),
                        "origin".to_string(),
                        base.to_string(),
                    ],
                    current_dir: Some(repo_root.to_string_lossy().to_string()),
                    timeout_ms: Some(60_000),
                    stdin_mode: StdinMode::Null,
                    environment_mode: EnvironmentMode::Inherit,
                    debug: false,
                },
                &NoSandbox,
            );
        }
    }
}

pub(super) fn resolve_worktree_start_point(
    repo_root: &Path,
    base: &str,
) -> Result<String, OrbitError> {
    // Prefer the local branch: after refresh_local_base_branch it reflects
    // the best-known state (remote updates + any local commits not yet pushed).
    if git_command_success(
        repo_root,
        &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
    )? {
        return Ok(base.to_string());
    }

    let remote_base = format!("origin/{base}");
    if git_command_success(
        repo_root,
        &[
            "rev-parse",
            "--verify",
            &format!("{remote_base}^{{commit}}"),
        ],
    )? {
        return Ok(remote_base);
    }

    Err(OrbitError::Execution(format!(
        "unable to resolve base ref '{base}' for task worktree creation"
    )))
}

pub(super) fn sanitize_worktree_token(value: &str) -> Result<String, OrbitError> {
    let sanitized: String = value
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized
        .trim_matches(|c: char| c == '-' || c == '.')
        .to_string();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "run_id '{value}' sanitizes to an empty string"
        )));
    }
    Ok(trimmed)
}

pub(super) fn resolve_worktree_path_from_prefix(
    repo_root: &Path,
    prefix: &str,
    run_id: &str,
) -> Result<PathBuf, OrbitError> {
    let sanitized = sanitize_worktree_token(run_id)?;
    let dir_name = format!("{prefix}-{sanitized}");
    match std::env::var("ORBIT_WORKTREE_ROOT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(root) => {
            let repo_name = repo_root
                .file_name()
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    OrbitError::Execution(format!(
                        "cannot derive repository name from '{}'",
                        repo_root.display()
                    ))
                })?;
            Ok(PathBuf::from(root).join(repo_name).join(dir_name))
        }
        None => Ok(repo_root
            .join(".orbit")
            .join("state")
            .join("worktrees")
            .join(dir_name)),
    }
}
