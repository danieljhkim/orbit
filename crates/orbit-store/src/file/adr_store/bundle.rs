use std::fs;
use std::path::Path;

use orbit_common::types::{Adr, NotFoundKind, OrbitError};
use orbit_common::utility::fs::atomic_write_text_volatile as write_atomic;

use super::doc::{AdrFileDocument, serialize_adr_doc_yaml};
use super::layout::{adr_doc_path, body_path, validate_adr_id};
use crate::file::yaml_doc::{read_yaml_with, write_yaml_atomic_with};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdrBundle {
    pub(super) doc: AdrFileDocument,
    pub(super) body: String,
}

pub(super) fn write_bundle_at(adr_dir: &Path, bundle: &AdrBundle) -> Result<(), OrbitError> {
    validate_bundle(bundle)?;
    fs::create_dir_all(adr_dir).map_err(|e| OrbitError::Io(e.to_string()))?;

    write_yaml_atomic_with(&adr_doc_path(adr_dir), &bundle.doc, serialize_adr_doc_yaml)?;
    write_atomic(&body_path(adr_dir), &bundle.body).map_err(|e| OrbitError::Io(e.to_string()))?;
    Ok(())
}

pub(super) fn read_bundle_at(adr_dir: &Path) -> Result<AdrBundle, OrbitError> {
    let doc_path = adr_doc_path(adr_dir);
    if !doc_path.exists() {
        let id = adr_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unknown>")
            .to_string();
        return Err(OrbitError::not_found(NotFoundKind::Adr, id));
    }

    let doc: AdrFileDocument = read_yaml_with(&doc_path, |path, err| {
        OrbitError::Store(format!("invalid ADR file {}: {err}", path.display()))
    })?;
    let body = read_companion_text(&body_path(adr_dir))?;
    Ok(AdrBundle { doc, body })
}

pub(super) fn bundle_to_adr(bundle: AdrBundle) -> Adr {
    bundle.doc.adr
}

pub(super) fn validate_bundle(bundle: &AdrBundle) -> Result<(), OrbitError> {
    validate_adr_id(&bundle.doc.adr.id)?;
    Ok(())
}

fn read_companion_text(path: &Path) -> Result<String, OrbitError> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(value),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use chrono::Utc;
    use orbit_common::types::{Adr, AdrStatus, LegacyValidation, NotFoundKind};
    use tempfile::tempdir;

    use super::super::constants::ADR_SCHEMA_VERSION;
    use super::*;

    fn sample_bundle(id: &str) -> AdrBundle {
        let ts = Utc.with_ymd_and_hms(2026, 5, 11, 0, 0, 0).unwrap();
        AdrBundle {
            doc: AdrFileDocument {
                schema_version: ADR_SCHEMA_VERSION,
                adr: Adr {
                    id: id.to_string(),
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
            },
            body: "## Context\n\nSomething.\n".to_string(),
        }
    }

    #[test]
    fn write_then_read_round_trips_the_bundle() {
        let tempdir = tempdir().expect("tempdir");
        let dir = tempdir.path().join("ADR-0001");
        let bundle = sample_bundle("ADR-0001");

        write_bundle_at(&dir, &bundle).expect("write bundle");
        let loaded = read_bundle_at(&dir).expect("read bundle");

        assert_eq!(loaded, bundle);
    }

    #[test]
    fn read_bundle_returns_empty_body_when_body_md_missing() {
        let tempdir = tempdir().expect("tempdir");
        let dir = tempdir.path().join("ADR-0001");
        fs::create_dir_all(&dir).expect("create adr dir");
        let bundle = sample_bundle("ADR-0001");
        write_yaml_atomic_with(&adr_doc_path(&dir), &bundle.doc, serialize_adr_doc_yaml)
            .expect("write doc only");

        let loaded = read_bundle_at(&dir).expect("read bundle");

        assert_eq!(loaded.body, "");
        assert_eq!(loaded.doc, bundle.doc);
    }

    #[test]
    fn read_bundle_on_nonexistent_dir_returns_adr_not_found() {
        let tempdir = tempdir().expect("tempdir");
        let dir = tempdir.path().join("ADR-9999");

        let err = read_bundle_at(&dir).expect_err("missing dir should error");

        assert!(
            matches!(
                err,
                OrbitError::NotFound {
                    kind: NotFoundKind::Adr,
                    ref id,
                } if id == "ADR-9999"
            ),
            "expected missing ADR error, got {err:?}"
        );
    }
}
