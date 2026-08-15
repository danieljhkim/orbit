//! Command implementations for all Orbit CLI subcommands.
//!
//! Each sub-module (task, job, activity, skill, audit, tool, init)
//! provides the data types and logic for one command group. Commands are
//! executed via the `Execute` trait, which receives an `&OrbitRuntime` and
//! produces an `OrbitError` on failure.
//!
//! The `init` module is special: it also provides `execute_without_runtime`
//! for bootstrapping a new Orbit root before a runtime can be constructed.
//! Default YAML assets (e.g., sample skills, config templates) are embedded
//! at compile time via `include_str!` and seeded to disk on first `orbit init`.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use orbit_common::utility::fs::{atomic_write_text, write_text_with_parent};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Audit identity used for system-initiated (non-agent) mutations.
/// `pub` because the direct v2 activity runner moved to `orbit-cmd`
/// [ORB-10016] and stamps the same identity.
pub const SYSTEM_AUDIT_IDENTITY: &str = "system";

const MANAGED_ASSET_MANIFEST_FILE: &str = ".orbit-managed-assets.json";
const MANAGED_ASSET_MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ManagedAssetReconciliation {
    pub refreshed: usize,
    pub retired: usize,
    pub warnings: Vec<String>,
}

/// How a manifest key maps to the file it manages, relative to the managed
/// directory.
///
/// Four of the five artifact kinds are flat single-document catalogs whose
/// manifest key is the definition name ([`ManagedAssetLayout::YamlStem`]).
/// Skills are directory trees — one `SKILL.md` plus optional reference files
/// per skill id — so their manifest keys are the relative paths themselves
/// ([`ManagedAssetLayout::RelativePath`]).
// ADR-0366 extends ADR-0346's provenance mechanism to tree-shaped assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedAssetLayout {
    /// `<name>.yaml` — activities, jobs, auto-tasks, routines.
    YamlStem,
    /// `<name>` verbatim, a `/`-separated relative path — skills.
    RelativePath,
}

