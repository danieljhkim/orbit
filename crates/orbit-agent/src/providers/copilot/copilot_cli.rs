use crate::providers::common::render_prompt_with_embedded_envelope;

/// Per-request command construction for the standalone GitHub Copilot CLI.
///
/// Static flags (`--allow-all-tools`, `--no-ask-user`, `--output-format json`,
/// …) live in the shipped `copilot` executor definition; this transport only
/// adds what varies per request. [ORB-10946]
pub(crate) struct CopilotCliTransport {
    model: Option<String>,
}

impl CopilotCliTransport {
    pub(crate) fn new(model: Option<String>) -> Self {
        Self { model }
    }

    /// Explicit model selection only. Copilot would otherwise fall back to
    /// `COPILOT_MODEL` or its own persisted `/model` choice, which would make
    /// the model actually used by a run depend on ambient operator state
    /// rather than on the crew assignment Orbit resolved.
    pub(crate) fn args(&self) -> Vec<String> {
        let mut args = Vec::new();

        if let Some(model) = &self.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }

        args
    }

    /// The prompt travels on **stdin**, never as a `-p <text>` argument.
    ///
    /// `copilot` documents both transports — its own no-prompt error reads
    /// "provide a prompt with -p or via standard in" — and stdin is the one
    /// that keeps the Orbit execution envelope out of argv. Argv is visible to
    /// every process on the host, is recorded in Orbit's audit argv, and is
    /// echoed back in spawn-failure messages; the envelope carries task
    /// context and instructions, so it must not travel there. [ORB-10946]
    pub(crate) fn stdin(&self, envelope_json: &[u8]) -> Vec<u8> {
        render_prompt_with_embedded_envelope(envelope_json)
    }

    pub(crate) fn model_name(&self) -> Option<&str> {
        self.model.as_deref()
    }
}
