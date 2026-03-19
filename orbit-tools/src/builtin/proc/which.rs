use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct ProcWhichTool;

impl Tool for ProcWhichTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "proc.which".to_string(),
            description: "Resolve a command path".to_string(),
            parameters: vec![ToolParam {
                name: "command".to_string(),
                description: "Command name to look up".to_string(),
                param_type: "string".to_string(),
                required: true,
            }],
            builtin: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| OrbitError::InvalidInput("missing `command`".to_string()))?;

        let which_program = if cfg!(target_os = "windows") {
            "where"
        } else {
            "which"
        };

        let result = run_process(
            &ExecRequest {
                program: which_program.to_string(),
                args: vec![command.to_string()],
                current_dir: None,
                timeout_ms: Some(1_000),
                stdin_mode: StdinMode::Inherit,
                environment_mode: EnvironmentMode::Inherit,
            },
            &NoSandbox,
        )?;

        Ok(json!({
            "command": command,
            "path": result.stdout.trim(),
            "found": result.success,
        }))
    }
}
