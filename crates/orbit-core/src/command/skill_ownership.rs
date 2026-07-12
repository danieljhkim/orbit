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
//! 1. **Exact known tree fingerprint** — the on-disk skill directory
//!    fingerprints to a value recorded here (either the currently-managed record
//!    or a retired tombstone). The fingerprint is a deterministic, no-follow
//!    digest of the *complete* generated skill tree — every file's relative
//!    path, entry type, and contents, not just `SKILL.md` — so a modified
//!    resource, a removed generated file, an added file, an embedded symlink, or
//!    any unexpected entry type all change the fingerprint and fail closed to
//!    [`SkillOwnership::Ambiguous`]. See `fingerprint` for the exact encoding.
//! 2. **Orbit-targeting managed symlink** — a link whose resolved target lands
//!    inside a known Orbit-owned root. Works even for broken links (the target
//!    directory was already removed) via a lexical prefix fallback.
//!
//! Anything else — a same-named user directory, a modified generated skill, or
//! a symlink pointing outside every Orbit root — is reported as ambiguous or
//! unmanaged and is never auto-claimed or removed.
//!
//! The `.ownership.json` manifest is the only metadata excluded from the
//! fingerprint, and it is excluded *structurally*: it lives at `<skills_root>`,
//! a sibling of every `<skills_root>/<skill_id>/` tree the fingerprint covers,
//! so the fingerprint never digests itself. No file *inside* a skill tree is
//! excluded — every entry counts, which is what makes the check fail closed.

use std::collections::{BTreeMap, BTreeSet};
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
    /// Whole-tree fingerprint (hex) of the complete generated skill tree as
    /// rendered for `owned_root` — see `fingerprint` for the encoding.
    pub tree_fingerprint: String,
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
    /// Every whole-tree fingerprint this skill was ever recorded with, retained
    /// so pre-retirement / pre-upgrade installs stay provably Orbit-owned.
    #[serde(default)]
    pub tree_fingerprints: Vec<String>,
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
    /// Proven Orbit-owned: an exact known tree fingerprint or an Orbit-targeting
    /// symlink. Safe for unlink/teardown/gc to remove.
    OrbitOwned,
    /// A candidate (right skill id, or a symlink) that lacks ownership proof — a
    /// modified/added/removed generated file, an embedded symlink or unexpected
    /// entry type, or a symlink targeting outside Orbit roots. Never auto-removed.
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

/// sha256 (hex) of arbitrary bytes.
pub fn content_hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Domain-separation tag mixed into every tree fingerprint. Bumped if the
/// fingerprint encoding (not the skill contents) ever changes.
const TREE_FINGERPRINT_DOMAIN: &[u8] = b"orbit.skill-tree.v1\n";

/// Entry type recorded for one node of a skill tree. Only [`EntryKind::File`]
/// carries content; everything else contributes its type alone, so a directory,
/// symlink, or exotic node where Orbit generated a regular file (or vice versa)
/// changes the fingerprint and fails closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Dir,
    File,
    Symlink,
    Other,
}

impl EntryKind {
    /// Stable single-byte tag folded into the fingerprint. Never reuse a value.
    fn tag(self) -> u8 {
        match self {
            EntryKind::Dir => b'd',
            EntryKind::File => b'f',
            EntryKind::Symlink => b'l',
            EntryKind::Other => b'o',
        }
    }
}

/// One node of a skill tree: its skill-root-relative path, entry type, and (for
/// regular files) the sha256 of its contents.
#[derive(Debug, Clone)]
struct TreeEntry {
    /// Path relative to the skill root, encoded with `/` separators (see
    /// [`encode_relative`]). Never empty (the root itself is not an entry).
    path: String,
    kind: EntryKind,
    /// sha256 of file contents for [`EntryKind::File`]; all-zero otherwise.
    digest: [u8; 32],
}

/// Deterministic, no-follow fingerprint of a complete skill tree.
///
/// # Encoding
///
/// The fingerprint is `sha256` over, in order:
///
/// 1. the domain tag [`TREE_FINGERPRINT_DOMAIN`];
/// 2. every entry, sorted by `(path, kind-tag)` (path bytes compared
///    lexicographically), each contributing the little-endian `u64` length of
///    its path, the path bytes, its one-byte kind tag, and its 32-byte content
///    digest (all-zero for non-file entries).
///
/// **Path encoding.** Paths are relative to the skill root and joined with `/`
/// on every platform (see [`encode_relative`]); the leading length prefix makes
/// the concatenation injective, so no path can forge another entry's boundary.
///
/// **Ordering.** Entries are sorted by encoded path then kind tag, so the
/// fingerprint is independent of directory-iteration order.
///
/// Directories are included as entries (so an added empty directory is
/// detected); symlinks and other node types are recorded by type only and are
/// never followed. The `.ownership.json` manifest is excluded structurally — it
/// is never inside a skill tree (see the module docs), so it is not special-cased
/// here.
fn fingerprint(entries_source: impl FingerprintSource) -> Result<String, OrbitError> {
    let mut entries = entries_source.collect_entries()?;
    entries.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.kind.tag().cmp(&b.kind.tag()))
    });
    let mut hasher = Sha256::new();
    hasher.update(TREE_FINGERPRINT_DOMAIN);
    for entry in &entries {
        hasher.update((entry.path.len() as u64).to_le_bytes());
        hasher.update(entry.path.as_bytes());
        hasher.update([entry.kind.tag()]);
        hasher.update(entry.digest);
    }
    Ok(hex(&hasher.finalize()))
}

