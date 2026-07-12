//! Persistent ownership + retirement metadata for Orbit-managed skills.
//!
//! Orbit can identify the *current* embedded default skills by re-rendering the
//! compiled-in templates and comparing byte-for-byte. That evidence disappears
//! the moment a skill is dropped from the default catalog: a retired
//! Orbit-generated copy on disk becomes indistinguishable from user-authored
//! content with the same name. This module records durable proof at seed/link
//! time so a later `orbit gc skills` (and today's unlink/teardown) can remove
//! *only* content Orbit provably owns.
//!
//! The manifest lives next to the skills it describes at
//! `<skills_root>/.ownership.json`. Two proofs establish Orbit ownership:
//!
//! 1. **Exact known hash** — the on-disk `SKILL.md` matches a generated-content
//!    hash recorded here (either the currently-managed record or a retired
//!    tombstone). Modified copies never match and stay [`SkillOwnership::Ambiguous`].
//! 2. **Orbit-targeting managed symlink** — a link whose resolved target lands
//!    inside a known Orbit-owned root. Works even for broken links (the target
//!    directory was already removed) via a lexical prefix fallback.
//!
//! Anything else — a same-named user directory, a modified generated skill, or
//! a symlink pointing outside every Orbit root — is reported as ambiguous or
//! unmanaged and is never auto-claimed or removed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use orbit_common::utility::fs::write_text_with_parent;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths::normalize_path_components;

/// Basename of the ownership manifest, colocated with the managed skills.
/// A leading dot keeps it out of skill-id enumeration (which only considers
/// directories containing a `SKILL.md`).
pub const OWNERSHIP_MANIFEST_FILE: &str = ".ownership.json";

/// Current on-disk schema version for the ownership manifest.
pub const OWNERSHIP_MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Durable record of Orbit-managed skill ownership and retirement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillOwnershipManifest {
    /// On-disk schema version, so future migrations can detect old layouts.
    pub schema_version: u32,
    /// Currently-managed skills, keyed by skill id.
    #[serde(default)]
    pub managed: BTreeMap<String, ManagedSkillRecord>,
    /// Retired skill ids whose ownership proof must outlive the catalog entry,
    /// keyed by skill id.
    #[serde(default)]
    pub tombstones: BTreeMap<String, SkillTombstone>,
}

impl Default for SkillOwnershipManifest {
    fn default() -> Self {
        Self {
            schema_version: OWNERSHIP_MANIFEST_SCHEMA_VERSION,
            managed: BTreeMap::new(),
            tombstones: BTreeMap::new(),
        }
    }
}

/// Ownership metadata for one currently-managed skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManagedSkillRecord {
    pub skill_id: String,
    /// sha256 (hex) of the generated `SKILL.md` as rendered for `owned_root`.
    pub content_hash: String,
    /// Catalog/content version at seed time, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The Orbit-owned root that holds the generated skill directory.
    pub owned_root: PathBuf,
    /// Every managed link destination created for this skill.
    #[serde(default)]
    pub link_destinations: Vec<PathBuf>,
}

/// Retained ownership proof for a skill that has left the default catalog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillTombstone {
    pub skill_id: String,
    /// Every generated-content hash this skill was ever recorded with.
    #[serde(default)]
    pub content_hashes: Vec<String>,
    /// The last version seen before retirement, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired_version: Option<String>,
    /// Owned roots the skill was generated into.
    #[serde(default)]
    pub owned_roots: Vec<PathBuf>,
    /// Managed link destinations recorded before retirement.
    #[serde(default)]
    pub link_destinations: Vec<PathBuf>,
}

/// Outcome of classifying an on-disk skill install against the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillOwnership {
    /// Proven Orbit-owned: an exact known hash or an Orbit-targeting symlink.
    /// Safe for unlink/teardown/gc to remove.
    OrbitOwned,
    /// A candidate (right skill id, or a symlink) that lacks ownership proof —
    /// a modified generated copy or a symlink targeting outside Orbit roots.
    /// Never auto-removed.
    Ambiguous,
    /// Nothing Orbit-related: an unknown skill id with no manifest record.
    Unmanaged,
}

impl SkillOwnership {
    pub fn is_orbit_owned(self) -> bool {
        matches!(self, SkillOwnership::OrbitOwned)
    }
}

/// Path to the ownership manifest for a given skills root.
pub fn manifest_path(skills_root: &Path) -> PathBuf {
    skills_root.join(OWNERSHIP_MANIFEST_FILE)
}

/// Load the ownership manifest for `skills_root`. A missing manifest yields the
/// default (empty) manifest; a corrupt one is surfaced as an error rather than
/// silently discarding accumulated tombstones.
pub fn load_manifest(skills_root: &Path) -> Result<SkillOwnershipManifest, OrbitError> {
    let path = manifest_path(skills_root);
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
            OrbitError::SkillValidation(format!(
                "corrupt skill ownership manifest at '{}': {e}",
                path.display()
            ))
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(SkillOwnershipManifest::default())
        }
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

