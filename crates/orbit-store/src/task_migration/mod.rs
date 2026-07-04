//! Task-migration tooling: export a workspace's task bundles to a portable
//! tar.zst archive and import them into another machine's registry, resolving
//! id collisions by renumbering through the local allocator.
//!
//! The engine lives in `orbit-store` because it needs the canonical bundle I/O
//! primitives ([`read_bundle_at`]/[`write_bundle_at`]) and the private
//! [`TaskRegistryStore`] internals. `orbit-core` exposes thin facades over it;
//! `orbit-cli` wires the `orbit task export/import/reindex` surfaces.
//!
//! # Model
//! - Canonical bundles live at `<global>/tasks/workspaces/<ws-id>/<ORB-xxxxx>/`.
//! - Task ids are a *global* primary key in `index.sqlite` and the allocator has
//!   a single `local` authority, so merging two machines' tasks collides on ids.
//! - Export copies bundle trees verbatim plus a [`TaskMigrationManifest`].
//! - Import validates everything *before* mutating state, then keeps free ids,
//!   renumbers collisions (rewriting relation targets within the imported set),
//!   rebuilds index rows from bundle YAML, bumps the allocator past the max
//!   landed id, recreates the `.orbit/tasks/` symlink projection, and is
//!   idempotent on re-run.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_common::types::{OrbitError, TASK_ARTIFACT_SCHEMA_VERSION, validate_orb_task_id};
use serde::{Deserialize, Serialize};

use crate::file::task_store::v2_bundle::{TaskBundleV2, read_bundle_at, write_bundle_at};
use crate::sqlite::task_registry::{
    BindWorkspaceParams, ProjectionRebuildResult, TaskRegistryStore, parse_orb_task_number,
};

mod archive;
mod reindex;

#[cfg(test)]
mod tests;

pub use reindex::{ReindexOutcome, reindex_workspace};

/// Archive container-format version. Bumped only when the archive *layout*
/// (manifest shape / entry paths) changes incompatibly.
pub const MIGRATION_FORMAT_VERSION: u32 = 1;

/// Manifest written at the root of every migration archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskMigrationManifest {
    /// [`MIGRATION_FORMAT_VERSION`] the archive was written with.
    pub format_version: u32,
    /// [`TASK_ARTIFACT_SCHEMA_VERSION`] of the bundles inside.
    pub task_schema_version: u32,
    /// Workspace id the bundles were exported from.
    pub source_workspace_id: String,
    /// Human-readable slug of the source workspace.
    pub source_workspace_slug: String,
    /// Task ids contained in the archive (canonical `ORB-00000` form).
    pub task_ids: Vec<String>,
    /// When the archive was produced.
    pub exported_at: DateTime<Utc>,
}

/// Which tasks an export should include.
#[derive(Debug, Clone)]
pub enum ExportSelection {
    /// Every task registered to the workspace.
    All,
    /// An explicit set of task ids (each must be registered to the workspace).
    Ids(Vec<String>),
}

/// Result of [`export_tasks`].
#[derive(Debug, Clone)]
pub struct ExportOutcome {
    /// Path the archive was written to.
    pub archive_path: PathBuf,
    /// Source workspace id.
    pub workspace_id: String,
    /// Task ids written into the archive.
    pub task_ids: Vec<String>,
}

/// How to resolve an imported task id that already exists locally (with
/// non-identical content).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportConflictPolicy {
    /// Allocate a fresh local id for the colliding task and rewrite references.
    Renumber,
    /// Leave the local task untouched and drop the incoming one.
    Skip,
    /// Abort the whole import on the first collision.
    Fail,
}

/// What happened to a single task during import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportAction {
    /// Landed under its original id (was free locally).
    Kept,
    /// Collided; landed under a freshly allocated id.
    Renumbered,
    /// Already present locally with identical content — skipped (idempotent).
    AlreadyPresent,
    /// Collided and `--on-conflict=skip` dropped it.
    SkippedConflict,
}

/// Per-task import record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedTask {
    /// Id in the source archive.
    pub source_id: String,
    /// Id the task landed under locally (equals `source_id` unless renumbered).
    pub final_id: String,
    /// Outcome for this task.
    pub action: ImportAction,
}

