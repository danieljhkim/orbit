//! Normalizing the Copilot CLI's JSONL agent-event stream into the bytes
//! Orbit's response/envelope contract is allowed to read. [ORB-10946]
//!
//! `copilot --output-format json` documents its stdout as "JSONL, one JSON
//! object per line". Every line is an agent-event frame:
//!
//! ```text
//! {"type":"session.warning","data":{…},"ephemeral":true,"id":"…","timestamp":"…"}
//! {"type":"assistant.message","data":{"content":"…"},"id":"…","timestamp":"…"}
//! ```
//!
//! Orbit's envelope finder searches a provider's stdout in reverse for a
//! recognizable envelope, descending through wrapper objects and JSON-encoded
//! strings. Handed a raw Copilot stream, that search is unsafe in one specific
//! way: Copilot echoes the *prompt* back as a `user.message` event, and every
//! Orbit prompt embeds a literal example envelope in its response contract
//! (`{"schemaVersion":1,"status":"success|failed|timeout",…}`). A run that
//! produced no model output at all still carries that echo, so the finder
//! could read Orbit's own instructions back as if they were the agent's
//! completion evidence.
//!
//! So this module reduces the stream to model-authored frames before any
//! protocol read. What survives is what Copilot itself produced; what is
//! dropped is Orbit's own prompt coming back and the session control plane.
//! When nothing survives — the auth-failure and cancellation shapes — the
//! result is empty, and an empty stdout carries no envelope, which is exactly
//! how a missing completion is meant to be reported.

/// Event-type prefixes whose frames are model output.
///
/// `assistant.` carries the agent's messages (`assistant.message`), reasoning,
/// and per-turn token accounting (`assistant.usage`) that Orbit's invocation
/// trace reads. `error.` carries the provider's own failure frames
/// (`error.exception`), which must reach the diagnostic path rather than be
/// silently discarded.
///
/// This is a prefix allowlist rather than an exact-type list so a new
/// `assistant.*` subtype in a later CLI release keeps flowing instead of being
/// dropped by a stale table — while `user.*` and `session.*`, the two families
/// that can replay Orbit's own prompt, stay excluded by construction.
const MODEL_OUTPUT_EVENT_PREFIXES: &[&str] = &["assistant.", "error."];

/// Reduce a Copilot JSONL stream to its model-authored frames.
///
/// Returns the retained frames as JSONL. Lines that are not valid JSON, frames
/// with no `type` string, and frames outside [`MODEL_OUTPUT_EVENT_PREFIXES`]
/// are dropped: a malformed or truncated stream degrades toward "no completion
/// evidence", never toward a false one.
pub(crate) fn normalize_copilot_stdout(stdout: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(stdout);
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if !is_model_output_frame(&value) {
            continue;
        }
        out.push_str(trimmed);
        out.push('\n');
    }
    out.into_bytes()
}

fn is_model_output_frame(value: &serde_json::Value) -> bool {
    value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|event_type| {
            MODEL_OUTPUT_EVENT_PREFIXES
                .iter()
                .any(|prefix| event_type.starts_with(prefix))
        })
}
