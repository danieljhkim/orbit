//! Git-history walker for task-ID attribution (T20260421-0528).
//!
//! Produces `CommitInfo` records from the commit DAG in topological, oldest-first
//! order. Each record carries parsed task IDs (regex `\[T\d{8}-\d{4}(?:-\d+)?\]`),
//! the commit's parent SHAs, the commit date, the subject line, and the per-file
//! diff hunks. Consumers attribute task IDs to graph nodes by mapping hunks to
//! extracted symbols in the tree at the commit.
//!
//! The walker shells out to `git` via `orbit_common::utility::git::run_git`.
//! There is no in-process git library dependency.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_common::utility::git::run_git;
use regex::Regex;

use crate::error::KnowledgeError;

/// Regex matching Orbit task-ID tags in commit messages, e.g. `[T20260421-0528]`
/// or `[T20260421-0528-2]`. The brackets are stripped before storage.
const TASK_ID_REGEX_STR: &str = r"\[T\d{8}-\d{4}(?:-\d+)?\]";

/// One commit's contribution to the history walk.
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub sha: String,
    /// Parent SHAs in the order git reports them. Length 1 for linear commits,
    /// length 2 for merges, length 0 for the root commit.
    pub parents: Vec<String>,
    pub date: DateTime<Utc>,
    /// First line of the commit message.
    pub summary: String,
    /// Parsed task IDs (without surrounding brackets), sorted and deduplicated.
    pub task_ids: Vec<String>,
    /// Per-file diff against the first parent (or tree vs. empty for root).
    pub files_changed: Vec<FileDiff>,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: PathBuf,
    pub change_kind: ChangeKind,
    /// Hunks in new-side coordinates. Empty for pure deletions.
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

/// A diff hunk in new-side line coordinates.
///
/// `new_start` is 1-indexed. `new_count == 0` means pure deletion (no new lines
/// remain at this position, but the surrounding symbol is still considered
/// touched by the hunk's `new_start` position).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
}

impl Hunk {
    /// Returns true if the hunk overlaps the given 1-indexed line range
    /// `[range_start, range_end]` (inclusive on both ends).
    pub fn touches_range(&self, range_start: u32, range_end: u32) -> bool {
        if self.new_count == 0 {
            // Pure deletion — we consider the insertion point as touching any
            // symbol spanning that line (or the line immediately following).
            let point = self.new_start.max(1);
            return range_start <= point && point <= range_end.saturating_add(1);
        }
        let hunk_end = self.new_start + self.new_count - 1;
        !(hunk_end < range_start || self.new_start > range_end)
    }
}

/// Walk the commit DAG from `from_exclusive..to_inclusive` (or full history when
/// `from_exclusive` is None), topological order, oldest-first.
pub fn walk_commits(
    repo: &Path,
    from_exclusive: Option<&str>,
    to_inclusive: &str,
) -> Result<Vec<CommitInfo>, KnowledgeError> {
    let range = match from_exclusive {
        Some(from) if !from.is_empty() => format!("{from}..{to_inclusive}"),
        _ => to_inclusive.to_string(),
    };

    let sha_output = run_git(
        repo,
        &["log", "--reverse", "--topo-order", "--format=%H", &range],
    )
    .map_err(|error| KnowledgeError::io(format!("git log failed: {error}")))?;
    if !sha_output.success {
        return Err(KnowledgeError::invalid_data(format!(
            "git log failed for range `{range}`: {}",
            sha_output.stderr.trim()
        )));
    }

    let shas: Vec<String> = sha_output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    let mut commits = Vec::with_capacity(shas.len());
    for sha in &shas {
        commits.push(commit_info(repo, sha)?);
    }
    Ok(commits)
}

/// Resolve HEAD to a commit SHA. Returns `None` when the repo has no commits.
pub fn resolve_head(repo: &Path) -> Result<Option<String>, KnowledgeError> {
    let output = run_git(repo, &["rev-parse", "--verify", "-q", "HEAD"])
        .map_err(|error| KnowledgeError::io(format!("git rev-parse failed: {error}")))?;
    if !output.success {
        return Ok(None);
    }
    let sha = output.stdout.trim();
    if sha.is_empty() {
        return Ok(None);
    }
    Ok(Some(sha.to_string()))
}