/// Result of [`import_tasks`].
#[derive(Debug, Clone)]
pub struct ImportOutcome {
    /// Target workspace the tasks landed in.
    pub workspace_id: String,
    /// True if import registered a new (previously unknown) workspace binding.
    pub registered_workspace: bool,
    /// Per-task records.
    pub tasks: Vec<ImportedTask>,
    /// Old→new id map for renumbered tasks (empty if none renumbered).
    pub id_remap: BTreeMap<String, String>,
    /// Path of the written old→new mapping file, if any renumbering occurred.
    pub id_map_path: Option<PathBuf>,
    /// Projection rebuild result for the target workspace.
    pub projection: ProjectionRebuildResult,
}

/// Export the selected tasks of `workspace_id` to a tar.zst archive at `out_path`.
pub fn export_tasks(
    registry: &TaskRegistryStore,
    workspace_id: &str,
    selection: ExportSelection,
    out_path: &Path,
    exported_at: DateTime<Utc>,
) -> Result<ExportOutcome, OrbitError> {
    let binding = registry
        .find_workspace_binding(workspace_id)?
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "workspace '{workspace_id}' is not registered locally"
            ))
        })?;
    let workspace_id = binding.workspace_id.clone();

    let registered: BTreeSet<String> = registry
        .tasks_for_workspace(&workspace_id)?
        .into_iter()
        .map(|task| task.task_id)
        .collect();

    let task_ids: Vec<String> = match selection {
        ExportSelection::All => registered.iter().cloned().collect(),
        ExportSelection::Ids(ids) => {
            let mut resolved = Vec::new();
            let mut seen = BTreeSet::new();
            for raw in ids {
                let id = raw.trim().to_string();
                validate_orb_task_id(&id)?;
                if !registered.contains(&id) {
                    return Err(OrbitError::InvalidInput(format!(
                        "task '{id}' is not registered to workspace '{workspace_id}'"
                    )));
                }
                if seen.insert(id.clone()) {
                    resolved.push(id);
                }
            }
            resolved.sort();
            resolved
        }
    };

    if task_ids.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "workspace '{workspace_id}' has no tasks to export"
        )));
    }

    let mut bundle_dirs = Vec::with_capacity(task_ids.len());
    for id in &task_ids {
        let dir = registry.canonical_task_bundle_path(&workspace_id, id)?;
        if !dir.is_dir() {
            return Err(OrbitError::Store(format!(
                "canonical bundle for '{id}' is missing at {}",
                dir.display()
            )));
        }
        bundle_dirs.push((id.clone(), dir));
    }

    let manifest = TaskMigrationManifest {
        format_version: MIGRATION_FORMAT_VERSION,
        task_schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
        source_workspace_id: workspace_id.clone(),
        source_workspace_slug: binding.slug.clone(),
        task_ids: task_ids.clone(),
        exported_at,
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| OrbitError::Store(format!("failed to encode manifest: {e}")))?;

    archive::write_archive(out_path, &manifest_json, &bundle_dirs)?;

    Ok(ExportOutcome {
        archive_path: out_path.to_path_buf(),
        workspace_id,
        task_ids,
    })
}

/// A validated, staged bundle read out of an archive before any state mutation.
struct StagedBundle {
    source_id: String,
    bundle: TaskBundleV2,
}

/// Resolved import target after workspace resolution.
struct ImportTarget {
    workspace_id: String,
    orbit_dir: PathBuf,
    /// Set when import must register a new binding for this workspace.
    register: Option<BindWorkspaceParams>,
}

