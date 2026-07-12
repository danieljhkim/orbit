//! Managed pipeline-worktree collector for the shared `orbit gc` framework.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::{JobRun, JobRunState, OrbitError};
use serde::{Deserialize, Serialize};

use crate::OrbitRuntime;
use crate::command::job::gc_owner_permits_reclaim;
use crate::runtime::run_claim_guard;

use super::{
    GcCandidate, GcCollector, GcContext, GcItemError, GcMutation, GcPlan, GcRequest,
    GcRevalidation, GcScope, GcSkip, GcTarget, SystemGcClock, execute_gc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorktreeGcPolicy {
    pub success_retention_days: u64,
    pub failure_retention_days: u64,
}

impl WorktreeGcPolicy {
    pub fn from_runtime(runtime: &OrbitRuntime) -> Self {
        Self {
            success_retention_days: runtime.worktree_gc_success_retention_days(),
            failure_retention_days: runtime.worktree_gc_failure_retention_days(),
        }
    }
}

pub struct WorktreeGcCollector<'a> {
    runtime: &'a OrbitRuntime,
    policy: WorktreeGcPolicy,
    only_run_id: Option<String>,
}

impl<'a> WorktreeGcCollector<'a> {
    pub fn new(runtime: &'a OrbitRuntime, policy: WorktreeGcPolicy) -> Self {
        Self {
            runtime,
            policy,
            only_run_id: None,
        }
    }

    pub(crate) fn for_run(
        runtime: &'a OrbitRuntime,
        policy: WorktreeGcPolicy,
        run_id: &str,
    ) -> Self {
        Self {
            runtime,
            policy,
            only_run_id: Some(run_id.to_string()),
        }
    }

    fn worktree_root(context: &GcContext<'_>) -> PathBuf {
        context.scope.root().join("state").join("worktrees")
    }

    fn classify_path(
        &self,
        path: &Path,
        context: &GcContext<'_>,
    ) -> Result<Classified, OrbitError> {
        let id = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<non-utf8>")
            .to_string();
        let Some(run_id) = extract_run_id(path) else {
            return Ok(Classified::skip(
                id,
                "unknown_directory",
                "directory is not named for an Orbit job run",
            ));
        };
        if self
            .only_run_id
            .as_deref()
            .is_some_and(|only| only != run_id)
        {
            return Ok(Classified::Ignored);
        }
        if current_worktree_contains(path) {
            return Ok(Classified::skip(
                id,
                "current_worktree",
                "worktree contains the current process directory",
            ));
        }
        let Some(run) = self.runtime.stores().jobs().get_run(&run_id)? else {
            return Ok(Classified::skip(
                id,
                "unknown_owner",
                "no persisted run record proves ownership of this worktree",
            ));
        };
        let Some(git) = inspect_git_worktree(
            &self.runtime.paths().repo_root,
            path,
            self.runtime.workflow_base_branch(),
        )?
        else {
            return Ok(Classified::skip(
                id,
                "unknown_owner",
                "directory is not registered by Git as a managed worktree",
            ));
        };
        if git.dirty {
            return Ok(Classified::skip(
                id,
                "dirty_source",
                "worktree has tracked or untracked source-bearing changes",
            ));
        }
        if let Some(branch) = git.branch.as_deref() {
            if !git.merged {
                return Ok(Classified::skip(
                    id,
                    "unmerged_branch",
                    format!("branch `{branch}` is not merged into the configured base"),
                ));
            }
            if is_task_branch(branch) && !git.pushed {
                return Ok(Classified::skip(
                    id,
                    "unpushed_branch",
                    format!("task branch `{branch}` is not present on a remote"),
                ));
            }
        }
        match classify_run(
            &run,
            self.policy,
            context.clock.now(),
            self.interrupted_is_resumable(&run.run_id)?,
        ) {
            RunClassification::Skip { code, reason } => Ok(Classified::skip(id, code, reason)),
            RunClassification::Eligible { retention_evidence } => {
                // Fail closed on owner liveness even for terminal rows: a run
                // can persist a terminal state while its worker process is
                // still winding down (0-day success retention races finalize).
                if !gc_owner_permits_reclaim(&run) {
                    return Ok(Classified::skip(
                        id,
                        "live_or_inconclusive",
                        "recorded owner process is still alive or its liveness is inconclusive",
                    ));
                }
                let bytes = directory_bytes_no_follow(path).ok();
                let expected = ExpectedState {
                    run_id: run.run_id.clone(),
                    state: run.state,
                    finished_at: run.finished_at,
                    pid: run.pid,
                    pid_start_time: run.pid_start_time.clone(),
                    head: git.head,
                    branch: git.branch,
                };
                Ok(Classified::Candidate(GcCandidate {
                    id,
                    action: "git_worktree_remove".to_string(),
                    path: Some(path.to_path_buf()),
                    bytes,
                    ownership_evidence: format!(
                        "git-registered worktree owned by run {}",
                        run.run_id
                    ),
                    retention_evidence,
                    expected_state: serde_json::to_string(&expected)
                        .map_err(|error| OrbitError::Execution(error.to_string()))?,
                    allow_owned_symlink: false,
                }))
            }
        }
    }

