use crate::providers::common::render_prompt_with_embedded_envelope;

fn codex_config_string_arg(key: &str, value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("{key}=\"{escaped}\"")
}

pub(crate) struct CodexCliTransport {
    model: Option<String>,
    sandbox: String,
    approval_policy: Option<String>,
    writable_dirs: Vec<String>,
}

impl CodexCliTransport {
    pub(crate) fn new(
        model: Option<String>,
        sandbox: String,
        approval_policy: Option<String>,
        writable_dirs: Vec<String>,
    ) -> Self {
        Self {
            model,
            sandbox,
            approval_policy,
            writable_dirs,
        }
    }

    // Static Codex CLI flags live in the executor definition; this transport
    // only adds per-request toggles.
    pub(crate) fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(approval_policy) = &self.approval_policy {
            args.push("--config".to_string());
            args.push(codex_config_string_arg("approval_policy", approval_policy));
        }
        if let Some(model) = &self.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }
        args.push("--sandbox".to_string());
        args.push(self.sandbox.clone());
        for dir in &self.writable_dirs {
            args.push("--add-dir".to_string());
            args.push(dir.clone());
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
