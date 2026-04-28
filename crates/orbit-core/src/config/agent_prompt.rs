//! Interactive prompts that collect per-role agent settings during
//! `orbit init`. Outputs a map of `role → RawAgentRoleConfig` ready to hand
//! to the config writer (T20260428-9 AC #5–#6).
//!
//! I/O is gated by [`Prompter`] so unit tests can drive the collector with
//! canned answers without touching real stdin/stdout.

use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};

use super::agent_detect::{DetectedAgents, default_backend, default_model_for, default_provider};
use super::raw::RawAgentRoleConfig;

/// Roles asked about during `orbit init`. Order is intentional: it controls
/// the prompt sequence the user sees.
pub const ROLE_PROMPT_ORDER: &[&str] = &["reviewer", "implementer", "planner"];

/// Injectable seam for prompt I/O. Real CLI uses [`StdinPrompter`]; tests use
/// [`testing::CannedPrompter`].
pub trait Prompter {
    /// Display `label` with `default` shown in brackets, read a line, and
    /// return the trimmed user input. Empty input means "accept default" —
    /// the caller is responsible for substituting the default value when the
    /// returned string is empty.
    fn prompt(&mut self, label: &str, default: &str) -> io::Result<String>;
}

/// Real prompter: writes to stdout, reads a line from stdin.
pub struct StdinPrompter;

impl Prompter for StdinPrompter {
    fn prompt(&mut self, label: &str, default: &str) -> io::Result<String> {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        write!(out, "{label} [{default}]: ")?;
        out.flush()?;

        let stdin = io::stdin();
        let mut line = String::new();
        stdin.lock().read_line(&mut line)?;
        Ok(line.trim().to_string())
    }
}

/// Read provider/backend/model for each of the `ROLE_PROMPT_ORDER` roles and
/// return a map keyed by role name suitable for serializing as
/// `[agent.<role>]` blocks. Returned configs always carry `Some` values for
/// the three fields — empty input substitutes the detection-derived default.
///
/// Detection results seed the per-role defaults so most users can just press
/// Enter through the prompts.
pub fn collect_role_settings(
    detected: &DetectedAgents,
    prompter: &mut dyn Prompter,
) -> io::Result<BTreeMap<String, RawAgentRoleConfig>> {
    let mut out = BTreeMap::new();
    for role in ROLE_PROMPT_ORDER {
        let cfg = collect_one_role(role, detected, prompter)?;
        out.insert((*role).to_string(), cfg);
    }
    Ok(out)
}

fn collect_one_role(
    role: &str,
    detected: &DetectedAgents,
    prompter: &mut dyn Prompter,
) -> io::Result<RawAgentRoleConfig> {
    let provider_default = default_provider(detected);
    let provider_label = format!("agent.{role}.provider");
    let provider = take_or_default(
        prompter.prompt(&provider_label, provider_default)?,
        provider_default,
    );

    let backend_default = default_backend(&provider, detected);
    let backend_label = format!("agent.{role}.backend");
    let backend = take_or_default(
        prompter.prompt(&backend_label, backend_default)?,
        backend_default,
    );

    let model_default = default_model_for(&provider).unwrap_or("");
    let model_label = format!("agent.{role}.model");
    let model_input = prompter.prompt(&model_label, model_default)?;
    let model_value = take_or_default(model_input, model_default);
    let model_field = if model_value.is_empty() {
        None
    } else {
        Some(model_value)
    };

    Ok(RawAgentRoleConfig {
        provider: Some(provider),
        backend: Some(backend),
        model: model_field,
    })
}

