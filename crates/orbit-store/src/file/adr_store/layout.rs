use std::path::{Path, PathBuf};

use orbit_common::types::AdrStatus;
#[cfg(test)]
use orbit_common::types::OrbitError;

use super::constants::{ADR_YAML, BODY_MD};
#[cfg(test)]
use crate::file::layout::read_child_dirs;

pub(super) use orbit_common::types::validate_adr_id;

/// Filesystem state directories for ADRs, one per [`AdrStatus`] variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdrStateDir {
    Proposed,
    Accepted,
    Superseded,
    Deleted,
}

impl AdrStateDir {
    pub(super) fn dir_name(&self) -> &'static str {
        match self {
            AdrStateDir::Proposed => "proposed",
            AdrStateDir::Accepted => "accepted",
            AdrStateDir::Superseded => "superseded",
            AdrStateDir::Deleted => "deleted",
        }
    }

    pub(super) fn all() -> &'static [AdrStateDir] {
        &[
            AdrStateDir::Proposed,
            AdrStateDir::Accepted,
            AdrStateDir::Superseded,
            AdrStateDir::Deleted,
        ]
    }

    pub(super) fn from_status(status: AdrStatus) -> Self {
        match status {
            AdrStatus::Proposed => AdrStateDir::Proposed,
            AdrStatus::Accepted => AdrStateDir::Accepted,
            AdrStatus::Superseded => AdrStateDir::Superseded,
            AdrStatus::Deleted => AdrStateDir::Deleted,
        }
    }

    pub(super) fn to_status(self) -> AdrStatus {
        match self {
            AdrStateDir::Proposed => AdrStatus::Proposed,
            AdrStateDir::Accepted => AdrStatus::Accepted,
            AdrStateDir::Superseded => AdrStatus::Superseded,
            AdrStateDir::Deleted => AdrStatus::Deleted,
        }
    }
}

pub(super) fn state_dir_path(root: &Path, state: AdrStateDir) -> PathBuf {
    root.join(state.dir_name())
}

pub(super) fn adr_dir(root: &Path, state: AdrStateDir, id: &str) -> PathBuf {
    state_dir_path(root, state).join(id)
}

pub(super) fn adr_doc_path(adr_dir: &Path) -> PathBuf {
    adr_dir.join(ADR_YAML)
}

pub(super) fn body_path(adr_dir: &Path) -> PathBuf {
    adr_dir.join(BODY_MD)
}

#[cfg(test)]
/// Allocates the next sequential ADR id (e.g. `ADR-0001`).
///
/// Scans all four state directories, parses any `ADR-NNNN` directory names, and
/// returns the next integer formatted with at least 4 digits of padding (wider
/// pads grow naturally once the counter exceeds 9999).
///
/// Runtime-backed stores use the SQLite id allocator; this scan helper remains
/// for layout-focused tests and legacy fallback checks.
pub(super) fn next_adr_id(root: &Path) -> Result<String, OrbitError> {
    let mut max_seen: u32 = 0;

    for state in AdrStateDir::all() {
        let dir = state_dir_path(root, *state);
        if !dir.exists() {
            continue;
        }
        for child in read_child_dirs(&dir)? {
            let Some(name) = child.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Some(n) = parse_adr_dir_name(name) {
                max_seen = max_seen.max(n);
            }
        }
    }

    let next = max_seen
        .checked_add(1)
        .ok_or_else(|| OrbitError::Execution("ADR id counter overflow".to_string()))?;
    let width = next.to_string().len().max(4);
    Ok(format!("ADR-{next:0width$}"))
}

#[cfg(test)]
fn parse_adr_dir_name(name: &str) -> Option<u32> {
    let suffix = name.strip_prefix("ADR-")?;
    if suffix.len() < 4 {
        return None;
    }
    if !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    suffix.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn all_returns_four_variants_in_stable_order() {
        let all = AdrStateDir::all();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0], AdrStateDir::Proposed);
        assert_eq!(all[1], AdrStateDir::Accepted);
        assert_eq!(all[2], AdrStateDir::Superseded);
        assert_eq!(all[3], AdrStateDir::Deleted);
    }

    #[test]
    fn from_status_to_status_round_trip_for_each_variant() {
        for status in [
            AdrStatus::Proposed,
            AdrStatus::Accepted,
            AdrStatus::Superseded,
            AdrStatus::Deleted,
        ] {
            let state = AdrStateDir::from_status(status);
            assert_eq!(state.to_status(), status);
        }
    }

    #[test]
    fn next_adr_id_returns_first_id_for_empty_root() {
        let tempdir = tempdir().expect("tempdir");
        let id = next_adr_id(tempdir.path()).expect("next adr id");
        assert_eq!(id, "ADR-0001");
    }

    #[test]
    fn next_adr_id_returns_max_plus_one_across_state_dirs() {
        let tempdir = tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::create_dir_all(root.join("proposed").join("ADR-0003")).expect("create proposed");
        fs::create_dir_all(root.join("accepted").join("ADR-0017")).expect("create accepted");

        let id = next_adr_id(root).expect("next adr id");
        assert_eq!(id, "ADR-0018");
    }

    #[test]
    fn next_adr_id_skips_non_conforming_directory_names() {
        let tempdir = tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::create_dir_all(root.join("proposed").join("tmp")).expect("create tmp");
        fs::create_dir_all(root.join("proposed").join("notes")).expect("create notes");
        fs::create_dir_all(root.join("proposed").join("ADR-foo")).expect("create ADR-foo");
        fs::create_dir_all(root.join("accepted").join("ADR-001")).expect("create short id");
        fs::create_dir_all(root.join("accepted").join("ADR-0005")).expect("create valid");

        let id = next_adr_id(root).expect("next adr id");
        assert_eq!(id, "ADR-0006");
    }

    #[test]
    fn next_adr_id_grows_pad_past_four_digits() {
        let tempdir = tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::create_dir_all(root.join("accepted").join("ADR-9999")).expect("create 9999");

        let id = next_adr_id(root).expect("next adr id");
        assert_eq!(id, "ADR-10000");
    }
}