/// Count commits in `from_exclusive..to_inclusive`. Returns 0 when `from_exclusive`
/// equals `to_inclusive` or the range is empty.
pub fn count_commits(
    repo: &Path,
    from_exclusive: &str,
    to_inclusive: &str,
) -> Result<u64, KnowledgeError> {
    if from_exclusive == to_inclusive {
        return Ok(0);
    }
    let range = format!("{from_exclusive}..{to_inclusive}");
    let output = run_git(repo, &["rev-list", "--count", &range])
        .map_err(|error| KnowledgeError::io(format!("git rev-list --count failed: {error}")))?;
    if !output.success {
        return Ok(0);
    }
    output
        .stdout
        .trim()
        .parse::<u64>()
        .map_err(|error| KnowledgeError::invalid_data(format!("parse rev-list count: {error}")))
}

/// Fetch the merge-base of two commits. Returns `None` when no common ancestor
/// exists.
pub fn merge_base(repo: &Path, a: &str, b: &str) -> Result<Option<String>, KnowledgeError> {
    let output = run_git(repo, &["merge-base", a, b])
        .map_err(|error| KnowledgeError::io(format!("git merge-base failed: {error}")))?;
    if !output.success {
        return Ok(None);
    }
    let base = output.stdout.trim();
    if base.is_empty() {
        return Ok(None);
    }
    Ok(Some(base.to_string()))
}

/// Read a file's contents at a given commit. Returns `None` when the path does
/// not exist in that commit's tree.
pub fn show_file_at_commit(
    repo: &Path,
    sha: &str,
    rel_path: &Path,
) -> Result<Option<String>, KnowledgeError> {
    let path_str = rel_path.to_string_lossy().into_owned();
    let spec = format!("{sha}:{path_str}");
    let output = run_git(repo, &["show", &spec])
        .map_err(|error| KnowledgeError::io(format!("git show {spec} failed: {error}")))?;
    if !output.success {
        // `git show <sha>:<path>` fails when the path does not exist at that
        // revision. Treat as absent rather than fatal.
        return Ok(None);
    }
    Ok(Some(output.stdout))
}

/// Parse task IDs out of a commit message. Deduplicated, sorted lexicographically.
pub fn parse_task_ids(message: &str) -> Vec<String> {
    let regex = Regex::new(TASK_ID_REGEX_STR).expect("task-ID regex compiles");
    let mut ids: Vec<String> = Vec::new();
    for found in regex.find_iter(message) {
        let raw = found.as_str();
        // strip `[` and `]`
        ids.push(raw[1..raw.len() - 1].to_string());
    }
    ids.sort();
    ids.dedup();
    ids
}

fn commit_info(repo: &Path, sha: &str) -> Result<CommitInfo, KnowledgeError> {
    let meta_output = run_git(
        repo,
        &[
            "show",
            "--no-patch",
            "--format=%H%x00%P%x00%cI%x00%s%x00%B",
            sha,
        ],
    )
    .map_err(|error| KnowledgeError::io(format!("git show --no-patch failed: {error}")))?;
    if !meta_output.success {
        return Err(KnowledgeError::invalid_data(format!(
            "git show --no-patch failed for {sha}: {}",
            meta_output.stderr.trim()
        )));
    }

    let parts: Vec<&str> = meta_output.stdout.splitn(5, '\0').collect();
    if parts.len() < 5 {
        return Err(KnowledgeError::invalid_data(format!(
            "unexpected git show metadata format for {sha}"
        )));
    }

    let actual_sha = parts[0].to_string();
    let parents: Vec<String> = parts[1].split_whitespace().map(ToOwned::to_owned).collect();
    let date = DateTime::parse_from_rfc3339(parts[2].trim())
        .map_err(|error| {
            KnowledgeError::invalid_data(format!("invalid commit date for {sha}: {error}"))
        })?
        .with_timezone(&Utc);
    let summary = parts[3].to_string();
    let full_message = parts[4].trim_end_matches('\n').to_string();
    let task_ids = parse_task_ids(&full_message);

    let files_changed = files_changed_for_commit(repo, &actual_sha, &parents)?;

    Ok(CommitInfo {
        sha: actual_sha,
        parents,
        date,
        summary,
        task_ids,
        files_changed,
    })
}

