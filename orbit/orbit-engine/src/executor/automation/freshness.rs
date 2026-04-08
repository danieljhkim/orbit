use std::path::Path;

use orbit_types::OrbitError;

use super::git::{
    fetch_remote_base, git_command_success, git_output, resolve_worktree_start_point,
};

#[derive(Debug, Clone)]
pub(super) struct BranchFreshness {
    pub(super) base_ref: String,
    pub(super) head_ref: String,
    pub(super) commits_behind: u64,
    pub(super) commits_ahead: u64,
}

#[derive(Debug, Clone)]
pub(super) struct RebaseOutcome {
    pub(super) freshness: BranchFreshness,
    pub(super) rebased: bool,
}

/// Ensure `head` is not behind `base`, attempting a rebase onto `base` when it is.
///
/// Fast path: if `ensure_branch_fresh_against_base` returns `Ok`, we return
/// the freshness unchanged with `rebased = false`.
///
/// Recovery path: if the freshness check fails, we recompute the divergence
/// directly (NOT by parsing the error string) to determine whether the failure
/// was caused by the branch being behind. If so, we attempt
/// `git rebase <base_ref>`; on success, we re-check freshness and return
/// `rebased = true`. On conflict, we run `git rebase --abort` best-effort and
/// return the original error so the caller sees the semantically correct
/// "behind by N" failure. If the recomputed divergence shows the branch is
/// NOT actually behind, we propagate the original error unchanged because it
/// means the freshness check failed for a different reason.
pub(super) fn ensure_branch_rebased_onto_base(
    repo_root: &Path,
    head: &str,
    base: &str,
) -> Result<RebaseOutcome, OrbitError> {
    let original_error = match ensure_branch_fresh_against_base(repo_root, head, base) {
        Ok(freshness) => {
            return Ok(RebaseOutcome {
                freshness,
                rebased: false,
            });
        }
        Err(error) => error,
    };

    // Recompute divergence directly — do NOT parse the original error string.
    fetch_remote_base(repo_root, base);
    let base_ref = match resolve_worktree_start_point(repo_root, base) {
        Ok(value) => value,
        Err(_) => return Err(original_error),
    };
    let divergence = match git_output(
        repo_root,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base_ref}...{head}"),
        ],
    ) {
        Ok(value) => value,
        Err(_) => return Err(original_error),
    };
    let commits_behind: u64 = divergence
        .split_whitespace()
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);

    if commits_behind == 0 {
        // Freshness check failed for a reason other than being behind base.
        return Err(original_error);
    }

    // Attempt the rebase. `git_command_success` returns Ok(false) on non-zero
    // exit rather than mapping it to an Err, which is exactly what we want so
    // we can distinguish "rebase had conflicts" from the freshness-check error.
    let rebase_ok = git_command_success(repo_root, &["rebase", &base_ref]).unwrap_or(false);

    if !rebase_ok {
        // Best-effort abort to restore a clean worktree. Ignore errors from
        // the abort itself — the goal is to leave no rebase in progress.
        let _ = git_command_success(repo_root, &["rebase", "--abort"]);
        return Err(original_error);
    }

    let freshness = ensure_branch_fresh_against_base(repo_root, head, base)?;
    Ok(RebaseOutcome {
        freshness,
        rebased: true,
    })
}

pub(super) fn ensure_branch_fresh_against_base(
    repo_root: &Path,
    head: &str,
    base: &str,
) -> Result<BranchFreshness, OrbitError> {
    fetch_remote_base(repo_root, base);
    let base_ref = resolve_worktree_start_point(repo_root, base)?;
    let divergence = git_output(
        repo_root,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base_ref}...{head}"),
        ],
    )?;
    let mut parts = divergence.split_whitespace();
    let commits_behind = parse_divergence_count(parts.next(), "behind", base, head)?;
    let commits_ahead = parse_divergence_count(parts.next(), "ahead", base, head)?;
    if parts.next().is_some() {
        return Err(OrbitError::Execution(format!(
            "unexpected git divergence output while comparing '{head}' to '{base_ref}': {divergence}"
        )));
    }

    if commits_behind > 0 {
        return Err(OrbitError::Execution(format!(
            "task branch '{head}' is behind base '{base_ref}' by {commits_behind} commit(s); refresh the task branch before opening or merging the PR"
        )));
    }

    Ok(BranchFreshness {
        base_ref,
        head_ref: head.to_string(),
        commits_behind,
        commits_ahead,
    })
}

