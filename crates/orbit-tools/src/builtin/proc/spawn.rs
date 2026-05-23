use orbit_common::tracing;
use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct ProcSpawnTool;

impl Tool for ProcSpawnTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "proc.spawn".to_string(),
            description: "Spawn a process with timeout and capture output".to_string(),
            parameters: vec![
                ToolParam {
                    name: "program".to_string(),
                    description: "Program to execute".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "args".to_string(),
                    description: "Arguments to pass to the program".to_string(),
                    param_type: "array".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "timeout_ms".to_string(),
                    description: "Execution timeout in milliseconds".to_string(),
                    param_type: "u64".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let program = input
            .get("program")
            .and_then(Value::as_str)
            .ok_or_else(|| OrbitError::InvalidInput("missing `program`".to_string()))?
            .to_string();

        // Enforce program allowlist when the call sits inside an activity-scoped
        // tool context, or when a legacy unrestricted context still has a
        // non-empty list. An activity-scoped call with an empty list denies
        // every program (fail-closed).
        let restricted = ctx.proc_spawn_activity_scoped || !ctx.proc_allowed_programs.is_empty();
        if restricted && !ctx.proc_allowed_programs.iter().any(|p| p == &program) {
            let matched_rule = if ctx.proc_allowed_programs.is_empty() {
                "<no allowed programs>".to_string()
            } else {
                ctx.proc_allowed_programs.join(", ")
            };
            tracing::warn!(
                target: "orbit.policy.deny",
                tool = "proc.spawn",
                path = program.as_str(),
                profile = "proc.allowed_programs",
                matched_rule = matched_rule.as_str(),
            );
            return Err(OrbitError::PolicyDenied(format!(
                "program '{}' is not in the allowed list: [{}]",
                program, matched_rule
            )));
        }

        let args = input
            .get("args")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let timeout_ms = input.get("timeout_ms").and_then(Value::as_u64);

        // Filter sensitive env vars instead of inheriting the full environment.
        let env_pairs: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| !is_sensitive_env_name(k))
            .collect();

        let current_dir = ctx
            .workspace_root
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());

        let exec_result = run_process(
            &ExecRequest {
                program,
                args,
                current_dir,
                timeout_ms,
                stdin_mode: StdinMode::Inherit,
                environment_mode: EnvironmentMode::ClearAndSet(env_pairs),
                debug: false,
            },
            &NoSandbox,
        )?;

        serde_json::to_value(exec_result)
            .map_err(|e| OrbitError::Execution(format!("serialize exec result: {e}")))
    }
}

/// Returns `true` if the environment variable name looks like it holds a secret.
fn is_sensitive_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let patterns = ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"];
    patterns.iter().any(|p| upper.contains(p))
}
