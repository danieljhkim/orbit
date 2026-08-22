//! Cursor's `--print --output-format json` stdout adapter. [ORB-10945]
//!
//! A successful CLI invocation emits one JSON object with terminal evidence:
//! `type=result`, `subtype=success`, `is_error=false`, and a model-authored
//! `result` string. Failures exit non-zero and need not emit JSON. Orbit must
//! validate the outer frame before searching the inner text for its response
//! envelope; otherwise a malformed or partial provider response could be
//! mistaken for successful completion.

/// Return the model-authored response only when Cursor emitted its complete,
/// documented success wrapper. Any malformed, failed, or partial shape
/// normalizes to empty bytes and therefore cannot satisfy Orbit's completion
/// contract.
pub(crate) fn normalize_cursor_stdout(stdout: &[u8]) -> Vec<u8> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(stdout) else {
        return Vec::new();
    };
    let is_success = value.get("type").and_then(serde_json::Value::as_str) == Some("result")
        && value.get("subtype").and_then(serde_json::Value::as_str) == Some("success")
        && value.get("is_error").and_then(serde_json::Value::as_bool) == Some(false);
    if !is_success {
        return Vec::new();
    }
    value
        .get("result")
        .and_then(serde_json::Value::as_str)
        .map_or_else(Vec::new, |result| result.as_bytes().to_vec())
}
