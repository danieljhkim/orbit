use chrono::{TimeZone, Utc};

use super::super::*;

#[test]
fn artifact_path_validation_rejects_absolute_and_parent_paths() {
    assert!(validate_relative_artifact_path("files/result.txt").is_ok());
    assert!(validate_relative_artifact_path("/tmp/result.txt").is_err());
    assert!(validate_relative_artifact_path("../result.txt").is_err());
    assert!(validate_relative_artifact_path("files/../result.txt").is_err());
    assert!(validate_relative_artifact_path(r"files\result.txt").is_err());
    assert!(validate_relative_artifact_path("./result.txt").is_err());
    assert!(validate_relative_artifact_path("   ").is_err());
}

#[test]
fn artifact_manifest_validates_file_metadata() {
    let manifest = ArtifactManifestV2 {
        schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
        files: vec![ArtifactManifestFileV2 {
            path: "outputs/report.md".to_string(),
            blob: "files/report.md".to_string(),
            sha256: "a".repeat(64),
            media_type: "text/markdown".to_string(),
            size_bytes: 12,
            created_by: "codex:gpt-5.5".to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap(),
        }],
    };
    assert!(manifest.validate().is_ok());

    let mut invalid = manifest;
    invalid.files[0].blob = "../blob".to_string();
    assert!(invalid.validate().is_err());
}
