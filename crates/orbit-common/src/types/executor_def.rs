use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ExecutorResourceSpec is the persisted wire shape; ExecutorDef is the runtime shape.
use super::resource::ExecutorResourceSpec;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorType {
    AgentCli,
    DirectAgent,
    CliCommand,
    /// Generic out-of-process executor speaking the External Executor Protocol
    /// v1 (see `docs/design/executors/specs/external-executor-protocol.md`).
    /// Lets operators register a homegrown binary/script without forking core.
    /// Shares the `direct_agent` subprocess transport but carries no
    /// agent-family `model_pair` semantics. See ADR-0196 / [ORB-00384].
    External,
}

impl ExecutorType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentCli => "agent_cli",
            Self::DirectAgent => "direct_agent",
            Self::CliCommand => "cli_command",
            Self::External => "external",
        }
    }
}

impl fmt::Display for ExecutorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Sandbox primitive applied to a CLI-backend agent invocation. The variant
/// names a concrete OS primitive; `orbit-exec` selects the implementation.
///
/// Today only `macos-sandbox-exec` is wired; a future Linux variant
/// (`linux-bwrap` or similar) can land alongside without changing the
/// schema shape.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutorSandboxKind {
    MacosSandboxExec,
}

impl ExecutorSandboxKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MacosSandboxExec => "macos-sandbox-exec",
        }
    }
}

impl fmt::Display for ExecutorSandboxKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StdoutFormat {
    Envelope,
    Json,
    Text,
}

impl StdoutFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Envelope => "envelope",
            Self::Json => "json",
            Self::Text => "text",
        }
    }
}

impl fmt::Display for StdoutFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutorDef {
    pub name: String,
    /// Executor family, serialized as "agent_cli", "direct_agent",
    /// "cli_command", or "external".
    pub executor_type: ExecutorType,
    /// For agent_cli: the CLI command (e.g., "claude", "codex")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Expected stdout format, serialized as "envelope", "json", or "text".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_format: Option<StdoutFormat>,
    /// Overrides the agent family's default `AgentModelPair` resolution for audit
    /// canonicalization, envelope rendering, and review attribution.
    ///
    /// Does NOT control which model the subprocess actually runs; operators
    /// should encode runtime model selection in `args`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_pair_override: Option<ModelPairOverride>,
    /// CLI flag name used to pass `JobStep.model` to a direct-agent subprocess.
    ///
    /// Carries only the flag name, for example `"-m"` or `"--model"`. At
    /// invocation time, when both `model_flag` and the step's runtime model are
    /// present, `direct_agent` appends `[model_flag, step.model]` after the
    /// operator-declared `args`. Orbit does not inspect `args` for duplicates;
    /// the CLI's own last-wins behavior resolves repeated model flags. When
    /// either field is absent, nothing is injected, so operators can still
    /// hardcode fixed model arguments such as `--model X` in `args`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_flag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// OS sandbox primitive to wrap the CLI invocation in. When `None`, the
    /// CLI is spawned bare (today's behavior). When `Some`, `orbit-exec`
    /// translates the activity's `FsProfile` into a sandbox payload and
    /// wraps the spawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<ExecutorSandboxKind>,
    /// When `sandbox` is set but the platform's trusted sandbox primitive is
    /// unavailable (e.g. `/usr/bin/sandbox-exec` is missing), should the runner
    /// degrade to bare exec? Default `false` (fail-closed).
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_fallback: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Override for an agent family's strong/weak `AgentModelPair`.
///
/// Controls how Orbit canonicalizes the agent's model for audit trail,
/// envelope rendering, and review automation attribution.
///
/// Does NOT control which model the subprocess actually runs. Operators must
/// encode the runtime model in `args`, and may set `ORBIT_AGENT_MODEL` via
/// `env:` for explicit audit attribution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct ModelPairOverride {
    pub strong: String,
    pub weak: String,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl ExecutorDef {
    pub fn from_resource_spec(
        name: String,
        spec: ExecutorResourceSpec,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        let ExecutorResourceSpec {
            executor_type,
            command,
            args,
            stdout_format,
            model_pair_override,
            model_flag,
            timeout_seconds,
            env,
            sandbox,
            allow_fallback,
            created_at: _,
            updated_at: _,
        } = spec;

        Self {
            name,
            executor_type,
            command,
            args,
            stdout_format,
            model_pair_override,
            model_flag,
            timeout_seconds,
            env,
            sandbox,
            allow_fallback,
            created_at,
            updated_at,
        }
    }

    pub fn model_pair_override(&self) -> Option<&ModelPairOverride> {
        self.model_pair_override.as_ref()
    }
}
