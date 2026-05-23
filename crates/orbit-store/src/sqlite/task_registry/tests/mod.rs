// Content moved from tests.rs per ORB-00231
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use orbit_common::types::{
    ORB_TASK_ID_MAX, OrbitError, TaskEnvelopeV2, TaskPriority, TaskRelation, TaskRelationType,
    TaskStatus, TaskType,
};
use orbit_common::utility::fs::{atomic_write_text, create_dir_symlink};
use rusqlite::{Connection, OptionalExtension, params};
use tempfile::TempDir;

use super::REGISTRY_SCHEMA_VERSION;
use super::schema::registry_user_version;
use super::util::{normalize_path, now_string};
use super::{
    BindWorkspaceParams, ProjectionRebuildResult, TaskIndexFilter, TaskRegistryStore,
    WorkspaceBinding, WorkspaceConfig, read_workspace_config, read_workspace_config_optional,
    task_registry_path, workspace_config_path, workspace_id_for_orbit_dir, write_workspace_config,
};

fn registry_path(temp: &TempDir) -> PathBuf {
    task_registry_path(temp.path())
}

fn store(temp: &TempDir) -> TaskRegistryStore {
    TaskRegistryStore::open(&registry_path(temp)).expect("open registry")
}

fn bind(store: &TaskRegistryStore, root: &Path) -> WorkspaceBinding {
    let orbit_dir = root.join(".orbit");
    fs::create_dir_all(&orbit_dir).expect("create orbit dir");
    store
        .bind_workspace(BindWorkspaceParams {
            workspace_id: Some("orbit-test-123456".into()),
            slug: "Orbit Test".into(),
            repo_root: root.to_path_buf(),
            workspace_path: root.to_path_buf(),
            orbit_dir,
            repo_fingerprint: None,
        })
        .expect("bind workspace")
}

fn create_canonical_bundle(
    store: &TaskRegistryStore,
    workspace: &WorkspaceBinding,
    task_id: &str,
) -> PathBuf {
    let bundle_dir = store
        .canonical_task_bundle_path(&workspace.workspace_id, task_id)
        .expect("canonical bundle path");
    fs::create_dir_all(&bundle_dir).expect("create bundle");
    bundle_dir
}

fn envelope(
    task_id: &str,
    status: TaskStatus,
    tags: Vec<String>,
    relations: Vec<TaskRelation>,
) -> TaskEnvelopeV2 {
    let now = Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
    TaskEnvelopeV2 {
        schema_version: orbit_common::types::TASK_ARTIFACT_SCHEMA_VERSION,
        id: task_id.to_string(),
        title: format!("Task {task_id}"),
        status,
        task_type: TaskType::Feature,
        priority: TaskPriority::High,
        complexity: None,
        job_run_id: None,
        crew: None,
        relations,
        tags,
        context_files: Vec::new(),
        external_refs: Vec::new(),
        created_by: Some("codex:gpt-5.5".to_string()),
        planned_by: None,
        implemented_by: None,
        created_at: now,
        updated_at: now,
    }
}

fn projection_links_supported(result: &ProjectionRebuildResult) -> bool {
    if let Some(reason) = &result.degraded_reason {
        #[cfg(unix)]
        panic!("symlink projection unexpectedly degraded on unix: {reason}");

        #[cfg(not(unix))]
        {
            assert!(!reason.is_empty());
            return false;
        }
    }
    true
}

fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("prepare table info");
    stmt.query_map([], |row| row.get::<_, String>(1))
        .expect("query table info")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect columns")
}

fn index_exists(conn: &Connection, index_name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1",
        [index_name],
        |_| Ok(()),
    )
    .optional()
    .expect("query sqlite_master")
    .is_some()
}

#[test]
fn allocator_returns_monotonic_orb_ids() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let workspace = bind(&store, temp.path());

    assert_eq!(
        store.allocate_task_id(&workspace.workspace_id).expect("id"),
        "ORB-00000"
    );
    assert_eq!(
        store.allocate_task_id(&workspace.workspace_id).expect("id"),
        "ORB-00001"
    );
}

