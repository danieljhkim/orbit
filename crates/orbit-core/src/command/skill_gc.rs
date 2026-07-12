//! `orbit gc skills` collector: reclaims retired Orbit-generated skill
//! directories and stale/broken Orbit-owned skill links, using the ownership
//! and retirement metadata from ORB-10181 (`skill_ownership`) as the only proof
//! of eligibility.
//!
//! Two independent removal classes are found and frozen into the shared
//! [`GcPlan`]:
//!
//! 1. **Retired generated directories** under the global skills root
//!    (`<global_root>/skills/<id>`) whose complete on-disk tree fingerprints to
//!    a known generated value (`classify_install == OrbitOwned`) *and* whose id
//!    has been retired into a tombstone. A modified generated tree fingerprints
//!    differently and stays `Ambiguous`; a same-named user directory with no
//!    manifest record is `Unmanaged`. Both are retained as conflict skips.
//! 2. **Stale / broken owned links** in the supported agent skill roots
//!    (`~/.agents/skills`, `~/.claude/skills`). A link is removable only when it
//!    is provably Orbit-owned (`classify_symlink == OrbitOwned`) *and* its id is
//!    tombstoned (retired) or its target is broken. Links targeting outside
//!    every Orbit-owned root are foreign and retained; non-symlink entries are
//!    user content and retained.
//!
//! Links whose id is a *currently managed* skill but whose target is missing or
//! wrong are reported as `link_repair` skips — a `link`/`init` concern, reported
//! separately from stale retirement and never deleted by GC.
//!
//! ## Mutation safety
//!
//! The genuinely dangerous mutation is the recursive directory delete, so
//! directory candidates carry `path = Some(..)` and pass through the framework's
//! non-bypassable [`validate_candidate_path`](super::gc::validate_candidate_path)
//! gate (root containment + no-follow component check) before removal; because a
//! matching fingerprint proves the tree holds only the exact generated regular
//! files and directories (any embedded symlink or extra entry would change the
//! fingerprint), `remove_dir_all` cannot traverse out of the owned tree. Link
//! candidates carry `path = None` (their agent roots are siblings of the global
//! root, outside the collector's single reported scope root) and the collector
//! performs its own no-follow `remove_file` on the link object after
//! re-classifying it under the GC lock; a symlink's target is never traversed.
//! Emptied Orbit-owned link directories are pruned with `remove_dir`, which only
//! ever removes an empty directory and never follows a symlink.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use orbit_common::types::OrbitError;

use super::gc::{
    GcCandidate, GcCollector, GcContext, GcMutation, GcPlan, GcRevalidation, GcScope, GcSkip,
    GcTarget,
};
use super::skill_ownership::{
    self, OWNERSHIP_MANIFEST_FILE, SkillOwnership, SkillOwnershipManifest,
};

/// GC collector for Orbit-generated skills. See the module docs for the two
/// removal classes and the mutation-safety split.
pub struct SkillsGcCollector {
    skills_root: PathBuf,
    link_roots: Vec<PathBuf>,
    /// Details for each frozen candidate id, populated by `plan` and consumed by
    /// `revalidate`/`apply` in the same `execute_gc` call. Rebuilt on every
    /// plan so repeated invocations stay independent and idempotent.
    planned: Mutex<HashMap<String, PlannedItem>>,
}

/// What a frozen candidate id maps to when it is applied.
#[derive(Debug, Clone)]
enum PlannedItem {
    /// A retired generated skill directory to remove wholesale.
    Directory { path: PathBuf, skill_id: String },
    /// A stale/broken owned skill link to unlink, plus the link root to prune if
    /// it (and its parent) becomes empty.
    Link { path: PathBuf, link_root: PathBuf },
}

