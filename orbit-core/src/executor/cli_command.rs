use std::collections::HashMap;

use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::OrbitError;
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::tempdir;

use crate::template::{TemplateContext, render};

#[derive(Debug, Clone, Deserialize)]
pub struct CliCommandSpec {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    pub timeout_seconds: Option<u64>,
    #[serde(default = "default_exit_codes")]
    pub expected_exit_codes: Vec<i32>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn default_exit_codes() -> Vec<i32> {
    vec![0]
}

pub fn execute(
    spec_config: &Value,
    template_context: &TemplateContext,
    timeout_seconds: u64,
) -> Result<(Value, u64, Option<i32>), OrbitError> {
    let spec: CliCommandSpec = serde_json::from_value(spec_config.clone()).map_err(|error| {
        OrbitError::InvalidInput(format!("invalid cli_command spec_config: {error}"))
    })?;

    let command = render(&spec.command, template_context)?;
    let args = spec
        .args
        .iter()
        .map(|arg| render(arg, template_context))
        .collect::<Result<Vec<_>, OrbitError>>()?;
    let current_dir = spec
        .working_dir
        .as_deref()
        .map(|value| render(value, template_context))
        .transpose()?;

    let temp_dir = tempdir().map_err(|error| {
        OrbitError::Execution(format!("failed to create cli_command temp dir: {error}"))
    })?;
    let output_path = temp_dir.path().join("orbit-output.json");

    let mut env = template_context.env.clone();
    for (key, value) in &spec.env {
        env.insert(key.clone(), render(value, template_context)?);
    }
    env.insert(
        "ORBIT_OUTPUT_FILE".to_string(),
        output_path.to_string_lossy().into_owned(),
    );

    let exec_result = run_process(
        &ExecRequest {
            program: command,
            args,
            current_dir,
            timeout_ms: Some(
                spec.timeout_seconds
                    .unwrap_or(timeout_seconds)
                    .saturating_mul(1000),
            ),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::ClearAndSet(env.into_iter().collect()),
        },
        &NoSandbox,
    )?;

    let exit_code = exec_result.exit_code.unwrap_or_default();
    if !spec.expected_exit_codes.contains(&exit_code) {
        return Err(OrbitError::Execution(format!(
            "cli_command exited with code {exit_code}; expected one of {:?}; stderr: {}",
            spec.expected_exit_codes,
            exec_result.stderr.trim()
        )));
    }

    let output = match std::fs::read_to_string(&output_path) {
        Ok(raw) if !raw.trim().is_empty() => {
            serde_json::from_str::<Value>(&raw).map_err(|error| {
                OrbitError::Execution(format!(
                    "cli_command wrote invalid JSON to ORBIT_OUTPUT_FILE: {error}"
                ))
            })?
        }
        Ok(_) => json!({ "exit_code": exit_code }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            json!({ "exit_code": exit_code })
        }
        Err(error) => {
            return Err(OrbitError::Execution(format!(
                "failed to read ORBIT_OUTPUT_FILE: {error}"
            )));
        }
    };

    Ok((output, exec_result.duration_ms, exec_result.exit_code))
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use serde_json::json;
    use tempfile::tempdir;

    use super::execute;
    use crate::template::TemplateContext;

    #[test]
    fn cli_command_falls_back_to_exit_code_when_no_output_file_is_written() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("script.sh");
        std::fs::write(&script_path, "#!/bin/sh\nexit 0\n").expect("write script");
        #[cfg(unix)]
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod");

        let (output, _, exit_code) = execute(
            &json!({
                "command": script_path,
                "expected_exit_codes": [0]
            }),
            &TemplateContext::default(),
            5,
        )
        .expect("execute");

        assert_eq!(output, json!({"exit_code": 0}));
        assert_eq!(exit_code, Some(0));
    }
}
