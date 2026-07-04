use std::fs;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use orbit_common::types::{
    TASK_ARTIFACT_SCHEMA_VERSION, TaskEnvelopeV2, TaskEventRowV2, TaskPriority, TaskRelation,
    TaskRelationType, TaskStatus, TaskType,
};
use tempfile::TempDir;

use crate::file::task_store::v2_bundle::{TaskBundleStoreV2, TaskBundleV2, read_bundle_at};
use crate::sqlite::task_registry::{
    BindWorkspaceParams, TaskRegistryStore, WorkspaceBinding, task_registry_path,
};

use super::*;

fn open_registry(global: &Path) -> TaskRegistryStore {
    TaskRegistryStore::open(&task_registry_path(global)).expect("open registry")
}

fn bind(registry: &TaskRegistryStore, global: &Path, ws_id: &str) -> WorkspaceBinding {
    let orbit_dir = global.join("repos").join(ws_id).join(".orbit");
    fs::create_dir_all(&orbit_dir).expect("create orbit dir");
    registry
        .bind_workspace(BindWorkspaceParams {
            workspace_id: Some(ws_id.to_string()),
            slug: "sample".to_string(),
            repo_root: orbit_dir.parent().unwrap().to_path_buf(),
            workspace_path: orbit_dir.parent().unwrap().to_path_buf(),
            orbit_dir,
            repo_fingerprint: None,
        })
        .expect("bind workspace")
}

fn bundle_store(registry: &TaskRegistryStore, binding: &WorkspaceBinding) -> TaskBundleStoreV2 {
    TaskBundleStoreV2::new(
        registry.clone(),
        binding.workspace_id.clone(),
        binding.orbit_dir.clone(),
    )
}

fn make_bundle(id: &str, title: &str, relations: Vec<TaskRelation>) -> TaskBundleV2 {
    let now = Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap();
    TaskBundleV2 {
        envelope: TaskEnvelopeV2 {
            schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
            id: id.to_string(),
            title: title.to_string(),
            status: TaskStatus::Backlog,
            task_type: TaskType::Feature,
            priority: TaskPriority::High,
            complexity: None,
            job_run_id: None,
            crew: None,
            relations,
            tags: vec!["migration".to_string()],
            context_files: Vec::new(),
            external_refs: Vec::new(),
            created_by: Some("codex".to_string()),
            planned_by: None,
            implemented_by: None,
            created_at: now,
            updated_at: now,
        },
        description: format!("description for {id}"),
        acceptance: "- [ ] done".to_string(),
        plan: "plan".to_string(),
        execution_summary: String::new(),
        events: vec![TaskEventRowV2 {
            schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
            event_id: "EV-0001".to_string(),
            at: now,
            by: "codex".to_string(),
            event_type: "created".to_string(),
            note: None,
            from_status: None,
            to_status: Some(TaskStatus::Backlog),
        }],
        comments: Vec::new(),
        review_threads: Vec::new(),
        artifact_manifest: None,
    }
}

fn child_of(target: &str) -> TaskRelation {
    TaskRelation {
        relation_type: TaskRelationType::ChildOf,
        target: target.to_string(),
    }
}

/// Write a bundle to disk and register+index it (no allocator advance — ids are
/// chosen explicitly by the test).
fn seed(
    store: &TaskBundleStoreV2,
    registry: &TaskRegistryStore,
    ws: &str,
    bundle: &TaskBundleV2,
) {
    store.create_bundle(bundle).expect("create bundle");
    registry
        .replace_task_index(ws, &bundle.envelope)
        .expect("index bundle");
}

fn exported_at() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 4, 0, 0, 0).unwrap()
}

/// Build a source registry with two related tasks and export it to `archive`.
/// Returns (source bundles for ORB-00000, ORB-00001).
fn build_source_archive(
    global: &Path,
    ws_id: &str,
    archive: &Path,
) -> (TaskBundleV2, TaskBundleV2) {
    let registry = open_registry(global);
    let binding = bind(&registry, global, ws_id);
    let store = bundle_store(&registry, &binding);
    let a = make_bundle("ORB-00000", "root task", Vec::new());
    // ORB-00001 is a child of ORB-00000 — its ChildOf target must be rewritten
    // if ORB-00000 is renumbered on import.
    let b = make_bundle("ORB-00001", "child task", vec![child_of("ORB-00000")]);
    seed(&store, &registry, ws_id, &a);
    seed(&store, &registry, ws_id, &b);
    let outcome = export_tasks(
        &registry,
        ws_id,
        ExportSelection::All,
        archive,
        exported_at(),
    )
    .expect("export");
    assert_eq!(outcome.task_ids, vec!["ORB-00000", "ORB-00001"]);
    assert!(archive.is_file());
    (a, b)
}