/// A source of tree entries — either the compiled-in generated files (expected)
/// or an on-disk directory walk (actual). Keeps [`fingerprint`] identical for
/// both sides so seeding and classification cannot diverge.
trait FingerprintSource {
    fn collect_entries(self) -> Result<Vec<TreeEntry>, OrbitError>;
}

/// Encode a path relative to `base` with `/` separators on every platform. A
/// path not under `base` is encoded verbatim (should not happen for entries
/// produced by walking `base`).
fn encode_relative(base: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    encode_components(rel)
}

/// Encode an already-relative path (from a `Path` or a `/`-style string) with
/// `/` separators, dropping `.`/root components.
fn encode_components(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(os) => Some(os.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Every ancestor directory of an encoded file path, from the shallowest to the
/// file's immediate parent (the file itself excluded). `"a/b/c.md"` →
/// `["a", "a/b"]`; `"SKILL.md"` → `[]`.
fn ancestor_dirs(encoded_path: &str) -> Vec<String> {
    let parts: Vec<&str> = encoded_path.split('/').collect();
    (1..parts.len()).map(|i| parts[..i].join("/")).collect()
}

/// One Orbit-generated file within a skill's tree, as the seeder would write it.
#[derive(Debug, Clone)]
pub struct GeneratedFile {
    /// Path relative to the skill's owned directory (e.g. `SKILL.md` or
    /// `references/guide.md`); encoded canonically when fingerprinted.
    pub relative_path: String,
    pub contents: Vec<u8>,
}

/// Expected-side source: the compiled-in files Orbit generates for one skill.
struct GeneratedFiles<'a>(&'a [GeneratedFile]);

impl FingerprintSource for GeneratedFiles<'_> {
    fn collect_entries(self) -> Result<Vec<TreeEntry>, OrbitError> {
        let mut entries = Vec::new();
        let mut dirs: BTreeSet<String> = BTreeSet::new();
        for file in self.0 {
            let path = encode_components(Path::new(&file.relative_path));
            for dir in ancestor_dirs(&path) {
                if dirs.insert(dir.clone()) {
                    entries.push(TreeEntry {
                        path: dir,
                        kind: EntryKind::Dir,
                        digest: [0u8; 32],
                    });
                }
            }
            entries.push(TreeEntry {
                path,
                kind: EntryKind::File,
                digest: Sha256::digest(&file.contents).into(),
            });
        }
        Ok(entries)
    }
}

/// Actual-side source: a no-follow walk of an on-disk skill directory.
struct OnDiskTree<'a>(&'a Path);

impl FingerprintSource for OnDiskTree<'_> {
    fn collect_entries(self) -> Result<Vec<TreeEntry>, OrbitError> {
        let mut entries = Vec::new();
        collect_on_disk(self.0, self.0, &mut entries)?;
        Ok(entries)
    }
}