    fn interrupted_is_resumable(&self, run_id: &str) -> Result<bool, OrbitError> {
        let Some(state) = self.runtime.read_run_state(run_id)? else {
            return Ok(false);
        };
        Ok(state.next_step_index > 0
            || !state.step_outputs.is_empty()
            || !state.pipeline_patches.is_empty()
            || !state.step_states.is_empty())
    }

    fn revalidate_candidate(
        &self,
        candidate: &GcCandidate,
        context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError> {
        let expected: ExpectedState =
            serde_json::from_str(&candidate.expected_state).map_err(|error| {
                OrbitError::Execution(format!("invalid frozen worktree state: {error}"))
            })?;
        let Some(path) = candidate.path.as_deref() else {
            return Ok(skip_revalidation(
                "missing_path",
                "candidate path is absent",
            ));
        };
        if current_worktree_contains(path) {
            return Ok(skip_revalidation(
                "current_worktree",
                "worktree became the current process directory",
            ));
        }
        let Some(run) = self.runtime.stores().jobs().get_run(&expected.run_id)? else {
            return Ok(skip_revalidation(
                "owner_changed",
                "owning run record disappeared after planning",
            ));
        };
        if run.state != expected.state
            || run.finished_at != expected.finished_at
            || run.pid != expected.pid
            || run.pid_start_time != expected.pid_start_time
        {
            return Ok(skip_revalidation(
                "owner_changed",
                "owning run state or owner identity changed after planning",
            ));
        }
        if !matches!(
            classify_run(
                &run,
                self.policy,
                context.clock.now(),
                self.interrupted_is_resumable(&run.run_id)?,
            ),
            RunClassification::Eligible { .. }
        ) {
            return Ok(skip_revalidation(
                "retention_changed",
                "owning run is no longer eligible",
            ));
        }
        // Fail closed if the owner became live (or its liveness is now
        // inconclusive) between planning and this final check — the process may
        // have re-acquired the tree even though the persisted row still reads
        // terminal.
        if !gc_owner_permits_reclaim(&run) {
            return Ok(skip_revalidation(
                "live_or_inconclusive",
                "recorded owner process is alive or its liveness is inconclusive at revalidation",
            ));
        }
        let Some(git) = inspect_git_worktree(
            &self.runtime.paths().repo_root,
            path,
            self.runtime.workflow_base_branch(),
        )?
        else {
            return Ok(skip_revalidation(
                "owner_changed",
                "Git worktree registration disappeared after planning",
            ));
        };
        if git.dirty || git.head != expected.head || git.branch != expected.branch {
            return Ok(skip_revalidation(
                "worktree_changed",
                "worktree content, branch, or HEAD changed after planning",
            ));
        }
        if git.branch.as_deref().is_some_and(|_| !git.merged)
            || git
                .branch
                .as_deref()
                .is_some_and(|branch| is_task_branch(branch) && !git.pushed)
        {
            return Ok(skip_revalidation(
                "branch_changed",
                "branch is no longer safely merged and pushed",
            ));
        }
        Ok(GcRevalidation::Ready)
    }
}

impl OrbitRuntime {
    /// Run the same classifier and apply primitive used by `orbit gc
    /// worktrees --apply`, restricted to the run that just terminalized.
    pub(crate) fn best_effort_collect_terminal_worktree(&self, run_id: &str) {
        let policy = WorktreeGcPolicy::from_runtime(self);
        let collector = WorktreeGcCollector::for_run(self, policy, run_id);
        let clock = SystemGcClock;
        let result = execute_gc(
            &collector,
            GcRequest {
                apply: true,
                scope: GcScope::Workspace {
                    workspace_id: None,
                    root: self.paths().orbit_dir.clone(),
                },
                retention_override: None,
                global_state_dir: &self.paths().global_dir.join("state"),
                clock: &clock,
            },
        );
        if let Err(error) = result {
            tracing::warn!(
                target: "orbit.core.worktree_gc",
                run_id,
                error = %error,
                "best-effort terminal worktree collection failed",
            );
        }
    }
}

impl GcCollector for WorktreeGcCollector<'_> {
    fn target(&self) -> GcTarget {
        GcTarget::Worktrees
    }

