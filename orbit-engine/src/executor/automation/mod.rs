mod check_review;
mod comments;
mod commit;
mod freshness;
mod git;
mod input;
mod pr;
mod push;
mod review;
mod task;
mod worktree;

use orbit_types::{Activity, JobRunState, OrbitError};
use serde::Deserialize;
use serde_json::Value;

use super::ActivityExecutor;
use crate::activity_runner::validate_activity_output_schema;
use crate::context::{ACTIVITY_EXECUTION_FAILED, AttemptOutcome, EngineHost, ExecutionContext};

const AUTOMATION_CREATE_TASK_WORKTREE: &str = "create_task_worktree";
const AUTOMATION_UPDATE_TASK: &str = "update_task";
const AUTOMATION_COMMIT_TASK_CHANGES: &str = "commit_task_changes";
const AUTOMATION_MERGE_PR_FROM_TASK: &str = "merge_pr_from_task";
const AUTOMATION_OPEN_PR_FROM_TASK: &str = "open_pr_from_task";
const AUTOMATION_FINALIZE_TASK_WORKTREE: &str = "finalize_task_worktree";
const AUTOMATION_CHECK_REVIEW_DECISION: &str = "check_review_decision";
const AUTOMATION_LOAD_PR_COMMENTS: &str = "load_pr_comments";
const AUTOMATION_PUSH_TASK_CHANGES: &str = "push_task_changes";

#[derive(Debug, Clone, Deserialize)]
struct AutomationSpec {
    action: String,
}

pub struct AutomationExecutor;

impl ActivityExecutor for AutomationExecutor {
    fn spec_type(&self) -> &str {
        "automation"
    }

    fn execute(&self, host: &dyn EngineHost, execution: &ExecutionContext) -> AttemptOutcome {
        match execute(host, &execution.activity, &execution.input) {
            Ok(result) => {
                if let Err(err) = validate_activity_output_schema(&execution.activity, &result) {
                    return AttemptOutcome {
                        exit_code: Some(0),
                        response_json: Some(result),
                        ..AttemptOutcome::failed(ACTIVITY_EXECUTION_FAILED, err.to_string())
                    };
                }
                AttemptOutcome {
                    state: JobRunState::Success,
                    exit_code: Some(0),
                    duration_ms: None,
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
#[cfg(test)]
pub(super) mod test_utils {
    use std::sync::{Mutex, OnceLock};

    /// Global mutex protecting `PATH` modifications across all automation tests.
    pub fn path_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }
}

pub fn execute<H: crate::context::RuntimeHost + crate::context::TaskHost + ?Sized>(
    host: &H,
    activity: &Activity,
    input: &Value,
) -> Result<Value, OrbitError> {
    let spec: AutomationSpec =
        serde_json::from_value(activity.spec_config.clone()).map_err(|error| {
            OrbitError::InvalidInput(format!("invalid automation spec_config: {error}"))
        })?;

    match spec.action.as_str() {
        AUTOMATION_CREATE_TASK_WORKTREE => worktree::create_task_worktree(host, input),
        AUTOMATION_UPDATE_TASK => task::update_task(host, input),
        AUTOMATION_COMMIT_TASK_CHANGES => commit::commit_task_changes(host, input),
        AUTOMATION_MERGE_PR_FROM_TASK => pr::merge_pr_from_task(host, input),
        AUTOMATION_OPEN_PR_FROM_TASK => pr::open_pr_from_task(host, input),
        AUTOMATION_FINALIZE_TASK_WORKTREE => worktree::finalize_task_worktree(input),
        AUTOMATION_CHECK_REVIEW_DECISION => check_review::check_review_decision(host, input),
        AUTOMATION_LOAD_PR_COMMENTS => comments::load_pr_comments(host, input),
        AUTOMATION_PUSH_TASK_CHANGES => push::push_task_changes(host, input),
        other => Err(OrbitError::InvalidInput(format!(
            "unsupported automation action '{other}'"
        ))),
    }
}
