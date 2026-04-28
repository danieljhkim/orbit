//! Agent environment detection used to seed `orbit init` prompt defaults.
//!
//! Probes which agent CLIs are on `PATH` and which provider API keys are set
//! in the environment, then derives a sensible default `(provider, backend,
//! model)` tuple for each role. The detection layer is gated by
//! [`AgentEnvProbe`] so unit tests can simulate environments without touching
//! real `PATH` or env vars (T20260428-9 AC #2).
//!
//! Only the writer path uses these helpers in this task. The follow-up
//! T20260428-12 reuses the probe trait when wiring resolution at dispatch
//! time.

use std::env;
use std::path::PathBuf;

/// Injectable seam for probing the host environment. Real code uses
/// [`RealAgentEnvProbe`]; tests construct [`MockAgentEnvProbe`].
pub trait AgentEnvProbe {
    /// Returns true when an executable named `name` is found on `PATH`.
    fn binary_on_path(&self, name: &str) -> bool;

    /// Returns the value of an environment variable, or `None` if unset.
    fn env_var(&self, name: &str) -> Option<String>;
}

/// Real probe: walks the process `PATH` manually (no extra crate dep) and
/// reads from `std::env`.
pub struct RealAgentEnvProbe;

impl AgentEnvProbe for RealAgentEnvProbe {
    fn binary_on_path(&self, name: &str) -> bool {
        let Some(path_var) = env::var_os("PATH") else {
            return false;
        };
        for dir in env::split_paths(&path_var) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            let candidate: PathBuf = dir.join(name);
            if is_executable_file(&candidate) {
                return true;
            }
            // On Windows the binary may have an extension. Orbit only ships on
            // Unix today, but this keeps the detector honest if that changes.
            #[cfg(windows)]
            for ext in ["exe", "cmd", "bat"] {
                let mut with_ext = candidate.clone();
                with_ext.set_extension(ext);
                if is_executable_file(&with_ext) {
                    return true;
                }
            }
        }
        false
    }

    fn env_var(&self, name: &str) -> Option<String> {
        env::var(name).ok()
    }
}

#[cfg(unix)]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && (meta.permissions().mode() & 0o111) != 0,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable_file(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file())
        .unwrap_or(false)
}

/// Snapshot of which agent surfaces are available. Each field is independent;
/// detection treats CLI presence and API-key presence as orthogonal so the
/// prompt-default logic can prefer CLI-backed providers while still honouring
/// HTTP-only setups.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetectedAgents {
    pub claude_cli: bool,
    pub codex_cli: bool,
    pub gemini_cli: bool,
    pub ollama_cli: bool,
    pub anthropic_api_key: bool,
    pub openai_api_key: bool,
    pub gemini_api_key: bool,
}

/// Probe the host environment using `probe` and return a [`DetectedAgents`]
/// snapshot.
pub fn detect(probe: &dyn AgentEnvProbe) -> DetectedAgents {
    DetectedAgents {
        claude_cli: probe.binary_on_path("claude"),
        codex_cli: probe.binary_on_path("codex"),
        gemini_cli: probe.binary_on_path("gemini"),
        ollama_cli: probe.binary_on_path("ollama"),
        anthropic_api_key: probe.env_var("ANTHROPIC_API_KEY").is_some(),
        openai_api_key: probe.env_var("OPENAI_API_KEY").is_some(),
        gemini_api_key: probe.env_var("GEMINI_API_KEY").is_some(),
    }
}

/// Hardcoded "latest known good" model per provider. Returned to seed prompt
/// defaults; users can override at the prompt. Update this map when new
/// flagship models ship.
pub fn default_model_for(provider: &str) -> Option<&'static str> {
    match provider {
        "claude" => Some("claude-opus-4-7"),
        "codex" => Some("gpt-5.5"),
        "gemini" => Some("gemini-3-pro"),
        _ => None,
    }
}

/// Pick a default provider for the role given a detection snapshot.
///
/// Preference order: first detected CLI in [claude, codex, gemini, ollama],
/// else first detected API key in [anthropic→claude, openai→codex,
/// gemini→gemini], else `claude` as a last resort.
pub fn default_provider(detected: &DetectedAgents) -> &'static str {
    if detected.claude_cli {
        return "claude";
    }
    if detected.codex_cli {
        return "codex";
    }
    if detected.gemini_cli {
        return "gemini";
    }
    if detected.ollama_cli {
        return "ollama";
    }
    if detected.anthropic_api_key {
        return "claude";
    }
    if detected.openai_api_key {
        return "codex";
    }
    if detected.gemini_api_key {
        return "gemini";
    }
    "claude"
}

