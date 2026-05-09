use std::path::Path;

use orbit_common::types::OrbitError;
use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::executor::automation) enum BaseSyncMode {
    Local,
    Remote,
}

pub(in crate::executor::automation) fn base_sync_mode_from_input(
    input: &Value,
) -> Result<BaseSyncMode, OrbitError> {
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

pub(crate) fn git_output_paths(
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

pub(crate) fn git_output(current_dir: &Path, args: &[&str]) -> Result<String, OrbitError> {
    Ok(git_output_raw(current_dir, args)?.trim().to_string())
}

pub(crate) fn git_output_raw(current_dir: &Path, args: &[&str]) -> Result<String, OrbitError> {
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

pub(crate) fn git_success(current_dir: &Path, args: &[&str]) -> Result<(), OrbitError> {
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

pub(crate) fn git_command_success(current_dir: &Path, args: &[&str]) -> Result<bool, OrbitError> {
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

pub(in crate::executor::automation) fn fetch_remote_base(
    repo_root: &Path,
    base: &str,
) -> Result<(), OrbitError> {
    let branch = normalize_base_branch(base)?;
    let result = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: vec![
                "fetch".to_string(),
                "origin".to_string(),
                format!("+refs/heads/{branch}:refs/remotes/origin/{branch}"),
            ],
            current_dir: Some(repo_root.to_string_lossy().to_string()),
            timeout_ms: Some(60_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "failed to fetch remote base 'origin/{branch}' in '{}': {}",
            repo_root.display(),
            result.stderr.trim()
        )));
    }

    Ok(())
}

pub(in crate::executor::automation) fn resolve_worktree_start_point(
    repo_root: &Path,
    base: &str,
    sync_mode: BaseSyncMode,
) -> Result<String, OrbitError> {
    let branch = normalize_base_branch(base)?;
    match sync_mode {
        BaseSyncMode::Local => resolve_local_base_ref(repo_root, &branch),
        BaseSyncMode::Remote => {
            fetch_remote_base(repo_root, &branch)?;
            resolve_remote_base_ref(repo_root, &branch)
        }
    }
}

pub(in crate::executor::automation) fn normalize_base_branch(
    base: &str,
) -> Result<String, OrbitError> {
    let branch = base
        .trim()
        .strip_prefix("origin/")
        .unwrap_or_else(|| base.trim())
        .trim();
    if branch.is_empty() {
        return Err(OrbitError::InvalidInput(
            "base branch must be a non-empty branch name".to_string(),
        ));
    }
    if branch.starts_with('-') {
        return Err(OrbitError::InvalidInput(format!(
            "base branch '{base}' must not start with '-'"
        )));
    }
    Ok(branch.to_string())
}

fn resolve_local_base_ref(repo_root: &Path, branch: &str) -> Result<String, OrbitError> {
    if git_command_success(
        repo_root,
        &["rev-parse", "--verify", &format!("{branch}^{{commit}}")],
    )? {
        return Ok(branch.to_string());
    }

    Err(OrbitError::Execution(format!(
        "unable to resolve local base ref '{branch}' for task worktree creation"
    )))
}

fn resolve_remote_base_ref(repo_root: &Path, branch: &str) -> Result<String, OrbitError> {
    let remote_base = format!("origin/{branch}");
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
        "unable to resolve fetched remote base ref '{remote_base}' for task worktree creation"
    )))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn remote_mode_fetches_origin_base_when_local_base_is_stale() {
        let temp = tempdir().unwrap();
        let remote = temp.path().join("remote.git");
        let seed = temp.path().join("seed");
        let local = temp.path().join("local");

        git(temp.path(), &["init", "--bare", remote.to_str().unwrap()]);
        init_repo(&seed, "agent-main");
        let local_v1 = commit_file(&seed, "base.txt", "v1");
        git(
            &seed,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&seed, &["push", "-u", "origin", "agent-main"]);

        git(
            temp.path(),
            &[
                "clone",
                "--branch",
                "agent-main",
                remote.to_str().unwrap(),
                local.to_str().unwrap(),
            ],
        );

        let remote_v2 = commit_file(&seed, "base.txt", "v2");
        git(&seed, &["push", "origin", "agent-main"]);

        assert_eq!(git(&local, &["rev-parse", "agent-main"]), local_v1);

        let start_point =
            resolve_worktree_start_point(&local, "agent-main", BaseSyncMode::Remote).unwrap();

        assert_eq!(start_point, "origin/agent-main");
        assert_eq!(git(&local, &["rev-parse", "agent-main"]), local_v1);
        assert_eq!(git(&local, &["rev-parse", "origin/agent-main"]), remote_v2);
    }

    #[test]
    fn local_mode_resolves_local_base_without_origin_remote() {
        let temp = tempdir().unwrap();
        let repo = temp.path().join("repo");
        init_repo(&repo, "agent-main");
        commit_file(&repo, "base.txt", "local-only");

        let start_point =
            resolve_worktree_start_point(&repo, "agent-main", BaseSyncMode::Local).unwrap();

        assert_eq!(start_point, "agent-main");
    }

    fn init_repo(path: &Path, branch: &str) {
        fs::create_dir_all(path).unwrap();
        git(path, &["init"]);
        git(path, &["checkout", "-b", branch]);
        git(path, &["config", "user.name", "Orbit Test"]);
        git(path, &["config", "user.email", "orbit-test@example.com"]);
    }

    fn commit_file(repo: &Path, file_name: &str, contents: &str) -> String {
        fs::write(repo.join(file_name), contents).unwrap();
        git(repo, &["add", file_name]);
        git(repo, &["commit", "-m", &format!("write {file_name}")]);
        git(repo, &["rev-parse", "HEAD"])
    }

    fn git(current_dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(current_dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed in {}:\nstdout: {}\nstderr: {}",
            args.join(" "),
            current_dir.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }
}
