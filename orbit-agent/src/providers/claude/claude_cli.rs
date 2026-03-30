use serde_json::Value;

use crate::providers::common::{
    build_envelope_schema, has_concrete_output_schema, render_prompt_with_embedded_envelope,
};

pub(crate) struct ClaudeCliTransport {
    model: Option<String>,
}

impl ClaudeCliTransport {
    pub(crate) fn new(model: Option<String>) -> Self {
        Self { model }
    }

    // Claude is prompt-in-stdin; operation metadata is embedded in the envelope,
    // so CLI args are identical for all operation types.
    pub(crate) fn args(&self, output_schema_json: Option<&Value>, verbose: bool) -> Vec<String> {
        let use_structured = has_concrete_output_schema(output_schema_json);

        let mut args = vec![
            "-p".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
            "--output-format".to_string(),
            // Always use "text" — "json" wraps the response in a CLI envelope
            // that breaks Orbit's own envelope parsing.
            "text".to_string(),
            "--no-session-persistence".to_string(),
        ];

        if verbose {
            args.push("--verbose".to_string());
        }

        if use_structured {
            let envelope_schema = build_envelope_schema(output_schema_json.unwrap());
            let schema_str = serde_json::to_string(&envelope_schema)
                .expect("envelope schema must serialize");
            args.push("--json-schema".to_string());
            args.push(schema_str);
        }

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