/// Import tasks from a tar.zst archive into the local registry.
///
/// `target_workspace_id` overrides the destination; otherwise the archive's
/// source workspace is used (registering it if unknown). Conflicts on
/// already-used ids are resolved by `policy`.
pub fn import_tasks(
    registry: &TaskRegistryStore,
    archive_path: &Path,
    target_workspace_id: Option<&str>,
    policy: ImportConflictPolicy,
) -> Result<ImportOutcome, OrbitError> {
    // ---- Phase 1: validate everything before touching any state. ----
    let staging = tempfile::Builder::new()
        .prefix("orbit-task-import-")
        .tempdir()
        .map_err(|e| OrbitError::Io(e.to_string()))?;
    archive::extract_archive(archive_path, staging.path())?;

    let manifest = read_manifest(staging.path())?;
    validate_manifest(&manifest)?;

    let staged = stage_bundles(staging.path(), &manifest)?;
    let target = resolve_target(registry, &manifest, target_workspace_id)?;

    // Classify each staged bundle against the current registry.
    let mut kept: Vec<StagedBundle> = Vec::new();
    let mut to_renumber: Vec<StagedBundle> = Vec::new();
    let mut records: Vec<ImportedTask> = Vec::new();
    for staged in staged {
        match registry.find_task_binding(&staged.source_id)? {
            None => kept.push(staged),
            Some(existing) => {
                let identical = existing.workspace_id == target.workspace_id
                    && read_bundle_at(&existing.canonical_path)
                        .ok()
                        .is_some_and(|current| current == staged.bundle);
                if identical {
                    records.push(ImportedTask {
                        source_id: staged.source_id.clone(),
                        final_id: staged.source_id,
                        action: ImportAction::AlreadyPresent,
                    });
                } else {
                    match policy {
                        ImportConflictPolicy::Fail => {
                            return Err(OrbitError::InvalidInput(format!(
                                "task id '{}' already exists locally; import aborted (--on-conflict=fail)",
                                staged.source_id
                            )));
                        }
                        ImportConflictPolicy::Skip => records.push(ImportedTask {
                            source_id: staged.source_id.clone(),
                            final_id: staged.source_id,
                            action: ImportAction::SkippedConflict,
                        }),
                        ImportConflictPolicy::Renumber => to_renumber.push(staged),
                    }
                }
            }
        }
    }

    // Nothing new to write (fully idempotent re-run, or everything skipped).
    if kept.is_empty() && to_renumber.is_empty() {
        let projection = rebuild_projection_best_effort(registry, &target);
        return Ok(ImportOutcome {
            workspace_id: target.workspace_id,
            registered_workspace: false,
            tasks: records,
            id_remap: BTreeMap::new(),
            id_map_path: None,
            projection,
        });
    }

    // ---- Phase 2: mutate. Track writes for best-effort rollback. ----
    let mut guard = WriteGuard::new(registry);

    let registered_workspace = if let Some(params) = &target.register {
        registry.bind_workspace(params.clone())?;
        true
    } else {
        false
    };

    // Reserve headroom so renumber allocations never collide with kept ids.
    let kept_max = kept
        .iter()
        .filter_map(|staged| parse_orb_task_number(&staged.source_id))
        .max();
    let existing_max = registry.max_registered_task_number()?;
    let floor = [kept_max, existing_max]
        .into_iter()
        .flatten()
        .max()
        .map(|value| value + 1)
        .unwrap_or(0);
    registry.bump_allocator_to_at_least(floor)?;

    // Allocate new ids for collisions (deterministic order by source id).
    to_renumber.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    let mut id_remap: BTreeMap<String, String> = BTreeMap::new();
    for staged in &to_renumber {
        let new_id = registry.allocate_task_id(&target.workspace_id)?;
        id_remap.insert(staged.source_id.clone(), new_id);
    }

    // Write kept + renumbered bundles, rewriting relation targets in the set.
    let write_result = (|| -> Result<Vec<u32>, OrbitError> {
        let mut landed_numbers = Vec::new();
        for staged in kept.iter().chain(to_renumber.iter()) {
            let final_id = id_remap
                .get(&staged.source_id)
                .cloned()
                .unwrap_or_else(|| staged.source_id.clone());
            let action = if id_remap.contains_key(&staged.source_id) {
                ImportAction::Renumbered
            } else {
                ImportAction::Kept
            };
            let bundle = remap_bundle(&staged.bundle, &final_id, &id_remap);
            let dir = registry.canonical_task_bundle_path(&target.workspace_id, &final_id)?;
            write_bundle_at(&dir, &bundle)?;
            guard.written_dirs.push(dir.clone());
            registry.register_task_bundle(&final_id, &target.workspace_id, &dir)?;
            guard.registered_ids.push(final_id.clone());
            if let Some(number) = parse_orb_task_number(&final_id) {
                landed_numbers.push(number);
            }
            records.push(ImportedTask {
                source_id: staged.source_id.clone(),
                final_id,
                action,
            });
        }
        Ok(landed_numbers)
    })();

    let landed_numbers = match write_result {
        Ok(numbers) => numbers,
        Err(err) => {
            guard.rollback();
            return Err(err);
        }
    };

    // Rebuild the whole workspace index from disk (pre-existing + imported).
    if let Err(err) = rebuild_index_from_disk(registry, &target.workspace_id) {
        guard.rollback();
        return Err(err);
    }

    // Bump the allocator past the highest landed id so future creates don't collide.
    if let Some(max) = landed_numbers.iter().copied().max() {
        registry.bump_allocator_to_at_least(max + 1)?;
    }

    let projection = rebuild_projection_best_effort(registry, &target);

    // Persist and surface the old→new mapping.
    let id_map_path = if id_remap.is_empty() {
        None
    } else {
        Some(write_id_map(archive_path, &id_remap)?)
    };

    guard.disarm();
    Ok(ImportOutcome {
        workspace_id: target.workspace_id,
        registered_workspace,
        tasks: records,
        id_remap,
        id_map_path,
        projection,
    })
}

