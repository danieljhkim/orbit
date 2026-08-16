use orbit_common::OrbitError;
use orbit_exec::{EnvironmentMode, ExecRequest, StdinMode};
use orbit_types::tool::{ToolParam, ToolSchema};
use serde_json::Value;

use crate::{ToolRegistry, require_str};

pub(super) fn gh_exec_request(
    args: Vec<String>,
    current_dir: Option<String>,
    timeout_ms: u64,
) -> ExecRequest {
    ExecRequest {
        program: "gh".to_string(),
        args,
        current_dir,
        timeout_ms: Some(timeout_ms),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    }
}

pub(super) fn gh_schema(name: &str, description: &str, parameters: Vec<ToolParam>) -> ToolSchema {
    ToolSchema {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
        builtin: true,
    }
}

pub(super) fn tool_param(
    name: &str,
    description: &str,
    param_type: &str,
    required: bool,
) -> ToolParam {
    ToolParam {
        name: name.to_string(),
        description: description.to_string(),
        param_type: param_type.to_string(),
        required,
    }
}

macro_rules! gh_tool {
    (
        $vis:vis struct $name:ident;
        name: $tool_name:expr;
        description: $description:expr;
        parameters: [$($param:expr),* $(,)?];
        request: |$request_ctx:ident, $request_input:ident| $request:block
        response: |$response_ctx:ident, $response_input:ident, $result:ident| $response:block
    ) => {
        $vis struct $name;

        impl crate::Tool for $name {
            fn schema(&self) -> orbit_types::tool::ToolSchema {
                super::gh_schema($tool_name, $description, vec![$($param),*])
            }

            fn execute(
                &self,
                ctx: &crate::ToolContext,
                input: serde_json::Value,
            ) -> Result<serde_json::Value, orbit_common::OrbitError> {
                let req = {
                    let $request_ctx = ctx;
                    let $request_input = &input;
                    $request
                }?;
                let exec_result = orbit_exec::run_process(&req, &orbit_exec::NoSandbox)?;
                let $response_ctx = ctx;
                let $response_input = &input;
                let $result = &exec_result;
                $response
            }
        }
    };
    (
        $vis:vis struct $name:ident;
        name: $tool_name:expr;
        description: $description:expr;
        parameters: [$($param:expr),* $(,)?];
        execute: |$execute_ctx:ident, $execute_input:ident| $execute:block
    ) => {
        $vis struct $name;

        impl crate::Tool for $name {
            fn schema(&self) -> orbit_types::tool::ToolSchema {
                super::gh_schema($tool_name, $description, vec![$($param),*])
            }

            fn execute(
                &self,
                ctx: &crate::ToolContext,
                input: serde_json::Value,
            ) -> Result<serde_json::Value, orbit_common::OrbitError> {
                let $execute_ctx = ctx;
                let $execute_input = input;
                $execute
            }
        }
    };
}

pub(super) use gh_tool;

pub mod auth;
pub mod pr_checkout;
pub mod pr_checks;
pub mod pr_close;
pub mod repo;

pub fn register(_registry: &mut ToolRegistry) {}

/// Extract a non-empty `pr` field from the tool input.
/// Accepts a numeric PR number or a GitHub PR URL (extracts the number from the path).
pub(super) fn require_pr(input: &Value) -> Result<String, OrbitError> {
    let pr = require_str(input, "pr")?;
    // Already numeric — use directly.
    if !pr.is_empty() && pr.chars().all(|c| c.is_ascii_digit()) {
        return Ok(pr);
    }
    // Try to extract PR number from a GitHub URL like
    // https://github.com/owner/repo/pull/123
    if pr.contains("github.com/")
        && pr.contains("/pull/")
        && let Some(num) = pr.rsplit('/').next()
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
    {
        return Ok(num.to_string());
    }
    Err(OrbitError::InvalidInput(format!(
        "invalid `pr`: \"{pr}\"; must be a numeric PR number or GitHub PR URL"
    )))
}
