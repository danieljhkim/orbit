use chrono::{DateTime, Utc};
use orbit_common::types::{Learning, LearningEvidence, LearningScope, LearningStatus, OrbitId};
use serde::{Deserialize, Serialize};

/// On-disk shape of a learning record (the contents of `<id>/learning.yaml`).
///
/// Wraps an in-memory [`Learning`] with the persisted `schema_version`
/// field, mirroring the `AdrFileDocument` pattern. The `Learning` payload
/// is flattened so the YAML reads cleanly without a nested key.
///
/// The phase-2 forward-compatible fields (`scope.symbols`,
/// `scope.semantic_seed`) live inside the embedded `Learning` and round-trip
/// because they're declared `#[serde(default)]` on the inner struct.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(super) struct LearningFileDocument {
    pub(super) schema_version: u8,
    #[serde(flatten)]
    pub(super) learning: Learning,
}

impl<'de> Deserialize<'de> for LearningFileDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawLearningFileDocument {
            #[serde(default = "default_schema_version", alias = "schemaVersion")]
            schema_version: u8,
            id: OrbitId,
            #[serde(default = "default_status")]
            status: LearningStatus,
            #[serde(default)]
            scope: LearningScope,
            #[serde(default)]
            summary: String,
            #[serde(default)]
            body: String,
            #[serde(default)]
            evidence: Vec<LearningEvidence>,
            #[serde(default)]
            supersedes: Option<OrbitId>,
            #[serde(default)]
            superseded_by: Option<OrbitId>,
            #[serde(default)]
            legacy_ids: Vec<OrbitId>,
            created_at: DateTime<Utc>,
            updated_at: DateTime<Utc>,
            #[serde(default)]
            created_by: Option<String>,
            #[serde(default)]
            priority: Option<u8>,
        }

        let raw = RawLearningFileDocument::deserialize(deserializer)?;
        Ok(LearningFileDocument {
            schema_version: raw.schema_version,
            learning: Learning {
                id: raw.id,
                status: raw.status,
                scope: raw.scope,
                summary: raw.summary,
                body: raw.body,
                evidence: raw.evidence,
                supersedes: raw.supersedes,
                superseded_by: raw.superseded_by,
                legacy_ids: raw.legacy_ids,
                created_at: raw.created_at,
                updated_at: raw.updated_at,
                created_by: raw.created_by,
                priority: raw.priority,
            },
        })
    }
}

fn default_schema_version() -> u8 {
    super::constants::LEARNING_SCHEMA_VERSION
}

fn default_status() -> LearningStatus {
    LearningStatus::Active
}

pub(super) fn serialize_learning_doc_yaml(
    doc: &LearningFileDocument,
) -> Result<String, orbit_common::types::OrbitError> {
    serde_yaml::to_string(doc).map_err(|e| orbit_common::types::OrbitError::Store(e.to_string()))
}
