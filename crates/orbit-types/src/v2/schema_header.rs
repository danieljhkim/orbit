use serde::{Deserialize, Serialize};

/// Minimal header parsed from a YAML asset to dispatch to the correct typed
/// deserialization path. Matches the outer envelope shape used by
/// [`crate::ResourceHeader`] but is narrower — only the `schemaVersion` field
/// is required.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaHeader {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
}

impl SchemaHeader {
    pub fn parse_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str::<SchemaHeader>(yaml)
    }
}