/// Verdict for one on-disk generated-skill directory entry.
enum DirVerdict {
    /// Retired + fingerprint-proven: eligible for removal.
    Remove,
    /// A currently managed generated skill in place: healthy, not a candidate.
    Healthy,
    /// Retained with a stable conflict code + human reason.
    Retain { code: &'static str, reason: String },
}

/// Verdict for one entry in an agent skill-link root.
enum LinkVerdict {
    /// Stale or broken owned link: eligible for unlink.
    Remove,
    /// A correct link for a currently managed skill: healthy, not a candidate.
    Healthy,
    /// A currently managed skill whose link is missing/wrong — reported
    /// separately from stale retirement, never deleted.
    Repair { reason: String },
    /// Retained with a stable conflict code + human reason.
    Retain { code: &'static str, reason: String },
}

impl SkillsGcCollector {
    /// Build the collector for the global Orbit root, resolving the generated
    /// skill root and per-agent link roots exactly as init seeds them.
    pub fn for_global_root(global_root: &Path) -> Self {
        let (skills_root, link_roots) = super::init::skill_gc_roots(global_root);
        Self::with_roots(skills_root, link_roots)
    }

    /// Build the collector from explicit roots (used by tests).
    pub fn with_roots(skills_root: PathBuf, link_roots: Vec<PathBuf>) -> Self {
        Self {
            skills_root,
            link_roots,
            planned: Mutex::new(HashMap::new()),
        }
    }

    /// The scope root reported for `orbit gc skills`: the global generated-skill
    /// root's parent (the global Orbit root). Directory candidates live beneath
    /// it and are validated against it by the framework gate.
    pub fn scope_root(&self) -> PathBuf {
        self.skills_root
            .parent()
            .map_or_else(|| self.skills_root.clone(), Path::to_path_buf)
    }

    /// Convenience for the CLI: the `GcScope` this collector operates in.
    pub fn scope(&self) -> GcScope {
        GcScope::Global {
            root: self.scope_root(),
        }
    }

    fn classify_dir(
        &self,
        manifest: &SkillOwnershipManifest,
        owned_roots: &[PathBuf],
        current: &BTreeSet<String>,
        tombstoned: &BTreeSet<String>,
        skill_id: &str,
        path: &Path,
    ) -> Result<DirVerdict, OrbitError> {
        // Only a real directory can be a generated skill tree. A symlink named
        // like a skill in the generated root is unexpected; never remove_dir_all
        // through it — retain it.
        let meta = fs::symlink_metadata(path).map_err(|e| OrbitError::Io(e.to_string()))?;
        if meta.file_type().is_symlink() {
            return Ok(DirVerdict::Retain {
                code: "ambiguous_owned",
                reason: "unexpected symlink in the generated skills root; left intact".to_string(),
            });
        }

        match skill_ownership::classify_install(manifest, owned_roots, skill_id, path)? {
            SkillOwnership::OrbitOwned => {
                if current.contains(skill_id) {
                    Ok(DirVerdict::Healthy)
                } else if tombstoned.contains(skill_id) {
                    Ok(DirVerdict::Remove)
                } else {
                    // OrbitOwned implies a known fingerprint, which implies a
                    // managed or tombstone record — this branch is unreachable,
                    // but fail closed rather than remove without a retirement
                    // proof.
                    Ok(DirVerdict::Retain {
                        code: "ambiguous_owned",
                        reason: "owned generated tree without a retirement record; left intact"
                            .to_string(),
                    })
                }
            }
            SkillOwnership::Ambiguous => Ok(DirVerdict::Retain {
                code: "modified_generated",
                reason: "generated skill content was modified or added to; left intact".to_string(),
            }),
            SkillOwnership::Unmanaged => Ok(DirVerdict::Retain {
                code: "user_content",
                reason: "same-named skill Orbit never generated; left intact".to_string(),
            }),
        }
    }