#[test]
fn open_creates_registry_parent_and_workspaces_dir() {
    let temp = TempDir::new().expect("tempdir");
    let path = registry_path(&temp);

    let _store = TaskRegistryStore::open(&path).expect("open registry");

    assert!(path.is_file());
    assert!(temp.path().join("tasks").join("workspaces").is_dir());

    let conn = Connection::open(path).expect("open registry sqlite");
    assert_eq!(
        registry_user_version(&conn).expect("read user_version"),
        REGISTRY_SCHEMA_VERSION
    );
}

#[test]
fn open_migrates_existing_task_index_columns_before_creating_indexes() {
    let temp = TempDir::new().expect("tempdir");
    let path = registry_path(&temp);
    fs::create_dir_all(path.parent().expect("registry parent")).expect("create parent");
    let conn = Connection::open(&path).expect("open sqlite");
    conn.execute_batch(
        "
        CREATE TABLE task_bundle_index (
            task_id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            status TEXT NOT NULL,
            priority TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        PRAGMA user_version = 2;
        ",
    )
    .expect("seed old registry shape");
    drop(conn);

    let _store = TaskRegistryStore::open(&path).expect("open migrated registry");

    let conn = Connection::open(&path).expect("reopen migrated sqlite");
    let columns = table_columns(&conn, "task_bundle_index");
    assert!(columns.iter().any(|column| column == "job_run_id"));
    assert!(columns.iter().any(|column| column == "terminal_month"));
    assert!(index_exists(
        &conn,
        "idx_task_bundle_index_workspace_job_run"
    ));
    assert!(index_exists(
        &conn,
        "idx_task_bundle_index_workspace_terminal"
    ));
    assert_eq!(
        registry_user_version(&conn).expect("read user_version"),
        REGISTRY_SCHEMA_VERSION
    );
}

#[test]
fn open_rejects_newer_registry_schema_version() {
    let temp = TempDir::new().expect("tempdir");
    let path = registry_path(&temp);
    fs::create_dir_all(path.parent().expect("registry parent")).expect("create parent");
    let conn = Connection::open(&path).expect("open sqlite");
    conn.pragma_update(None, "user_version", i64::from(REGISTRY_SCHEMA_VERSION + 1))
        .expect("set user_version");
    drop(conn);

    let err = match TaskRegistryStore::open(&path) {
        Ok(_) => panic!("opened newer registry schema"),
        Err(err) => err,
    };
    assert!(matches!(err, OrbitError::Store(message) if message.contains("newer than supported")));
}

#[test]
fn allocator_is_global_across_workspaces() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let first = bind(&store, temp.path());
    let second_root = temp.path().join("second");
    fs::create_dir_all(second_root.join(".orbit")).expect("create second orbit dir");
    let second = store
        .bind_workspace(BindWorkspaceParams {
            workspace_id: Some("second-abcdef".into()),
            slug: "Second".into(),
            repo_root: second_root.clone(),
            workspace_path: second_root.clone(),
            orbit_dir: second_root.join(".orbit"),
            repo_fingerprint: None,
        })
        .expect("bind second workspace");

    assert_eq!(
        store
            .allocate_task_id(&first.workspace_id)
            .expect("first id"),
        "ORB-00000"
    );
    assert_eq!(
        store
            .allocate_task_id(&second.workspace_id)
            .expect("second id"),
        "ORB-00001"
    );
}

#[test]
fn allocator_reports_exhaustion() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let workspace = bind(&store, temp.path());

    {
        let conn = store.conn.lock().expect("lock registry");
        conn.execute(
            "UPDATE allocator_state SET next_number = ?1, updated_at = ?2
             WHERE authority = 'local'",
            params![i64::from(ORB_TASK_ID_MAX) + 1, now_string()],
        )
        .expect("force allocator exhaustion");
    }

    assert!(matches!(
        store.allocate_task_id(&workspace.workspace_id),
        Err(OrbitError::Store(message)) if message.contains("exhausted")
    ));
}

#[test]
fn bind_workspace_is_idempotent_for_orbit_dir() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let first = bind(&store, temp.path());
    let second = store
        .bind_workspace(BindWorkspaceParams {
            workspace_id: Some(first.workspace_id.clone()),
            slug: "Changed".into(),
            repo_root: temp.path().join("."),
            workspace_path: temp.path().join("."),
            orbit_dir: temp.path().join(".orbit").join("..").join(".orbit"),
            repo_fingerprint: Some("changed".into()),
        })
        .expect("idempotent bind");

    assert_eq!(first.workspace_id, second.workspace_id);
    assert_eq!(first.slug, second.slug);
}

