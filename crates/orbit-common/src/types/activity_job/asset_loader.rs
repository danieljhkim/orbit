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

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_loop_activity_yaml(name: &str, tools: &str) -> String {
        format!(
            r#"schemaVersion: 2
kind: Activity
metadata:
  name: {name}
spec:
  type: agent_loop
  description: Test agent loop.
  instruction: Test.
  tools:
{tools}"#
        )
    }

    #[test]
    fn load_activity_asset_accepts_task_wildcard_tool_allowlist() {
        let yaml = agent_loop_activity_yaml("task_tools", "    - orbit.task.*\n");

        let asset = load_activity_asset(&yaml).expect("activity should load");

        assert_eq!(asset.name, "task_tools");
    }

    #[test]
    fn load_activity_asset_rejects_top_level_orbit_wildcard() {
        let yaml = agent_loop_activity_yaml("broad_tools", "    - orbit.*\n");

        let err = load_activity_asset(&yaml).expect_err("broad wildcard should fail");
        let message = err.to_string();

        assert!(message.contains("orbit.*"), "{message}");
        assert!(message.contains("wildcard root not permitted"), "{message}");
    }

    #[test]
    fn load_activity_asset_rejects_empty_tool_name() {
        let yaml = agent_loop_activity_yaml("empty_tools", "    - \"\"\n");

        let err = load_activity_asset(&yaml).expect_err("empty tool should fail");
        let message = err.to_string();

        assert!(message.contains("empty tool name"), "{message}");
        assert!(message.contains("index 0"), "{message}");
    }
}