    fn classify_link(
        &self,
        owned_roots: &[PathBuf],
        current: &BTreeSet<String>,
        tombstoned: &BTreeSet<String>,
        skill_id: &str,
        link_path: &Path,
    ) -> Result<LinkVerdict, OrbitError> {
        let meta = fs::symlink_metadata(link_path).map_err(|e| OrbitError::Io(e.to_string()))?;
        if !meta.file_type().is_symlink() {
            return Ok(LinkVerdict::Retain {
                code: "user_content",
                reason: "non-symlink entry in an agent skills root; left intact".to_string(),
            });
        }

        if !skill_ownership::classify_symlink(link_path, owned_roots).is_orbit_owned() {
            return Ok(LinkVerdict::Retain {
                code: "foreign_link",
                reason: "symlink targets outside every Orbit-owned root; left intact".to_string(),
            });
        }

        let expected = self.skills_root.join(skill_id);
        let correct = link_points_at(link_path, &expected);
        let target_exists = link_target_exists(link_path);

        if current.contains(skill_id) {
            if correct {
                Ok(LinkVerdict::Healthy)
            } else {
                Ok(LinkVerdict::Repair {
                    reason: "managed skill link is missing its target or points elsewhere; run `orbit skill link`".to_string(),
                })
            }
        } else if tombstoned.contains(skill_id) {
            Ok(LinkVerdict::Remove)
        } else if !target_exists {
            // Broken owned link whose id Orbit no longer tracks: the target is
            // already gone, so removing the dangling link is safe.
            Ok(LinkVerdict::Remove)
        } else {
            Ok(LinkVerdict::Retain {
                code: "ambiguous_link",
                reason: "owned link with an unknown skill id and a live target; left intact"
                    .to_string(),
            })
        }
    }
}

impl GcCollector for SkillsGcCollector {
    fn target(&self) -> GcTarget {
        GcTarget::Skills
    }

    fn plan(&self, _context: &GcContext<'_>) -> Result<GcPlan, OrbitError> {
        let manifest = skill_ownership::load_manifest(&self.skills_root)?;
        let owned_roots = skill_ownership::owned_roots(&self.skills_root, &manifest);
        let current: BTreeSet<String> = manifest.managed.keys().cloned().collect();
        let tombstoned: BTreeSet<String> = manifest.tombstones.keys().cloned().collect();

        let mut planned = lock_planned(&self.planned)?;
        planned.clear();

        let mut plan = GcPlan::empty(GcTarget::Skills);
        let mut scanned = 0u64;
        let mut scanned_bytes = 0u64;

        // 1. Retired generated directories in the global skills root.
        if self.skills_root.is_dir() {
            for entry in read_dir_sorted(&self.skills_root)? {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == OWNERSHIP_MANIFEST_FILE {
                    continue;
                }
                let path = entry.path();
                scanned += 1;
                let size = dir_size(&path).unwrap_or(0);
                scanned_bytes = scanned_bytes.saturating_add(size);
                match self.classify_dir(
                    &manifest,
                    &owned_roots,
                    &current,
                    &tombstoned,
                    &name,
                    &path,
                )? {
                    DirVerdict::Remove => {
                        let id = format!("dir:{name}");
                        planned.insert(
                            id.clone(),
                            PlannedItem::Directory {
                                path: path.clone(),
                                skill_id: name.clone(),
                            },
                        );
                        plan.candidates.push(GcCandidate {
                            id,
                            action: "delete".to_string(),
                            path: Some(path),
                            bytes: Some(size),
                            ownership_evidence: format!(
                                "generated tree fingerprint match; retired skill `{name}` tombstoned"
                            ),
                            retention_evidence: "retired from the default skill catalog".to_string(),
                            expected_state: "retired-generated-directory".to_string(),
                            allow_owned_symlink: false,
                        });
                    }
                    DirVerdict::Healthy => {}
                    DirVerdict::Retain { code, reason } => plan.skipped.push(GcSkip {
                        id: format!("dir:{name}"),
                        code: code.to_string(),
                        reason,
                    }),
                }
            }
        }

        // 2. Stale/broken owned links + current-skill repair reporting.
        for link_root in &self.link_roots {
            if !link_root.is_dir() {
                continue;
            }
            let mut present: BTreeSet<String> = BTreeSet::new();
            for entry in read_dir_sorted(link_root)? {
                let name = entry.file_name().to_string_lossy().into_owned();
                present.insert(name.clone());
                let path = entry.path();
                scanned += 1;
                let size = fs::symlink_metadata(&path).map(|m| m.len()).unwrap_or(0);
                scanned_bytes = scanned_bytes.saturating_add(size);
                match self.classify_link(&owned_roots, &current, &tombstoned, &name, &path)? {
                    LinkVerdict::Remove => {
                        let id = format!("link:{}", path.display());
                        planned.insert(
                            id.clone(),
                            PlannedItem::Link {
                                path: path.clone(),
                                link_root: link_root.clone(),
                            },
                        );
                        plan.candidates.push(GcCandidate {
                            id,
                            action: "unlink".to_string(),
                            // Agent link roots are siblings of the global root
                            // (outside the reported scope root); the collector
                            // validates and unlinks the link object itself.
                            path: None,
                            bytes: Some(size),
                            ownership_evidence: "Orbit-owned symlink (target within an owned root)"
                                .to_string(),
                            retention_evidence: "retired skill or broken target".to_string(),
                            expected_state: "stale-owned-link".to_string(),
                            allow_owned_symlink: true,
                        });
                    }
                    LinkVerdict::Healthy => {}
                    LinkVerdict::Repair { reason } => plan.skipped.push(GcSkip {
                        id: format!("link:{}", path.display()),
                        code: "link_repair".to_string(),
                        reason,
                    }),
                    LinkVerdict::Retain { code, reason } => plan.skipped.push(GcSkip {
                        id: format!("link:{}", path.display()),
                        code: code.to_string(),
                        reason,
                    }),
                }
            }

            // Currently managed skills with no link at all in this managed root
            // are a repair concern, reported separately from stale retirement.
            for skill_id in &current {
                if !present.contains(skill_id) {
                    plan.skipped.push(GcSkip {
                        id: format!("link:{}", link_root.join(skill_id).display()),
                        code: "link_repair".to_string(),
                        reason:
                            "managed skill has no link in this agent root; run `orbit skill link`"
                                .to_string(),
                    });
                }
            }
        }

        plan.scanned = scanned;
        plan.scanned_bytes = Some(scanned_bytes);
        Ok(plan)
    }