/// Persist the ownership manifest for `skills_root`.
pub fn save_manifest(
    skills_root: &Path,
    manifest: &SkillOwnershipManifest,
) -> Result<(), OrbitError> {
    let path = manifest_path(skills_root);
    let mut serialized = serde_json::to_string_pretty(manifest)
        .map_err(|e| OrbitError::SkillValidation(e.to_string()))?;
    serialized.push('\n');
    write_text_with_parent(&path, &serialized).map_err(|e| OrbitError::Io(e.to_string()))
}

/// sha256 (hex) of arbitrary bytes — the canonical generated-content hash.
pub fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// One generated skill's identity as produced by the seeder.
#[derive(Debug, Clone)]
pub struct GeneratedSkill {
    pub skill_id: String,
    /// sha256 of the rendered `SKILL.md` for the target root.
    pub content_hash: String,
    pub version: Option<String>,
}

/// Reconcile the manifest after seeding the current default skills into
/// `skills_root`.
///
/// - Every currently-generated skill gets/refreshes a managed record (id, hash,
///   version, owned root) while preserving already-recorded link destinations.
/// - Any previously-managed skill absent from `generated` is retired into a
///   tombstone, carrying its recorded hash(es), version, owned root, and links
///   forward so ownership stays provable after the catalog entry is gone.
pub fn reconcile_managed_skills(
    skills_root: &Path,
    generated: &[GeneratedSkill],
) -> Result<(), OrbitError> {
    let mut manifest = load_manifest(skills_root)?;
    let before = manifest.clone();
    let owned_root = skills_root.to_path_buf();

    let current: std::collections::HashSet<&str> =
        generated.iter().map(|g| g.skill_id.as_str()).collect();

    // Retire managed records that are no longer generated.
    let retired_ids: Vec<String> = manifest
        .managed
        .keys()
        .filter(|id| !current.contains(id.as_str()))
        .cloned()
        .collect();
    for id in retired_ids {
        if let Some(record) = manifest.managed.remove(&id) {
            retire_record(&mut manifest.tombstones, record);
        }
    }

    // Upsert managed records for the current generated skills.
    for skill in generated {
        let entry = manifest
            .managed
            .entry(skill.skill_id.clone())
            .or_insert_with(|| ManagedSkillRecord {
                skill_id: skill.skill_id.clone(),
                content_hash: skill.content_hash.clone(),
                version: skill.version.clone(),
                owned_root: owned_root.clone(),
                link_destinations: Vec::new(),
            });
        // If the content hash changed (template update), retain the old hash as
        // a known-generated hash on the tombstone so prior installs remain
        // provably Orbit-owned.
        if entry.content_hash != skill.content_hash {
            let previous = entry.content_hash.clone();
            let tombstone = manifest
                .tombstones
                .entry(skill.skill_id.clone())
                .or_insert_with(|| SkillTombstone {
                    skill_id: skill.skill_id.clone(),
                    content_hashes: Vec::new(),
                    retired_version: None,
                    owned_roots: Vec::new(),
                    link_destinations: Vec::new(),
                });
            push_unique(&mut tombstone.content_hashes, previous);
        }
        entry.content_hash = skill.content_hash.clone();
        entry.version = skill.version.clone();
        entry.owned_root = owned_root.clone();
    }

    // Idempotent bootstraps re-run reconcile constantly; only write when the
    // manifest actually changed (or does not yet exist on disk).
    if manifest != before || !manifest_path(skills_root).exists() {
        save_manifest(skills_root, &manifest)?;
    }
    Ok(())
}

/// Record managed link destinations after (re)creating symlinks. Each entry is
/// a `(skill_id, link_path)` pair. Only annotates already-managed records; a
/// pair for an unknown skill id is ignored (the manifest tracks generated
/// skills, not arbitrary links). Persists once for the whole batch.
pub fn record_link_destinations(
    skills_root: &Path,
    links: &[(String, PathBuf)],
) -> Result<(), OrbitError> {
    let mut manifest = load_manifest(skills_root)?;
    let mut changed = false;
    for (skill_id, link_path) in links {
        if let Some(record) = manifest.managed.get_mut(skill_id)
            && !record.link_destinations.contains(link_path)
        {
            record.link_destinations.push(link_path.clone());
            changed = true;
        }
    }
    if changed {
        save_manifest(skills_root, &manifest)?;
    }
    Ok(())
}

fn retire_record(tombstones: &mut BTreeMap<String, SkillTombstone>, record: ManagedSkillRecord) {
    let tombstone = tombstones
        .entry(record.skill_id.clone())
        .or_insert_with(|| SkillTombstone {
            skill_id: record.skill_id.clone(),
            content_hashes: Vec::new(),
            retired_version: None,
            owned_roots: Vec::new(),
            link_destinations: Vec::new(),
        });
    push_unique(&mut tombstone.content_hashes, record.content_hash);
    push_unique(&mut tombstone.owned_roots, record.owned_root);
    for link in record.link_destinations {
        push_unique(&mut tombstone.link_destinations, link);
    }
    if record.version.is_some() {
        tombstone.retired_version = record.version;
    }
}