#[test]
fn round_trip_keeps_ids_and_content() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    let archive = src.path().join("tasks.tar.zst");
    let ws = "orbit-src-aaaaaa";
    let (a, b) = build_source_archive(src.path(), ws, &archive);

    // Target registry with the same workspace already registered but no tasks.
    let registry = open_registry(dst.path());
    let binding = bind(&registry, dst.path(), ws);

    let outcome = import_tasks(&registry, &archive, None, ImportConflictPolicy::Renumber)
        .expect("import");
    assert_eq!(outcome.workspace_id, ws);
    assert!(!outcome.registered_workspace);
    assert!(outcome.id_remap.is_empty());
    assert!(outcome.id_map_path.is_none());
    assert_eq!(outcome.tasks.len(), 2);
    assert!(
        outcome
            .tasks
            .iter()
            .all(|task| task.action == ImportAction::Kept)
    );

    // Bundles landed byte-for-byte identical.
    let landed_a = read_bundle_at(
        &registry
            .canonical_task_bundle_path(ws, "ORB-00000")
            .unwrap(),
    )
    .unwrap();
    let landed_b = read_bundle_at(
        &registry
            .canonical_task_bundle_path(ws, "ORB-00001")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(landed_a, a);
    assert_eq!(landed_b, b);

    // Index rows exist and allocator advanced past ORB-00001.
    assert_eq!(
        registry.tasks_for_workspace(ws).unwrap().len(),
        2,
        "both tasks registered"
    );
    assert!(registry.allocator_next_number().unwrap() >= 2);

    // Projection symlinks materialized.
    let projection = binding.orbit_dir.join("tasks");
    assert!(projection.join("ORB-00000").exists());
    assert!(projection.join("ORB-00001").exists());
}

#[test]
fn collision_renumber_rewrites_relations() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    let archive = src.path().join("tasks.tar.zst");
    let src_ws = "orbit-src-aaaaaa";
    build_source_archive(src.path(), src_ws, &archive);

    // Target registry already owns ORB-00000 (different content) in another ws.
    let registry = open_registry(dst.path());
    let target_ws = "orbit-dst-bbbbbb";
    let binding = bind(&registry, dst.path(), target_ws);
    let store = bundle_store(&registry, &binding);
    seed(
        &store,
        &registry,
        target_ws,
        &make_bundle("ORB-00000", "pre-existing local task", Vec::new()),
    );

    let outcome = import_tasks(
        &registry,
        &archive,
        Some(target_ws),
        ImportConflictPolicy::Renumber,
    )
    .expect("import");

    // ORB-00000 collided and was renumbered; ORB-00001 stayed (free) but its
    // ChildOf target was rewritten to the new id.
    let new_id = outcome
        .id_remap
        .get("ORB-00000")
        .expect("ORB-00000 renumbered")
        .clone();
    assert_ne!(new_id, "ORB-00000");
    assert!(outcome.id_map_path.as_ref().is_some_and(|p| p.is_file()));

    let child = read_bundle_at(
        &registry
            .canonical_task_bundle_path(target_ws, "ORB-00001")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(child.envelope.relations.len(), 1);
    assert_eq!(child.envelope.relations[0].target, new_id);

    // The renumbered task's envelope id matches its directory / new id.
    let renumbered = read_bundle_at(
        &registry
            .canonical_task_bundle_path(target_ws, &new_id)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(renumbered.envelope.id, new_id);

    // Original local ORB-00000 is untouched.
    let original = read_bundle_at(
        &registry
            .canonical_task_bundle_path(target_ws, "ORB-00000")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(original.envelope.title, "pre-existing local task");
}

#[test]
fn import_into_unregistered_workspace_registers_it() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    let archive = src.path().join("tasks.tar.zst");
    let ws = "orbit-src-aaaaaa";
    build_source_archive(src.path(), ws, &archive);

    // Fresh target registry with nothing bound.
    let registry = open_registry(dst.path());
    assert!(registry.find_workspace_binding(ws).unwrap().is_none());

    let outcome =
        import_tasks(&registry, &archive, None, ImportConflictPolicy::Fail).expect("import");
    assert_eq!(outcome.workspace_id, ws);
    assert!(outcome.registered_workspace);
    assert!(registry.find_workspace_binding(ws).unwrap().is_some());
    assert_eq!(registry.tasks_for_workspace(ws).unwrap().len(), 2);
}

#[test]
fn allocator_bumped_past_max_imported_id() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    let archive = src.path().join("tasks.tar.zst");
    let ws = "orbit-hi-cccccc";

    // Source with a single high id ORB-00042.
    let registry = open_registry(src.path());
    let binding = bind(&registry, src.path(), ws);
    let store = bundle_store(&registry, &binding);
    seed(
        &store,
        &registry,
        ws,
        &make_bundle("ORB-00042", "high id", Vec::new()),
    );
    export_tasks(
        &registry,
        ws,
        ExportSelection::All,
        &archive,
        exported_at(),
    )
    .unwrap();

    let target = open_registry(dst.path());
    import_tasks(&target, &archive, None, ImportConflictPolicy::Fail).unwrap();
    assert_eq!(target.allocator_next_number().unwrap(), 43);
}

