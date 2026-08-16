//! Claim-gated remote command execution [ADR-0351, ORB-10711].
//!
//! The one operation the owned tunnel (§5.3 of `docs/design/mcp-bridge/2_design.md`)
//! adds beyond the existing read/write tool surface: an explicit argv plus an
//! explicit working directory, never a shell string, so quoting and
//! operator-precedence bugs are impossible by construction. It requires operator
//! capability (enforced at the MCP policy layer and, uniformly across every
//! entry point, at the governed-operation chokepoint in
//! `orbit_common::authorization`) and the workspace claim (enforced in
//! `OrbitRuntime::execute_remote_command`, the same chokepoint `orbit.workflow.ship`
//! uses).
//!
//! The self-dispatch guard below mirrors `orbit.workflow.ship`/`run.resume`: a
//! managed run's leaf agent must not reach a general command surface, which
//! would let it bypass every tool-specific policy by invoking the CLI directly.

use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitCommandExecTool;

impl Tool for OrbitCommandExecTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "argv".to_string(),
                description: "The command as a JSON array of argv strings, program first (e.g. \
                     [\"git\", \"status\"]). Never a shell string: no shell interprets this \
                     input, so quoting and operator-precedence bugs cannot occur."
                    .to_string(),
                param_type: "string_list".to_string(),
                required: true,
            },
            ToolParam {
                name: "working_directory".to_string(),
                description: "Explicit working directory the command runs in.".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "claim_token".to_string(),
                description: "Token for this workspace's exclusive claim, required when \
                     another operator holds one. Falls back to `ORBIT_WORKSPACE_CLAIM_TOKEN`."
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::model_identity_params());
        ToolSchema {
            name: "orbit.command.exec".to_string(),
            description:
                "Execute a command as an explicit argv array in an explicit working directory \
                 and return its stdout, stderr, and exit status. Requires operator capability \
                 and the workspace claim; withheld from managed runs."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        if ctx
            .orbit_host
            .as_ref()
            .is_some_and(|host| host.task_scope().run_id.is_some())
        {
            return Err(OrbitError::CapabilityDenied(
                "managed runs cannot execute remote commands; a leaf agent invoking the CLI \
                 from inside a run would bypass the self-dispatch guard"
                    .to_string(),
            ));
        }
        super::execute_host_action(ctx, input, OrbitBuiltinAction::CommandExec)
    }
}
