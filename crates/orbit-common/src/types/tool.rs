use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSessionContext {
    /// Legacy caller-supplied workspace address. This value is deliberately
    /// untrusted until an adapter/runtime resolves it to [`Self::workspace_id`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Stable logical workspace identity after trusted local resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_machine_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_host_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_machine_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_host_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<McpTransport>,
    /// Complete effective session grants. This is a set, never a scalar
    /// ceiling; callers authorize by membership.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub effective_capabilities: BTreeSet<McpCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_call_id: Option<String>,
}

impl ToolSessionContext {
    pub fn with_workspace(workspace: impl Into<String>) -> Self {
        Self {
            workspace: Some(workspace.into()),
            ..Self::default()
        }
    }

    /// Construct the trusted defaults for the local standalone MCP adapter.
    pub fn trusted_local(
        workspace_id: Option<String>,
        machine_id: Option<String>,
        host_id: Option<String>,
    ) -> Self {
        Self {
            workspace: None,
            workspace_id,
            caller_machine_id: machine_id.clone(),
            caller_host_id: host_id.clone(),
            process_machine_id: machine_id,
            process_host_id: host_id,
            transport: Some(McpTransport::Local),
            effective_capabilities: BTreeSet::from([McpCapability::Agent]),
            origin_session_id: None,
            mcp_call_id: None,
        }
    }

    pub fn has_capability(&self, capability: McpCapability) -> bool {
        self.effective_capabilities.contains(&capability)
    }
}

/// Transport that delivered an MCP call to the executing process.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum McpTransport {
    Local,
    SshMcp,
}

impl Display for McpTransport {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Local => "local",
            Self::SshMcp => "ssh-mcp",
        })
    }
}

impl FromStr for McpTransport {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "local" => Ok(Self::Local),
            "ssh-mcp" => Ok(Self::SshMcp),
            other => Err(format!("unknown MCP transport: {other}")),
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
///
/// ADR-0355 collapsed the former `Hub` class into [`Self::Owner`]: every
/// machine is its own coordination host for the workspaces it owns, so "the
/// coordinating machine for this tool's workspace" is in-process for an owned
/// workspace and there is no machine-level hub left to name.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum McpToolPlacement {
    Owner,
    LocalDerived,
    Composite,
}

impl McpToolPlacement {
    /// Stable contract spelling used in diagnostics and the conformance fixture.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::LocalDerived => "local-derived",
            Self::Composite => "composite",
        }
    }
}

impl Display for McpToolPlacement {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether an MCP tool requires a logical workspace in its trusted session.
///
/// Existing tools are workspace-scoped by default. Registry-wide discovery is
/// the narrow exception: a global tool operates without selecting or inferring
/// a workspace.
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "kebab-case")]
pub enum McpToolScope {
    #[default]
    WorkspaceRequired,
    Global,
}

/// A capability that may be granted to an MCP session.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum McpCapability {
    Agent,
    Operator,
    /// Not a v1 MCP bridge capability. ADR-0358 withdrew `runner` from the
    /// bridge along with registration, presence, and leases, so no tool policy
    /// allows it and no MCP session may hold it (see
    /// [`McpCapability::BRIDGE_V1`]). It survives as the in-process grant a run
    /// stamps onto its own dispatcher so the run can perform the destruction it
    /// exists to perform (ORB-10453) — execution authorization, not a session
    /// capability. Execution placement returns it to the bridge in v2.
    Runner,
}

impl McpCapability {
    /// The capabilities an MCP bridge v1 session or route may carry.
    ///
    /// `operator` does not imply `agent` and `agent` does not imply `operator`;
    /// this is a flat allowlist, not a hierarchy.
    pub const BRIDGE_V1: &'static [Self] = &[Self::Agent, Self::Operator];

    /// Whether this capability is part of the v1 MCP bridge surface.
    pub fn is_bridge_v1(self) -> bool {
        Self::BRIDGE_V1.contains(&self)
    }
}

impl Display for McpCapability {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Agent => "agent",
            Self::Operator => "operator",
            Self::Runner => "runner",
        })
    }
}

impl FromStr for McpCapability {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "agent" => Ok(Self::Agent),
            "operator" => Ok(Self::Operator),
            "runner" => Ok(Self::Runner),
            other => Err(format!("unknown MCP capability: {other}")),
        }
    }
}

