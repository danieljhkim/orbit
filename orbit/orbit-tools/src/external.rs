use std::io::Write;
use std::process::{Command, Stdio};

use orbit_types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct ExternalTool {
    pub name: String,
    pub path: String,
    pub description: String,
}

impl Tool for ExternalTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: vec![],
            builtin: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let input_json = serde_json::to_string(&input)
            .map_err(|e| OrbitError::Execution(format!("failed to serialize input: {e}")))?;

        let mut child = Command::new(&self.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| OrbitError::Execution(format!("failed to spawn '{}': {e}", self.path)))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input_json.as_bytes())
                .map_err(|e| OrbitError::Execution(format!("failed to write stdin: {e}")))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|e| OrbitError::Execution(format!("failed to wait for process: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(OrbitError::Execution(format!(
                "tool '{}' exited with {}: {}",
                self.name,
                output.status,
                stderr.trim()
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(stdout.trim()).map_err(|e| {
            OrbitError::Execution(format!(
                "tool '{}' produced invalid JSON output: {e}",
                self.name
            ))
        })
    }
}
