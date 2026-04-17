#[path = "response/envelope.rs"]
mod envelope;
#[path = "response/tool_calls.rs"]
mod tool_calls;
#[path = "response/trace.rs"]
mod trace;
#[path = "response/usage.rs"]
mod usage;

use orbit_types::{OrbitError, ToolCallTrace};
use serde_json::Value;

pub use envelope::{is_timeout, parse_and_validate_response};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentResponse {
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
        orbit_types::AgentResponseEnvelope,
        AgentResponseStatus,
        orbit_types::InvocationTrace,
    ),
    OrbitError,
>;

type JsonMap = serde_json::Map<String, Value>;

#[derive(Default)]
struct ToolCallCollector {
    calls: Vec<ToolCallTrace>,
    by_id: std::collections::HashMap<String, usize>,
}