/// Typed placement and authorization metadata for one MCP tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpToolPolicy {
    placement: McpToolPlacement,
    allowed_capabilities: BTreeSet<McpCapability>,
    #[serde(default)]
    scope: McpToolScope,
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
        let policy = Self {
            placement,
            allowed_capabilities: capabilities,
            scope: McpToolScope::WorkspaceRequired,
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn placement(&self) -> McpToolPlacement {
        self.placement
    }

    pub fn allowed_capabilities(&self) -> &BTreeSet<McpCapability> {
        &self.allowed_capabilities
    }

    pub fn scope(&self) -> McpToolScope {
        self.scope
    }

    /// Override the default workspace requirement for a deliberately global
    /// tool. Keeping this schema-adjacent prevents name-based scope inference.
    pub fn with_scope(mut self, scope: McpToolScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn validate(&self) -> Result<(), McpToolPolicyError> {
        if self.allowed_capabilities.is_empty() {
            return Err(McpToolPolicyError::EmptyCapabilities);
        }
        // A withdrawn capability on a tool policy is unreachable rather than
        // merely unused: no v1 session can hold it, so the tool would advertise
        // to nobody. Reject it here so the registry cannot drift back.
        if let Some(withdrawn) = self
            .allowed_capabilities
            .iter()
            .find(|capability| !capability.is_bridge_v1())
        {
            return Err(McpToolPolicyError::WithdrawnCapability(*withdrawn));
        }
        Ok(())
    }

    /// Policy for the ordinary agent and trusted operator surfaces.
    pub fn agent_and_operator(placement: McpToolPlacement) -> Self {
        Self {
            placement,
            allowed_capabilities: BTreeSet::from([McpCapability::Agent, McpCapability::Operator]),
            scope: McpToolScope::WorkspaceRequired,
        }
    }

    /// Policy for an operator-only surface.
    pub fn operator_only(placement: McpToolPlacement) -> Self {
        Self {
            placement,
            allowed_capabilities: BTreeSet::from([McpCapability::Operator]),
            scope: McpToolScope::WorkspaceRequired,
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

/// Why an MCP policy or canonical registry is invalid.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum McpToolPolicyError {
    #[error("MCP tool policy has no allowed capabilities")]
    EmptyCapabilities,
    #[error("MCP tool policy repeats capability {0:?}")]
    DuplicateCapability(McpCapability),
    #[error(
        "MCP tool policy allows capability '{0}', which the v1 bridge withdrew (ADR-0358); only agent and operator remain"
    )]
    WithdrawnCapability(McpCapability),
    #[error("canonical MCP tool name must not be empty")]
    EmptyCanonicalName,
    #[error("duplicate canonical MCP tool name: {0}")]
    DuplicateCanonicalName(String),
    #[error("duplicate advertised MCP tool name: {0}")]
    DuplicateAdvertisedName(String),
}

/// Convert a canonical Orbit tool name to its MCP-advertised form.
pub fn mcp_advertised_tool_name(canonical_name: &str) -> String {
    canonical_name.replace('.', "_")
}

/// Validate schema-adjacent MCP definitions, including both canonical and advertised names.
pub fn validate_mcp_tool_definitions(
    definitions: &[McpToolDefinition],
) -> Result<(), McpToolPolicyError> {
    let mut canonical_names = BTreeSet::new();
    let mut advertised_names = BTreeSet::new();
    for definition in definitions {
        let canonical_name = definition.schema.name.as_str();
        if canonical_name.trim().is_empty() {
            return Err(McpToolPolicyError::EmptyCanonicalName);
        }
        definition.policy.validate()?;
        if !canonical_names.insert(canonical_name) {
            return Err(McpToolPolicyError::DuplicateCanonicalName(
                canonical_name.to_string(),
            ));
        }
        let advertised_name = mcp_advertised_tool_name(canonical_name);
        if !advertised_names.insert(advertised_name.clone()) {
            return Err(McpToolPolicyError::DuplicateAdvertisedName(advertised_name));
        }
    }
    Ok(())
}

/// Capability-by-placement coverage generated from the canonical registry.
pub type McpCapabilityPlacementMatrix =
    BTreeMap<McpToolPlacement, BTreeMap<McpCapability, Vec<String>>>;

/// Build the capability-by-placement matrix without a second allowlist.
pub fn mcp_capability_placement_matrix(
    definitions: &[McpToolDefinition],
) -> Result<McpCapabilityPlacementMatrix, McpToolPolicyError> {
    validate_mcp_tool_definitions(definitions)?;
    let mut matrix = BTreeMap::new();
    for definition in definitions {
        for capability in definition.policy.allowed_capabilities() {
            matrix
                .entry(definition.policy.placement())
                .or_insert_with(BTreeMap::new)
                .entry(*capability)
                .or_insert_with(Vec::new)
                .push(definition.schema.name.clone());
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
