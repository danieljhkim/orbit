mod check_review;
mod commit;
mod commit_and_pr;
mod freshness;
mod git;
mod input;
mod parallel;
mod pr;
mod push;
pub(crate) mod review;
mod snapshot;
mod sync_review;
mod task;
mod verify;

use orbit_types::{Activity, JobRunState, OrbitError};
use serde::Deserialize;
use serde_json::Value;

use super::ActivityExecutor;
use crate::activity_runner::validate_activity_output_schema;
use crate::context::{ACTIVITY_EXECUTION_FAILED, AttemptOutcome, EngineHost, ExecutionContext};

const AUTOMATION_UPDATE_TASK: &str = "update_task";
const AUTOMATION_RUN_PARALLEL_TASK_PIPELINE: &str = "run_parallel_task_pipeline";
const AUTOMATION_COMMIT_BATCH_CHANGES: &str = "commit_batch_changes";
const AUTOMATION_OPEN_BATCH_PR: &str = "open_batch_pr";
const AUTOMATION_COMMIT_AND_OPEN_BATCH_PR: &str = "commit_and_open_batch_pr";
const AUTOMATION_SNAPSHOT_BATCH_STATE: &str = "snapshot_batch_state";
const AUTOMATION_VERIFY_BATCH: &str = "verify_batch";
const AUTOMATION_MERGE_BATCH_PR: &str = "merge_batch_pr";
const AUTOMATION_CHECK_BATCH_REVIEW_DECISION: &str = "check_batch_review_decision";
const AUTOMATION_SYNC_BATCH_REVIEW_TO_GITHUB: &str = "sync_batch_review_to_github";
const AUTOMATION_PUSH_BATCH_CHANGES: &str = "push_batch_changes";

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
        match execute(host, &execution.activity, &execution.input, execution.debug) {
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
pub fn execute<H: crate::context::RuntimeHost + crate::context::TaskHost + Sync + ?Sized>(
    host: &H,
    activity: &Activity,
    input: &Value,
    debug: bool,
) -> Result<Value, OrbitError> {
    let spec: AutomationSpec =
        serde_json::from_value(activity.spec_config.clone()).map_err(|error| {
            OrbitError::InvalidInput(format!("invalid automation spec_config: {error}"))
        })?;

    match spec.action.as_str() {
        AUTOMATION_UPDATE_TASK => task::update_task(host, input),
        AUTOMATION_RUN_PARALLEL_TASK_PIPELINE => {
            parallel::run_parallel_task_pipeline(host, input, debug)
        }
        AUTOMATION_COMMIT_BATCH_CHANGES => commit::commit_batch_changes(host, input),
        AUTOMATION_OPEN_BATCH_PR => pr::open_batch_pr(host, input),
        AUTOMATION_COMMIT_AND_OPEN_BATCH_PR => commit_and_pr::commit_and_open_batch_pr(host, input),
        AUTOMATION_SNAPSHOT_BATCH_STATE => snapshot::snapshot_batch_state(host, input),
        AUTOMATION_VERIFY_BATCH => verify::verify_batch(host, input),
        AUTOMATION_MERGE_BATCH_PR => pr::merge_batch_pr(host, input),
        AUTOMATION_CHECK_BATCH_REVIEW_DECISION => {
            check_review::check_batch_review_decision(host, input)
        }
        AUTOMATION_SYNC_BATCH_REVIEW_TO_GITHUB => {
            sync_review::sync_batch_review_to_github(host, input)
        }
        AUTOMATION_PUSH_BATCH_CHANGES => push::push_batch_changes(host, input),
        other => Err(OrbitError::InvalidInput(format!(
            "unsupported automation action '{other}'"
        ))),
    }
}