#[test]
fn reimport_is_idempotent() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    let archive = src.path().join("tasks.tar.zst");
    let ws = "orbit-src-aaaaaa";
    build_source_archive(src.path(), ws, &archive);

    let registry = open_registry(dst.path());
    bind(&registry, dst.path(), ws);
    import_tasks(&registry, &archive, None, ImportConflictPolicy::Renumber).unwrap();

    let second = import_tasks(&registry, &archive, None, ImportConflictPolicy::Renumber).unwrap();
    assert!(second.id_remap.is_empty());
    assert!(
        second
            .tasks
            .iter()
            .all(|task| task.action == ImportAction::AlreadyPresent)
    );
    assert_eq!(registry.tasks_for_workspace(ws).unwrap().len(), 2);
}

#[test]
fn on_conflict_skip_leaves_local_and_drops_incoming() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    let archive = src.path().join("tasks.tar.zst");
    let src_ws = "orbit-src-aaaaaa";
    build_source_archive(src.path(), src_ws, &archive);

    let registry = open_registry(dst.path());
    let target_ws = "orbit-dst-bbbbbb";
    let binding = bind(&registry, dst.path(), target_ws);
    let store = bundle_store(&registry, &binding);
    seed(
        &store,
        &registry,
        target_ws,
        &make_bundle("ORB-00000", "local keep", Vec::new()),
    );

    let outcome = import_tasks(
        &registry,
        &archive,
        Some(target_ws),
        ImportConflictPolicy::Skip,
    )
    .unwrap();
    // ORB-00000 skipped, ORB-00001 kept.
    assert!(outcome.id_remap.is_empty());
    let skipped = outcome
        .tasks
        .iter()
        .find(|t| t.source_id == "ORB-00000")
        .unwrap();
    assert_eq!(skipped.action, ImportAction::SkippedConflict);
    // local task unchanged
    let local = read_bundle_at(
        &registry
            .canonical_task_bundle_path(target_ws, "ORB-00000")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(local.envelope.title, "local keep");
}

#[test]
fn on_conflict_fail_aborts_without_writing() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    let archive = src.path().join("tasks.tar.zst");
    let src_ws = "orbit-src-aaaaaa";
    build_source_archive(src.path(), src_ws, &archive);

    let registry = open_registry(dst.path());
    let target_ws = "orbit-dst-bbbbbb";
    let binding = bind(&registry, dst.path(), target_ws);
    let store = bundle_store(&registry, &binding);
    seed(
        &store,
        &registry,
        target_ws,
        &make_bundle("ORB-00000", "local keep", Vec::new()),
    );

    let err = import_tasks(
        &registry,
        &archive,
        Some(target_ws),
        ImportConflictPolicy::Fail,
    )
    .unwrap_err();
    assert!(format!("{err}").contains("already exists"));
    // Nothing new landed: only the pre-existing local task remains.
    assert_eq!(registry.tasks_for_workspace(target_ws).unwrap().len(), 1);
    assert!(
        !registry
            .canonical_task_bundle_path(target_ws, "ORB-00001")
            .unwrap()
            .exists()
    );
}

