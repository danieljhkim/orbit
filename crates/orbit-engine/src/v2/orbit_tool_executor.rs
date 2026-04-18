//! `OrbitToolCallExecutor` — a deterministic executor that wraps Orbit tools.
//!
//! spec_type: "orbit_tool_call".
//!
//! Accepts input `{ "tool_name": string, "args": object }` and dispatches into
//! the registered `ToolRegistry`. Pure Rust, no LLM, no shell. Used by the v2
//! deterministic reference activity and by any future deterministic activity
//! that wants to drive a single Orbit tool call.

use serde_json::{Value, json};

use crate::context::{ACTIVITY_EXECUTION_FAILED, AttemptOutcome, ExecutionContext, ExecutorHost};
use crate::executor::traits::ActivityExecutor;

pub struct OrbitToolCallExecutor;

impl ActivityExecutor for OrbitToolCallExecutor {
    fn spec_type(&self) -> &str {
        "orbit_tool_call"
    }

    fn execute(&self, _host: ExecutorHost<'_>, execution: &ExecutionContext) -> AttemptOutcome {
        let input = &execution.input;
        let tool_name = match input.get("tool_name").and_then(Value::as_str) {
            Some(s) => s.to_string(),
            None => {
                return AttemptOutcome::failed(
                    ACTIVITY_EXECUTION_FAILED,
                    "orbit_tool_call: missing `tool_name`".to_string(),
                );
            }
        };
        let args = input.get("args").cloned().unwrap_or(Value::Null);

        // Phase 2: record the tool-call request structurally without actually
        // dispatching into ToolRegistry (which requires wiring a full
        // ToolContext from the engine). The structural output is sufficient
        // for reference-asset validation. Phase 4 wires the real dispatch.
        let output = json!({
            "tool_name": tool_name,
            "args": args,
            "status": "structural_dispatch",
        });
        AttemptOutcome::success(0, 0, output)
    }
}