    fn revalidate(
        &self,
        candidate: &GcCandidate,
        _context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError> {
        let planned = lock_planned(&self.planned)?;
        let Some(item) = planned.get(&candidate.id) else {
            return Ok(GcRevalidation::Skip {
                code: "state_changed".to_string(),
                reason: "candidate is no longer part of the frozen plan".to_string(),
            });
        };

        // Re-derive ownership under the GC lock so a concurrent edit between plan
        // and apply cannot let a now-ambiguous entry through.
        let manifest = skill_ownership::load_manifest(&self.skills_root)?;
        let owned_roots = skill_ownership::owned_roots(&self.skills_root, &manifest);
        let current: BTreeSet<String> = manifest.managed.keys().cloned().collect();
        let tombstoned: BTreeSet<String> = manifest.tombstones.keys().cloned().collect();

        let still_removable = match item {
            PlannedItem::Directory { path, skill_id } => {
                path.exists()
                    && matches!(
                        self.classify_dir(
                            &manifest,
                            &owned_roots,
                            &current,
                            &tombstoned,
                            skill_id,
                            path
                        )?,
                        DirVerdict::Remove
                    )
            }
            PlannedItem::Link { path, .. } => {
                let skill_id = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                fs::symlink_metadata(path).is_ok()
                    && matches!(
                        self.classify_link(&owned_roots, &current, &tombstoned, &skill_id, path)?,
                        LinkVerdict::Remove
                    )
            }
        };

        if still_removable {
            Ok(GcRevalidation::Ready)
        } else {
            Ok(GcRevalidation::Skip {
                code: "state_changed".to_string(),
                reason: "ownership or state changed since planning; left intact".to_string(),
            })
        }
    }

    fn apply(
        &self,
        candidate: &GcCandidate,
        _context: &GcContext<'_>,
    ) -> Result<GcMutation, OrbitError> {
        let item = {
            let planned = lock_planned(&self.planned)?;
            planned.get(&candidate.id).cloned().ok_or_else(|| {
                OrbitError::Execution(format!("no planned skill GC item for `{}`", candidate.id))
            })?
        };
        match item {
            PlannedItem::Directory { path, .. } => {
                // The framework already validated containment + no-follow path
                // components for this `Some(path)` candidate; the matching
                // fingerprint proves the tree contains no symlinks.
                fs::remove_dir_all(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
                Ok(GcMutation {
                    reclaimed_bytes: candidate.bytes,
                })
            }
            PlannedItem::Link { path, link_root } => {
                let meta =
                    fs::symlink_metadata(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
                if !meta.file_type().is_symlink() {
                    return Err(OrbitError::PolicyDenied(format!(
                        "refusing to remove non-symlink skill link candidate: {}",
                        path.display()
                    )));
                }
                // remove_file on a symlink removes the link object, never its
                // target, and never follows it.
                fs::remove_file(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
                prune_empty_dirs(&link_root);
                Ok(GcMutation {
                    reclaimed_bytes: candidate.bytes,
                })
            }
        }
    }
}

/// Lock the frozen-plan map, translating a poisoned lock into an `OrbitError`
/// rather than propagating a panic across the crate boundary.
fn lock_planned(
    planned: &Mutex<HashMap<String, PlannedItem>>,
) -> Result<std::sync::MutexGuard<'_, HashMap<String, PlannedItem>>, OrbitError> {
    planned
        .lock()
        .map_err(|_| OrbitError::Execution("skills GC plan state lock was poisoned".to_string()))
}

/// Read a directory's entries sorted by file name for deterministic planning.
fn read_dir_sorted(dir: &Path) -> Result<Vec<fs::DirEntry>, OrbitError> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(dir)
        .map_err(|e| OrbitError::Io(e.to_string()))?
        .collect::<Result<_, _>>()
        .map_err(|e| OrbitError::Io(e.to_string()))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

/// Best-effort recursive byte total of a directory, never following symlinks.
fn dir_size(dir: &Path) -> Option<u64> {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).ok()? {
            let entry = entry.ok()?;
            let meta = fs::symlink_metadata(entry.path()).ok()?;
            let file_type = meta.file_type();
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Some(total)
}

/// Whether `link_path`'s resolved target equals `expected` and that target
/// currently exists. Uses canonicalization so `..`/symlinked roots compare
/// equal; a broken link yields `false`.
fn link_points_at(link_path: &Path, expected: &Path) -> bool {
    let Some(resolved) = resolve_link_target(link_path) else {
        return false;
    };
    match (resolved.canonicalize(), expected.canonicalize()) {
        (Ok(actual), Ok(want)) => actual == want,
        _ => false,
    }
}

/// Whether the symlink at `link_path` resolves to an existing target.
fn link_target_exists(link_path: &Path) -> bool {
    resolve_link_target(link_path).is_some_and(|target| target.exists())
}

/// The absolute target of a symlink, resolving a relative target against the
/// link's parent. `None` if the path is not a readable symlink.
fn resolve_link_target(link_path: &Path) -> Option<PathBuf> {
    let target = fs::read_link(link_path).ok()?;
    if target.is_absolute() {
        Some(target)
    } else {
        Some(
            link_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(target),
        )
    }
}

/// Prune `link_root` and then its parent when each is a genuinely empty
/// directory, mirroring `orbit skill unlink`. `remove_dir` only removes an empty
/// directory and never follows a symlink, so a non-empty or symlinked path is
/// left untouched.
fn prune_empty_dirs(link_root: &Path) {
    if remove_if_empty_dir(link_root)
        && let Some(parent) = link_root.parent()
    {
        let _ = remove_if_empty_dir(parent);
    }
}

fn remove_if_empty_dir(dir: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(dir) else {
        return false;
    };
    if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
        return false;
    }
    let Ok(mut entries) = fs::read_dir(dir) else {
        return false;
    };
    if entries.next().is_some() {
        return false;
    }
    fs::remove_dir(dir).is_ok()
}