#[test]
fn export_ids_subset_and_rejects_unknown() {
    let src = TempDir::new().unwrap();
    let archive = src.path().join("subset.tar.zst");
    let ws = "orbit-src-aaaaaa";
    build_source_archive(src.path(), ws, &archive);
    let registry = open_registry(src.path());

    let subset = src.path().join("only-child.tar.zst");
    let outcome = export_tasks(
        &registry,
        ws,
        ExportSelection::Ids(vec!["ORB-00001".to_string()]),
        &subset,
        exported_at(),
    )
    .unwrap();
    assert_eq!(outcome.task_ids, vec!["ORB-00001"]);

    let err = export_tasks(
        &registry,
        ws,
        ExportSelection::Ids(vec!["ORB-09999".to_string()]),
        &src.path().join("bad.tar.zst"),
        exported_at(),
    )
    .unwrap_err();
    assert!(format!("{err}").contains("not registered"));
}

#[test]
fn corrupt_archive_fails_before_mutation() {
    let dst = TempDir::new().unwrap();
    let bogus = dst.path().join("bogus.tar.zst");
    fs::write(&bogus, b"not a real zstd archive").unwrap();
    let registry = open_registry(dst.path());
    let err =
        import_tasks(&registry, &bogus, None, ImportConflictPolicy::Renumber).unwrap_err();
    assert!(format!("{err}").contains("zstd") || format!("{err}").contains("archive"));
}

#[test]
fn reindex_reregisters_disk_bundles_and_drops_stale() {
    let temp = TempDir::new().unwrap();
    let ws = "orbit-idx-dddddd";
    let registry = open_registry(temp.path());
    let binding = bind(&registry, temp.path(), ws);
    let store = bundle_store(&registry, &binding);
    seed(&store, &registry, ws, &make_bundle("ORB-00000", "a", Vec::new()));
    seed(&store, &registry, ws, &make_bundle("ORB-00003", "b", Vec::new()));

    // Simulate drift: drop ORB-00003's index+binding (dir still on disk) and add
    // a stale binding for ORB-00009 whose dir does not exist.
    registry.unregister_task_bundle("ORB-00003", ws).unwrap();
    let stale_dir = registry.canonical_task_bundle_path(ws, "ORB-00009").unwrap();
    fs::create_dir_all(&stale_dir).unwrap();
    registry
        .register_task_bundle("ORB-00009", ws, &stale_dir)
        .unwrap();
    fs::remove_dir_all(&stale_dir).unwrap();

    let outcome = reindex_workspace(&registry, ws).expect("reindex");
    assert_eq!(outcome.indexed, 2, "two on-disk bundles reindexed");
    assert_eq!(outcome.removed_stale, 1, "stale ORB-00009 dropped");

    let registered: Vec<String> = registry
        .tasks_for_workspace(ws)
        .unwrap()
        .into_iter()
        .map(|t| t.task_id)
        .collect();
    assert_eq!(registered, vec!["ORB-00000", "ORB-00003"]);
    // Allocator moved past the highest on-disk id.
    assert!(registry.allocator_next_number().unwrap() >= 4);
}

fn read_id_map(path: &PathBuf) -> std::collections::BTreeMap<String, String> {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn renumber_writes_id_map_file() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    let archive = src.path().join("tasks.tar.zst");
    build_source_archive(src.path(), "orbit-src-aaaaaa", &archive);

    let registry = open_registry(dst.path());
    let target_ws = "orbit-dst-bbbbbb";
    let binding = bind(&registry, dst.path(), target_ws);
    let store = bundle_store(&registry, &binding);
    seed(
        &store,
        &registry,
        target_ws,
        &make_bundle("ORB-00000", "collide", Vec::new()),
    );

    let outcome = import_tasks(
        &registry,
        &archive,
        Some(target_ws),
        ImportConflictPolicy::Renumber,
    )
    .unwrap();
    let map_path = outcome.id_map_path.expect("id map written");
    let map = read_id_map(&map_path);
    assert_eq!(map.get("ORB-00000"), outcome.id_remap.get("ORB-00000"));
}
