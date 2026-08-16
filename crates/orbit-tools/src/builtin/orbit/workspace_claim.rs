//! The exclusive workspace claim [ADR-0352, ORB-10709].
//!
//! Registered alongside the task-lock tools, and for the same reason: these are
//! coordination-hold operations an operator reaches directly, not tools an agent
//! should be handed inside a run. Enforcement of the claim lives at the shared
//! run-submission path in `orbit-core`, never here — a tool-layer gate would be
//! one adapter's opinion, and a caller with a shell would route around it.

use orbit_common::OrbitError;
use orbit_types::tool::{ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitWorkspaceClaimAcquireTool;
pub struct OrbitWorkspaceClaimReleaseTool;
pub struct OrbitWorkspaceClaimShowTool;

impl Tool for OrbitWorkspaceClaimAcquireTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "ttl_seconds".to_string(),
                description: "Optional claim TTL in seconds. Defaults to 3600; max 43200."
                    .to_string(),
                param_type: "u64".to_string(),
                required: false,
            },
            ToolParam {
                name: "machine_id".to_string(),
                description:
                    "Optional machine identity, recorded for diagnostics only; the claim is keyed \
                     on the returned token."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "session_id".to_string(),
                description:
                    "Optional session identity, recorded for diagnostics only; never load-bearing, \
                     so a reconnecting client does not orphan the workspace."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.workspace.claim.acquire".to_string(),
            description:
                "Take the exclusive, TTL-bounded workspace claim required to dispatch workflow \
                 runs while another operator holds one, and return the claim token to present on \
                 subsequent dispatches. Contention rejects with the current holder and expiry."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::WorkspaceClaimAcquire)
    }
}

impl Tool for OrbitWorkspaceClaimReleaseTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "claim_token".to_string(),
                description: "The holder's claim token. Required unless `force` is set."
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "force".to_string(),
                description:
                    "Release the claim without its token, for a holder that has gone away. \
                     Audited with who forced it and whom they displaced."
                        .to_string(),
                param_type: "boolean".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.workspace.claim.release".to_string(),
            description: "Release the workspace claim with its token, or force-release it."
                .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::WorkspaceClaimRelease)
    }
}

impl Tool for OrbitWorkspaceClaimShowTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.workspace.claim.show".to_string(),
            description:
                "Report the active workspace claim — holder, expiry, and diagnostics — or that the \
                 workspace is unclaimed."
                    .to_string(),
            parameters: Vec::new(),
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::WorkspaceClaimShow)
    }
}