#[test]
fn bind_workspace_rejects_explicit_workspace_id_conflict() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    bind(&store, temp.path());

    let result = store.bind_workspace(BindWorkspaceParams {
        workspace_id: Some("other-abcdef".into()),
        slug: "Changed".into(),
        repo_root: temp.path().join("."),
        workspace_path: temp.path().join("."),
        orbit_dir: temp.path().join(".orbit").join("..").join(".orbit"),
        repo_fingerprint: Some("changed".into()),
    });

    assert!(matches!(result, Err(OrbitError::InvalidInput(_))));
}

#[test]
fn workspace_config_round_trips_and_validates() {
    let temp = TempDir::new().expect("tempdir");
    let orbit_dir = temp.path().join(".orbit");
    write_workspace_config(
        &orbit_dir,
        &WorkspaceConfig {
            schema_version: 1,
            workspace_id: "orbit-test-abcdef".into(),
        },
    )
    .expect("write config");

    let read = read_workspace_config(&orbit_dir).expect("read config");
    assert_eq!(read.workspace_id, "orbit-test-abcdef");

    atomic_write_text(
        &workspace_config_path(&orbit_dir),
        "schema_version: 2\nworkspace_id: orbit-test-abcdef\n",
    )
    .expect("write wrong schema");
    assert!(matches!(
        read_workspace_config(&orbit_dir),
        Err(OrbitError::InvalidInput(_))
    ));

    atomic_write_text(
        &workspace_config_path(&orbit_dir),
        "schema_version: 1\nworkspace_id: ''\n",
    )
    .expect("write empty id");
    assert!(matches!(
        read_workspace_config(&orbit_dir),
        Err(OrbitError::InvalidInput(_))
    ));

    atomic_write_text(
        &workspace_config_path(&orbit_dir),
        "schema_version: 1\nworkspace_id: orbit-test-abcdef\nextra: nope\n",
    )
    .expect("write unknown field");
    assert!(matches!(
        read_workspace_config(&orbit_dir),
        Err(OrbitError::InvalidInput(_))
    ));
}

#[test]
fn workspace_config_optional_distinguishes_missing_file() {
    let temp = TempDir::new().expect("tempdir");

    assert_eq!(
        read_workspace_config_optional(&temp.path().join(".orbit")).expect("read optional config"),
        None
    );
}

#[test]
fn workspace_id_for_orbit_dir_returns_id_from_config() {
    let temp = TempDir::new().expect("tempdir");
    let orbit_dir = temp.path().join(".orbit");
    write_workspace_config(
        &orbit_dir,
        &WorkspaceConfig {
            schema_version: 1,
            workspace_id: "ws-test-abcdef".into(),
        },
    )
    .expect("write config");

    assert_eq!(
        workspace_id_for_orbit_dir(&orbit_dir).expect("workspace id"),
        "ws-test-abcdef"
    );
}

#[test]
fn workspace_id_for_orbit_dir_missing_file_names_path_and_key() {
    let temp = TempDir::new().expect("tempdir");
    let orbit_dir = temp.path().join(".orbit");

    let err = workspace_id_for_orbit_dir(&orbit_dir).expect_err("missing config");
    let message = err.to_string();
    assert!(message.contains("config.yaml"));
    assert!(message.contains("workspace_id"));
}

#[test]
fn workspace_id_for_orbit_dir_malformed_yaml_returns_invalid_input() {
    let temp = TempDir::new().expect("tempdir");
    let orbit_dir = temp.path().join(".orbit");
    atomic_write_text(&workspace_config_path(&orbit_dir), "not: [valid")
        .expect("write malformed config");

    assert!(matches!(
        workspace_id_for_orbit_dir(&orbit_dir),
        Err(OrbitError::InvalidInput(_))
    ));
}

#[test]
fn rebind_candidates_match_normalized_paths() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let workspace = bind(&store, temp.path());

    let candidates = store
        .find_rebind_candidates(
            &temp.path().join("."),
            &temp.path().join("nested").join(".."),
            &workspace.orbit_dir.join("..").join(".orbit"),
        )
        .expect("candidates");

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].workspace_id, workspace.workspace_id);
}

