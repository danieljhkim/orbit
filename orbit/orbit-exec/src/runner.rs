use std::io::Write;
use std::time::Instant;

use orbit_types::{ExecutionResult, OrbitError, is_sensitive_env_name};

use crate::sandbox::Sandbox;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StdinMode {
    #[default]
    Inherit,
    Null,
    Bytes(Vec<u8>),
}

#[derive(Clone, PartialEq, Eq, Default)]
pub enum EnvironmentMode {
    #[default]
    Inherit,
    ClearAndSet(Vec<(String, String)>),
}

impl std::fmt::Debug for EnvironmentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inherit => write!(f, "Inherit"),
            Self::ClearAndSet(pairs) => {
                let redacted: Vec<(&str, &str)> = pairs
                    .iter()
                    .map(|(k, v)| {
                        if is_sensitive_env_name(k) {
                            (k.as_str(), "[REDACTED]")
                        } else {
                            (k.as_str(), v.as_str())
                        }
                    })
                    .collect();
                f.debug_tuple("ClearAndSet").field(&redacted).finish()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecRequest {
    pub program: String,
    pub args: Vec<String>,
    pub current_dir: Option<String>,
    pub timeout_ms: Option<u64>,
    pub stdin_mode: StdinMode,
    pub environment_mode: EnvironmentMode,
    /// When `true`, stream agent stderr directly to the terminal and tee
    /// stdout to stderr while accumulating it for JSON parsing.
    pub debug: bool,
}

pub fn run_process(
    req: &ExecRequest,
    sandbox: &dyn Sandbox,
) -> Result<ExecutionResult, OrbitError> {
    sandbox.validate(req)?;

    let started = Instant::now();
    let mut child = crate::process::spawn(req)?;
    if let StdinMode::Bytes(bytes) = &req.stdin_mode {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(bytes).map_err(|e| {
                OrbitError::Execution(format!("failed to write process stdin: {e}"))
            })?;
        } else {
            return Err(OrbitError::Execution(
                "stdin requested but no stdin pipe available".to_string(),
            ));
        }
    }
    let result = crate::timeout::wait_with_optional_timeout(child, req.timeout_ms, req.debug)?;

    Ok(ExecutionResult {
        success: result.exit_success,
        stdout: String::from_utf8_lossy(&result.stdout).to_string(),
        stderr: String::from_utf8_lossy(&result.stderr).to_string(),
        exit_code: result.exit_code,
        duration_ms: started.elapsed().as_millis() as u64,
        output: None,
    })
}
