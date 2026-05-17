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

const ROLE_DESCRIPTIONS: &[(&str, &str)] = &[
    ("Reviewer", "checks changes and leaves feedback"),
    ("Implementer", "writes code and applies fixes"),
    ("Planner", "drafts plans and decomposes tasks"),
];

/// Injectable seam for prompt I/O. Real CLI uses [`StdinPrompter`]; tests use
/// `testing::CannedPrompter`.
pub trait Prompter {
    /// Display non-interactive text to the user.
    fn message(&mut self, text: &str) -> io::Result<()>;

    /// Display `prompt`, read a line, and return the trimmed user input.
    fn prompt(&mut self, prompt: &str) -> io::Result<String>;
}

/// Real prompter: writes to stdout, reads a line from stdin.
pub struct StdinPrompter;

impl Prompter for StdinPrompter {
    fn message(&mut self, text: &str) -> io::Result<()> {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        writeln!(out, "{text}")?;
        Ok(())
    }

    fn prompt(&mut self, prompt: &str) -> io::Result<String> {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        write!(out, "{prompt}")?;
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
/// provider/backend/model values. Crew-based config requires a concrete model
/// for every role, so custom providers re-prompt until a model is supplied.
///
/// Detection results seed the per-role defaults so most users can just press
/// Enter once to accept the recommended setup.
pub fn collect_role_settings(
    detected: &DetectedAgents,
    prompter: &mut dyn Prompter,
) -> io::Result<BTreeMap<String, RawAgentRoleConfig>> {
    let mut settings = recommended_role_settings(detected);

    prompter.message(&intro_text(detected, &settings))?;

    if yes_by_default(&prompter.prompt("Use this setup? [Y/n]: ")?) {
        return Ok(settings);
    }

    loop {
        let role = prompter
            .prompt("Which role do you want to change? [reviewer/implementer/planner/done]: ")?;
        let role = role.trim().to_ascii_lowercase();
        if role.is_empty() || role == "done" || role == "d" {
            break;
        }
        if !ROLE_PROMPT_ORDER.contains(&role.as_str()) {
            prompter.message("Please enter one of: reviewer, implementer, planner, or done.")?;
            continue;
        }

        let cfg = collect_one_role(&role, detected, prompter)?;
        settings.insert(role, cfg);
        prompter.message(&format_role_summary("Updated setup:", &settings))?;
    }

    Ok(settings)
}

fn collect_one_role(
    role: &str,
    detected: &DetectedAgents,
    prompter: &mut dyn Prompter,
) -> io::Result<RawAgentRoleConfig> {
    let options = agent_options(role, detected);
    prompter.message(&format_agent_options(role, &options))?;

    loop {
        let choice = prompter.prompt("Choice [1]: ")?;
        let choice = choice.trim();
        if choice.eq_ignore_ascii_case("custom") || choice.eq_ignore_ascii_case("c") {
            return collect_custom_role(role, detected, prompter);
        }
        if choice
            .parse::<usize>()
            .is_ok_and(|n| n == options.len() + 1)
        {
            return collect_custom_role(role, detected, prompter);
        }

        let selected = if choice.is_empty() {
            Some(0)
        } else {
            choice.parse::<usize>().ok().and_then(|n| n.checked_sub(1))
        };

        if let Some(index) = selected
            && let Some(option) = options.get(index)
        {
            let model = collect_model_override(option.model, prompter)?;
            return Ok(RawAgentRoleConfig {
                provider: Some(option.provider.to_string()),
                backend: Some(option.backend.to_string()),
                model,
            });
        }

        let custom_index = options.len() + 1;
        prompter.message(&format!(
            "Please enter 1-{custom_index}, or `custom` for a manual provider."
        ))?;
    }
}

fn collect_custom_role(
    role: &str,
    detected: &DetectedAgents,
    prompter: &mut dyn Prompter,
) -> io::Result<RawAgentRoleConfig> {
    let (provider_default, _) = recommended_provider_backend_for_role(role, detected);
    let provider = take_or_default(
        prompter.prompt(&format!("Provider [{provider_default}]: "))?,
        provider_default,
    );
    let backend_default = default_backend(&provider, detected);
    let backend = take_or_default(
        prompter.prompt(&format!("Backend (cli/http) [{backend_default}]: "))?,
        backend_default,
    );
    let model_default = default_model_for(&provider).unwrap_or("");
    let model = collect_model_override(model_default, prompter)?;

    Ok(RawAgentRoleConfig {
        provider: Some(provider),
        backend: Some(backend),
        model,
    })
}

fn collect_model_override(
    model_default: &str,
    prompter: &mut dyn Prompter,
) -> io::Result<Option<String>> {
    let prompt = if model_default.is_empty() {
        "Model: ".to_string()
    } else {
        format!("Model [{model_default}]: ")
    };
    loop {
        let model_value = take_or_default(prompter.prompt(&prompt)?, model_default);
        if !model_value.is_empty() {
            return Ok(Some(model_value));
        }
        prompter.message("Model is required for crew role assignments.")?;
    }
}

fn take_or_default(input: String, default: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

fn yes_by_default(input: &str) -> bool {
    let trimmed = input.trim();
    trimmed.is_empty() || trimmed.eq_ignore_ascii_case("y") || trimmed.eq_ignore_ascii_case("yes")
}

fn recommended_role_settings(detected: &DetectedAgents) -> BTreeMap<String, RawAgentRoleConfig> {
    let mut out = BTreeMap::new();
    for role in ROLE_PROMPT_ORDER {
        let (provider, backend) = recommended_provider_backend_for_role(role, detected);
        let model = default_model_for(provider).map(str::to_string);
        out.insert(
            (*role).to_string(),
            RawAgentRoleConfig {
                provider: Some(provider.to_string()),
                backend: Some(backend.to_string()),
                model: model.clone(),
            },
        );
    }
    out
}

fn recommended_provider_backend_for_role(
    role: &str,
    detected: &DetectedAgents,
) -> (&'static str, &'static str) {
    let preferred = match role {
        "reviewer" | "implementer" => codex_surface(detected),
        "planner" => claude_surface(detected),
        _ => None,
    };

    preferred.unwrap_or_else(|| {
        let provider = default_provider(detected);
        (provider, default_backend(provider, detected))
    })
}

fn codex_surface(detected: &DetectedAgents) -> Option<(&'static str, &'static str)> {
    if detected.codex_cli {
        Some(("codex", "cli"))
    } else if detected.openai_api_key {
        Some(("codex", "http"))
    } else {
        None
    }
}

fn claude_surface(detected: &DetectedAgents) -> Option<(&'static str, &'static str)> {
    if detected.claude_cli {
        Some(("claude", "cli"))
    } else if detected.anthropic_api_key {
        Some(("claude", "http"))
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentOption {
    label: &'static str,
    provider: &'static str,
    backend: &'static str,
    model: &'static str,
}

fn agent_options(role: &str, detected: &DetectedAgents) -> Vec<AgentOption> {
    let mut options = Vec::new();

    if detected.claude_cli {
        options.push(agent_option("Claude CLI", "claude", "cli"));
    }
    if detected.codex_cli {
        options.push(agent_option("Codex CLI", "codex", "cli"));
    }
    if detected.gemini_cli {
        options.push(agent_option("Gemini CLI", "gemini", "cli"));
    }
    if detected.grok_cli {
        options.push(agent_option("Grok CLI", "grok", "cli"));
    }
    if detected.ollama_cli {
        options.push(agent_option("Ollama CLI", "ollama", "cli"));
    }
    if detected.anthropic_api_key {
        options.push(agent_option("Claude API", "claude", "http"));
    }
    if detected.openai_api_key {
        options.push(agent_option("Codex API", "codex", "http"));
    }
    if detected.gemini_api_key {
        options.push(agent_option("Gemini API", "gemini", "http"));
    }

    let (provider, backend) = recommended_provider_backend_for_role(role, detected);
    if let Some(index) = options
        .iter()
        .position(|option| option.provider == provider && option.backend == backend)
    {
        let recommended = options.remove(index);
        options.insert(0, recommended);
    } else {
        options.insert(
            0,
            agent_option(agent_label(provider, backend), provider, backend),
        );
    }

    options
}

fn agent_option(label: &'static str, provider: &'static str, backend: &'static str) -> AgentOption {
    AgentOption {
        label,
        provider,
        backend,
        model: default_model_for(provider).unwrap_or(""),
    }
}

fn agent_label(provider: &str, backend: &str) -> &'static str {
    match (provider, backend) {
        ("claude", "cli") => "Claude CLI",
        ("claude", "http") => "Claude API",
        ("codex", "cli") => "Codex CLI",
        ("codex", "http") => "Codex API",
        ("gemini", "cli") => "Gemini CLI",
        ("gemini", "http") => "Gemini API",
        ("grok", "cli") => "Grok CLI",
        ("grok", "http") => "Grok API",
        ("ollama", "cli") => "Ollama CLI",
        ("ollama", "http") => "Ollama API",
        _ => "Custom agent",
    }
}

fn intro_text(
    detected: &DetectedAgents,
    settings: &BTreeMap<String, RawAgentRoleConfig>,
) -> String {
    format!(
        "Orbit uses agents for three workflow roles:\n\n{}\n\nDetected agents:\n{}\n\n{}",
        role_description_lines(),
        detection_lines(detected),
        format_role_summary("Recommended setup:", settings)
    )
}

fn role_description_lines() -> String {
    ROLE_DESCRIPTIONS
        .iter()
        .map(|(role, description)| format!("  {role:<12} {description}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn detection_lines(detected: &DetectedAgents) -> String {
    [
        ("Claude CLI", detected.claude_cli),
        ("Codex CLI", detected.codex_cli),
        ("Gemini CLI", detected.gemini_cli),
        ("Grok CLI", detected.grok_cli),
        ("Ollama CLI", detected.ollama_cli),
        ("ANTHROPIC_API_KEY", detected.anthropic_api_key),
        ("OPENAI_API_KEY", detected.openai_api_key),
        ("GEMINI_API_KEY", detected.gemini_api_key),
    ]
    .into_iter()
    .map(|(label, found)| {
        let status = if found { "found" } else { "not found" };
        format!("  {label:<18} {status}")
    })
    .collect::<Vec<_>>()
    .join("\n")
}

fn format_role_summary(title: &str, settings: &BTreeMap<String, RawAgentRoleConfig>) -> String {
    let mut lines = vec![
        title.to_string(),
        String::new(),
        format!("  {:<13} {:<28} {}", "Role", "Agent", "Model"),
    ];

    for role in ROLE_PROMPT_ORDER {
        if let Some(cfg) = settings.get(*role) {
            lines.push(format!(
                "  {:<13} {:<28} {}",
                title_case_role(role),
                agent_display_name(cfg),
                cfg.model.as_deref().unwrap_or("(not set)")
            ));
        }
    }

    lines.join("\n")
}

fn format_agent_options(role: &str, options: &[AgentOption]) -> String {
    let mut lines = vec![format!("Choose an agent for {}:", title_case_role(role))];
    lines.push(String::new());
    for (index, option) in options.iter().enumerate() {
        let model = if option.model.is_empty() {
            "(model not set)"
        } else {
            option.model
        };
        lines.push(format!("  {}. {:<16} {}", index + 1, option.label, model));
    }
    lines.push(format!("  {}. Custom", options.len() + 1));
    lines.join("\n")
}

fn agent_display_name(cfg: &RawAgentRoleConfig) -> String {
    let provider = cfg.provider.as_deref().unwrap_or("custom");
    let backend = cfg.backend.as_deref().unwrap_or("custom");
    let label = agent_label(provider, backend);
    if label == "Custom agent" {
        format!("{provider} ({backend})")
    } else {
        label.to_string()
    }
}

fn title_case_role(role: &str) -> &'static str {
    match role {
        "reviewer" => "Reviewer",
        "implementer" => "Implementer",
        "planner" => "Planner",
        _ => "Role",
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
        messages: Vec<String>,
        prompts: Vec<String>,
    }

    impl CannedPrompter {
        pub(crate) fn new<I: IntoIterator<Item = &'static str>>(answers: I) -> Self {
            Self {
                answers: answers.into_iter().map(String::from).collect(),
                messages: Vec::new(),
                prompts: Vec::new(),
            }
        }

        pub(crate) fn transcript(&self) -> String {
            self.messages
                .iter()
                .chain(self.prompts.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    impl Prompter for CannedPrompter {
        fn message(&mut self, text: &str) -> io::Result<()> {
            self.messages.push(text.to_string());
            Ok(())
        }

        fn prompt(&mut self, prompt: &str) -> io::Result<String> {
            self.prompts.push(prompt.to_string());
            self.answers.pop_front().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("no canned answer for prompt `{prompt}`"),
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
    fn empty_answer_accepts_role_aware_recommended_setup() {
        let detected = DetectedAgents {
            claude_cli: true,
            codex_cli: true,
            ..DetectedAgents::default()
        };
        let mut prompter = CannedPrompter::new([""]);
        let result = collect_role_settings(&detected, &mut prompter).unwrap();

        let reviewer = result.get("reviewer").expect("reviewer entry");
        assert_eq!(reviewer.provider.as_deref(), Some("codex"));
        assert_eq!(reviewer.backend.as_deref(), Some("cli"));
        assert_eq!(reviewer.model.as_deref(), Some("gpt-5.5"));

        let implementer = result.get("implementer").expect("implementer entry");
        assert_eq!(implementer.provider.as_deref(), Some("codex"));
        assert_eq!(implementer.backend.as_deref(), Some("cli"));
        assert_eq!(implementer.model.as_deref(), Some("gpt-5.5"));

        let planner = result.get("planner").expect("planner entry");
        assert_eq!(planner.provider.as_deref(), Some("claude"));
        assert_eq!(planner.backend.as_deref(), Some("cli"));
        assert_eq!(planner.model.as_deref(), Some("claude-opus-4-7"));

        let transcript = prompter.transcript();
        assert!(transcript.contains("Orbit uses agents for three workflow roles"));
        assert!(transcript.contains("Recommended setup:"));
        assert!(transcript.contains("Use this setup? [Y/n]: "));
    }

    #[test]
    fn claude_only_detection_still_recommends_claude_for_all_roles() {
        let detected = DetectedAgents {
            claude_cli: true,
            ..DetectedAgents::default()
        };
        let mut prompter = CannedPrompter::new([""]);
        let result = collect_role_settings(&detected, &mut prompter).unwrap();

        let reviewer = result.get("reviewer").expect("reviewer entry");
        assert_eq!(reviewer.provider.as_deref(), Some("claude"));
        assert_eq!(reviewer.backend.as_deref(), Some("cli"));
        assert_eq!(reviewer.model.as_deref(), Some("claude-opus-4-7"));

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
    fn customization_enter_selects_role_recommendation() {
        let detected = DetectedAgents {
            claude_cli: true,
            codex_cli: true,
            ..DetectedAgents::default()
        };
        let mut prompter = CannedPrompter::new(["n", "reviewer", "", "", ""]);
        let result = collect_role_settings(&detected, &mut prompter).unwrap();

        let reviewer = result.get("reviewer").expect("reviewer entry");
        assert_eq!(reviewer.provider.as_deref(), Some("codex"));
        assert_eq!(reviewer.backend.as_deref(), Some("cli"));
        assert_eq!(reviewer.model.as_deref(), Some("gpt-5.5"));

        let implementer = result.get("implementer").expect("implementer entry");
        assert_eq!(implementer.provider.as_deref(), Some("codex"));
        assert_eq!(implementer.backend.as_deref(), Some("cli"));
        assert_eq!(implementer.model.as_deref(), Some("gpt-5.5"));

        let planner = result.get("planner").expect("planner entry");
        assert_eq!(planner.provider.as_deref(), Some("claude"));
        assert_eq!(planner.backend.as_deref(), Some("cli"));
        assert_eq!(planner.model.as_deref(), Some("claude-opus-4-7"));

        let transcript = prompter.transcript();
        assert!(transcript.contains("Choose an agent for Reviewer:"));
        assert!(transcript.contains("  1. Codex CLI"));
        assert!(transcript.contains("Updated setup:"));
    }

    #[test]
    fn custom_provider_prompts_for_backend_and_model() {
        let detected = DetectedAgents::default();
        let mut prompter = CannedPrompter::new([
            "n",
            "reviewer",
            "custom",
            "openai_compat",
            "http",
            "my-model",
            "",
        ]);
        let result = collect_role_settings(&detected, &mut prompter).unwrap();
        let reviewer = result.get("reviewer").expect("reviewer entry");
        assert_eq!(reviewer.provider.as_deref(), Some("openai_compat"));
        assert_eq!(reviewer.backend.as_deref(), Some("http"));
        assert_eq!(reviewer.model.as_deref(), Some("my-model"));

        let implementer = result.get("implementer").expect("implementer entry");
        assert_eq!(implementer.provider.as_deref(), Some("claude"));
        assert_eq!(implementer.backend.as_deref(), Some("http"));
        assert_eq!(implementer.model.as_deref(), Some("claude-opus-4-7"));
    }

    #[test]
    fn custom_provider_reprompts_for_blank_unknown_model() {
        let detected = DetectedAgents::default();
        let mut prompter = CannedPrompter::new([
            "n",
            "reviewer",
            "custom",
            "openai_compat",
            "http",
            "",
            "my-model",
            "",
        ]);
        let result = collect_role_settings(&detected, &mut prompter).unwrap();
        let reviewer = result.get("reviewer").expect("reviewer entry");
        assert_eq!(reviewer.provider.as_deref(), Some("openai_compat"));
        assert_eq!(reviewer.backend.as_deref(), Some("http"));
        assert_eq!(reviewer.model.as_deref(), Some("my-model"));
        assert!(
            prompter
                .transcript()
                .contains("Model is required for crew role assignments.")
        );
    }
}