    fn plan(&self, context: &GcContext<'_>) -> Result<GcPlan, OrbitError> {
        if context.retention_override.is_some() {
            return Err(OrbitError::InvalidInput(
                "worktrees have separate success and failure retention classes; use the qualified worktree retention options"
                    .to_string(),
            ));
        }
        let root = Self::worktree_root(context);
        let mut plan = GcPlan::empty(GcTarget::Worktrees);
        plan.config_source = "workspace".to_string();
        if !root.exists() {
            return Ok(plan);
        }
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            let path = entry.path();
            plan.scanned = plan.scanned.saturating_add(1);
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                plan.skipped.push(GcSkip {
                    id: entry.file_name().to_string_lossy().into_owned(),
                    code: "unknown_directory".to_string(),
                    reason: "entry is not a plain directory".to_string(),
                });
                continue;
            }
            match self.classify_path(&path, context) {
                Ok(Classified::Candidate(candidate)) => {
                    plan.scanned_bytes = add_optional_bytes(plan.scanned_bytes, candidate.bytes);
                    plan.candidates.push(candidate);
                }
                Ok(Classified::Skip(skip)) => {
                    plan.scanned_bytes = add_optional_bytes(
                        plan.scanned_bytes,
                        directory_bytes_no_follow(&path).ok(),
                    );
                    plan.skipped.push(skip);
                }
                Ok(Classified::Ignored) => {
                    plan.scanned = plan.scanned.saturating_sub(1);
                }
                Err(error) => {
                    plan.scanned_bytes = None;
                    plan.errors.push(GcItemError {
                        id: entry.file_name().to_string_lossy().into_owned(),
                        phase: "scan".to_string(),
                        code: "inspection_failed".to_string(),
                        message: error.to_string(),
                    });
                }
            }
        }
        Ok(plan)
    }

    fn revalidate(
        &self,
        candidate: &GcCandidate,
        context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError> {
        self.revalidate_candidate(candidate, context)
    }

    fn apply(
        &self,
        candidate: &GcCandidate,
        context: &GcContext<'_>,
    ) -> Result<GcMutation, OrbitError> {
        let path = candidate.path.as_deref().ok_or_else(|| {
            OrbitError::Execution("worktree GC candidate has no path".to_string())
        })?;
        // Recover the owning run id from the frozen plan so we can take that
        // run's claim guard.
        let expected: ExpectedState =
            serde_json::from_str(&candidate.expected_state).map_err(|error| {
                OrbitError::Execution(format!("invalid frozen worktree state: {error}"))
            })?;

        // Atomic ownership guard (ORB-10182). Acquire the per-run claim guard —
        // the *same* advisory lock the run claim/start path
        // (`mark_run_running` / `claim_pending_run_owner` /
        // `take_over_running_run`) holds across its ownership transition — and
        // keep holding it while we (1) re-run the full owner state / identity /
        // liveness + Git revalidation and (2) `git worktree remove`. No store
        // write or fs sync intervenes. Lock ordering: `execute_gc` already holds
        // the host-global GC lock (ADR-0220); we take the per-run guard beneath
        // it, then mutate the filesystem — GC host lock → per-run guard →
        // filesystem, and never the global SQLite write lock across the git
        // operation. A concurrent claim contends on this same guard: if it
        // committed first, revalidation observes the live/changed owner and we
        // fail closed; if we hold first, the claimant blocks until removal
        // completes and then re-evaluates (its worktree setup recreates the
        // tree) rather than entering a removed worktree.
        let _run_guard =
            run_claim_guard::acquire(&self.runtime.paths().state_dir, &expected.run_id)?;
        match self.revalidate_candidate(candidate, context)? {
            GcRevalidation::Ready => {}
            GcRevalidation::Skip { code, reason } => {
                return Err(OrbitError::Execution(format!(
                    "worktree ownership changed between planning and removal ({code}): {reason}"
                )));
            }
        }
        let path_arg = path.to_string_lossy();
        git_checked(
            &self.runtime.paths().repo_root,
            &["worktree", "remove", path_arg.as_ref()],
        )?;
        git_checked(&self.runtime.paths().repo_root, &["worktree", "prune"])?;
        Ok(GcMutation {
            reclaimed_bytes: candidate.bytes,
        })
    }
}

#[derive(Debug)]
enum Classified {
    Candidate(GcCandidate),
    Skip(GcSkip),
    Ignored,
}

