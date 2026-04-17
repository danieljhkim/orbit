use crate::providers::common::render_prompt_with_embedded_envelope;

fn claude_cli_model_arg(model: &str) -> String {
    let trimmed = model.trim();
    if let Some(version) = trimmed.strip_prefix("opus-") {
        return format!("claude-opus-{}", version.replace('.', "-"));
    }
    if let Some(version) = trimmed.strip_prefix("sonnet-") {
        return format!("claude-sonnet-{}", version.replace('.', "-"));
    }
    trimmed.to_string()
}

pub(crate) struct ClaudeCliTransport {
    model: Option<String>,
}

impl ClaudeCliTransport {
    pub(crate) fn new(model: Option<String>) -> Self {
        Self { model }
    }

    // Static Claude CLI flags live in the executor definition; this transport
    // only adds per-request toggles.
    pub(crate) fn args(&self, verbose: bool) -> Vec<String> {
        let mut args = Vec::new();

        if verbose {
            args.push("--verbose".to_string());
        }

        if let Some(model) = &self.model {
            args.push("--model".to_string());
            args.push(claude_cli_model_arg(model));
        }
        args
    }

    pub(crate) fn stdin(&self, envelope_json: &[u8]) -> Vec<u8> {
        render_prompt_with_embedded_envelope(envelope_json)
    }

    pub(crate) fn model_name(&self) -> Option<&str> {
        self.model.as_deref()
    }
}
