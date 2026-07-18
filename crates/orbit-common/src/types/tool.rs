use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSessionContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

impl ToolSessionContext {
    pub fn with_workspace(workspace: impl Into<String>) -> Self {
        Self {
            workspace: Some(workspace.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolParam {
    pub name: String,
    pub description: String,
    pub param_type: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Vec<ToolParam>,
    pub builtin: bool,
}

/// Where an MCP-exposed tool executes in the host-registry topology.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum McpToolPlacement {
    Hub,
    Owner,
    LocalDerived,
    Composite,
}

/// A capability that may be granted to an MCP session.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum McpCapability {
    Agent,
    Operator,
    Runner,
}

/// Typed placement and authorization metadata for one MCP tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpToolPolicy {
    placement: McpToolPlacement,
    allowed_capabilities: BTreeSet<McpCapability>,
}

impl McpToolPolicy {
    pub fn new(
        placement: McpToolPlacement,
        allowed_capabilities: impl IntoIterator<Item = McpCapability>,
    ) -> Result<Self, McpToolPolicyError> {
        let mut capabilities = BTreeSet::new();
        for capability in allowed_capabilities {
            if !capabilities.insert(capability) {
                return Err(McpToolPolicyError::DuplicateCapability(capability));
            }
        }
        if capabilities.is_empty() {
            return Err(McpToolPolicyError::EmptyCapabilities);
        }
        Ok(Self {
            placement,
            allowed_capabilities: capabilities,
        })
    }

    pub fn placement(&self) -> McpToolPlacement {
        self.placement
    }

    pub fn allowed_capabilities(&self) -> &BTreeSet<McpCapability> {
        &self.allowed_capabilities
    }

    pub fn validate(&self) -> Result<(), McpToolPolicyError> {
        if self.allowed_capabilities.is_empty() {
            Err(McpToolPolicyError::EmptyCapabilities)
        } else {
            Ok(())
        }
    }
}

/// A schema paired with the policy required for MCP exposure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpToolDefinition {
    pub schema: ToolSchema,
    pub policy: McpToolPolicy,
}

impl McpToolDefinition {
    pub fn new(schema: ToolSchema, policy: McpToolPolicy) -> Result<Self, McpToolPolicyError> {
        policy.validate()?;
        Ok(Self { schema, policy })
    }
}

/// One immutable entry in Orbit's canonical MCP exposure registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalMcpToolPolicy {
    pub canonical_name: &'static str,
    pub placement: McpToolPlacement,
    pub allowed_capabilities: &'static [McpCapability],
}

impl CanonicalMcpToolPolicy {
    pub fn policy(self) -> Result<McpToolPolicy, McpToolPolicyError> {
        McpToolPolicy::new(self.placement, self.allowed_capabilities.iter().copied())
    }

    pub fn advertised_name(self) -> String {
        mcp_advertised_tool_name(self.canonical_name)
    }
}

/// Why an MCP policy or canonical registry is invalid.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum McpToolPolicyError {
    #[error("MCP tool policy has no allowed capabilities")]
    EmptyCapabilities,
    #[error("MCP tool policy repeats capability {0:?}")]
    DuplicateCapability(McpCapability),
    #[error("canonical MCP tool name must not be empty")]
    EmptyCanonicalName,
    #[error("duplicate canonical MCP tool name: {0}")]
    DuplicateCanonicalName(String),
    #[error("duplicate advertised MCP tool name: {0}")]
    DuplicateAdvertisedName(String),
}

const AGENT_OPERATOR: &[McpCapability] = &[McpCapability::Agent, McpCapability::Operator];
const OPERATOR_ONLY: &[McpCapability] = &[McpCapability::Operator];

const fn mcp_policy(
    canonical_name: &'static str,
    placement: McpToolPlacement,
    allowed_capabilities: &'static [McpCapability],
) -> CanonicalMcpToolPolicy {
    CanonicalMcpToolPolicy {
        canonical_name,
        placement,
        allowed_capabilities,
    }
}