impl Classified {
    fn skip(id: String, code: &str, reason: impl Into<String>) -> Self {
        Self::Skip(GcSkip {
            id,
            code: code.to_string(),
            reason: reason.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RunClassification {
    Eligible { retention_evidence: String },
    Skip { code: &'static str, reason: String },
}

fn classify_run(
    run: &JobRun,
    policy: WorktreeGcPolicy,
    now: DateTime<Utc>,
    interrupted_is_resumable: bool,
) -> RunClassification {
    if matches!(
        run.state,
        JobRunState::Pending | JobRunState::Running | JobRunState::Retrying
    ) {
        return RunClassification::Skip {
            code: "live_or_inconclusive",
            reason: "run is active or owner liveness is inconclusive".to_string(),
        };
    }
    if run.state == JobRunState::Interrupted && interrupted_is_resumable {
        return RunClassification::Skip {
            code: "resumable_interrupted",
            reason: "interrupted run has persisted recovery checkpoints".to_string(),
        };
    }
    let Some(finished_at) = run.finished_at else {
        return RunClassification::Skip {
            code: "missing_retention_clock",
            reason: "run has no persisted terminal timestamp".to_string(),
        };
    };
    let retention_days = match run.state {
        JobRunState::Success | JobRunState::Cancelled => policy.success_retention_days,
        JobRunState::Failed | JobRunState::Timeout | JobRunState::Interrupted => {
            policy.failure_retention_days
        }
        JobRunState::Skipped => {
            return RunClassification::Skip {
                code: "non_terminal_state",
                reason: "skipped run is not a persisted terminal owner".to_string(),
            };
        }
        JobRunState::Pending | JobRunState::Running | JobRunState::Retrying => unreachable!(),
    };
    let eligible_at = finished_at + Duration::days(retention_days.min(i64::MAX as u64) as i64);
    if now < eligible_at {
        return RunClassification::Skip {
            code: "retained",
            reason: format!(
                "{} run retained until {}",
                run.state,
                eligible_at.to_rfc3339()
            ),
        };
    }
    RunClassification::Eligible {
        retention_evidence: format!(
            "state={}, finished_at={}, retention_days={retention_days}",
            run.state,
            finished_at.to_rfc3339()
        ),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ExpectedState {
    run_id: String,
    state: JobRunState,
    finished_at: Option<DateTime<Utc>>,
    /// Persisted owner PID and process-start-identity token captured at plan
    /// time. A concurrent claim / takeover rewrites these, so revalidation
    /// rejects a candidate whose owner identity changed after planning.
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    pid_start_time: Option<String>,
    head: String,
    branch: Option<String>,
}

struct GitInspection {
    head: String,
    branch: Option<String>,
    dirty: bool,
    merged: bool,
    pushed: bool,
}

fn inspect_git_worktree(
    repo_root: &Path,
    path: &Path,
    base_branch: &str,
) -> Result<Option<GitInspection>, OrbitError> {
    let listing = git_output(repo_root, &["worktree", "list", "--porcelain"])?;
    let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut registered = false;
    let mut branch = None;
    for block in listing.split("\n\n") {
        let mut block_path = None;
        let mut block_branch = None;
        for line in block.lines() {
            if let Some(value) = line.strip_prefix("worktree ") {
                block_path = Some(PathBuf::from(value));
            } else if let Some(value) = line.strip_prefix("branch refs/heads/") {
                block_branch = Some(value.to_string());
            }
        }
        if block_path
            .as_deref()
            .map(|value| value.canonicalize().unwrap_or_else(|_| value.to_path_buf()))
            .as_deref()
            == Some(target.as_path())
        {
            registered = true;
            branch = block_branch;
            break;
        }
    }
    if !registered {
        return Ok(None);
    }
    let head = git_output(path, &["rev-parse", "HEAD"])?;
    let dirty = !git_output(path, &["status", "--porcelain=v1", "--untracked-files=all"])?
        .trim()
        .is_empty();
    let merged = branch.as_deref().is_none_or(|_| {
        is_ancestor(repo_root, head.trim(), &format!("refs/heads/{base_branch}"))
            || is_ancestor(
                repo_root,
                head.trim(),
                &format!("refs/remotes/origin/{base_branch}"),
            )
    });
    let pushed = branch.as_deref().is_none_or(|_| {
        git_output(
            repo_root,
            &[
                "for-each-ref",
                "--contains",
                head.trim(),
                "--format=%(refname)",
                "refs/remotes",
            ],
        )
        .is_ok_and(|refs| !refs.trim().is_empty())
    });
    Ok(Some(GitInspection {
        head: head.trim().to_string(),
        branch,
        dirty,
        merged,
        pushed,
    }))
}

fn is_ancestor(repo_root: &Path, ancestor: &str, descendant: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn git_output(repo_root: &Path, args: &[&str]) -> Result<String, OrbitError> {
    let output = git(repo_root, args)?;
    if !output.status.success() {
        return Err(OrbitError::Execution(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout).map_err(|error| OrbitError::Execution(error.to_string()))
}

fn git_checked(repo_root: &Path, args: &[&str]) -> Result<(), OrbitError> {
    git_output(repo_root, args).map(|_| ())
}

fn git(repo_root: &Path, args: &[&str]) -> Result<Output, OrbitError> {
    Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .map_err(|error| OrbitError::Execution(format!("failed to execute git: {error}")))
}

fn current_worktree_contains(path: &Path) -> bool {
    let Ok(cwd) = std::env::current_dir() else {
        return true;
    };
    let cwd = cwd.canonicalize().unwrap_or(cwd);
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    path_contains_cwd(&path, &cwd)
}

fn path_contains_cwd(path: &Path, cwd: &Path) -> bool {
    cwd.starts_with(path)
}

fn is_task_branch(branch: &str) -> bool {
    branch.contains("/ORB-") || branch.starts_with("ORB-")
}

fn extract_run_id(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let index = name.find("jrun-")?;
    Some(name[index..].to_string())
}

fn directory_bytes_no_follow(path: &Path) -> Result<u64, OrbitError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        // Count the link object itself without following its target. Symlinks
        // inside a worktree are ordinary source entries and must not make the
        // whole byte estimate unknown.
        return Ok(metadata.len());
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut total = 0_u64;
    for entry in fs::read_dir(path)? {
        total = total.saturating_add(directory_bytes_no_follow(&entry?.path())?);
    }
    Ok(total)
}

fn add_optional_bytes(total: Option<u64>, value: Option<u64>) -> Option<u64> {
    Some(total?.saturating_add(value?))
}

fn skip_revalidation(code: &str, reason: &str) -> GcRevalidation {
    GcRevalidation::Skip {
        code: code.to_string(),
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_common::utility::process_identity::process_start_identity_token;
    use rusqlite::{Connection, params};
    use tempfile::TempDir;

    static CLOCK: SystemGcClock = SystemGcClock;

    fn run(state: JobRunState, finished_at: Option<DateTime<Utc>>) -> JobRun {
        let now = Utc::now();
        JobRun {
            run_id: "jrun-test".to_string(),
            job_id: "job".to_string(),
            attempt: 1,
            state,
            scheduled_at: now,
            started_at: Some(now),
            finished_at,
            duration_ms: None,
            created_at: now,
            pid: None,
            pid_start_time: None,
            input: None,
            retry_source_run_id: None,
            knowledge_metrics: None,
            resolved_crew: None,
            crew_model: None,
            steps: Vec::new(),
        }
    }

    #[test]
    fn active_and_resumable_runs_are_protected() {
        let now = Utc::now();
        let policy = WorktreeGcPolicy {
            success_retention_days: 0,
            failure_retention_days: 7,
        };
        assert!(matches!(
            classify_run(&run(JobRunState::Running, None), policy, now, false),
            RunClassification::Skip {
                code: "live_or_inconclusive",
                ..
            }
        ));
        assert!(matches!(
            classify_run(
                &run(JobRunState::Interrupted, Some(now - Duration::days(30))),
                policy,
                now,
                true,
            ),
            RunClassification::Skip {
                code: "resumable_interrupted",
                ..
            }
        ));
    }

    #[test]
    fn terminal_classes_use_separate_persisted_clocks() {
        let now = Utc::now();
        let policy = WorktreeGcPolicy {
            success_retention_days: 0,
            failure_retention_days: 7,
        };
        assert!(matches!(
            classify_run(&run(JobRunState::Success, Some(now)), policy, now, false),
            RunClassification::Eligible { .. }
        ));
        assert!(matches!(
            classify_run(
                &run(JobRunState::Failed, Some(now - Duration::days(1))),
                policy,
                now,
                false,
            ),
            RunClassification::Skip {
                code: "retained",
                ..
            }
        ));
        assert!(matches!(
            classify_run(
                &run(JobRunState::Failed, Some(now - Duration::days(8))),
                policy,
                now,
                false,
            ),
            RunClassification::Eligible { .. }
        ));
    }

    #[test]
    fn missing_terminal_clock_is_never_aged_from_the_filesystem() {
        let policy = WorktreeGcPolicy {
            success_retention_days: 0,
            failure_retention_days: 0,
        };
        assert!(matches!(
            classify_run(&run(JobRunState::Success, None), policy, Utc::now(), false),
            RunClassification::Skip {
                code: "missing_retention_clock",
                ..
            }
        ));
    }

    #[test]
    fn current_worktree_and_descendants_are_protected() {
        assert!(path_contains_cwd(
            Path::new("/tmp/worktree"),
            Path::new("/tmp/worktree/subdir")
        ));
        assert!(!path_contains_cwd(
            Path::new("/tmp/worktree"),
            Path::new("/tmp/other")
        ));
    }

    struct Fixture {
        _temp: TempDir,
        runtime: OrbitRuntime,
        global_state: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp root");
            let global = temp.path().join("global");
            let repo = temp.path().join("repo");
            fs::create_dir_all(&global).expect("global root");
            fs::create_dir_all(repo.join(".orbit")).expect("orbit root");
            git_ok(&repo, &["init", "-q", "-b", "agent-main"]);
            git_ok(&repo, &["config", "user.email", "gc@test.invalid"]);
            git_ok(&repo, &["config", "user.name", "gc test"]);
            fs::write(repo.join("README.md"), "seed").expect("seed file");
            git_ok(&repo, &["add", "README.md"]);
            git_ok(&repo, &["commit", "-q", "-m", "seed"]);
            fs::write(
                repo.join(".orbit/config.toml"),
                "[workflow]\nbase_branch = \"agent-main\"\n",
            )
            .expect("workspace config");
            let runtime =
                OrbitRuntime::from_roots(&global, &repo.join(".orbit")).expect("test runtime");
            let global_state = global.join("state");
            Self {
                _temp: temp,
                runtime,
                global_state,
            }
        }

        fn insert_run(&self, state: JobRunState) -> JobRun {
            let run = self
                .runtime
                .stores()
                .jobs()
                .insert_run("job", 1, Utc::now(), None, None)
                .expect("insert run");
            if state != JobRunState::Pending {
                self.runtime
                    .stores()
                    .jobs()
                    .mark_run_running(&run.run_id, Utc::now(), std::process::id())
                    .expect("mark running");
            }
            if !matches!(state, JobRunState::Pending | JobRunState::Running) {
                self.runtime
                    .stores()
                    .jobs()
                    .finalize_run(&run.run_id, state, Utc::now(), Some(0))
                    .expect("finalize run");
            }
            self.runtime
                .stores()
                .jobs()
                .get_run(&run.run_id)
                .expect("read run")
                .expect("stored run")
        }

        fn add_worktree(&self, run_id: &str) -> PathBuf {
            let path = self
                .runtime
                .paths()
                .worktrees_dir
                .join(format!("orbit-{run_id}"));
            fs::create_dir_all(path.parent().expect("worktree parent")).expect("worktree root");
            git_ok(
                &self.runtime.paths().repo_root,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    &format!("orbit/shared-{run_id}"),
                    path.to_str().expect("utf8 path"),
                    "agent-main",
                ],
            );
            path
        }

        fn request(&self, apply: bool) -> GcRequest<'_> {
            GcRequest {
                apply,
                scope: GcScope::Workspace {
                    workspace_id: Some("test".to_string()),
                    root: self.runtime.paths().orbit_dir.clone(),
                },
                retention_override: None,
                global_state_dir: &self.global_state,
                clock: &CLOCK,
            }
        }

        fn collector(&self) -> WorktreeGcCollector<'_> {
            WorktreeGcCollector::new(
                &self.runtime,
                WorktreeGcPolicy {
                    success_retention_days: 0,
                    failure_retention_days: 7,
                },
            )
        }
    }

    #[test]
    fn managed_gc_keeps_live_dirty_missing_and_unknown_and_is_idempotent() {
        let fixture = Fixture::new();
        let success = fixture.insert_run(JobRunState::Success);
        let success_path = fixture.add_worktree(&success.run_id);

        let live = fixture.insert_run(JobRunState::Running);
        let live_path = fixture.add_worktree(&live.run_id);

        let dirty = fixture.insert_run(JobRunState::Success);
        let dirty_path = fixture.add_worktree(&dirty.run_id);
        fs::write(dirty_path.join("untracked-source.rs"), "fn pending() {}").expect("dirty source");

        let missing_path = fixture.add_worktree("jrun-missing-record");

        let unmerged = fixture.insert_run(JobRunState::Success);
        let unmerged_path = fixture.add_worktree(&unmerged.run_id);
        fs::write(unmerged_path.join("committed.rs"), "fn committed() {}")
            .expect("unmerged source");
        git_ok(&unmerged_path, &["add", "committed.rs"]);
        git_ok(&unmerged_path, &["commit", "-q", "-m", "unmerged work"]);

        let unpushed = fixture.insert_run(JobRunState::Success);
        let unpushed_path = fixture.add_worktree(&unpushed.run_id);
        git_ok(
            &unpushed_path,
            &[
                "branch",
                "-m",
                &format!("orbit/ORB-99999-{}", unpushed.run_id),
            ],
        );
        let unknown_path = fixture
            .runtime
            .paths()
            .worktrees_dir
            .join("unknown-directory");
        fs::create_dir_all(&unknown_path).expect("unknown directory");

        let plan = execute_gc(&fixture.collector(), fixture.request(false)).expect("GC plan");
        let target = &plan.targets[0];
        assert_eq!(target.counts.eligible, 1);
        for code in [
            "live_or_inconclusive",
            "dirty_source",
            "unknown_owner",
            "unknown_directory",
            "unmerged_branch",
            "unpushed_branch",
        ] {
            assert!(
                target.skipped.iter().any(|skip| skip.code == code),
                "{code}"
            );
        }
        assert!(success_path.exists(), "planning is non-mutating");

        let first = execute_gc(&fixture.collector(), fixture.request(true)).expect("first apply");
        assert_eq!(first.targets[0].counts.reclaimed, 1);
        assert!(!success_path.exists());
        assert!(live_path.exists());
        assert!(dirty_path.exists());
        assert!(missing_path.exists());
        assert!(unmerged_path.exists());
        assert!(unpushed_path.exists());
        assert!(unknown_path.exists());

        let second = execute_gc(&fixture.collector(), fixture.request(true)).expect("second apply");
        assert_eq!(second.targets[0].counts.reclaimed, 0);
    }

    #[test]
    fn pending_run_becoming_live_is_never_a_candidate_or_cancelled() {
        let fixture = Fixture::new();
        let pending = fixture.insert_run(JobRunState::Pending);
        let path = fixture.add_worktree(&pending.run_id);
        let plan = execute_gc(&fixture.collector(), fixture.request(false)).expect("pending plan");
        assert_eq!(plan.targets[0].counts.eligible, 0);

        fixture
            .runtime
            .stores()
            .jobs()
            .mark_run_running(&pending.run_id, Utc::now(), std::process::id())
            .expect("pending run becomes live");
        let apply = execute_gc(&fixture.collector(), fixture.request(true)).expect("live apply");
        assert_eq!(apply.targets[0].counts.reclaimed, 0);
        assert!(path.exists());
        assert_eq!(
            fixture
                .runtime
                .stores()
                .jobs()
                .get_run(&pending.run_id)
                .expect("read live run")
                .expect("live run exists")
                .state,
            JobRunState::Running
        );
    }

    /// Reaps a spawned owner process even if the test panics.
    struct ChildGuard(std::process::Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn spawn_live_owner() -> ChildGuard {
        ChildGuard(
            Command::new("sleep")
                .arg("30")
                .spawn()
                .expect("spawn live owner process"),
        )
    }

    fn gc_context<'a>(scope: &'a GcScope) -> GcContext<'a> {
        GcContext {
            scope,
            retention_override: None,
            clock: &CLOCK,
        }
    }

    fn workspace_scope(fixture: &Fixture) -> GcScope {
        GcScope::Workspace {
            workspace_id: Some("test".to_string()),
            root: fixture.runtime.paths().orbit_dir.clone(),
        }
    }

    /// Rewrites only the persisted owner identity of a run, leaving its
    /// terminal state intact — models a reused/handed-off PID.
    fn set_run_owner(runtime: &OrbitRuntime, run_id: &str, pid: u32, token: Option<&str>) {
        let conn = Connection::open(runtime.global_root().join("orbit.db")).expect("open orbit db");
        conn.execute(
            "UPDATE job_runs SET pid = ?3, pid_start_time = ?4 \
             WHERE workspace_id = ?1 AND run_id = ?2",
            params![
                runtime.workspace_id().expect("workspace id"),
                run_id,
                pid as i64,
                token,
            ],
        )
        .expect("set run owner");
    }

    /// Transitions a run back to a live `running` owner — models a concurrent
    /// claim/resume that takes the run live between planning and mutation.
    fn claim_run_live(runtime: &OrbitRuntime, run_id: &str, pid: u32, token: Option<&str>) {
        let conn = Connection::open(runtime.global_root().join("orbit.db")).expect("open orbit db");
        conn.execute(
            "UPDATE job_runs SET state = 'running', started_at = ?3, finished_at = NULL, \
             pid = ?4, pid_start_time = ?5 \
             WHERE workspace_id = ?1 AND run_id = ?2",
            params![
                runtime.workspace_id().expect("workspace id"),
                run_id,
                Utc::now().to_rfc3339(),
                pid as i64,
                token,
            ],
        )
        .expect("claim run live");
    }

    #[test]
    fn terminal_run_with_live_owner_is_retained() {
        // A row can read terminal while its worker process is still alive
        // (e.g. 0-day success retention racing finalize). The persisted
        // PID + process-start-identity probe must fail closed even here.
        let fixture = Fixture::new();
        let run = fixture.insert_run(JobRunState::Success);
        let path = fixture.add_worktree(&run.run_id);

        let owner = spawn_live_owner();
        let pid = owner.0.id();
        let token = process_start_identity_token(pid);
        set_run_owner(&fixture.runtime, &run.run_id, pid, token.as_deref());

        let plan = execute_gc(&fixture.collector(), fixture.request(false)).expect("plan");
        let target = &plan.targets[0];
        assert_eq!(target.counts.eligible, 0, "live owner is never eligible");
        assert!(
            target
                .skipped
                .iter()
                .any(|skip| skip.code == "live_or_inconclusive"),
            "expected live_or_inconclusive skip, got {:?}",
            target.skipped
        );
        assert!(path.exists());
    }

    #[test]
    fn frozen_candidate_is_not_removed_when_owner_claimed_before_mutation() {
        // The deterministic revalidate-to-remove race: an eligible candidate is
        // frozen by planning, the owner is then claimed live between planning
        // and mutation, and the frozen candidate must be refused — the worktree
        // is neither removed nor the run cancelled/mutated by GC.
        let fixture = Fixture::new();
        let run = fixture.insert_run(JobRunState::Success);
        let path = fixture.add_worktree(&run.run_id);

        let scope = workspace_scope(&fixture);
        let context = gc_context(&scope);
        let collector = fixture.collector();

        // Plan freezes an eligible candidate against the terminal owner.
        let plan = collector.plan(&context).expect("frozen plan");
        assert_eq!(
            plan.candidates.len(),
            1,
            "candidate is eligible before claim"
        );
        let frozen = plan.candidates[0].clone();

        // A concurrent worker claims the run live after planning.
        let owner = spawn_live_owner();
        let pid = owner.0.id();
        let token = process_start_identity_token(pid);
        claim_run_live(&fixture.runtime, &run.run_id, pid, token.as_deref());

        // Revalidation refuses the frozen candidate...
        assert!(
            matches!(
                collector
                    .revalidate(&frozen, &context)
                    .expect("revalidate frozen candidate"),
                GcRevalidation::Skip { .. }
            ),
            "revalidation must refuse a claimed owner"
        );

        // ...and the apply-time atomic guard fails closed rather than removing.
        let applied = collector.apply(&frozen, &context);
        assert!(
            applied.is_err(),
            "apply must fail closed on a claimed owner, got {applied:?}"
        );

        // The worktree survives and GC left the run exactly as the claimer set
        // it — not cancelled, not removed.
        assert!(path.exists(), "worktree must survive the race");
        let reread = fixture
            .runtime
            .stores()
            .jobs()
            .get_run(&run.run_id)
            .expect("read run")
            .expect("run still exists");
        assert_eq!(reread.state, JobRunState::Running);
    }

    #[test]
    fn apply_blocks_on_a_claimant_held_run_guard_and_fails_closed() {
        // Two real actors contend for the *same* per-run claim guard that the
        // run claim/start path holds. A claimant grabs the guard first and
        // records a live owner (models a takeover). The collector's `apply`
        // then blocks acquiring that guard — proving the collector genuinely
        // takes it — and, once it wins the guard after the claimant releases,
        // its revalidation observes the live owner and fails closed. Ordering
        // is deterministic: the GC actor is only started after the claimant is
        // already holding the guard, so `apply` provably cannot revalidate
        // until the claim has committed and released.
        let fixture = Fixture::new();
        let run = fixture.insert_run(JobRunState::Success);
        let path = fixture.add_worktree(&run.run_id);

        let scope = workspace_scope(&fixture);
        let context = gc_context(&scope);
        let collector = fixture.collector();

        // Freeze an eligible candidate against the terminal, currently-dead
        // owner.
        let plan = collector.plan(&context).expect("frozen plan");
        assert_eq!(plan.candidates.len(), 1, "candidate eligible before claim");
        let frozen = plan.candidates[0].clone();

        let state_dir = fixture.runtime.paths().state_dir.clone();
        let run_id = run.run_id.clone();

        std::thread::scope(|threads| {
            // Claimant holds the guard first, exactly as the claim/start path
            // does around its ownership transition.
            let guard =
                run_claim_guard::acquire(&state_dir, &run_id).expect("claimant acquires guard");

            let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
            let collector_ref = &collector;
            let context_ref = &context;
            let frozen_ref = &frozen;
            let apply_handle = threads.spawn(move || {
                // Only begin the GC apply once the claimant holds the guard, so
                // `apply` must block on it rather than racing ahead.
                started_rx.recv().expect("gc actor start signal");
                collector_ref.apply(frozen_ref, context_ref)
            });
            started_tx.send(()).expect("signal gc actor");

            // Record a live owner while holding the guard — a concurrent
            // takeover that GC must observe once it finally acquires the guard.
            let owner = spawn_live_owner();
            let token = process_start_identity_token(owner.0.id());
            set_run_owner(&fixture.runtime, &run_id, owner.0.id(), token.as_deref());

            drop(guard); // release; the blocked GC apply now proceeds

            let applied = apply_handle.join().expect("gc apply thread");
            assert!(
                applied.is_err(),
                "apply must fail closed once the claim won the guard, got {applied:?}"
            );
        });

        assert!(path.exists(), "worktree survives because the claim won");
    }

    #[test]
    fn claimant_blocks_on_a_gc_held_run_guard_and_re_evaluates_after_removal() {
        // The mirror interleaving: GC wins the guard and removes the worktree
        // while holding it. A concurrent claimant contending for the same
        // per-run guard must block until removal completes and then re-evaluate
        // — it can never enter a worktree that GC is removing. GC's real
        // removal sequence (`git worktree remove` + `prune`) runs under the
        // guard here, mirroring `apply`; `apply_blocks_on_a_claimant_held_run_guard_and_fails_closed`
        // proves `apply` itself acquires this guard, so the two together cover
        // the collector holding it continuously across revalidation and removal.
        let fixture = Fixture::new();
        let run = fixture.insert_run(JobRunState::Success);
        let path = fixture.add_worktree(&run.run_id);
        assert!(path.exists());

        let state_dir = fixture.runtime.paths().state_dir.clone();
        let run_id = run.run_id.clone();
        let repo_root = fixture.runtime.paths().repo_root.clone();
        let path_for_claimant = path.clone();

        std::thread::scope(|threads| {
            // GC holds the guard for the whole removal.
            let guard = run_claim_guard::acquire(&state_dir, &run_id).expect("gc acquires guard");

            let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
            let state_dir_ref = &state_dir;
            let run_id_ref = &run_id;
            let claimant = threads.spawn(move || {
                started_rx.recv().expect("claimant start signal");
                // Contend for the same guard, then re-evaluate under it.
                let _held = run_claim_guard::acquire(state_dir_ref, run_id_ref)
                    .expect("claimant eventually acquires guard");
                path_for_claimant.exists()
            });
            started_tx.send(()).expect("signal claimant");

            // Remove the worktree under the guard, as `apply` does.
            git_ok(
                &repo_root,
                &["worktree", "remove", path.to_str().expect("utf8")],
            );
            git_ok(&repo_root, &["worktree", "prune"]);

            drop(guard); // release; the blocked claimant now proceeds

            let claimant_entered_live_tree = claimant.join().expect("claimant thread");
            assert!(
                !claimant_entered_live_tree,
                "claimant must block on the guard and re-evaluate after removal, never enter a removed tree"
            );
        });

        assert!(
            !path.exists(),
            "GC removed the worktree while holding the guard"
        );
    }

    fn git_ok(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("execute git");
        assert!(
            output.status.success(),
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
