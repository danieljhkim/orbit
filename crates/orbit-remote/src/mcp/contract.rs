//! Frozen private contract negotiated by spoke and hub before tool dispatch.

use orbit_common::types::{
    McpCapability, McpToolDefinition, McpToolPlacement, OrbitError, mcp_advertised_tool_name,
    validate_mcp_tool_definitions,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::schema::remote_input_schema;

/// Revision 3 was advanced by the strict connector-private hub knowledge-ID
/// allocator, withdrawn with the global knowledge allocator ([ADR-0357],
/// [ORB-10725]); v1 negotiates no private knowledge-allocation method. The
/// revision itself is history and is never rolled back — reusing 3 for a
/// different contract would let a peer built against the allocator seam
/// negotiate successfully with a hub that no longer has it.
pub const MCP_CONTRACT_REVISION: u32 = 3;
/// Revision 2 adds the operator-only workflow execution family [ORB-10534].
pub const CANONICAL_MCP_REGISTRY_REVISION: u32 = 2;
pub const HUB_SCHEMA_DOMAIN: &str = "orbit.mcp.hub-schema.v1";
pub const HUB_CONTRACT_INSTRUCTIONS_PREFIX: &str = "orbit-hub-contract-v1:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubServerContractV1 {
    pub contract_revision: u32,
    pub canonical_registry_revision: u32,
    pub hub_machine_id: String,
    pub effective_capability: McpCapability,
    pub hub_schema_digest: String,
}

impl HubServerContractV1 {
    pub fn instructions(&self) -> Result<String, OrbitError> {
        let json = serde_json::to_string(self)
            .map_err(|error| OrbitError::Execution(format!("serialize hub contract: {error}")))?;
        Ok(format!("{HUB_CONTRACT_INSTRUCTIONS_PREFIX}{json}"))
    }

    pub fn parse_instructions(instructions: Option<&str>) -> Result<Self, OrbitError> {
        let instructions = instructions.ok_or_else(|| {
            OrbitError::HubNegotiation("hub initialize omitted its private contract".to_string())
        })?;
        let payload = instructions
            .strip_prefix(HUB_CONTRACT_INSTRUCTIONS_PREFIX)
            .ok_or_else(|| {
                OrbitError::HubNegotiation(
                    "hub initialize instructions are not the frozen Orbit hub contract".to_string(),
                )
            })?;
        serde_json::from_str(payload).map_err(|error| {
            OrbitError::HubNegotiation(format!("invalid hub initialize contract: {error}"))
        })
    }
}

/// Domain-separated canonical compact JSON used by both sides of negotiation.
pub fn canonical_hub_schema_bytes(
    definitions: &[McpToolDefinition],
    capability: McpCapability,
) -> Result<Vec<u8>, OrbitError> {
    validate_mcp_tool_definitions(definitions)
        .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
    let mut tools = definitions
        .iter()
        .filter(|definition| {
            definition.policy.placement() == McpToolPlacement::Hub
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
        .map_err(|error| OrbitError::Execution(format!("serialize hub schema: {error}")))?;
    let mut bytes = Vec::with_capacity(HUB_SCHEMA_DOMAIN.len() + 1 + compact.len());
    bytes.extend_from_slice(HUB_SCHEMA_DOMAIN.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&compact);
    Ok(bytes)
}

pub fn hub_schema_digest(
    definitions: &[McpToolDefinition],
    capability: McpCapability,
) -> Result<String, OrbitError> {
    let bytes = canonical_hub_schema_bytes(definitions, capability)?;
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
