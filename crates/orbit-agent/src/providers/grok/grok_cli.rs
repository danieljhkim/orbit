use crate::providers::common::render_prompt_with_embedded_envelope;

pub(crate) struct GrokCliTransport {
    model: Option<String>,
}

impl GrokCliTransport {
    pub(crate) fn new(model: Option<String>) -> Self {
        Self { model }
    }

    // Static Grok CLI flags live in the executor definition; this transport
    // only adds per-request toggles.
    pub(crate) fn args(&self) -> Vec<String> {
        let mut args = Vec::new();

        if let Some(model) = &self.model {
            args.push("--model".to_string());
            args.push(model.clone());
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

#[cfg(test)]
mod tests;