fn take_or_default(input: String, default: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Canned-answer prompter used by unit tests in this crate and tests
    //! living in the same crate's `init` module.

    use super::Prompter;
    use std::collections::VecDeque;
    use std::io;

    /// Pops scripted answers off a queue. Returns an `UnexpectedEof` error
    /// when the queue runs dry so test failures point at the missing answer.
    #[derive(Debug, Default)]
    pub(crate) struct CannedPrompter {
        answers: VecDeque<String>,
    }

    impl CannedPrompter {
        pub(crate) fn new<I: IntoIterator<Item = &'static str>>(answers: I) -> Self {
            Self {
                answers: answers.into_iter().map(String::from).collect(),
            }
        }
    }

    impl Prompter for CannedPrompter {
        fn prompt(&mut self, label: &str, _default: &str) -> io::Result<String> {
            self.answers.pop_front().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("no canned answer for prompt `{label}`"),
                )
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::CannedPrompter;
    use super::*;

    #[test]
    fn empty_answers_accept_detection_defaults() {
        let detected = DetectedAgents {
            claude_cli: true,
            ..DetectedAgents::default()
        };
        // 3 roles × 3 prompts = 9 empty answers.
        let mut prompter = CannedPrompter::new(["", "", "", "", "", "", "", "", ""]);
        let result = collect_role_settings(&detected, &mut prompter).unwrap();

        let reviewer = result.get("reviewer").expect("reviewer entry");
        assert_eq!(reviewer.provider.as_deref(), Some("claude"));
        assert_eq!(reviewer.backend.as_deref(), Some("cli"));
        assert_eq!(reviewer.model.as_deref(), Some("claude-opus-4-7"));

        // Defaults flow uniformly across roles.
        let implementer = result.get("implementer").expect("implementer entry");
        assert_eq!(implementer.provider.as_deref(), Some("claude"));
        assert_eq!(implementer.backend.as_deref(), Some("cli"));
        assert_eq!(implementer.model.as_deref(), Some("claude-opus-4-7"));

        let planner = result.get("planner").expect("planner entry");
        assert_eq!(planner.provider.as_deref(), Some("claude"));
        assert_eq!(planner.backend.as_deref(), Some("cli"));
        assert_eq!(planner.model.as_deref(), Some("claude-opus-4-7"));
    }

    #[test]
    fn user_overrides_provider_then_backend_and_model_recompute() {
        // No CLIs detected → default provider would be claude/http. User
        // picks codex; backend default for codex (no CLI) is http; model
        // default becomes gpt-5.5 from the registry.
        let detected = DetectedAgents::default();
        let mut prompter = CannedPrompter::new([
            "codex", "", "", // reviewer: provider=codex, accept backend, accept model
            "", "", "", // implementer: accept claude/http/claude-opus-4-7
            "", "", "", // planner: accept claude/http/claude-opus-4-7
        ]);
        let result = collect_role_settings(&detected, &mut prompter).unwrap();

        let reviewer = result.get("reviewer").expect("reviewer entry");
        assert_eq!(reviewer.provider.as_deref(), Some("codex"));
        assert_eq!(reviewer.backend.as_deref(), Some("http"));
        assert_eq!(reviewer.model.as_deref(), Some("gpt-5.5"));

        let implementer = result.get("implementer").expect("implementer entry");
        assert_eq!(implementer.provider.as_deref(), Some("claude"));
        assert_eq!(implementer.backend.as_deref(), Some("http"));
        assert_eq!(implementer.model.as_deref(), Some("claude-opus-4-7"));
    }

    #[test]
    fn unknown_provider_yields_empty_model_when_user_accepts_default() {
        // Unknown provider has no MODEL_REGISTRY entry, so the default model
        // string is empty. Empty model means we omit the field entirely so
        // it stays out of the serialized TOML.
        let detected = DetectedAgents::default();
        let mut prompter = CannedPrompter::new(["openai_compat", "", "", "", "", "", "", "", ""]);
        let result = collect_role_settings(&detected, &mut prompter).unwrap();
        let reviewer = result.get("reviewer").expect("reviewer entry");
        assert_eq!(reviewer.provider.as_deref(), Some("openai_compat"));
        assert_eq!(reviewer.model, None);
    }

    #[test]
    fn user_supplies_custom_model_string() {
        let detected = DetectedAgents {
            claude_cli: true,
            ..DetectedAgents::default()
        };
        let mut prompter = CannedPrompter::new([
            "",
            "",
            "claude-haiku-4-5", // reviewer model override
            "",
            "",
            "",
            "",
            "",
            "",
        ]);
        let result = collect_role_settings(&detected, &mut prompter).unwrap();
        assert_eq!(
            result.get("reviewer").unwrap().model.as_deref(),
            Some("claude-haiku-4-5")
        );
        assert_eq!(
            result.get("implementer").unwrap().model.as_deref(),
            Some("claude-opus-4-7")
        );
    }
}
