use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_policy::PolicyContext;
use orbit_types::{ExecutionResult, OrbitEvent, PolicyDecision, Role};

use crate::{OrbitError, OrbitRuntime};

const PROCESS_TOOL_NAME: &str = "proc.spawn";
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

impl OrbitRuntime {
    pub(crate) fn execute_shell_command(
        &self,
        entrypoint: &str,
        command: &str,
    ) -> Result<ExecutionResult, OrbitError> {
        let decision = self.context.policy.evaluate(&PolicyContext {
            entrypoint: entrypoint.to_string(),
            tool_name: Some(PROCESS_TOOL_NAME.to_string()),
            role: Role::Admin,
        });

        if let PolicyDecision::Deny { reason } = decision {
            self.with_mutation(|_| {
                Ok((
                    (),
                    OrbitEvent::PolicyDenied {
                        tool: PROCESS_TOOL_NAME.to_string(),
                    },
                ))
            })?;
            return Err(OrbitError::PolicyDenied(reason));
        }

        let result = run_process(
            &ExecRequest {
                program: "sh".to_string(),
                args: vec!["-c".to_string(), command.to_string()],
                timeout_ms: Some(DEFAULT_TIMEOUT_MS),
                stdin_mode: StdinMode::Inherit,
                environment_mode: EnvironmentMode::Inherit,
            },
            &NoSandbox,
        )?;

        self.with_mutation(|_| {
            Ok((
                (),
                OrbitEvent::ToolExecuted {
                    name: PROCESS_TOOL_NAME.to_string(),
                },
            ))
        })?;

        Ok(result)
    }
}
