use crate::providers::common::render_prompt_with_embedded_envelope;
use crate::types::AgentOperation;

pub(crate) struct ClaudeCliTransport {
    model: Option<String>,
}

impl ClaudeCliTransport {
    pub(crate) fn new(model: Option<String>) -> Self {
        Self { model }
    }

    pub(crate) fn args(&self, _operation: &AgentOperation) -> Vec<String> {
        let mut args = vec![
            "-p".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
            "--output-format".to_string(),
            "text".to_string(),
            "--no-session-persistence".to_string(),
        ];
        if let Some(model) = &self.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }
        args
    }

    pub(crate) fn stdin(&self, envelope_json: &[u8]) -> Vec<u8> {
        render_prompt_with_embedded_envelope(envelope_json)
    }
}
