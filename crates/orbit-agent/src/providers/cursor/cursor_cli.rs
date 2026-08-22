use crate::providers::common::render_prompt_with_embedded_envelope;

/// Per-request command construction for Cursor Agent CLI.
///
/// The shipped executor owns the static headless flags; this transport adds
/// only the model chosen by Orbit's crew resolution. [ORB-10945]
pub(crate) struct CursorCliTransport {
    model: Option<String>,
}

impl CursorCliTransport {
    pub(crate) fn new(model: Option<String>) -> Self {
        Self { model }
    }

    pub(crate) fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(model) = &self.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }
        args
    }

    /// Cursor accepts a prompt from piped stdin in print mode. Keeping the
    /// execution envelope off argv prevents task context from entering process
    /// listings, audit argv, or spawn diagnostics.
    pub(crate) fn stdin(&self, envelope_json: &[u8]) -> Vec<u8> {
        render_prompt_with_embedded_envelope(envelope_json)
    }

    pub(crate) fn model_name(&self) -> Option<&str> {
        self.model.as_deref()
    }
}