#[test]
fn register_task_bundle_rejects_non_canonical_path() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let workspace = bind(&store, temp.path());
    let wrong_path = temp.path().join("other-workspace").join("ORB-00000");
    fs::create_dir_all(&wrong_path).expect("create wrong bundle");

    assert!(matches!(
        store.register_task_bundle("ORB-00000", &workspace.workspace_id, &wrong_path),
        Err(OrbitError::InvalidInput(_))
    ));
}

#[test]
fn generated_task_index_filters_by_status_priority_and_tags() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let workspace = bind(&store, temp.path());

    for task_id in ["ORB-00000", "ORB-00001"] {
        let bundle_dir = create_canonical_bundle(&store, &workspace, task_id);
        store
            .register_task_bundle(task_id, &workspace.workspace_id, &bundle_dir)
            .expect("register bundle");
    }

    store
        .replace_task_index(
            &workspace.workspace_id,
            &envelope(
                "ORB-00000",
                TaskStatus::Backlog,
                vec!["Task-Artifacts".into(), "v2".into()],
                Vec::new(),
            ),
        )
        .expect("index first task");
    store
        .replace_task_index(
            &workspace.workspace_id,
            &envelope(
                "ORB-00001",
                TaskStatus::Review,
                vec!["v2".into(), "review".into()],
                Vec::new(),
            ),
        )
        .expect("index second task");

    assert_eq!(
        store
            .indexed_task_count_for_workspace(&workspace.workspace_id)
            .expect("index count"),
        2
    );
    assert_eq!(
        store
            .indexed_task_ids_filtered(
                &workspace.workspace_id,
                &TaskIndexFilter {
                    status: Some(TaskStatus::Review),
                    priority: Some(TaskPriority::High),
                    job_run_id: None,
                    tags: vec!["review".into()],
                },
            )
            .expect("filtered ids"),
        vec!["ORB-00001"]
    );
    assert_eq!(
        store
            .indexed_task_ids_filtered(
                &workspace.workspace_id,
                &TaskIndexFilter {
                    status: None,
                    priority: None,
                    job_run_id: None,
                    tags: vec!["task-artifacts".into(), "v2".into()],
                },
            )
            .expect("tagged ids"),
        vec!["ORB-00000"]
    );
}

#[test]
fn generated_relation_index_supports_forward_and_inverse_lookup() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let workspace = bind(&store, temp.path());
    let bundle_dir = create_canonical_bundle(&store, &workspace, "ORB-00000");
    store
        .register_task_bundle("ORB-00000", &workspace.workspace_id, &bundle_dir)
        .expect("register bundle");

    store
        .replace_task_index(
            &workspace.workspace_id,
            &envelope(
                "ORB-00000",
                TaskStatus::Backlog,
                Vec::new(),
                vec![
                    TaskRelation {
                        relation_type: TaskRelationType::BlockedBy,
                        target: "ORB-00001".to_string(),
                    },
                    TaskRelation {
                        relation_type: TaskRelationType::RelatedTo,
                        target: "ORB-00002".to_string(),
                    },
                ],
            ),
        )
        .expect("index relations");

    assert_eq!(
        store
            .indexed_relation_targets(
                &workspace.workspace_id,
                "ORB-00000",
                TaskRelationType::BlockedBy,
            )
            .expect("targets"),
        vec!["ORB-00001"]
    );
    assert_eq!(
        store
            .indexed_relation_sources(
                &workspace.workspace_id,
                "ORB-00001",
                TaskRelationType::BlockedBy,
            )
            .expect("sources"),
        vec!["ORB-00000"]
    );
}