fn push_unique<T: PartialEq>(list: &mut Vec<T>, value: T) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// Every Orbit-owned root implied by the manifest, plus `skills_root` itself.
/// A symlink resolving inside any of these is proven Orbit-owned.
pub fn owned_roots(skills_root: &Path, manifest: &SkillOwnershipManifest) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = vec![skills_root.to_path_buf()];
    for record in manifest.managed.values() {
        push_unique(&mut roots, record.owned_root.clone());
    }
    for tombstone in manifest.tombstones.values() {
        for root in &tombstone.owned_roots {
            push_unique(&mut roots, root.clone());
        }
    }
    roots
}

/// Known generated-content hashes recorded for `skill_id` (managed + tombstone).
fn known_hashes(manifest: &SkillOwnershipManifest, skill_id: &str) -> Vec<String> {
    let mut hashes = Vec::new();
    if let Some(record) = manifest.managed.get(skill_id) {
        push_unique(&mut hashes, record.content_hash.clone());
    }
    if let Some(tombstone) = manifest.tombstones.get(skill_id) {
        for hash in &tombstone.content_hashes {
            push_unique(&mut hashes, hash.clone());
        }
    }
    hashes
}

/// Classify an on-disk skill install at `path` (a `<skills-dir>/<skill_id>`
/// entry) against the manifest.
///
/// - **Symlink** → [`SkillOwnership::OrbitOwned`] iff its resolved target lands
///   inside an owned root (broken links fall back to a lexical prefix check);
///   otherwise [`SkillOwnership::Ambiguous`].
/// - **Directory** → [`SkillOwnership::OrbitOwned`] iff its `SKILL.md` hashes to
///   a known generated hash for `skill_id`; a modified copy is
///   [`SkillOwnership::Ambiguous`]; an id with no manifest record is
///   [`SkillOwnership::Unmanaged`].
pub fn classify_install(
    manifest: &SkillOwnershipManifest,
    owned_roots: &[PathBuf],
    skill_id: &str,
    path: &Path,
) -> Result<SkillOwnership, OrbitError> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Ok(SkillOwnership::Unmanaged);
    };

    if meta.file_type().is_symlink() {
        return Ok(classify_symlink(path, owned_roots));
    }

    // A recorded managed link destination that is somehow no longer a symlink is
    // still ours only if it proves out by content; fall through to hash checks.
    let known = known_hashes(manifest, skill_id);
    if known.is_empty() {
        return Ok(SkillOwnership::Unmanaged);
    }

    let skill_md = path.join("SKILL.md");
    let Ok(bytes) = std::fs::read(&skill_md) else {
        // A managed id whose content is unreadable/absent: not proven ours.
        return Ok(SkillOwnership::Ambiguous);
    };
    let hash = content_hash(&bytes);
    if known.contains(&hash) {
        Ok(SkillOwnership::OrbitOwned)
    } else {
        Ok(SkillOwnership::Ambiguous)
    }
}

/// Classify a symlink purely by where it points. Exposed for unlink/teardown,
/// which decide per-link without a skill id.
pub fn classify_symlink(link_path: &Path, owned_roots: &[PathBuf]) -> SkillOwnership {
    let Ok(target) = std::fs::read_link(link_path) else {
        return SkillOwnership::Ambiguous;
    };
    let resolved = if target.is_absolute() {
        target
    } else {
        link_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(target)
    };

    if target_within_owned_root(&resolved, owned_roots) {
        SkillOwnership::OrbitOwned
    } else {
        SkillOwnership::Ambiguous
    }
}

fn target_within_owned_root(resolved: &Path, owned_roots: &[PathBuf]) -> bool {
    // Prefer canonical comparison (handles `..`, symlinked roots), but fall back
    // to a lexical prefix check so broken links into a removed owned root still
    // classify as Orbit-owned.
    let canonical_target = resolved.canonicalize().ok();
    let lexical_target = normalize_path_components(resolved);

    owned_roots.iter().any(|root| {
        if let (Some(ct), Ok(cr)) = (canonical_target.as_ref(), root.canonicalize())
            && ct.starts_with(&cr)
        {
            return true;
        }
        lexical_target.starts_with(normalize_path_components(root))
    })
}

/// Remove only proven Orbit-owned skill symlinks in `links_dir`, leaving user
/// symlinks (targeting outside Orbit roots) and regular files/dirs intact.
/// Returns the number of removed links.
pub fn remove_owned_skill_links(
    links_dir: &Path,
    owned_roots: &[PathBuf],
) -> Result<usize, OrbitError> {
    if !links_dir.exists() {
        return Ok(0);
    }
    let entries = std::fs::read_dir(links_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    let mut removed = 0usize;
    for entry in entries {
        let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
        if !meta.file_type().is_symlink() {
            continue;
        }
        if classify_symlink(&path, owned_roots).is_orbit_owned() {
            std::fs::remove_file(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
            removed += 1;
        }
    }
    Ok(removed)
}