static CANONICAL_MCP_TOOL_POLICIES: &[CanonicalMcpToolPolicy] = &[
    mcp_policy("orbit.task.add", McpToolPlacement::Hub, AGENT_OPERATOR),
    mcp_policy("orbit.task.approve", McpToolPlacement::Hub, AGENT_OPERATOR),
    mcp_policy(
        "orbit.task.artifact.put",
        McpToolPlacement::Hub,
        AGENT_OPERATOR,
    ),
    mcp_policy("orbit.task.list", McpToolPlacement::Hub, AGENT_OPERATOR),
    mcp_policy(
        "orbit.task.review_thread.add",
        McpToolPlacement::Hub,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.task.review_thread.list",
        McpToolPlacement::Hub,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.task.review_thread.reply",
        McpToolPlacement::Hub,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.task.review_thread.resolve",
        McpToolPlacement::Hub,
        AGENT_OPERATOR,
    ),
    mcp_policy("orbit.task.show", McpToolPlacement::Hub, AGENT_OPERATOR),
    mcp_policy("orbit.task.start", McpToolPlacement::Hub, AGENT_OPERATOR),
    mcp_policy("orbit.task.update", McpToolPlacement::Hub, AGENT_OPERATOR),
    mcp_policy("orbit.friction.add", McpToolPlacement::Hub, AGENT_OPERATOR),
    mcp_policy("orbit.friction.tags", McpToolPlacement::Hub, AGENT_OPERATOR),
    mcp_policy(
        "orbit.friction.update",
        McpToolPlacement::Hub,
        OPERATOR_ONLY,
    ),
    mcp_policy(
        "orbit.graph.sync",
        McpToolPlacement::LocalDerived,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.graph.search",
        McpToolPlacement::LocalDerived,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.graph.show",
        McpToolPlacement::LocalDerived,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.graph.refs",
        McpToolPlacement::LocalDerived,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.graph.callees",
        McpToolPlacement::LocalDerived,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.graph.impact",
        McpToolPlacement::LocalDerived,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.graph.trace",
        McpToolPlacement::LocalDerived,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.graph.overview",
        McpToolPlacement::LocalDerived,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.graph.implementors",
        McpToolPlacement::LocalDerived,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.graph.deps",
        McpToolPlacement::LocalDerived,
        AGENT_OPERATOR,
    ),
    mcp_policy("orbit.search", McpToolPlacement::Composite, AGENT_OPERATOR),
    mcp_policy("orbit.adr.add", McpToolPlacement::Composite, AGENT_OPERATOR),
    mcp_policy("orbit.adr.show", McpToolPlacement::Owner, AGENT_OPERATOR),
    mcp_policy(
        "orbit.adr.supersede",
        McpToolPlacement::Owner,
        AGENT_OPERATOR,
    ),
    mcp_policy("orbit.adr.update", McpToolPlacement::Owner, AGENT_OPERATOR),
    mcp_policy(
        "orbit.learning.add",
        McpToolPlacement::Composite,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.learning.show",
        McpToolPlacement::Owner,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.learning.update",
        McpToolPlacement::Owner,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.learning.supersede",
        McpToolPlacement::Owner,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.auto_task.add",
        McpToolPlacement::Owner,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.auto_task.show",
        McpToolPlacement::Owner,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.auto_task.update",
        McpToolPlacement::Owner,
        AGENT_OPERATOR,
    ),
    mcp_policy(
        "orbit.auto_task.toggle",
        McpToolPlacement::Owner,
        AGENT_OPERATOR,
    ),
];

/// Convert a canonical Orbit tool name to its MCP-advertised form.
pub fn mcp_advertised_tool_name(canonical_name: &str) -> String {
    canonical_name.replace('.', "_")
}

/// Validate a policy registry, including both canonical and advertised names.
pub fn validate_mcp_tool_policies(
    entries: &[CanonicalMcpToolPolicy],
) -> Result<(), McpToolPolicyError> {
    let mut canonical_names = BTreeSet::new();
    let mut advertised_names = BTreeSet::new();
    for entry in entries {
        if entry.canonical_name.trim().is_empty() {
            return Err(McpToolPolicyError::EmptyCanonicalName);
        }
        entry.policy()?;
        if !canonical_names.insert(entry.canonical_name) {
            return Err(McpToolPolicyError::DuplicateCanonicalName(
                entry.canonical_name.to_string(),
            ));
        }
        let advertised_name = entry.advertised_name();
        if !advertised_names.insert(advertised_name.clone()) {
            return Err(McpToolPolicyError::DuplicateAdvertisedName(advertised_name));
        }
    }
    Ok(())
}

/// Return the validated canonical MCP registry. An invalid registry fails closed.
pub fn canonical_mcp_tool_policies() -> Result<&'static [CanonicalMcpToolPolicy], McpToolPolicyError>
{
    validate_mcp_tool_policies(CANONICAL_MCP_TOOL_POLICIES)?;
    Ok(CANONICAL_MCP_TOOL_POLICIES)
}

/// Resolve one tool policy from the canonical registry, failing closed on drift.
pub fn canonical_mcp_tool_policy(canonical_name: &str) -> Option<McpToolPolicy> {
    canonical_mcp_tool_policies()
        .ok()?
        .iter()
        .find(|entry| entry.canonical_name == canonical_name)
        .and_then(|entry| entry.policy().ok())
}

/// Capability-by-placement coverage generated from the canonical registry.
pub type McpCapabilityPlacementMatrix =
    BTreeMap<McpToolPlacement, BTreeMap<McpCapability, Vec<&'static str>>>;

/// Build the capability-by-placement matrix without a second allowlist.
pub fn mcp_capability_placement_matrix() -> Result<McpCapabilityPlacementMatrix, McpToolPolicyError>
{
    let mut matrix = BTreeMap::new();
    for entry in canonical_mcp_tool_policies()? {
        for capability in entry.allowed_capabilities {
            matrix
                .entry(entry.placement)
                .or_insert_with(BTreeMap::new)
                .entry(*capability)
                .or_insert_with(Vec::new)
                .push(entry.canonical_name);
        }
    }
    Ok(matrix)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredTool {
    pub name: String,
    pub path: String,
    pub description: String,
    pub enabled: bool,
    pub builtin: bool,
    #[serde(default)]
    pub parameters: Vec<ToolParam>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub output: Option<Value>,
}