#[test]
fn unregister_task_bundle_removes_binding_indexes_and_relation_edges() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let workspace = bind(&store, temp.path());
    for task_id in ["ORB-00000", "ORB-00001"] {
        let bundle_dir = create_canonical_bundle(&store, &workspace, task_id);
        store
            .register_task_bundle(task_id, &workspace.workspace_id, &bundle_dir)
            .expect("register bundle");
    }
    store
        .replace_task_index(
            &workspace.workspace_id,
            &envelope(
                "ORB-00000",
                TaskStatus::Backlog,
                vec!["v2".into()],
                vec![TaskRelation {
                    relation_type: TaskRelationType::BlockedBy,
                    target: "ORB-00001".to_string(),
                }],
            ),
        )
        .expect("index source relation");

    assert!(
        store
            .unregister_task_bundle("ORB-00000", &workspace.workspace_id)
            .expect("unregister")
    );
    assert_eq!(
        store
            .tasks_for_workspace(&workspace.workspace_id)
            .expect("tasks")
            .into_iter()
            .map(|binding| binding.task_id)
            .collect::<Vec<_>>(),
        vec!["ORB-00001"]
    );
    assert_eq!(
        store
            .indexed_task_count_for_workspace(&workspace.workspace_id)
            .expect("index count"),
        0
    );
    assert_eq!(
        store
            .indexed_relation_sources(
                &workspace.workspace_id,
                "ORB-00001",
                TaskRelationType::BlockedBy,
            )
            .expect("inverse relation"),
        Vec::<String>::new()
    );
}

#[test]
fn projection_rebuild_creates_and_repairs_symlinks() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let workspace = bind(&store, temp.path());
    let bundle_dir = create_canonical_bundle(&store, &workspace, "ORB-00000");
    let wrong_bundle_dir = temp.path().join("wrong-task-target");
    fs::create_dir_all(&wrong_bundle_dir).expect("create wrong bundle");

    store
        .register_task_bundle("ORB-00000", &workspace.workspace_id, &bundle_dir)
        .expect("register bundle");
    let first = store
        .rebuild_projection(&workspace.orbit_dir, &workspace.workspace_id)
        .expect("rebuild");
    if !projection_links_supported(&first) {
        return;
    }
    assert_eq!(first.projected, 1);

    let link_path = workspace.orbit_dir.join("tasks").join("ORB-00000");
    fs::remove_file(&link_path).expect("remove correct link");
    create_dir_symlink(&wrong_bundle_dir, &link_path).expect("create wrong link");

    let second = store
        .rebuild_projection(&workspace.orbit_dir, &workspace.workspace_id)
        .expect("rebuild repair");
    assert_eq!(second.repaired, 1);
    assert_eq!(
        normalize_path(&fs::read_link(&link_path).expect("read link")),
        normalize_path(&bundle_dir)
    );
}

#[test]
fn projection_rebuild_recovers_after_reopen_and_projection_delete() {
    let temp = TempDir::new().expect("tempdir");
    let path = registry_path(&temp);
    let store = TaskRegistryStore::open(&path).expect("open registry");
    let workspace = bind(&store, temp.path());
    let bundle_dir = create_canonical_bundle(&store, &workspace, "ORB-00000");
    store
        .register_task_bundle("ORB-00000", &workspace.workspace_id, &bundle_dir)
        .expect("register bundle");

    let first = store
        .rebuild_projection(&workspace.orbit_dir, &workspace.workspace_id)
        .expect("initial rebuild");
    if !projection_links_supported(&first) {
        return;
    }
    fs::remove_dir_all(workspace.orbit_dir.join("tasks")).expect("delete projection");
    drop(store);

    let reopened = TaskRegistryStore::open(&path).expect("reopen registry");
    let rebuilt = reopened
        .rebuild_projection(&workspace.orbit_dir, &workspace.workspace_id)
        .expect("rebuild after reopen");
    if !projection_links_supported(&rebuilt) {
        return;
    }
    assert_eq!(rebuilt.projected, 1);
    assert_eq!(
        normalize_path(
            &fs::read_link(workspace.orbit_dir.join("tasks").join("ORB-00000")).expect("read link")
        ),
        normalize_path(&bundle_dir)
    );
}

#[test]
fn projection_rebuild_errors_on_non_symlink_blocker() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let workspace = bind(&store, temp.path());
    let bundle_dir = create_canonical_bundle(&store, &workspace, "ORB-00000");
    store
        .register_task_bundle("ORB-00000", &workspace.workspace_id, &bundle_dir)
        .expect("register bundle");
    let projection_dir = workspace.orbit_dir.join("tasks");
    fs::create_dir_all(&projection_dir).expect("create projection");
    fs::write(projection_dir.join("ORB-00000"), "blocker").expect("write blocker");

    assert!(matches!(
        store.rebuild_projection(&workspace.orbit_dir, &workspace.workspace_id),
        Err(OrbitError::Store(_))
    ));
}