fn parse_divergence_count(
    value: Option<&str>,
    label: &str,
    base: &str,
    head: &str,
) -> Result<u64, OrbitError> {
    let raw = value.ok_or_else(|| {
        OrbitError::Execution(format!(
            "missing {label} divergence count while comparing '{head}' to '{base}'"
        ))
    })?;
    raw.parse::<u64>().map_err(|error| {
        OrbitError::Execution(format!(
            "invalid {label} divergence count '{raw}' while comparing '{head}' to '{base}': {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    fn run_git(repo_root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .status()
            .expect("run git");
        assert!(status.success(), "git {:?} failed", args);
    }

    fn git_stdout(repo_root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .expect("run git");
        assert!(output.status.success(), "git {:?} failed", args);
        String::from_utf8(output.stdout)
            .expect("utf8")
            .trim()
            .to_string()
    }

    fn init_repo() -> tempfile::TempDir {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo_root = tempdir.path();
        run_git(repo_root, &["init", "--initial-branch=main"]);
        run_git(repo_root, &["config", "user.name", "Orbit Tests"]);
        run_git(
            repo_root,
            &["config", "user.email", "orbit-tests@example.com"],
        );
        run_git(repo_root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo_root.join("a.txt"), "A\n").expect("write a.txt");
        run_git(repo_root, &["add", "a.txt"]);
        run_git(repo_root, &["commit", "-m", "A"]);
        tempdir
    }

    #[test]
    fn ensure_branch_rebased_onto_base_noop_when_already_fresh() {
        let repo = init_repo();
        let repo_root = repo.path();

        run_git(repo_root, &["checkout", "-b", "feature"]);
        std::fs::write(repo_root.join("x.txt"), "X\n").expect("write x.txt");
        run_git(repo_root, &["add", "x.txt"]);
        run_git(repo_root, &["commit", "-m", "X"]);

        let outcome = ensure_branch_rebased_onto_base(repo_root, "feature", "main")
            .expect("fresh branch should succeed");

        assert!(!outcome.rebased, "should not rebase a fresh branch");
        assert_eq!(outcome.freshness.commits_behind, 0);
        assert_eq!(outcome.freshness.commits_ahead, 1);
    }

    #[test]
    fn ensure_branch_rebased_onto_base_rebases_cleanly_when_behind() {
        let repo = init_repo();
        let repo_root = repo.path();

        // feature branches off A with commit X touching x.txt.
        run_git(repo_root, &["checkout", "-b", "feature"]);
        std::fs::write(repo_root.join("x.txt"), "X\n").expect("write x.txt");
        run_git(repo_root, &["add", "x.txt"]);
        run_git(repo_root, &["commit", "-m", "X"]);

        // main advances with B and C, both in non-conflicting files.
        run_git(repo_root, &["checkout", "main"]);
        std::fs::write(repo_root.join("b.txt"), "B\n").expect("write b.txt");
        run_git(repo_root, &["add", "b.txt"]);
        run_git(repo_root, &["commit", "-m", "B"]);
        std::fs::write(repo_root.join("c.txt"), "C\n").expect("write c.txt");
        run_git(repo_root, &["add", "c.txt"]);
        run_git(repo_root, &["commit", "-m", "C"]);

        run_git(repo_root, &["checkout", "feature"]);

        let outcome = ensure_branch_rebased_onto_base(repo_root, "feature", "main")
            .expect("clean rebase should succeed");

        assert!(outcome.rebased, "should have performed a rebase");
        assert_eq!(outcome.freshness.commits_behind, 0);
        assert_eq!(outcome.freshness.commits_ahead, 1);

        // main should now be an ancestor of feature.
        let ancestor_ok = Command::new("git")
            .args(["merge-base", "--is-ancestor", "main", "feature"])
            .current_dir(repo_root)
            .status()
            .expect("run git");
        assert!(
            ancestor_ok.success(),
            "main should be an ancestor of rebased feature"
        );

        // Feature tip is still the X commit.
        let tip_subject = git_stdout(repo_root, &["log", "-1", "--format=%s", "feature"]);
        assert_eq!(tip_subject, "X");
    }

    #[test]
    fn ensure_branch_rebased_onto_base_aborts_on_conflict_and_returns_original_error() {
        let repo = init_repo();
        let repo_root = repo.path();

        // Add conflict.txt = base on main.
        std::fs::write(repo_root.join("conflict.txt"), "base\n").expect("write conflict.txt");
        run_git(repo_root, &["add", "conflict.txt"]);
        run_git(repo_root, &["commit", "-m", "conflict base"]);

        // feature changes conflict.txt to feature-version.
        run_git(repo_root, &["checkout", "-b", "feature"]);
        std::fs::write(repo_root.join("conflict.txt"), "feature-version\n").expect("write feature");
        run_git(repo_root, &["add", "conflict.txt"]);
        run_git(repo_root, &["commit", "-m", "feature edit"]);

        // main changes the same line to main-version.
        run_git(repo_root, &["checkout", "main"]);
        std::fs::write(repo_root.join("conflict.txt"), "main-version\n").expect("write main");
        run_git(repo_root, &["add", "conflict.txt"]);
        run_git(repo_root, &["commit", "-m", "main edit"]);

        run_git(repo_root, &["checkout", "feature"]);
        let feature_sha_before = git_stdout(repo_root, &["rev-parse", "HEAD"]);

        let error = ensure_branch_rebased_onto_base(repo_root, "feature", "main")
            .expect_err("rebase with conflict should return error");
        let error_msg = error.to_string();
        assert!(
            error_msg.contains("is behind base"),
            "expected behind-base error, got: {error_msg}"
        );
        assert!(
            error_msg.contains("commit(s)"),
            "expected commit count in error, got: {error_msg}"
        );

        // HEAD should be back where it started.
        let feature_sha_after = git_stdout(repo_root, &["rev-parse", "HEAD"]);
        assert_eq!(
            feature_sha_before, feature_sha_after,
            "feature HEAD should be restored after aborted rebase"
        );

        // Worktree should be clean.
        let status = git_stdout(repo_root, &["status", "--porcelain"]);
        assert!(
            status.is_empty(),
            "expected clean worktree, got: {status:?}"
        );

        // No rebase-in-progress state.
        assert!(
            !repo_root.join(".git").join("rebase-merge").exists(),
            ".git/rebase-merge should not exist"
        );
        assert!(
            !repo_root.join(".git").join("rebase-apply").exists(),
            ".git/rebase-apply should not exist"
        );
    }
}
