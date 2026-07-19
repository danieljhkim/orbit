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

/// Create a complete ADR bundle without adopting or clobbering an existing
/// supplied-ID directory. The directory creation is the exclusive publish
/// boundary; any later write failure removes only the directory created by
/// this call.
pub(super) fn create_bundle_exclusive(
    adr_dir: &Path,
    bundle: &AdrBundle,
) -> Result<bool, OrbitError> {
    validate_bundle(bundle)?;
    let parent = adr_dir.parent().ok_or_else(|| {
        OrbitError::Store(format!(
            "cannot determine ADR state directory for '{}'",
            adr_dir.display()
        ))
    })?;
    fs::create_dir_all(parent).map_err(|error| OrbitError::Io(error.to_string()))?;
    match fs::create_dir(adr_dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(OrbitError::Io(error.to_string())),
    }
    if let Err(error) = write_bundle_at(adr_dir, bundle) {
        let _ = fs::remove_dir_all(adr_dir);
        return Err(error);
    }
    Ok(true)
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
    fs::read_to_string(path).map_err(|err| OrbitError::Io(err.to_string()))
}

#[cfg(test)]
#[cfg(test)]
mod tests;