/// Decide the default backend for a chosen provider. CLI when the matching
/// CLI binary is detected, otherwise HTTP.
pub fn default_backend(provider: &str, detected: &DetectedAgents) -> &'static str {
    let cli_present = match provider {
        "claude" => detected.claude_cli,
        "codex" => detected.codex_cli,
        "gemini" => detected.gemini_cli,
        "ollama" => detected.ollama_cli,
        _ => false,
    };
    if cli_present { "cli" } else { "http" }
}

#[cfg(test)]
pub(crate) mod testing {
    //! In-crate test double exposed at `pub(crate)` so the `init` integration
    //! tests can reuse it without copying the implementation.

    use super::AgentEnvProbe;
    use std::collections::{HashMap, HashSet};

    /// Test double with seedable PATH and env-var results.
    #[derive(Debug, Default, Clone)]
    pub(crate) struct MockAgentEnvProbe {
        binaries: HashSet<String>,
        env: HashMap<String, String>,
    }

    impl MockAgentEnvProbe {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        pub(crate) fn with_binary(mut self, name: &str) -> Self {
            self.binaries.insert(name.to_string());
            self
        }

        pub(crate) fn with_env(mut self, name: &str, value: &str) -> Self {
            self.env.insert(name.to_string(), value.to_string());
            self
        }
    }

    impl AgentEnvProbe for MockAgentEnvProbe {
        fn binary_on_path(&self, name: &str) -> bool {
            self.binaries.contains(name)
        }

        fn env_var(&self, name: &str) -> Option<String> {
            self.env.get(name).cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::MockAgentEnvProbe;
    use super::*;

    #[test]
    fn detect_reflects_probe_results() {
        let probe = MockAgentEnvProbe::new()
            .with_binary("claude")
            .with_binary("ollama")
            .with_env("ANTHROPIC_API_KEY", "sk-test");
        let detected = detect(&probe);
        assert_eq!(
            detected,
            DetectedAgents {
                claude_cli: true,
                ollama_cli: true,
                anthropic_api_key: true,
                ..DetectedAgents::default()
            }
        );
    }

    #[test]
    fn empty_probe_detects_nothing() {
        let probe = MockAgentEnvProbe::new();
        assert_eq!(detect(&probe), DetectedAgents::default());
    }

    #[test]
    fn default_provider_prefers_cli_in_documented_order() {
        // claude wins when present
        let detected = DetectedAgents {
            claude_cli: true,
            codex_cli: true,
            gemini_cli: true,
            ollama_cli: true,
            ..DetectedAgents::default()
        };
        assert_eq!(default_provider(&detected), "claude");

        // codex wins when claude absent
        let detected = DetectedAgents {
            codex_cli: true,
            gemini_cli: true,
            ollama_cli: true,
            ..DetectedAgents::default()
        };
        assert_eq!(default_provider(&detected), "codex");

        // gemini wins when claude/codex absent
        let detected = DetectedAgents {
            gemini_cli: true,
            ollama_cli: true,
            ..DetectedAgents::default()
        };
        assert_eq!(default_provider(&detected), "gemini");

        // ollama wins when nothing else
        let detected = DetectedAgents {
            ollama_cli: true,
            ..DetectedAgents::default()
        };
        assert_eq!(default_provider(&detected), "ollama");
    }

    #[test]
    fn default_provider_falls_back_to_api_keys() {
        // anthropic key → claude (http)
        let detected = DetectedAgents {
            anthropic_api_key: true,
            openai_api_key: true,
            ..DetectedAgents::default()
        };
        assert_eq!(default_provider(&detected), "claude");

        let detected = DetectedAgents {
            openai_api_key: true,
            gemini_api_key: true,
            ..DetectedAgents::default()
        };
        assert_eq!(default_provider(&detected), "codex");

        let detected = DetectedAgents {
            gemini_api_key: true,
            ..DetectedAgents::default()
        };
        assert_eq!(default_provider(&detected), "gemini");
    }

    #[test]
    fn default_provider_last_resort_is_claude() {
        assert_eq!(default_provider(&DetectedAgents::default()), "claude");
    }

    #[test]
    fn default_backend_picks_cli_when_matching_cli_present() {
        let detected = DetectedAgents {
            codex_cli: true,
            ..DetectedAgents::default()
        };
        assert_eq!(default_backend("codex", &detected), "cli");
        assert_eq!(default_backend("claude", &detected), "http");
    }

    #[test]
    fn default_backend_unknown_provider_is_http() {
        assert_eq!(
            default_backend("openai_compat", &DetectedAgents::default()),
            "http"
        );
    }

    #[test]
    fn model_registry_returns_expected_defaults() {
        assert_eq!(default_model_for("claude"), Some("claude-opus-4-7"));
        assert_eq!(default_model_for("codex"), Some("gpt-5.5"));
        assert_eq!(default_model_for("gemini"), Some("gemini-3-pro"));
        assert_eq!(default_model_for("ollama"), None);
        assert_eq!(default_model_for("unknown"), None);
    }
}
