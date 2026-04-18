use thiserror::Error;

use crate::{ActivityResource, JobResource, ResourceKind};

use super::activity_v2::ActivityV2;
use super::job_v2::JobV2;
use super::schema_header::SchemaHeader;

/// Outcome of loading an activity YAML — either v1 (ActivityResource) or v2
/// (ActivityV2). Callers pattern-match to route to the appropriate runtime.
#[derive(Debug, Clone)]
pub enum ActivityAsset {
    V1(ActivityResource),
    V2(ActivityV2Asset),
}

/// Wrapper around a v2 activity plus the envelope metadata we preserve for
/// round-trip serialization.
#[derive(Debug, Clone)]
pub struct ActivityV2Asset {
    pub name: String,
    pub spec: ActivityV2,
}

/// Outcome of loading a job YAML — either v1 (JobResource) or v2 (JobV2).
#[derive(Debug, Clone)]
pub enum JobAsset {
    V1(JobResource),
    V2(JobV2Asset),
}

#[derive(Debug, Clone)]
pub struct JobV2Asset {
    pub name: String,
    pub spec: JobV2,
}

#[derive(Debug, Error)]
pub enum AssetLoadError {
    #[error("failed to parse schema header: {0}")]
    HeaderParse(serde_yaml::Error),
    #[error("unsupported schemaVersion: {0}")]
    UnsupportedVersion(u32),
    #[error("v1 parse failed: {0}")]
    V1Parse(serde_yaml::Error),
    #[error("v2 parse failed: {0}")]
    V2Parse(serde_yaml::Error),
    #[error("kind mismatch: expected `{expected}`, got `{actual}`")]
    KindMismatch { expected: String, actual: String },
}

/// Two-pass activity-asset loader (§8.1 Candidate 2).
pub fn load_activity_asset(yaml: &str) -> Result<ActivityAsset, AssetLoadError> {
    let header = SchemaHeader::parse_yaml(yaml).map_err(AssetLoadError::HeaderParse)?;
    match header.schema_version {
        1 => {
            let res: ActivityResource =
                serde_yaml::from_str(yaml).map_err(AssetLoadError::V1Parse)?;
            Ok(ActivityAsset::V1(res))
        }
        2 => {
            let res: V2EnvelopeYaml<ActivityV2> =
                serde_yaml::from_str(yaml).map_err(AssetLoadError::V2Parse)?;
            require_kind(&res.kind, ResourceKind::Activity)?;
            Ok(ActivityAsset::V2(ActivityV2Asset {
                name: res.metadata.name,
                spec: res.spec,
            }))
        }
        other => Err(AssetLoadError::UnsupportedVersion(other)),
    }
}

/// Two-pass job-asset loader.
pub fn load_job_asset(yaml: &str) -> Result<JobAsset, AssetLoadError> {
    let header = SchemaHeader::parse_yaml(yaml).map_err(AssetLoadError::HeaderParse)?;
    match header.schema_version {
        1 => {
            let res: JobResource = serde_yaml::from_str(yaml).map_err(AssetLoadError::V1Parse)?;
            Ok(JobAsset::V1(res))
        }
        2 => {
            let res: V2EnvelopeYaml<JobV2> =
                serde_yaml::from_str(yaml).map_err(AssetLoadError::V2Parse)?;
            require_kind(&res.kind, ResourceKind::Job)?;
            Ok(JobAsset::V2(JobV2Asset {
                name: res.metadata.name,
                spec: res.spec,
            }))
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
    metadata: crate::ResourceMetadata,
    spec: T,
}
