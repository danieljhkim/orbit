//! Frozen contract negotiated by a client and a workspace owner before tool
//! dispatch [ORB-10727].

use orbit_common::types::{
    McpCapability, McpToolDefinition, McpToolPlacement, OrbitError, mcp_advertised_tool_name,
    validate_mcp_tool_definitions,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::schema::remote_input_schema;

/// Revision numbers are append-only and are never rolled back: reusing a
/// revision for a different contract would let a peer built against the older
/// seam negotiate successfully with an endpoint that no longer has it.
///
/// Revisions 2 and 3 were advanced by the two connector-private methods
/// (`orbit/private/register-spoke/v1` and `orbit/private/allocate-knowledge-id/v1`).
/// Both are withdrawn ([ADR-0357], [ADR-0358]) and v1 negotiates no private
/// connector method at all. Revision 4 covers that removal together with the
/// owner-placement recomposition ([ADR-0355], [ORB-10727]).
pub const MCP_CONTRACT_REVISION: u32 = 4;
/// Revision 2 added the operator-only workflow execution family [ORB-10534].
/// Revision 3 is the v1 tool matrix: `hub` placement collapsed into `owner`,
/// `orbit.workspace.list` moved to `local-derived`, and the `orbit.adr.*`,
/// `orbit.run.*`, and `orbit.host.list` families were withdrawn [ORB-10727].
/// Revision 4 removes the native project-learning tool family [ORB-10736].
/// Revision 5 adds the workspace session-log family [ORB-10784].
pub const CANONICAL_MCP_REGISTRY_REVISION: u32 = 5;
pub const OWNER_SCHEMA_DOMAIN: &str = "orbit.mcp.owner-schema.v1";
pub const OWNER_CONTRACT_INSTRUCTIONS_PREFIX: &str = "orbit-owner-contract-v1:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerServerContractV1 {
    pub contract_revision: u32,
    pub canonical_registry_revision: u32,
    pub owner_machine_id: String,
    pub effective_capability: McpCapability,
    pub owner_schema_digest: String,
}

impl OwnerServerContractV1 {
    pub fn instructions(&self) -> Result<String, OrbitError> {
        let json = serde_json::to_string(self)
            .map_err(|error| OrbitError::Execution(format!("serialize owner contract: {error}")))?;
        Ok(format!("{OWNER_CONTRACT_INSTRUCTIONS_PREFIX}{json}"))
    }

    pub fn parse_instructions(instructions: Option<&str>) -> Result<Self, OrbitError> {
        let instructions = instructions.ok_or_else(|| {
            OrbitError::OwnerNegotiation("owner initialize omitted its contract".to_string())
        })?;
        let payload = instructions
            .strip_prefix(OWNER_CONTRACT_INSTRUCTIONS_PREFIX)
            .ok_or_else(|| {
                OrbitError::OwnerNegotiation(
                    "owner initialize instructions are not the frozen Orbit owner contract"
                        .to_string(),
                )
            })?;
        serde_json::from_str(payload).map_err(|error| {
            OrbitError::OwnerNegotiation(format!("invalid owner initialize contract: {error}"))
        })
    }
}

/// Domain-separated canonical compact JSON used by both sides of negotiation.
pub fn canonical_owner_schema_bytes(
    definitions: &[McpToolDefinition],
    capability: McpCapability,
) -> Result<Vec<u8>, OrbitError> {
    validate_mcp_tool_definitions(definitions)
        .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
    let mut tools = definitions
        .iter()
        .filter(|definition| {
            definition.policy.placement() == McpToolPlacement::Owner
                && definition
                    .policy
                    .allowed_capabilities()
                    .contains(&capability)
        })
        .map(|definition| {
            let input_schema = remote_input_schema(definition)?;
            Ok(json!({
                "advertised_name": mcp_advertised_tool_name(&definition.schema.name),
                "canonical_name": definition.schema.name,
                "description": definition.schema.description,
                "input_schema": input_schema,
            }))
        })
        .collect::<Result<Vec<_>, OrbitError>>()?;
    tools.sort_by(|left, right| {
        left["canonical_name"]
            .as_str()
            .cmp(&right["canonical_name"].as_str())
    });
    let value = json!({
        "canonical_registry_revision": CANONICAL_MCP_REGISTRY_REVISION,
        "capability": capability,
        "tools": tools,
    });
    let canonical = canonicalize_json(value);
    let compact = serde_json::to_vec(&canonical)
        .map_err(|error| OrbitError::Execution(format!("serialize owner schema: {error}")))?;
    let mut bytes = Vec::with_capacity(OWNER_SCHEMA_DOMAIN.len() + 1 + compact.len());
    bytes.extend_from_slice(OWNER_SCHEMA_DOMAIN.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&compact);
    Ok(bytes)
}

pub fn owner_schema_digest(
    definitions: &[McpToolDefinition],
    capability: McpCapability,
) -> Result<String, OrbitError> {
    let bytes = canonical_owner_schema_bytes(definitions, capability)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let mut sorted = Map::new();
            for key in keys {
                if let Some(value) = object.get(&key) {
                    sorted.insert(key, canonicalize_json(value.clone()));
                }
            }
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        scalar => scalar,
    }
}