/// Produce the file-level diff for a commit against its first parent. For the
/// root commit, yields every file as `Added`. For merges, uses `--first-parent`
/// and `-m` so `git show` emits a single diff.
fn files_changed_for_commit(
    repo: &Path,
    sha: &str,
    parents: &[String],
) -> Result<Vec<FileDiff>, KnowledgeError> {
    if parents.is_empty() {
        return root_commit_files(repo, sha);
    }

    let mut args: Vec<&str> = vec!["show", "--unified=0", "--no-renames", "--format="];
    if parents.len() > 1 {
        args.push("-m");
        args.push("--first-parent");
    }
    args.push(sha);

    let output = run_git(repo, &args).map_err(|error| {
        KnowledgeError::io(format!("git show --unified=0 failed for {sha}: {error}"))
    })?;
    if !output.success {
        return Err(KnowledgeError::invalid_data(format!(
            "git show --unified=0 failed for {sha}: {}",
            output.stderr.trim()
        )));
    }

    Ok(parse_unified_diff(&output.stdout))
}

fn root_commit_files(repo: &Path, sha: &str) -> Result<Vec<FileDiff>, KnowledgeError> {
    let output = run_git(repo, &["ls-tree", "-r", "--name-only", sha]).map_err(|error| {
        KnowledgeError::io(format!("git ls-tree failed for root {sha}: {error}"))
    })?;
    if !output.success {
        return Err(KnowledgeError::invalid_data(format!(
            "git ls-tree failed for root {sha}: {}",
            output.stderr.trim()
        )));
    }

    let files = output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| FileDiff {
            path: PathBuf::from(line),
            change_kind: ChangeKind::Added,
            hunks: Vec::new(),
        })
        .collect();

    Ok(files)
}

/// Parse unified diff output from `git show --unified=0`. Recognizes `diff --git`,
/// `new file mode`, `deleted file mode`, `+++`, `---`, and `@@` markers.
fn parse_unified_diff(raw: &str) -> Vec<FileDiff> {
    let mut diffs: Vec<FileDiff> = Vec::new();
    let mut current: Option<FileDiff> = None;
    let mut change_is_new = false;
    let mut change_is_deleted = false;

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(diff) = current.take() {
                diffs.push(diff);
            }
            let path = parse_diff_git_path(rest);
            current = Some(FileDiff {
                path,
                change_kind: ChangeKind::Modified,
                hunks: Vec::new(),
            });
            change_is_new = false;
            change_is_deleted = false;
            continue;
        }

        if line.starts_with("new file mode") {
            change_is_new = true;
            continue;
        }
        if line.starts_with("deleted file mode") {
            change_is_deleted = true;
            continue;
        }

        if let Some(diff) = current.as_mut() {
            if line.starts_with("+++ ") || line.starts_with("--- ") {
                if change_is_new {
                    diff.change_kind = ChangeKind::Added;
                } else if change_is_deleted {
                    diff.change_kind = ChangeKind::Deleted;
                }
                continue;
            }
            if let Some(hunk) = parse_hunk_header(line) {
                diff.hunks.push(hunk);
            }
        }
    }

    if let Some(diff) = current.take() {
        diffs.push(diff);
    }

    diffs
}

fn parse_diff_git_path(rest: &str) -> PathBuf {
    // `rest` looks like `a/path/to/file b/path/to/file`. Prefer the b-side
    // (post-change path); fall back to a-side for deletions.
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let b_side = tokens.iter().rev().find(|t| t.starts_with("b/"));
    if let Some(b) = b_side {
        return PathBuf::from(b.trim_start_matches("b/"));
    }
    let a_side = tokens.iter().find(|t| t.starts_with("a/"));
    if let Some(a) = a_side {
        return PathBuf::from(a.trim_start_matches("a/"));
    }
    PathBuf::from(rest.trim())
}

fn parse_hunk_header(line: &str) -> Option<Hunk> {
    // `@@ -<old_start>[,<old_count>] +<new_start>[,<new_count>] @@ ...`
    let rest = line.strip_prefix("@@ ")?;
    let end = rest.find(" @@")?;
    let spec = &rest[..end];
    let mut parts = spec.split_whitespace();
    let old_part = parts.next()?.strip_prefix('-')?;
    let new_part = parts.next()?.strip_prefix('+')?;
    let (old_start, old_count) = parse_range(old_part)?;
    let (new_start, new_count) = parse_range(new_part)?;
    Some(Hunk {
        old_start,
        old_count,
        new_start,
        new_count,
    })
}

