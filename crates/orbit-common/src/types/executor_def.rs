use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorType {
    AgentCli,
    DirectAgent,
    CliCommand,
}

impl ExecutorType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentCli => "agent_cli",
            Self::DirectAgent => "direct_agent",
            Self::CliCommand => "cli_command",
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
    /// Executor family, serialized as "agent_cli", "direct_agent", or "cli_command".
    pub executor_type: ExecutorType,
    /// For agent_cli: the CLI command (e.g., "claude", "codex")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Expected stdout format, serialized as "envelope", "json", or "text".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_format: Option<StdoutFormat>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub models: HashMap<String, String>,
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

fn is_false(value: &bool) -> bool {
    !*value
}

impl ExecutorDef {
    pub fn model_for_tier(&self, tier: &str) -> Option<&str> {
        self.models
            .get(tier)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}
