use orbit_types::InvocationTrace;
use serde_json::Value;

use super::{tool_calls::extract_tool_calls, usage::sum_usage};

pub(super) fn extract_invocation_trace(documents: &[Value], duration_ms: u64) -> InvocationTrace {
    let usage = sum_usage(documents);
    let tool_calls = extract_tool_calls(documents);
    InvocationTrace {
        usage,
        tool_calls,
        duration_ms,
    }
}