fn read_manifest(staging: &Path) -> Result<TaskMigrationManifest, OrbitError> {
    let path = staging.join(archive::MANIFEST_ENTRY);
    let raw = std::fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            OrbitError::Store("archive is missing manifest.json".to_string())
        } else {
            OrbitError::Io(e.to_string())
        }
    })?;
    serde_json::from_slice(&raw)
        .map_err(|e| OrbitError::Store(format!("invalid migration manifest: {e}")))
}

fn validate_manifest(manifest: &TaskMigrationManifest) -> Result<(), OrbitError> {
    if manifest.format_version != MIGRATION_FORMAT_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "archive format version {} is not supported (this build expects {})",
            manifest.format_version, MIGRATION_FORMAT_VERSION
        )));
    }
    if manifest.task_schema_version != TASK_ARTIFACT_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "archive task schema version {} is not supported (this build expects {})",
            manifest.task_schema_version, TASK_ARTIFACT_SCHEMA_VERSION
        )));
    }
    Ok(())
}

/// Read + validate every bundle referenced by the manifest into memory. Any
/// integrity failure aborts before state is touched.
fn stage_bundles(
    staging: &Path,
    manifest: &TaskMigrationManifest,
) -> Result<Vec<StagedBundle>, OrbitError> {
    let mut staged = Vec::with_capacity(manifest.task_ids.len());
    let mut seen = BTreeSet::new();
    for id in &manifest.task_ids {
        validate_orb_task_id(id)?;
        if !seen.insert(id.clone()) {
            return Err(OrbitError::InvalidInput(format!(
                "archive manifest lists task '{id}' more than once"
            )));
        }
        let dir = staging.join(archive::BUNDLES_DIR).join(id);
        if !dir.is_dir() {
            return Err(OrbitError::Store(format!(
                "archive manifest references '{id}' but its bundle is missing"
            )));
        }
        // read_bundle_at validates the envelope, jsonl rows, and artifact hashes.
        let bundle = read_bundle_at(&dir)?;
        if bundle.envelope.id != *id {
            return Err(OrbitError::Store(format!(
                "archive bundle '{id}' contains mismatched envelope id '{}'",
                bundle.envelope.id
            )));
        }
        staged.push(StagedBundle {
            source_id: id.clone(),
            bundle,
        });
    }
    Ok(staged)
}

/// Resolve where imported tasks should land, and whether a new workspace binding
/// must be registered.
fn resolve_target(
    registry: &TaskRegistryStore,
    manifest: &TaskMigrationManifest,
    target_workspace_id: Option<&str>,
) -> Result<ImportTarget, OrbitError> {
    if let Some(requested) = target_workspace_id {
        let binding = registry.find_workspace_binding(requested)?.ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "target workspace '{requested}' is not registered locally; initialize it first or omit --workspace to register the source workspace"
            ))
        })?;
        return Ok(ImportTarget {
            workspace_id: binding.workspace_id,
            orbit_dir: binding.orbit_dir,
            register: None,
        });
    }

    if let Some(binding) = registry.find_workspace_binding(&manifest.source_workspace_id)? {
        return Ok(ImportTarget {
            workspace_id: binding.workspace_id,
            orbit_dir: binding.orbit_dir,
            register: None,
        });
    }

    // Source workspace is unknown locally and no target was named: register a
    // detached binding pinned to the source id. Its synthetic orbit_dir lives
    // beside the workspaces tree so the projection has somewhere to land until
    // the real repo is initialized with this workspace id.
    let detached_root = registry
        .workspaces_dir()
        .parent()
        .map(|parent| parent.join("detached"))
        .unwrap_or_else(|| registry.workspaces_dir().join("detached"));
    let orbit_dir = detached_root.join(&manifest.source_workspace_id);
    std::fs::create_dir_all(&orbit_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    let params = BindWorkspaceParams {
        workspace_id: Some(manifest.source_workspace_id.clone()),
        slug: manifest.source_workspace_slug.clone(),
        repo_root: orbit_dir.clone(),
        workspace_path: orbit_dir.clone(),
        orbit_dir: orbit_dir.clone(),
        repo_fingerprint: None,
    };
    Ok(ImportTarget {
        workspace_id: manifest.source_workspace_id.clone(),
        orbit_dir,
        register: Some(params),
    })
}

