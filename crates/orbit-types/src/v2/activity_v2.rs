use serde::{Deserialize, Serialize};
use serde_json::Value;

/// v2 activity definition. Corresponds to the v2 YAML asset shape:
/// ```yaml
/// schemaVersion: 2
/// kind: Activity
/// metadata:
///   name: <name>
/// spec:
///   type: agent_loop | deterministic | shell
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
    #[serde(flatten)]
    pub spec: ActivityV2Spec,
}

/// v2 activity type discriminator. Serialized as `type: agent_loop|deterministic|shell`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivityV2Spec {
    AgentLoop(AgentLoopSpec),
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