fn parse_range(raw: &str) -> Option<(u32, u32)> {
    match raw.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((raw.parse().ok()?, 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_task_ids_extracts_valid_tags() {
        let msg = "[T20260421-0528] add task_ids\n\nCo-authored: foo";
        assert_eq!(parse_task_ids(msg), vec!["T20260421-0528".to_string()]);
    }

    #[test]
    fn parse_task_ids_handles_amended_suffix() {
        let msg = "[T20260421-0528-2] incremental fix";
        assert_eq!(parse_task_ids(msg), vec!["T20260421-0528-2".to_string()]);
    }

    #[test]
    fn parse_task_ids_deduplicates_and_sorts() {
        let msg = "[T20260421-0528] first\n[T20260421-0342] second\n[T20260421-0528] dup";
        let got = parse_task_ids(msg);
        assert_eq!(
            got,
            vec!["T20260421-0342".to_string(), "T20260421-0528".to_string()]
        );
    }

    #[test]
    fn parse_task_ids_ignores_malformed_tags() {
        let msg = "[T1234] wrong shape\n[T20260421-0528] right";
        assert_eq!(parse_task_ids(msg), vec!["T20260421-0528".to_string()]);
    }

    #[test]
    fn parse_task_ids_empty_on_no_tags() {
        assert!(parse_task_ids("merge pull request #42").is_empty());
    }

    #[test]
    fn hunk_touches_range_overlap() {
        let hunk = Hunk {
            old_start: 1,
            old_count: 1,
            new_start: 10,
            new_count: 5,
        };
        assert!(hunk.touches_range(12, 15));
        assert!(hunk.touches_range(5, 11));
        assert!(!hunk.touches_range(1, 9));
        assert!(!hunk.touches_range(15, 20));
    }

    #[test]
    fn hunk_touches_range_pure_deletion_at_boundary() {
        let hunk = Hunk {
            old_start: 10,
            old_count: 5,
            new_start: 10,
            new_count: 0,
        };
        // The deletion "point" lands between new-file lines 9 and 10, so a
        // range that contains line 10 (or ends at line 9 and is adjacent) is
        // considered touched. Ranges entirely above or below are not.
        assert!(hunk.touches_range(8, 10));
        assert!(hunk.touches_range(9, 9));
        assert!(hunk.touches_range(10, 10));
        assert!(!hunk.touches_range(11, 20));
        assert!(!hunk.touches_range(1, 7));
    }

    #[test]
    fn parse_hunk_header_with_counts() {
        let line = "@@ -10,5 +20,3 @@ fn foo";
        let hunk = parse_hunk_header(line).unwrap();
        assert_eq!(hunk.old_start, 10);
        assert_eq!(hunk.old_count, 5);
        assert_eq!(hunk.new_start, 20);
        assert_eq!(hunk.new_count, 3);
    }

    #[test]
    fn parse_hunk_header_single_line() {
        let line = "@@ -10 +20 @@";
        let hunk = parse_hunk_header(line).unwrap();
        assert_eq!(hunk.old_count, 1);
        assert_eq!(hunk.new_count, 1);
    }

    #[test]
    fn parse_hunk_header_rejects_malformed() {
        assert!(parse_hunk_header("not a hunk").is_none());
        assert!(parse_hunk_header("@@ missing @@").is_none());
    }

    #[test]
    fn parse_diff_git_path_prefers_b_side() {
        let path = parse_diff_git_path("a/foo/bar.rs b/foo/bar.rs");
        assert_eq!(path, std::path::PathBuf::from("foo/bar.rs"));
    }

    #[test]
    fn parse_unified_diff_captures_file_and_hunks() {
        let raw = "\
diff --git a/src/foo.rs b/src/foo.rs
index abcdef..123456 100644
--- a/src/foo.rs
+++ b/src/foo.rs
@@ -5,2 +5,3 @@ fn foo
@@ -20 +21,2 @@ fn bar
";
        let diffs = parse_unified_diff(raw);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, std::path::PathBuf::from("src/foo.rs"));
        assert_eq!(diffs[0].change_kind, ChangeKind::Modified);
        assert_eq!(diffs[0].hunks.len(), 2);
        assert_eq!(diffs[0].hunks[0].new_start, 5);
        assert_eq!(diffs[0].hunks[0].new_count, 3);
    }

    #[test]
    fn parse_unified_diff_detects_new_file() {
        let raw = "\
diff --git a/new.rs b/new.rs
new file mode 100644
--- /dev/null
+++ b/new.rs
@@ -0,0 +1,5 @@
";
        let diffs = parse_unified_diff(raw);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].change_kind, ChangeKind::Added);
    }

    #[test]
    fn parse_unified_diff_detects_deleted_file() {
        let raw = "\
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
--- a/gone.rs
+++ /dev/null
@@ -1,5 +0,0 @@
";
        let diffs = parse_unified_diff(raw);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].change_kind, ChangeKind::Deleted);
    }
}
