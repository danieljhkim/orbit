mod batch;
mod command;
mod duel;
mod input;
pub(crate) mod review;
mod task_update;
pub(crate) mod vcs;

use std::collections::HashMap;
use std::path::PathBuf;

use orbit_common::types::{Activity, InvocationTrace, JobRunState, OrbitError};
use orbit_store::state_io;
use serde::Deserialize;
use serde_json::Value;

use super::ActivityExecutor;
use super::helpers::validate_activity_output_schema;
use crate::context::{ACTIVITY_EXECUTION_FAILED, AttemptOutcome, ExecutionContext, ExecutorHost};

// ---- retained internal actions (still referenced by duel/worker jobs) ----
const UPDATE_TASK_ACTION: &str = "update_task";
const RUN_PARALLEL_TASK_PIPELINE_ACTION: &str = "run_parallel_task_pipeline";
const SELECT_DUEL_TASK_ACTION: &str = "select_duel_task";
const SELECT_DUEL_ROLES_ACTION: &str = "select_duel_roles";
const RECORD_DUEL_SCORES_ACTION: &str = "record_duel_scores";
const RUN_PLANNING_DUEL_ACTION: &str = "run_planning_duel";

// ---- generic built-in automation actions ----
const GIT_COMMIT_ACTION: &str = "git_commit";
const GIT_PUSH_ACTION: &str = "git_push";
const GIT_PULL_ACTION: &str = "git_pull";
const GIT_MERGE_ACTION: &str = "git_merge";
const WORKTREE_SETUP_ACTION: &str = "worktree_setup";
const WORKTREE_CLEANUP_ACTION: &str = "worktree_cleanup";
const PR_OPEN_ACTION: &str = "pr_open";
const PR_SYNC_REVIEWS_ACTION: &str = "pr_sync_reviews";
const CHECK_TASK_VALUE_ACTION: &str = "check_task_value";
const DISPATCH_BATCH_ACTION: &str = "dispatch_batch";
const RUN_COMMAND_ACTION: &str = "run_command";

#[derive(Debug, Clone, Deserialize)]
struct AutomationSpec {
    action: String,
}

pub struct AutomationExecutor;

#[derive(Debug, Clone, Default)]
pub struct StateExecutionContext {
    pub run_id: Option<String>,
    pub step_index: Option<u32>,
    pub state_dir: Option<PathBuf>,
}

impl ActivityExecutor for AutomationExecutor {
    fn spec_type(&self) -> &str {
        "automation"
    }

    fn execute(&self, host: ExecutorHost<'_>, execution: &ExecutionContext) -> AttemptOutcome {
        let automation_host = host.automation();
        match execute(
            &automation_host,
            &execution.activity,
            &execution.input,
            execution.debug,
            &execution.steps_outputs,
            Some(&StateExecutionContext {
                run_id: execution.run_id.clone(),
                step_index: execution.step_index,
                state_dir: execution.state_dir.clone(),
            }),
        ) {
            Ok(result) => {
                if let Err(err) = validate_activity_output_schema(&execution.activity, &result) {
                    return AttemptOutcome {
                        exit_code: Some(0),
                        response_json: Some(result),
                        ..AttemptOutcome::failed(ACTIVITY_EXECUTION_FAILED, err.to_string())
                    };
                }
                if let (Some(state_dir), Some(step_index)) =
                    (execution.state_dir.as_deref(), execution.step_index)
                    && let Err(error) = state_io::write_step_output(state_dir, step_index, &result)
                {
                    return AttemptOutcome {
                        exit_code: Some(1),
                        response_json: Some(result),
                        ..AttemptOutcome::failed(
                            ACTIVITY_EXECUTION_FAILED,
                            format!("failed to persist automation step output: {error}"),
                        )
                    };
                }
                AttemptOutcome {
                    state: JobRunState::Success,
                    exit_code: Some(0),
                    duration_ms: None,
                    invocation_trace: InvocationTrace::default(),
                    response_json: Some(result),
                    error_code: None,
                    error_message: None,
                    protocol_violation: false,
                    retry_count: 0,
                }
            }
            Err(err) => AttemptOutcome::failed(ACTIVITY_EXECUTION_FAILED, err.to_string()),
        }
    }
}

/// Shared test utilities for automation sub-modules.
pub fn execute<
    H: crate::context::RuntimeHost
        + crate::context::TaskHost
        + crate::context::EnvironmentHost
        + Sync
        + ?Sized,
>(
    host: &H,
    activity: &Activity,
    input: &Value,
    debug: bool,
    steps_outputs: &HashMap<String, Value>,
    state_context: Option<&StateExecutionContext>,
) -> Result<Value, OrbitError> {
    let spec: AutomationSpec =
        serde_json::from_value(activity.spec_config.clone()).map_err(|error| {
            OrbitError::InvalidInput(format!("invalid automation spec_config: {error}"))
        })?;

    execute_action(
        host,
        &spec.action,
        input,
        debug,
        steps_outputs,
        state_context,
    )
}

pub fn execute_action<
    H: crate::context::RuntimeHost
        + crate::context::TaskHost
        + crate::context::EnvironmentHost
        + Sync
        + ?Sized,
>(
    host: &H,
    action: &str,
    input: &Value,
    debug: bool,
    steps_outputs: &HashMap<String, Value>,
    state_context: Option<&StateExecutionContext>,
) -> Result<Value, OrbitError> {
    match action {
        // ---- retained internal actions ----
        UPDATE_TASK_ACTION => task_update::update_task(host, input),
        RUN_PARALLEL_TASK_PIPELINE_ACTION => batch::run_parallel_task_pipeline(host, input, debug),
        SELECT_DUEL_TASK_ACTION => duel::select_duel_task(host, input),
        SELECT_DUEL_ROLES_ACTION => duel::select_duel_roles(host, input),
        RECORD_DUEL_SCORES_ACTION => duel::record_duel_scores(host, input),
        RUN_PLANNING_DUEL_ACTION => duel::run_planning_duel(host, input, debug),

        // ---- generic built-in actions ----
        GIT_COMMIT_ACTION => vcs::git_commit(host, input),
        GIT_PUSH_ACTION => vcs::push_batch_changes(host, input),
        GIT_PULL_ACTION => vcs::pull_batch_changes(host, input),
        GIT_MERGE_ACTION => vcs::git_merge(host, input),
        WORKTREE_SETUP_ACTION => vcs::setup_worktree(host, input),
        WORKTREE_CLEANUP_ACTION => vcs::cleanup_worktree(host, input),
        PR_OPEN_ACTION => vcs::pr_open(host, input),
        PR_SYNC_REVIEWS_ACTION => review::sync_batch_review_to_github(host, input),
        CHECK_TASK_VALUE_ACTION => review::check_task_value(host, input),
        DISPATCH_BATCH_ACTION => batch::dispatch_batch(host, input),
        RUN_COMMAND_ACTION => command::run_command(host, input, steps_outputs, state_context),

        other => Err(OrbitError::InvalidInput(format!(
            "unsupported automation action '{other}'"
        ))),
    }
}
