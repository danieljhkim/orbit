use crate::providers::common::render_prompt_with_embedded_envelope;
use crate::types::AgentOperation;

pub(crate) struct CodexCliTransport {
    model: Option<String>,
    sandbox: Option<String>,
    approval_policy: Option<String>,
}

impl CodexCliTransport {
    pub(crate) fn new(
        model: Option<String>,
        sandbox: Option<String>,
        approval_policy: Option<String>,
    ) -> Self {
        Self {
            model,
            sandbox,
            approval_policy,
        }
    }

    pub(crate) fn args(&self, _operation: &AgentOperation) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(approval_policy) = &self.approval_policy {
            args.push("--ask-for-approval".to_string());
            args.push(approval_policy.clone());
        }
        args.push("exec".to_string());
        if let Some(model) = &self.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }
        args.push("--sandbox".to_string());
        args.push(
            self.sandbox
                .clone()
                .unwrap_or_else(|| "workspace-write".to_string()),
        );
        args
    }

    pub(crate) fn stdin(&self, envelope_json: &[u8]) -> Vec<u8> {
        render_prompt_with_embedded_envelope(envelope_json)
    }
}
