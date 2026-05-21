use thiserror::Error;

use crate::types::ResourceKind;

use super::activity_v2::ActivityV2;
use super::job_v2::JobV2;
use super::schema_header::SchemaHeader;
use super::tool_allowlist::{ToolAllowlistError, validate_activity_tool_allowlist};

/// Loaded schemaVersion 2 activity asset plus its envelope metadata.
#[derive(Debug, Clone)]
pub struct ActivityAsset {
    pub name: String,
    pub spec: ActivityV2,
}

/// Loaded schemaVersion 2 job asset plus its envelope metadata.
#[derive(Debug, Clone)]
pub struct JobAsset {
    pub name: String,
    pub spec: JobV2,
}

#[derive(Debug, Error)]
pub enum AssetLoadError {
    #[error("failed to parse schema header: {0}")]
    HeaderParse(serde_yaml::Error),
    #[error("schemaVersion {0} assets were retired; migrate this asset to schemaVersion 2")]
    RetiredVersion(u32),
    #[error("unsupported schemaVersion: {0}")]
    UnsupportedVersion(u32),
    #[error("schemaVersion 2 parse failed: {0}")]
    Parse(serde_yaml::Error),
    #[error("kind mismatch: expected `{expected}`, got `{actual}`")]
    KindMismatch { expected: String, actual: String },
    #[error("activity `{activity}` tool allowlist invalid: {source}")]
    ToolAllowlist {
        activity: String,
        source: ToolAllowlistError,
    },
}

/// Two-pass activity-asset loader for schemaVersion 2 assets.
pub fn load_activity_asset(yaml: &str) -> Result<ActivityAsset, AssetLoadError> {
    let header = SchemaHeader::parse_yaml(yaml).map_err(AssetLoadError::HeaderParse)?;
    match header.schema_version {
        1 => Err(AssetLoadError::RetiredVersion(1)),
        2 => {
            let res: V2EnvelopeYaml<ActivityV2> =
                serde_yaml::from_str(yaml).map_err(AssetLoadError::Parse)?;
            require_kind(&res.kind, ResourceKind::Activity)?;
            validate_activity_tool_allowlist(&res.spec).map_err(|source| {
                AssetLoadError::ToolAllowlist {
                    activity: res.metadata.name.clone(),
                    source,
                }
            })?;
            Ok(ActivityAsset {
                name: res.metadata.name,
                spec: res.spec,
            })
        }
        other => Err(AssetLoadError::UnsupportedVersion(other)),
    }
}

/// Two-pass job-asset loader for schemaVersion 2 assets.
pub fn load_job_asset(yaml: &str) -> Result<JobAsset, AssetLoadError> {
    let header = SchemaHeader::parse_yaml(yaml).map_err(AssetLoadError::HeaderParse)?;
    match header.schema_version {
        1 => Err(AssetLoadError::RetiredVersion(1)),
        2 => {
            let res: V2EnvelopeYaml<JobV2> =
                serde_yaml::from_str(yaml).map_err(AssetLoadError::Parse)?;
            require_kind(&res.kind, ResourceKind::Job)?;
            Ok(JobAsset {
                name: res.metadata.name,
                spec: res.spec,
            })
        }
        other => Err(AssetLoadError::UnsupportedVersion(other)),
    }
}

fn require_kind(actual: &ResourceKind, expected: ResourceKind) -> Result<(), AssetLoadError> {
    if actual == &expected {
        Ok(())
    } else {
        Err(AssetLoadError::KindMismatch {
            expected: expected.to_string(),
            actual: actual.to_string(),
        })
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
struct V2EnvelopeYaml<T> {
    #[serde(rename = "schemaVersion")]
    _schema_version: u32,
    kind: ResourceKind,
    metadata: crate::types::ResourceMetadata,
    spec: T,
}
