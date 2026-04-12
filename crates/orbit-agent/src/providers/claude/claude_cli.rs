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

    // Claude is prompt-in-stdin; operation metadata is embedded in the envelope,
    // so CLI args are identical for all operation types.
    pub(crate) fn args(&self, verbose: bool) -> Vec<String> {
        let mut args = vec![
            "-p".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
            "--output-format".to_string(),
            "json".to_string(),
            "--no-session-persistence".to_string(),
        ];

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

#[cfg(test)]
mod tests {
    use super::ClaudeCliTransport;

    #[test]
    fn args_translate_orbit_canonical_claude_models_to_cli_ids() {
        let cli = ClaudeCliTransport::new(Some("opus-4.6".to_string()));
        let args = cli.args(false);
        assert_eq!(
            args.windows(2)
                .find(|pair| pair[0] == "--model")
                .map(|pair| pair[1].as_str()),
            Some("claude-opus-4-6")
        );
        assert_eq!(cli.model_name(), Some("opus-4.6"));
    }

    #[test]
    fn args_leave_alias_models_unchanged() {
        let cli = ClaudeCliTransport::new(Some("opus".to_string()));
        let args = cli.args(false);
        assert_eq!(
            args.windows(2)
                .find(|pair| pair[0] == "--model")
                .map(|pair| pair[1].as_str()),
            Some("opus")
        );
    }
}
