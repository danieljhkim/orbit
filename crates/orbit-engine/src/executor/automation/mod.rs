mod duel;
mod input;
pub(crate) mod review;
mod task_update;
pub(crate) mod vcs;

use std::collections::HashMap;
use std::path::PathBuf;

use orbit_common::types::OrbitError;
use serde_json::Value;

// ---- retained internal actions (still referenced by duel/worker jobs) ----
const UPDATE_TASK_ACTION: &str = "update_task";
const RUN_PLANNING_DUEL_ACTION: &str = "run_planning_duel";

// ---- generic built-in automation actions ----
const GIT_COMMIT_ACTION: &str = "git_commit";
const GIT_REBASE_ACTION: &str = "git_rebase";
const PR_FAILURE_HANDOFF_ACTION: &str = "pr_failure_handoff";
const GIT_PUSH_ACTION: &str = "git_push";
const GIT_MERGE_ACTION: &str = "git_merge";
const WORKTREE_SETUP_ACTION: &str = "worktree_setup";
const WORKTREE_GC_ACTION: &str = "worktree_gc";
const PR_OPEN_ACTION: &str = "pr_open";
const PR_PREPARE_ACTION: &str = "pr_prepare";
const PR_PROMOTE_ACTION: &str = "pr_promote";

#[derive(Debug, Clone, Default)]
pub struct StateExecutionContext {
    pub run_id: Option<String>,
    pub step_index: Option<u32>,
    pub state_dir: Option<PathBuf>,
    pub agent: Option<String>,
    pub model: Option<String>,
}

pub fn execute_action<
    H: crate::context::DeterministicActionHost
        + crate::context::TaskHost
        + crate::context::EnvironmentHost
        + Sync
        + ?Sized,
>(
    host: &H,
    action: &str,
    input: &Value,
    debug: bool,
    _steps_outputs: &HashMap<String, Value>,
    state_context: Option<&StateExecutionContext>,
) -> Result<Value, OrbitError> {
    match action {
        // ---- retained internal actions ----
        UPDATE_TASK_ACTION => task_update::update_task(host, input, state_context),
        RUN_PLANNING_DUEL_ACTION => duel::run_planning_duel(host, input, debug),

        // ---- generic built-in actions ----
        GIT_COMMIT_ACTION => vcs::git_commit(host, input),
        GIT_REBASE_ACTION => vcs::rebase_pr_branch(host, input),
        PR_FAILURE_HANDOFF_ACTION => vcs::pr_failure_handoff(host, input),
        GIT_PUSH_ACTION => vcs::push_batch_changes(host, input),
        GIT_MERGE_ACTION => vcs::git_merge(host, input),
        WORKTREE_SETUP_ACTION => vcs::setup_worktree(host, input),
        WORKTREE_GC_ACTION => {
            let runs = host.list_job_runs_for_gc()?;
            let repo_root = host.repo_root()?;
            let older_than = input
                .get("older_than_hours")
                .and_then(Value::as_u64)
                .map(|hours| {
                    let hours = i64::try_from(hours).map_err(|_| {
                        OrbitError::InvalidInput("older_than_hours is too large".to_string())
                    })?;
                    chrono::Utc::now()
                        .checked_sub_signed(chrono::Duration::hours(hours))
                        .ok_or_else(|| {
                            OrbitError::InvalidInput("older_than_hours is too large".to_string())
                        })
                })
                .transpose()?;
            let result = vcs::collect_worktrees(
                std::path::Path::new(&repo_root),
                &runs,
                host,
                &vcs::WorktreeGcOptions {
                    delete: true,
                    run_id: input
                        .get("run_id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    older_than,
                },
            )?;
            serde_json::to_value(result).map_err(|error| {
                OrbitError::Execution(format!("failed to serialize worktree GC result: {error}"))
            })
        }
        PR_OPEN_ACTION => vcs::pr_open(host, input),
        PR_PREPARE_ACTION => vcs::prepare_pr_handoff(host, input),
        PR_PROMOTE_ACTION => vcs::pr_promote(host, input),

        other => Err(OrbitError::InvalidInput(format!(
            "unsupported automation action '{other}'"
        ))),
    }
}

#[cfg(test)]
mod tests;
