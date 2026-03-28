use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
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

        // Enforce program allowlist when configured.
        if !ctx.proc_allowed_programs.is_empty()
            && !ctx.proc_allowed_programs.iter().any(|p| p == &program)
        {
            return Err(OrbitError::PolicyDenied(format!(
                "program '{}' is not in the allowed list: [{}]",
                program,
                ctx.proc_allowed_programs.join(", ")
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

        let exec_result = run_process(
            &ExecRequest {
                program,
                args,
                current_dir: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_env_detection() {
        assert!(is_sensitive_env_name("GITHUB_TOKEN"));
        assert!(is_sensitive_env_name("AWS_SECRET_ACCESS_KEY"));
        assert!(is_sensitive_env_name("DB_PASSWORD"));
        assert!(is_sensitive_env_name("api_key"));
        assert!(is_sensitive_env_name("GOOGLE_CREDENTIALS"));
        assert!(!is_sensitive_env_name("HOME"));
        assert!(!is_sensitive_env_name("PATH"));
        assert!(!is_sensitive_env_name("ORBIT_AGENT_NAME"));
    }

    #[test]
    fn allowlist_blocks_disallowed_program() {
        let tool = ProcSpawnTool;
        let ctx = ToolContext {
            proc_allowed_programs: vec!["cargo".to_string(), "git".to_string()],
            ..Default::default()
        };
        let input = serde_json::json!({ "program": "curl" });
        let result = tool.execute(&ctx, input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("policy denied"), "got: {err}");
        assert!(err.contains("curl"));
    }

    #[test]
    fn empty_allowlist_permits_any_program() {
        // Just verify it doesn't error on the allowlist check — the spawn itself
        // will fail with a nonexistent program, but that's fine.
        let tool = ProcSpawnTool;
        let ctx = ToolContext::default();
        let input = serde_json::json!({ "program": "echo", "args": ["hello"] });
        // This should either succeed or fail on spawn, not on policy.
        let result = tool.execute(&ctx, input);
        match result {
            Ok(_) => {} // echo worked
            Err(e) => assert!(
                !e.to_string().contains("policy denied"),
                "should not be policy denied: {e}"
            ),
        }
    }
}