impl ManagedAssetLayout {
    /// Resolve one manifest key to its path relative to the managed directory.
    fn relative_path(self, name: &str) -> PathBuf {
        match self {
            Self::YamlStem => PathBuf::from(format!("{name}.yaml")),
            Self::RelativePath => PathBuf::from(name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManagedAssetManifest {
    schema_version: u32,
    asset_kind: String,
    assets: BTreeMap<String, String>,
}

/// Materialize the current embedded resource set and reconcile assets retired
/// since the previous manifest-aware seed.
///
/// The manifest records the digest Orbit last wrote for each managed file.
/// Retired files that still match that digest are deleted. Locally modified
/// retired files are moved outside the recursively loaded catalog tree, so
/// their content survives without keeping a removed subsystem active. A
/// legacy directory without a manifest is migrated conservatively: exact
/// current defaults gain provenance, while every other YAML file stays in
/// place and produces an actionable warning.
// ADR-0346: content provenance, rather than filenames, authorizes retirement.
pub(crate) fn reconcile_managed_assets<'a>(
    dir: &Path,
    asset_kind: &str,
    layout: ManagedAssetLayout,
    files: &'a [(&'a str, &'a str)],
    overwrite: bool,
    mut render: impl FnMut(&'a str, &'a str) -> Result<Cow<'a, str>, OrbitError>,
) -> Result<ManagedAssetReconciliation, OrbitError> {
    validate_managed_asset_name(asset_kind, ManagedAssetLayout::YamlStem, "asset kind")?;
    for (name, _) in files {
        validate_managed_asset_name(name, layout, "embedded asset")?;
    }

    let manifest_path = dir.join(MANAGED_ASSET_MANIFEST_FILE);
    let previous = load_managed_asset_manifest(&manifest_path, asset_kind, layout)?;
    let current_names: BTreeSet<&str> = files.iter().map(|(name, _)| *name).collect();
    let mut result = ManagedAssetReconciliation::default();

    if let Some(previous) = &previous {
        for (name, managed_digest) in &previous.assets {
            if current_names.contains(name.as_str()) {
                continue;
            }
            let path = dir.join(layout.relative_path(name));
            if !path.exists() {
                continue;
            }
            let content = fs::read_to_string(&path).map_err(|error| {
                OrbitError::Io(format!(
                    "read retired managed {asset_kind} '{}': {error}",
                    path.display()
                ))
            })?;
            if sha256_hex(content.as_bytes()) == *managed_digest {
                fs::remove_file(&path).map_err(|error| {
                    OrbitError::Io(format!(
                        "retire managed {asset_kind} '{}': {error}",
                        path.display()
                    ))
                })?;
            } else {
                let preserved =
                    preserve_modified_retired_asset(dir, asset_kind, layout, name, &path)?;
                result.warnings.push(format!(
                    "retired managed {asset_kind} `{name}` was locally modified; Orbit removed it from the active catalog and preserved it at '{}'. Review that file, then migrate it under a new user-authored name or delete it",
                    preserved.display()
                ));
            }
            result.retired += 1;
        }
    }

    let mut next_assets = BTreeMap::new();
    for (name, embedded) in files {
        let path = dir.join(layout.relative_path(name));
        let rendered = render(name, embedded)?;
        let rendered_digest = sha256_hex(rendered.as_bytes());

        if path.exists() {
            let previous_digest = previous
                .as_ref()
                .and_then(|manifest| manifest.assets.get(*name));
            if previous_digest.is_none() {
                let existing = fs::read_to_string(&path).map_err(|error| {
                    OrbitError::Io(format!(
                        "read existing {asset_kind} '{}': {error}",
                        path.display()
                    ))
                })?;
                if sha256_hex(existing.as_bytes()) == rendered_digest {
                    next_assets.insert((*name).to_string(), rendered_digest);
                } else if previous.is_some() {
                    result.warnings.push(format!(
                        "untracked user-authored {asset_kind} '{}' collides with bundled default `{name}` and was preserved in place. Move or rename it, then rerun `orbit init` to install the bundled default",
                        path.display()
                    ));
                }
                continue;
            }

            if !overwrite {
                let Some(previous_digest) = previous_digest else {
                    continue;
                };
                next_assets.insert((*name).to_string(), previous_digest.clone());
                continue;
            }

            // The manifest records the last embedded content written for this
            // asset. If it already matches the current embedded content, this
            // bootstrap has nothing to refresh. Avoid touching the asset so a
            // steady-state runtime can operate with global resources mounted
            // read-only.
            if previous_digest == Some(&rendered_digest) {
                next_assets.insert((*name).to_string(), rendered_digest);
                continue;
            }
        }

        write_text_with_parent(&path, &rendered)?;
        next_assets.insert((*name).to_string(), rendered_digest);
        result.refreshed += 1;
    }

    // The legacy sweep only makes sense for the flat YAML catalogs: a skill
    // tree's untracked files are ordinary reference material inside an
    // otherwise-managed directory, not stray definitions the loader would pick
    // up.
    if previous.is_none() && layout == ManagedAssetLayout::YamlStem {
        let ambiguous = ambiguous_legacy_yaml_files(dir, &next_assets)?;
        if !ambiguous.is_empty() {
            result.warnings.push(format!(
                "untracked {asset_kind} YAML assets have no managed provenance and were preserved in place: {}. If any came from an older Orbit release, move or delete them manually before retrying catalog/list commands",
                ambiguous
                    .iter()
                    .map(|path| format!("'{}'", path.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    let manifest = ManagedAssetManifest {
        schema_version: MANAGED_ASSET_MANIFEST_SCHEMA_VERSION,
        asset_kind: asset_kind.to_string(),
        assets: next_assets,
    };
    if previous.as_ref() != Some(&manifest) {
        write_managed_asset_manifest(&manifest_path, &manifest)?;
    }

    for warning in &result.warnings {
        tracing::warn!(
            target: "orbit.core.managed_assets",
            asset_kind,
            warning,
            "managed asset reconciliation requires operator attention"
        );
    }

    Ok(result)
}

/// Persist one managed-asset manifest. Callers compare against the previous
/// manifest first so a steady-state bootstrap performs no write at all.
fn write_managed_asset_manifest(
    manifest_path: &Path,
    manifest: &ManagedAssetManifest,
) -> Result<(), OrbitError> {
    let asset_kind = &manifest.asset_kind;
    let mut encoded = serde_json::to_string_pretty(manifest).map_err(|error| {
        OrbitError::Store(format!(
            "serialize managed {asset_kind} asset manifest: {error}"
        ))
    })?;
    encoded.push('\n');
    atomic_write_text(manifest_path, &encoded).map_err(|error| {
        OrbitError::Io(format!(
            "write managed {asset_kind} asset manifest '{}': {error}",
            manifest_path.display()
        ))
    })
}

fn load_managed_asset_manifest(
    path: &Path,
    expected_kind: &str,
    layout: ManagedAssetLayout,
) -> Result<Option<ManagedAssetManifest>, OrbitError> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(|error| {
        OrbitError::Io(format!(
            "read managed asset manifest '{}': {error}",
            path.display()
        ))
    })?;
    let manifest: ManagedAssetManifest = serde_json::from_str(&raw).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "managed asset manifest '{}' is invalid: {error}; repair it or move it aside only after reviewing the managed YAML files",
            path.display()
        ))
    })?;
    if manifest.schema_version != MANAGED_ASSET_MANIFEST_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "managed asset manifest '{}' uses unsupported schemaVersion {}; expected {}",
            path.display(),
            manifest.schema_version,
            MANAGED_ASSET_MANIFEST_SCHEMA_VERSION
        )));
    }
    if manifest.asset_kind != expected_kind {
        return Err(OrbitError::InvalidInput(format!(
            "managed asset manifest '{}' is for `{}`, expected `{expected_kind}`",
            path.display(),
            manifest.asset_kind
        )));
    }
    for name in manifest.assets.keys() {
        validate_managed_asset_name(name, layout, "manifest asset")?;
    }
    Ok(Some(manifest))
}

