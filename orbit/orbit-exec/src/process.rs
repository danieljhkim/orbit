use std::process::{Child, Command, Stdio};

use orbit_types::OrbitError;

use crate::runner::{EnvironmentMode, ExecRequest, StdinMode};

pub(crate) fn spawn(req: &ExecRequest) -> Result<Child, OrbitError> {
    let mut command = Command::new(&req.program);
    command.args(&req.args).stdout(Stdio::piped());
    if req.debug {
        command.stderr(Stdio::inherit());
    } else {
        command.stderr(Stdio::piped());
    }
    if let Some(current_dir) = &req.current_dir {
        command.current_dir(current_dir);
    }

    // Make the child a process group leader (pgid = pid).  This lets us send
    // SIGKILL to the entire group after the child exits, ensuring that any
    // orphan subprocesses the agent spawned (which may have inherited the
    // stdout/stderr pipe write ends) are also killed.  Without this, those
    // orphans keep the pipes open and `wait_with_output` hangs indefinitely.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

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
