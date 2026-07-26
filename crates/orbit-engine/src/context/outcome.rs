//! Attempt/run outcome types, error-code constants, and workflow-failure
//! helpers.

use orbit_common::types::{InvocationTrace, JobRunState, TaskStatus};
use serde_json::Value;

use super::hosts::TaskAutomationUpdate;

pub const AGENT_INVOCATION_FAILED: &str = "AGENT_INVOCATION_FAILED";
pub const AGENT_TIMEOUT: &str = "AGENT_TIMEOUT";
pub const ACTIVITY_EXECUTION_FAILED: &str = "ACTIVITY_EXECUTION_FAILED";
pub const WORKFLOW_RUN_FAILED_EVENT: &str = "workflow_run_failed";

pub fn workflow_failure_note(
    job_id: &str,
    run_id: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
) -> String {
    let error_code = error_code
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-");
    let error_message = error_message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-");

    format!(
        "workflow run failed: job={job_id}, run_id={run_id}, error_code={error_code}, error={error_message}"
    )
}

pub fn blocked_workflow_failure_update(
    job_id: &str,
    run_id: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
) -> TaskAutomationUpdate {
    TaskAutomationUpdate {
        status: Some(TaskStatus::Blocked),
        status_event: Some(WORKFLOW_RUN_FAILED_EVENT.to_string()),
        status_note: Some(workflow_failure_note(
            job_id,
            run_id,
            error_code,
            error_message,
        )),
        ..TaskAutomationUpdate::default()
    }
}

#[derive(Debug, Clone)]
pub struct AttemptOutcome {
    pub state: JobRunState,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub invocation_trace: InvocationTrace,
    pub response_json: Option<Value>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub protocol_violation: bool,
    /// Number of retries that occurred before this final outcome (0 = first attempt succeeded/failed).
    pub retry_count: u32,
}

impl AttemptOutcome {
    pub fn failed(error_code: &str, message: String) -> Self {
        Self {
            state: JobRunState::Failed,
            exit_code: Some(1),
            duration_ms: None,
            invocation_trace: InvocationTrace::default(),
            response_json: None,
            error_code: Some(error_code.to_string()),
            error_message: Some(message),
            protocol_violation: false,
            retry_count: 0,
        }
    }

    pub fn success(exit_code: i32, duration_ms: u64, response_json: Value) -> Self {
        Self {
            state: JobRunState::Success,
            exit_code: Some(exit_code),
            duration_ms: Some(duration_ms),
            invocation_trace: InvocationTrace {
                duration_ms,
                ..InvocationTrace::default()
            },
            response_json: Some(response_json),
            error_code: None,
            error_message: None,
            protocol_violation: false,
            retry_count: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActivityInvocationResult {
    pub response_json: Option<Value>,
    pub invocation_trace: InvocationTrace,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
}
