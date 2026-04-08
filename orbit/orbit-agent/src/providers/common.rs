use tracing::debug;

pub(crate) fn render_prompt_with_embedded_envelope(envelope_json: &[u8]) -> Vec<u8> {
    debug!(
        envelope_bytes = envelope_json.len(),
        "constructed embedded Orbit execution prompt"
    );
    let prompt = "You are Orbit's agent executor.\n\
Read the execution envelope JSON and perform the requested work.\n\
Return exactly one JSON object and nothing else.\n\
Required response schema:\n\
{\"schemaVersion\":1,\"status\":\"success|failed|timeout\",\"result\":{...},\"error\":null}\n\
Rules:\n\
- Output valid JSON only. No markdown fences. No explanatory text.\n\
- result MUST be a JSON object (never null, never omitted). It may be {} for side-effect-only activities.\n\
- If execution cannot complete, return status=\"failed\" with non-empty error.code and error.message (result may be {}).\n\
- Persist meaningful state via task artifacts (orbit.task.update), not via the result object.";
    let envelope_text = String::from_utf8_lossy(envelope_json);
    format!("{prompt}\nExecution envelope:\n{envelope_text}\n").into_bytes()
}
