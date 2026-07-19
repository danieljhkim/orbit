//! Frozen private contract negotiated by spoke and hub before tool dispatch.

use orbit_common::types::{
    McpCapability, McpToolDefinition, McpToolPlacement, OrbitError, mcp_advertised_tool_name,
    validate_mcp_tool_definitions,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::adapter::schema::build_input_schema;

pub const MCP_CONTRACT_REVISION: u32 = 1;
pub const CANONICAL_MCP_REGISTRY_REVISION: u32 = 1;
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
            json!({
                "advertised_name": mcp_advertised_tool_name(&definition.schema.name),
                "canonical_name": definition.schema.name,
                "description": definition.schema.description,
                "input_schema": build_input_schema(
                    &definition.schema.name,
                    &definition.schema.parameters,
                ),
            })
        })
        .collect::<Vec<_>>();
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

#[cfg(test)]
mod tests {
    use orbit_common::types::{McpToolPolicy, ToolParam, ToolSchema};

    use super::*;

    fn fixture_definitions() -> Vec<McpToolDefinition> {
        vec![
            McpToolDefinition::new(
                ToolSchema {
                    name: "orbit.task.show".to_string(),
                    description: "Show one task".to_string(),
                    parameters: vec![ToolParam {
                        name: "id".to_string(),
                        description: "Task ID".to_string(),
                        param_type: "string".to_string(),
                        required: true,
                    }],
                    builtin: true,
                },
                McpToolPolicy::agent_and_operator(McpToolPlacement::Hub),
            )
            .expect("definition"),
        ]
    }

    #[test]
    fn frozen_hub_schema_golden_vector() {
        let bytes = canonical_hub_schema_bytes(&fixture_definitions(), McpCapability::Agent)
            .expect("canonical bytes");
        assert_eq!(
            String::from_utf8(bytes).expect("utf8"),
            concat!(
                "orbit.mcp.hub-schema.v1\0",
                r#"{"canonical_registry_revision":1,"capability":"agent","tools":[{"advertised_name":"orbit_task_show","canonical_name":"orbit.task.show","description":"Show one task","input_schema":{"additionalProperties":true,"properties":{"id":{"description":"Task ID","type":"string"}},"required":["id"],"type":"object"}}]}"#,
            )
        );
        assert_eq!(
            hub_schema_digest(&fixture_definitions(), McpCapability::Agent).expect("digest"),
            "ec8ef56c153562d0f4125cee1b3932c33ed30eb8509601aa5652a351a7b6a8f7"
        );
    }
}