/// Recursively collect tree entries under `dir` (rooted at `root`) without ever
/// following a symlink: symlinks are recorded by type and never traversed, so a
/// link cannot redirect the walk outside the skill root.
fn collect_on_disk(
    root: &Path,
    dir: &Path,
    entries: &mut Vec<TreeEntry>,
) -> Result<(), OrbitError> {
    let read = std::fs::read_dir(dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    for entry in read {
        let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
        let file_type = meta.file_type();
        let encoded = encode_relative(root, &path);
        if file_type.is_symlink() {
            entries.push(TreeEntry {
                path: encoded,
                kind: EntryKind::Symlink,
                digest: [0u8; 32],
            });
        } else if file_type.is_dir() {
            entries.push(TreeEntry {
                path: encoded,
                kind: EntryKind::Dir,
                digest: [0u8; 32],
            });
            collect_on_disk(root, &path, entries)?;
        } else if file_type.is_file() {
            let bytes = std::fs::read(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
            entries.push(TreeEntry {
                path: encoded,
                kind: EntryKind::File,
                digest: Sha256::digest(&bytes).into(),
            });
        } else {
            entries.push(TreeEntry {
                path: encoded,
                kind: EntryKind::Other,
                digest: [0u8; 32],
            });
        }
    }
    Ok(())
}

/// Fingerprint the complete set of files Orbit generates for one skill (the
/// expected value stored in the manifest at seed time).
pub fn fingerprint_generated(files: &[GeneratedFile]) -> Result<String, OrbitError> {
    fingerprint(GeneratedFiles(files))
}

/// One generated skill's identity as produced by the seeder: its id, the
/// whole-tree fingerprint of every file Orbit generates for it, and the catalog
/// version.
#[derive(Debug, Clone)]
pub struct GeneratedSkill {
    pub skill_id: String,
    /// Whole-tree fingerprint of the complete generated skill tree.
    pub tree_fingerprint: String,
    pub version: Option<String>,
}

impl GeneratedSkill {
    /// Build a [`GeneratedSkill`] from the exact files Orbit generates for it,
    /// fingerprinting the whole tree so the recorded value matches what
    /// [`classify_install`] later computes from disk.
    pub fn from_files(
        skill_id: impl Into<String>,
        version: Option<String>,
        files: &[GeneratedFile],
    ) -> Result<Self, OrbitError> {
        Ok(Self {
            skill_id: skill_id.into(),
            tree_fingerprint: fingerprint_generated(files)?,
            version,
        })
    }
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
                tree_fingerprint: skill.tree_fingerprint.clone(),
                version: skill.version.clone(),
                owned_root: owned_root.clone(),
                link_destinations: Vec::new(),
            });
        // If the tree fingerprint changed (template/resource update), retain the
        // old fingerprint as a known-generated one on the tombstone so prior
        // installs remain provably Orbit-owned.
        if entry.tree_fingerprint != skill.tree_fingerprint {
            let previous = entry.tree_fingerprint.clone();
            let tombstone = manifest
                .tombstones
                .entry(skill.skill_id.clone())
                .or_insert_with(|| SkillTombstone {
                    skill_id: skill.skill_id.clone(),
                    tree_fingerprints: Vec::new(),
                    retired_version: None,
                    owned_roots: Vec::new(),
                    link_destinations: Vec::new(),
                });
            push_unique(&mut tombstone.tree_fingerprints, previous);
        }
        entry.tree_fingerprint = skill.tree_fingerprint.clone();
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
            tree_fingerprints: Vec::new(),
            retired_version: None,
            owned_roots: Vec::new(),
            link_destinations: Vec::new(),
        });
    push_unique(&mut tombstone.tree_fingerprints, record.tree_fingerprint);
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

/// Known whole-tree fingerprints recorded for `skill_id` (managed + tombstone).
fn known_fingerprints(manifest: &SkillOwnershipManifest, skill_id: &str) -> Vec<String> {
    let mut fingerprints = Vec::new();
    if let Some(record) = manifest.managed.get(skill_id) {
        push_unique(&mut fingerprints, record.tree_fingerprint.clone());
    }
    if let Some(tombstone) = manifest.tombstones.get(skill_id) {
        for fingerprint in &tombstone.tree_fingerprints {
            push_unique(&mut fingerprints, fingerprint.clone());
        }
    }
    fingerprints
}

/// Classify an on-disk skill install at `path` (a `<skills-dir>/<skill_id>`
/// entry) against the manifest.
///
/// - **Symlink** → [`SkillOwnership::OrbitOwned`] iff its resolved target lands
///   inside an owned root (broken links fall back to a lexical prefix check);
///   otherwise [`SkillOwnership::Ambiguous`].
/// - **Directory** → [`SkillOwnership::OrbitOwned`] iff its complete on-disk
///   tree fingerprints (no-follow, see `fingerprint`) to a known generated
///   value for `skill_id`; any modified/added/removed file, embedded symlink, or
///   unexpected entry type yields a different fingerprint and is
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

    let known = known_fingerprints(manifest, skill_id);
    if known.is_empty() {
        return Ok(SkillOwnership::Unmanaged);
    }

    // A managed id that is not a directory can't be a generated skill tree: fail
    // closed rather than claiming it.
    if !meta.file_type().is_dir() {
        return Ok(SkillOwnership::Ambiguous);
    }

    // Fingerprint the whole tree with no symlink following. If the tree can't be
    // read we cannot prove ownership, so fail closed to Ambiguous rather than
    // erroring the classification.
    let actual = match fingerprint(OnDiskTree(path)) {
        Ok(fp) => fp,
        Err(_) => return Ok(SkillOwnership::Ambiguous),
    };
    if known.contains(&actual) {
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
