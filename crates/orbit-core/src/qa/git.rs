//! Read-only git probes for qa-sweep [ORB-10039].
//!
//! The sweep validates the **live checkout at its local HEAD** and never
//! mutates the repo: no fetch, no checkout, no merge. Direct-push workspaces
//! on this host are the working copies commits land in (local commit → push),
//! so local HEAD is exactly the state that is live here; a `git fetch` would
//! not move HEAD or the working tree, i.e. it cannot change what the checks
//! observe, and background mutation of a live repo is out of scope.

use std::path::Path;
use std::process::Command;

use orbit_common::types::OrbitError;

/// Run a git command in `repo` and return trimmed stdout.
fn git(repo: &Path, args: &[&str]) -> Result<String, OrbitError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|error| {
            OrbitError::Execution(format!(
                "run git {} in {}: {error}",
                args[0],
                repo.display()
            ))
        })?;
    if !output.status.success() {
        return Err(OrbitError::Execution(format!(
            "git {} failed in {}: {}",
            args.join(" "),
            repo.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Current HEAD commit sha.
pub(crate) fn head_sha(repo: &Path) -> Result<String, OrbitError> {
    git(repo, &["rev-parse", "HEAD"])
}

/// Current branch name (`HEAD` when detached).
pub(crate) fn current_branch(repo: &Path) -> Result<String, OrbitError> {
    git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
}

/// `--oneline` summaries for `from..to`, newest first, capped at `limit`.
///
/// `Ok(None)` means the range could not be resolved (e.g. the watermark
/// commit was rewritten away by a force-push or gc) — the caller treats the
/// range as unknown and re-validates HEAD.
pub(crate) fn commit_range(
    repo: &Path,
    from: &str,
    to: &str,
    limit: usize,
) -> Result<Option<Vec<String>>, OrbitError> {
    let range = format!("{from}..{to}");
    match git(
        repo,
        &[
            "log",
            "--oneline",
            "--no-decorate",
            &format!("--max-count={limit}"),
            &range,
        ],
    ) {
        Ok(log) => Ok(Some(
            log.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
        )),
        // An unresolvable endpoint is expected after history rewrites; every
        // other failure (not a repo, missing binary) surfaced above.
        Err(_) if git(repo, &["cat-file", "-e", from]).is_err() => Ok(None),
        Err(error) => Err(error),
    }
}
