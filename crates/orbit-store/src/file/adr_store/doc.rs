use orbit_common::types::{Adr, OrbitError};
use serde::{Deserialize, Serialize};

/// On-disk shape of an ADR record (the contents of `adr.yaml`).
///
/// Wraps an in-memory [`Adr`] with the persisted `schema_version` field so that
/// future schema bumps can migrate older files without changing the in-memory
/// type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct AdrFileDocument {
    pub(super) schema_version: u8,
    #[serde(flatten)]
    pub(super) adr: Adr,
}

pub(super) fn serialize_adr_doc_yaml(doc: &AdrFileDocument) -> Result<String, OrbitError> {
    serde_yaml::to_string(doc).map_err(|e| OrbitError::Store(e.to_string()))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use chrono::Utc;
    use orbit_common::types::{AdrStatus, LegacyValidation};

    use super::super::constants::ADR_SCHEMA_VERSION;
    use super::*;

    fn sample_doc() -> AdrFileDocument {
        let ts = Utc.with_ymd_and_hms(2026, 5, 11, 0, 0, 0).unwrap();
        AdrFileDocument {
            schema_version: ADR_SCHEMA_VERSION,
            adr: Adr {
                id: "ADR-0001".to_string(),
                title: "Test decision".to_string(),
                status: AdrStatus::Proposed,
                owner: "claude".to_string(),
                created_at: ts,
                accepted_at: None,
                last_updated: ts,
                related_features: vec![],
                related_tasks: vec![],
                tags: vec![],
                paths: vec![],
                supersedes: vec![],
                superseded_by: None,
                legacy_ids: vec![],
                validation_warnings: vec![],
                legacy_validation: LegacyValidation::None,
            },
        }
    }

    #[test]
    fn round_trip_through_yaml_preserves_schema_version() {
        let mut doc = sample_doc();
        doc.adr.tags = vec!["adr-schema".to_string(), "cross-cutting".to_string()];
        doc.adr.paths = vec!["crates/orbit-store/**".to_string()];
        let yaml = serialize_adr_doc_yaml(&doc).expect("serialize");
        assert!(
            yaml.contains("schema_version: 2"),
            "yaml should contain schema_version: 2; got:\n{yaml}"
        );
        let back: AdrFileDocument = serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn v1_yaml_without_tags_or_paths_defaults_to_empty_lists() {
        let yaml = r#"schema_version: 1
id: ADR-0001
title: Test decision
status: proposed
owner: claude
created_at: 2026-05-11T00:00:00Z
last_updated: 2026-05-11T00:00:00Z
"#;

        let doc: AdrFileDocument = serde_yaml::from_str(yaml).expect("deserialize v1");

        assert_eq!(doc.schema_version, 1);
        assert!(doc.adr.tags.is_empty());
        assert!(doc.adr.paths.is_empty());
    }
}
