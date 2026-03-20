use std::io::Write;
use std::time::Instant;

use orbit_types::{ExecutionResult, OrbitError};

use crate::sandbox::Sandbox;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StdinMode {
    #[default]
    Inherit,
    Null,
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EnvironmentMode {
    #[default]
    Inherit,
    ClearAndSet(Vec<(String, String)>),
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

#[cfg(test)]
mod tests {
    use crate::sandbox::NoSandbox;

    use super::*;

    #[test]
    fn captures_stdout() {
        let result = run_process(
            &ExecRequest {
                program: "sh".to_string(),
                args: vec!["-c".to_string(), "printf hello".to_string()],
                current_dir: None,
                timeout_ms: Some(1000),
                stdin_mode: StdinMode::Inherit,
                environment_mode: EnvironmentMode::Inherit,
                debug: false,
            },
            &NoSandbox,
        )
        .expect("process succeeds");

        assert!(result.success);
        assert_eq!(result.stdout, "hello");
    }

    #[test]
    fn supports_stdin_bytes() {
        let result = run_process(
            &ExecRequest {
                program: "sh".to_string(),
                args: vec!["-c".to_string(), "cat".to_string()],
                current_dir: None,
                timeout_ms: Some(1000),
                stdin_mode: StdinMode::Bytes(b"hello-stdin".to_vec()),
                environment_mode: EnvironmentMode::Inherit,
                debug: false,
            },
            &NoSandbox,
        )
        .expect("process succeeds");

        assert!(result.success);
        assert_eq!(result.stdout, "hello-stdin");
    }

    #[test]
    fn enforces_timeout() {
        let result = run_process(
            &ExecRequest {
                program: "sh".to_string(),
                args: vec!["-c".to_string(), "sleep 1".to_string()],
                current_dir: None,
                timeout_ms: Some(100),
                stdin_mode: StdinMode::Inherit,
                environment_mode: EnvironmentMode::Inherit,
                debug: false,
            },
            &NoSandbox,
        )
        .expect("process returns timed out result");

        assert!(!result.success);
        assert!(result.stderr.contains("timed out"));
    }

    #[test]
    fn clear_and_set_environment_drops_unlisted_variables() {
        let result = run_process(
            &ExecRequest {
                program: "env".to_string(),
                args: Vec::new(),
                current_dir: None,
                timeout_ms: Some(1000),
                stdin_mode: StdinMode::Inherit,
                environment_mode: EnvironmentMode::ClearAndSet(Vec::new()),
                debug: false,
            },
            &NoSandbox,
        )
        .expect("process succeeds");

        assert!(!result.stdout.lines().any(|line| line.starts_with("PATH=")));
    }

    #[test]
    fn clear_and_set_environment_allows_allowlisted_variables() {
        let path = std::env::var("PATH").unwrap_or_default();
        let result = run_process(
            &ExecRequest {
                program: "env".to_string(),
                args: Vec::new(),
                current_dir: None,
                timeout_ms: Some(1000),
                stdin_mode: StdinMode::Inherit,
                environment_mode: EnvironmentMode::ClearAndSet(vec![(
                    "PATH".to_string(),
                    path.clone(),
                )]),
                debug: false,
            },
            &NoSandbox,
        )
        .expect("process succeeds");

        assert!(
            result
                .stdout
                .lines()
                .any(|line| line == format!("PATH={path}"))
        );
    }

    #[test]
    fn clear_and_set_with_macos_system_vars_does_not_crash() {
        // Simulates the default hermetic allowlist. On macOS, spawned processes
        // need TMPDIR, __CF_USER_TEXT_ENCODING, and USER to avoid panics in
        // system-configuration / CoreFoundation code paths.
        let pairs: Vec<(String, String)> =
            ["HOME", "PATH", "TMPDIR", "__CF_USER_TEXT_ENCODING", "USER"]
                .iter()
                .filter_map(|name| std::env::var(name).ok().map(|v| (name.to_string(), v)))
                .collect();

        let result = run_process(
            &ExecRequest {
                program: "sh".to_string(),
                args: vec!["-c".to_string(), "echo ok".to_string()],
                current_dir: None,
                timeout_ms: Some(2000),
                stdin_mode: StdinMode::Inherit,
                environment_mode: EnvironmentMode::ClearAndSet(pairs),
                debug: false,
            },
            &NoSandbox,
        )
        .expect("process must not crash in hermetic env with macOS system vars");

        assert!(result.success);
        assert_eq!(result.stdout.trim(), "ok");
    }

    #[test]
    fn inherit_environment_keeps_existing_variables() {
        let result = run_process(
            &ExecRequest {
                program: "env".to_string(),
                args: Vec::new(),
                current_dir: None,
                timeout_ms: Some(1000),
                stdin_mode: StdinMode::Inherit,
                environment_mode: EnvironmentMode::Inherit,
                debug: false,
            },
            &NoSandbox,
        )
        .expect("process succeeds");

        assert!(result.stdout.lines().any(|line| line.starts_with("PATH=")));
    }
}
