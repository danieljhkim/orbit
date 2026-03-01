use std::process::{Child, Command, Stdio};

use orbit_types::OrbitError;

use crate::runner::{EnvironmentMode, ExecRequest, StdinMode};

pub(crate) fn spawn(req: &ExecRequest) -> Result<Child, OrbitError> {
    let mut command = Command::new(&req.program);
    command
        .args(&req.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let EnvironmentMode::ClearAndSet(pairs) = &req.environment_mode {
        command.env_clear();
        command.envs(pairs.iter().cloned());
    }

    match req.stdin_mode {
        StdinMode::Inherit => {
            command.stdin(Stdio::inherit());
        }
        StdinMode::Null => {
            command.stdin(Stdio::null());
        }
        StdinMode::Bytes(_) => {
            command.stdin(Stdio::piped());
        }
    }

    command
        .spawn()
        .map_err(|e| OrbitError::Execution(format!("failed to spawn `{}`: {e}", req.program)))
}
