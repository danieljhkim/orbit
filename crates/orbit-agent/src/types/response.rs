#[path = "response/envelope.rs"]
// Visible to sibling-layout tests for this file-rooted module without
// reintroducing a nested `response/tests` tree.
pub(in crate::types) mod envelope;
#[path = "response/tool_calls.rs"]
mod tool_calls;
#[path = "response/trace.rs"]
mod trace;
#[path = "response/usage.rs"]
// Visible to sibling-layout tests for this file-rooted module without
// reintroducing a nested `response/tests` tree.
pub(in crate::types) mod usage;

use orbit_common::types::{OrbitError, ToolCallTrace};
use serde_json::Value;

pub use envelope::{is_timeout, parse_and_validate_response, peek_response_status};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInvocationSpec {
    pub runtime_key: &'static str,
    pub program: String,
    pub args: Vec<String>,
    pub stdin: Vec<u8>,
    pub stdout_schema_json: Option<Value>,
    pub required_env_vars: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentResponseStatus {
    Success,
    Failed,
    Timeout,
}

type ResponseParseResult = Result<
    (
        orbit_common::types::AgentResponseEnvelope,
        AgentResponseStatus,
        orbit_common::types::InvocationTrace,
    ),
    OrbitError,
>;

type JsonMap = serde_json::Map<String, Value>;

#[derive(Default)]
struct ToolCallCollector {
    calls: Vec<ToolCallTrace>,
    by_id: std::collections::HashMap<String, usize>,
}
