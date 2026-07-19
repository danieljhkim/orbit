use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{OrbitError, validate_registry_identifier};

pub const HUB_KNOWLEDGE_ALLOCATION_METHOD_V1: &str = "orbit/private/allocate-knowledge-id/v1";
pub const HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeIdKind {
    Adr,
    Learning,
}

impl KnowledgeIdKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Adr => "adr",
            Self::Learning => "learning",
        }
    }

    pub fn parse_id(self, id: &str) -> Option<u32> {
        let prefix = match self {
            Self::Adr => "ADR-",
            Self::Learning => "L-",
        };
        let suffix = id.strip_prefix(prefix)?;
        if suffix.len() < 4 || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        suffix.parse().ok()
    }

    pub fn format_id(self, sequence: u32) -> String {
        let width = sequence.to_string().len().max(4);
        match self {
            Self::Adr => format!("ADR-{sequence:0width$}"),
            Self::Learning => format!("L-{sequence:0width$}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubKnowledgeAllocationRequestV1 {
    pub schema_version: u8,
    pub workspace_id: String,
    pub kind: KnowledgeIdKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl HubKnowledgeAllocationRequestV1 {
    pub fn validate(&self) -> Result<(), OrbitError> {
        if self.schema_version != HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION {
            return Err(OrbitError::InvalidInput(format!(
                "unsupported hub knowledge allocation request schema_version {}; expected {}",
                self.schema_version, HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION
            )));
        }
        validate_registry_identifier("workspace_id", &self.workspace_id)?;
        if self
            .model
            .as_deref()
            .is_some_and(|model| model.trim().is_empty() || model.trim() != model)
        {
            return Err(OrbitError::InvalidInput(
                "hub knowledge allocation model must be normalized and non-empty when present"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubKnowledgeAllocationV1 {
    pub schema_version: u8,
    pub workspace_id: String,
    pub kind: KnowledgeIdKind,
    pub id: String,
    pub sequence: u32,
    pub mcp_call_id: String,
    pub allocated_at: DateTime<Utc>,
}

impl HubKnowledgeAllocationV1 {
    pub fn validate(&self) -> Result<(), OrbitError> {
        if self.schema_version != HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION {
            return Err(OrbitError::InvalidInput(format!(
                "unsupported hub knowledge allocation result schema_version {}; expected {}",
                self.schema_version, HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION
            )));
        }
        validate_registry_identifier("workspace_id", &self.workspace_id)?;
        if self.mcp_call_id.trim().is_empty() || self.mcp_call_id.trim() != self.mcp_call_id {
            return Err(OrbitError::InvalidInput(
                "hub knowledge allocation result requires a normalized mcp_call_id".to_string(),
            ));
        }
        if self.kind.parse_id(&self.id) != Some(self.sequence) {
            return Err(OrbitError::InvalidInput(format!(
                "hub knowledge allocation id '{}' does not match kind '{}' sequence {}",
                self.id,
                self.kind.as_str(),
                self.sequence
            )));
        }
        Ok(())
    }
}
