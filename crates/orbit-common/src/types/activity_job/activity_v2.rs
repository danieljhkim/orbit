use serde::{Deserialize, Serialize};
use serde_json::Value;

/// v2 activity definition. Corresponds to the v2 YAML asset shape:
/// ```yaml
/// schemaVersion: 2
/// kind: Activity
/// metadata:
///   name: <name>
/// spec:
///   type: agent_loop | groundhog | deterministic | shell
///   description: <text>
///   input_schema_json: {...}
///   output_schema_json: {...}
///   tools: [...]
///   on_denial: terminate | continue  # agent_loop only; default terminate
///   ...type-specific fields
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityV2 {
    pub description: String,
    #[serde(default)]
    pub input_schema_json: Value,
    #[serde(default)]
    pub output_schema_json: Value,
    #[serde(rename = "fsProfile", default, skip_serializing_if = "Option::is_none")]
    pub fs_profile: Option<String>,
    #[serde(flatten)]
    pub spec: ActivityV2Spec,
}

/// v2 activity type discriminator. Serialized as
/// `type: agent_loop|groundhog|deterministic|shell`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivityV2Spec {
    AgentLoop(AgentLoopSpec),
    Groundhog(GroundhogSpec),
    Deterministic(DeterministicSpec),
    Shell(ShellSpec),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentLoopSpec {
    /// System prompt / instruction delivered to the agent loop.
    #[serde(default)]
    pub instruction: String,
    /// Tool allowlist (§6). Empty means no tools are allowed.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Behavior when a denied tool is requested (§6 / §12 Q6).
    #[serde(default)]
    pub on_denial: OnDenial,
    /// Optional model override (provider-specific name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Upper bound on loop iterations.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Execution backend (§3.1). Missing values default to the v1 `Cli`
    /// release path. `Auto` is resolved to `Http` or `Cli` once per Run at
    /// load time per the precedence rules in §3.1 and then never observed by
    /// the dispatcher — everything downstream sees the concrete backend.
    #[serde(default)]
    pub backend: Backend,
    /// Provider whose runtime executes this activity (§3.1).
    #[serde(default)]
    pub provider: Provider,
    /// Wall-clock timeout for a CLI invocation (§7.6). Ignored in HTTP mode
    /// where the loop engine applies its own timeout.
    #[serde(default = "default_cli_wall_clock_timeout_seconds")]
    pub wall_clock_timeout_seconds: u64,
    /// Optional role tag (ADR-029). When set, the engine consults
    /// `[agent.<role>]` in `config.toml` and overrides `provider`/`model`/
    /// `backend` field-by-field at dispatch time. The step-level role on
    /// `TargetStep` takes precedence over this activity-level role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<AgentRole>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GroundhogSpec {
    /// System prompt / instruction delivered to each Groundhog attempt.
    #[serde(default)]
    pub instruction: String,
    /// Additional tool allowlist entries. Groundhog-required tools are
    /// injected by the runner even when omitted here.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Behavior when a denied tool is requested.
    #[serde(default)]
    pub on_denial: OnDenial,
    /// Optional model override (provider-specific name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Upper bound on loop iterations per attempt.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Provider whose HTTP runtime executes each attempt.
    #[serde(default)]
    pub provider: Provider,
    /// Wall-clock timeout for one Groundhog attempt.
    #[serde(default = "default_cli_wall_clock_timeout_seconds")]
    pub wall_clock_timeout_seconds: u64,
    /// Fallback attempt budget when a checkpoint omits `attempt_budget`.
    #[serde(default = "default_groundhog_attempt_budget")]
    pub attempt_budget_default: u32,
    /// Optional role tag (ADR-029). Mirrors `AgentLoopSpec::role`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<AgentRole>,
}

impl GroundhogSpec {
    pub fn as_agent_loop_spec(&self) -> AgentLoopSpec {
        AgentLoopSpec {
            instruction: self.instruction.clone(),
            tools: self.tools.clone(),
            on_denial: self.on_denial,
            model: self.model.clone(),
            max_iterations: self.max_iterations,
            backend: Backend::Http,
            provider: self.provider,
            wall_clock_timeout_seconds: self.wall_clock_timeout_seconds,
            role: self.role,
        }
    }
}

