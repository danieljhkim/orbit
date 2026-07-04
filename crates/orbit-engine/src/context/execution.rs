//! The [`ExecutionContext`] passed through executor dispatch, plus
//! working-directory resolution helpers.

use orbit_common::types::{Activity, Job};
use serde_json::Value;
use std::path::PathBuf;

use super::hosts::TaskReadHost;

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub activity: Activity,
    pub job: Option<Job>,
    pub agent_cli: String,
    pub model: Option<String>,
    pub timeout_seconds: u64,
    pub env_extra: Vec<String>,
    /// Explicit env var key-value pairs that override same-named vars from
    /// `env_extra` or the global allowlist.
    pub env_set: std::collections::HashMap<String, String>,
    pub input: Value,
    /// When `true`, stream agent stderr to the terminal and tee stdout live.
    pub debug: bool,
    /// Accumulated outputs from completed steps, keyed by step id (or target_id).
    /// Used to populate the `steps` namespace in TemplateContext.
    pub steps_outputs: std::collections::HashMap<String, Value>,
    pub run_id: Option<String>,
    pub step_index: Option<u32>,
    pub state_dir: Option<PathBuf>,
}

pub fn input_workspace_path(input: &Value) -> Option<String> {
    input
        .as_object()
        .and_then(|map| map.get("workspace_path"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub fn execution_working_directory(execution: &ExecutionContext) -> Option<String> {
    execution
        .activity
        .workspace_path
        .clone()
        .or_else(|| input_workspace_path(&execution.input))
}

/// Resolve the working directory for an execution context, falling back to the
/// task's workspace_path when neither the activity nor input provides one.
/// This is the preferred variant for agent_invoke and cli_command executors
/// where a [`TaskHost`](super::hosts::TaskHost) is available.
pub fn execution_working_directory_with_task<H: TaskReadHost + ?Sized>(
    _host: &H,
    execution: &ExecutionContext,
) -> Option<String> {
    execution_working_directory(execution)
}