/// Clone `bundle`, setting the envelope id to `final_id` and rewriting every
/// relation target that is being renumbered within the imported set.
fn remap_bundle(
    bundle: &TaskBundleV2,
    final_id: &str,
    id_remap: &BTreeMap<String, String>,
) -> TaskBundleV2 {
    let mut out = bundle.clone();
    out.envelope.id = final_id.to_string();
    for relation in &mut out.envelope.relations {
        // Rewrite whenever the target is being renumbered, regardless of relation
        // type. A `ChildOf` target is the task's parent, so this covers parent
        // rewrites; `Produces`/`Resolves` may point at F-/L-/ADR- ids that are
        // never in the renumber set, so they are left untouched.
        if let Some(mapped) = id_remap.get(&relation.target) {
            relation.target = mapped.clone();
        }
    }
    out
}

/// Rebuild the target workspace's index rows from the bundles currently on disk.
fn rebuild_index_from_disk(
    registry: &TaskRegistryStore,
    workspace_id: &str,
) -> Result<(), OrbitError> {
    let bindings = registry.tasks_for_workspace(workspace_id)?;
    let mut envelopes = Vec::with_capacity(bindings.len());
    for binding in bindings {
        envelopes.push(read_bundle_at(&binding.canonical_path)?.envelope);
    }
    registry.replace_workspace_task_indexes(workspace_id, &envelopes)
}

fn rebuild_projection_best_effort(
    registry: &TaskRegistryStore,
    target: &ImportTarget,
) -> ProjectionRebuildResult {
    match registry.rebuild_projection(&target.orbit_dir, &target.workspace_id) {
        Ok(result) => result,
        Err(err) => ProjectionRebuildResult {
            projected: 0,
            repaired: 0,
            degraded_reason: Some(format!("projection rebuild failed after import: {err}")),
        },
    }
}

fn write_id_map(
    archive_path: &Path,
    id_remap: &BTreeMap<String, String>,
) -> Result<PathBuf, OrbitError> {
    let mut os = archive_path.as_os_str().to_owned();
    os.push(".idmap.json");
    let path = PathBuf::from(os);
    let json = serde_json::to_vec_pretty(id_remap)
        .map_err(|e| OrbitError::Store(format!("failed to encode id map: {e}")))?;
    std::fs::write(&path, json).map_err(|e| OrbitError::Io(e.to_string()))?;
    Ok(path)
}

/// Tracks filesystem/registry writes so a mid-import failure can be rolled back.
struct WriteGuard<'a> {
    registry: &'a TaskRegistryStore,
    written_dirs: Vec<PathBuf>,
    registered_ids: Vec<String>,
    armed: bool,
}

impl<'a> WriteGuard<'a> {
    fn new(registry: &'a TaskRegistryStore) -> Self {
        Self {
            registry,
            written_dirs: Vec::new(),
            registered_ids: Vec::new(),
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    /// Best-effort undo of everything written so far. Registry rows are removed
    /// first (they reference the dirs), then the bundle directories. Failures
    /// are logged, not propagated — the caller is already returning the original
    /// error.
    fn rollback(&mut self) {
        for id in self.registered_ids.drain(..) {
            // workspace_id is not needed to look up the (global) binding, but the
            // API takes it; recover it from the binding.
            if let Ok(Some(binding)) = self.registry.find_task_binding(&id) {
                let _ = self
                    .registry
                    .unregister_task_bundle(&id, &binding.workspace_id);
            }
        }
        for dir in self.written_dirs.drain(..) {
            let _ = std::fs::remove_dir_all(&dir);
        }
        self.armed = false;
    }
}

impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.rollback();
        }
    }
}