/// Reject any manifest key that would not stay inside the managed directory.
///
/// A stem is a single path component of the safe charset. A relative path is a
/// `/`-separated sequence of such components: no absolute prefix, no `.`/`..`
/// component, and a restricted charset per component, so a manifest can never
/// steer a write or a removal outside the directory it manages.
fn validate_managed_asset_name(
    name: &str,
    layout: ManagedAssetLayout,
    source: &str,
) -> Result<(), OrbitError> {
    let component_ok = |component: &str| {
        !component.is_empty()
            && component != "."
            && component != ".."
            && component.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' || byte == b'.'
            })
    };
    let valid = match layout {
        // A stem is one component and never carries an extension separator.
        ManagedAssetLayout::YamlStem => {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        }
        ManagedAssetLayout::RelativePath => {
            !name.is_empty()
                && !name.starts_with('/')
                && !name.contains('\\')
                && name.split('/').all(component_ok)
        }
    };
    if !valid {
        return Err(OrbitError::InvalidInput(format!(
            "{source} name `{name}` is not a safe managed asset path"
        )));
    }
    Ok(())
}

fn preserve_modified_retired_asset(
    active_dir: &Path,
    asset_kind: &str,
    layout: ManagedAssetLayout,
    name: &str,
    source: &Path,
) -> Result<PathBuf, OrbitError> {
    let backup_root = active_dir
        .parent()
        .unwrap_or(active_dir)
        .join(".retired-managed")
        .join(managed_asset_kind_directory(asset_kind));
    let relative = layout.relative_path(name);

    let mut suffix = 0usize;
    loop {
        // Disambiguate on the file stem so a preserved `SKILL.md` keeps its
        // extension and stays readable in place.
        let destination = if suffix == 0 {
            backup_root.join(&relative)
        } else {
            let file_name = relative
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    OrbitError::InvalidInput(format!(
                        "retired managed {asset_kind} `{name}` has no file name to preserve"
                    ))
                })?;
            let (stem, extension) = file_name
                .rsplit_once('.')
                .map_or((file_name, String::new()), |(stem, extension)| {
                    (stem, format!(".{extension}"))
                });
            backup_root
                .join(&relative)
                .with_file_name(format!("{stem}.{suffix}{extension}"))
        };
        if destination.exists() {
            suffix += 1;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                OrbitError::Io(format!(
                    "create retired managed asset backup '{}': {error}",
                    parent.display()
                ))
            })?;
        }
        fs::rename(source, &destination).map_err(|error| {
            OrbitError::Io(format!(
                "preserve modified retired {asset_kind} '{}' as '{}': {error}",
                source.display(),
                destination.display()
            ))
        })?;
        return Ok(destination);
    }
}

fn managed_asset_kind_directory(asset_kind: &str) -> String {
    match asset_kind {
        "activity" => "activities".to_string(),
        "job" => "jobs".to_string(),
        other => format!("{other}s"),
    }
}

fn ambiguous_legacy_yaml_files(
    dir: &Path,
    managed_assets: &BTreeMap<String, String>,
) -> Result<Vec<PathBuf>, OrbitError> {
    let mut ambiguous = Vec::new();
    let entries = fs::read_dir(dir).map_err(|error| {
        OrbitError::Io(format!(
            "inspect legacy managed asset directory '{}': {error}",
            dir.display()
        ))
    })?;
    for entry in entries {
        let path = entry
            .map_err(|error| OrbitError::Io(error.to_string()))?
            .path();
        let is_yaml = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension == "yaml" || extension == "yml");
        if !is_yaml {
            continue;
        }
        let stem = path.file_stem().and_then(|stem| stem.to_str());
        if stem.is_none_or(|stem| !managed_assets.contains_key(stem)) {
            ambiguous.push(path);
        }
    }
    ambiguous.sort();
    Ok(ambiguous)
}

fn sha256_hex(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

pub(crate) mod activity;
pub mod artifact_health;
pub mod audit_event;
pub(crate) mod docs;
pub(crate) mod executor;
pub mod gc;
pub mod init;
pub mod job;
pub(crate) mod policy;
pub(crate) mod routine;
pub(crate) mod search;
pub mod semantic;
pub mod skill;
pub mod task;
pub mod task_migration;
pub mod tool;
pub(crate) mod workflow;

#[cfg(test)]
mod tests;