/// Execution backend for an `agent_loop` activity (§3.1). `Auto` resolves at
/// load time per the precedence chain in §3.1: flag → env → config → default.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Http,
    #[default]
    Cli,
    Auto,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Http => "http",
            Backend::Cli => "cli",
            Backend::Auto => "auto",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "http" => Some(Backend::Http),
            "cli" => Some(Backend::Cli),
            "auto" => Some(Backend::Auto),
            _ => None,
        }
    }
}

/// Named provider whose runtime executes an `agent_loop` activity. The enum is
/// closed-set: adding a provider means wiring a new HTTP transport AND/OR a new
/// CLI runtime factory, both of which are code changes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Claude,
    Codex,
    Gemini,
    Grok,
    Ollama,
    #[serde(rename = "openai_compat", alias = "openai-compat")]
    OpenaiCompat,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::Gemini => "gemini",
            Provider::Grok => "grok",
            Provider::Ollama => "ollama",
            Provider::OpenaiCompat => "openai_compat",
        }
    }

    /// Whether Phase 2c wires an HTTP transport for this provider. Used by the
    /// dispatcher's §3.1 no-silent-fallback check: `backend: http` against a
    /// provider whose HTTP transport is not wired must fail structurally, not
    /// silently fall back to CLI.
    pub fn has_http_transport(self) -> bool {
        matches!(self, Provider::Claude)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeterministicSpec {
    /// Registered deterministic action name. The dispatcher looks this up in
    /// the `ActivityExecutorRegistry` at runtime.
    pub action: String,
    /// Optional literal configuration passed through to the action.
    #[serde(default)]
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShellSpec {
    /// Program to invoke. Must be in `allowed_programs`.
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Program allowlist enforced before spawn.
    pub allowed_programs: Vec<String>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    /// Exit codes treated as success. Empty means `[0]`.
    #[serde(default)]
    pub expected_exit_codes: Vec<i32>,
}

/// Role tag for an `agent_loop` / `groundhog` activity (ADR-029). Maps to
/// `[agent.<role>]` blocks in `config.toml`; the dispatcher resolves the
/// effective role to a `(provider, model, backend)` triple before invoking
/// the runner. The set is closed because `orbit init` only prompts for these
/// three roles today; widening it requires a config-schema change.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AgentRole {
    Reviewer,
    Implementer,
    Planner,
}

impl AgentRole {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentRole::Reviewer => "reviewer",
            AgentRole::Implementer => "implementer",
            AgentRole::Planner => "planner",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "reviewer" => Some(AgentRole::Reviewer),
            "implementer" => Some(AgentRole::Implementer),
            "planner" => Some(AgentRole::Planner),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnDenial {
    /// Terminate the loop on a denied tool call (default per §12 Q6).
    #[default]
    Terminate,
    /// Continue the loop with the structured tool-result error.
    Continue,
}

const fn default_max_iterations() -> u32 {
    25
}

const fn default_timeout_seconds() -> u64 {
    60
}

const fn default_cli_wall_clock_timeout_seconds() -> u64 {
    300
}

const fn default_groundhog_attempt_budget() -> u32 {
    3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_role_serde_roundtrips_lowercase() {
        for (value, expected) in [
            (AgentRole::Reviewer, "\"reviewer\""),
            (AgentRole::Implementer, "\"implementer\""),
            (AgentRole::Planner, "\"planner\""),
        ] {
            let serialized = serde_json::to_string(&value).expect("serialize role");
            assert_eq!(serialized, expected);
            let parsed: AgentRole = serde_json::from_str(expected).expect("deserialize role");
            assert_eq!(parsed, value);
        }
    }

    #[test]
    fn agent_loop_spec_yaml_includes_role_when_present() {
        let yaml = "instruction: hi\nrole: implementer\n";
        let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
        assert_eq!(parsed.role, Some(AgentRole::Implementer));
    }

    #[test]
    fn agent_loop_spec_yaml_role_is_optional() {
        let yaml = "instruction: hi\n";
        let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
        assert_eq!(parsed.role, None);
    }

    #[test]
    fn agent_loop_spec_defaults_to_cli_backend() {
        let yaml = "instruction: hi\n";
        let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
        assert_eq!(parsed.backend, Backend::Cli);
    }
}
