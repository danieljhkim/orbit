use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use orbit_lock::{FileLockStore, apply_lock_schema};
use orbit_policy::PolicyEngine;
use orbit_store::{
    LayeredActivityStore, LayeredJobStore, Store, activity_store_file, audit_event_store_sqlite,
    job_store_file, task_store_file, tool_store_sqlite,
};

use orbit_tools::ToolRegistry;
use orbit_tools::external::ExternalTool;
use orbit_types::{OrbitError, WorkspacePaths};
use rusqlite::Connection;

use crate::OrbitContext;
use crate::config::RuntimeConfig;
use crate::context::ActorIdentity;
use crate::skill_catalog::SkillCatalog;

/// Legacy single-root builder. Treats data_root as both global and workspace root.
pub(crate) fn build_context_from_data_root(data_root: &Path) -> Result<OrbitContext, OrbitError> {
    build_context_from_roots(data_root, data_root)
}

/// Two-root builder. Global root provides activities, jobs, skills, config, SQLite.
/// Workspace root provides tasks. Workspace can optionally override activities, jobs,
/// skills, and config.
pub(crate) fn build_context_from_roots(
    global_root: &Path,
    workspace_root: &Path,
) -> Result<OrbitContext, OrbitError> {
    let runtime_config = RuntimeConfig::load_layered(global_root, workspace_root)?;
    let persistence = &runtime_config.persistence;

    let store = Store::open(&persistence.audit_db)?;

    // Build task store (workspace only).
    let task_store = task_store_file(persistence.task_dir.clone());

    // Build activity store — layered if workspace differs from global.
    let activity_store = if persistence.activity_dir != persistence.global_activity_dir {
        let ws = activity_store_file(persistence.activity_dir.clone());
        let gl = activity_store_file(persistence.global_activity_dir.clone());
        Arc::new(LayeredActivityStore::new(ws, gl)) as Arc<dyn orbit_store::ActivityStoreBackend>
    } else {
        activity_store_file(persistence.activity_dir.clone())
    };

    // Build job store — layered if workspace differs from global.
    let job_store = if persistence.job_dir != persistence.global_job_dir {
        let ws = job_store_file(persistence.job_dir.clone());
        let gl = job_store_file(persistence.global_job_dir.clone());
        Arc::new(LayeredJobStore::new(ws, gl)) as Arc<dyn orbit_store::JobStoreBackend>
    } else {
        job_store_file(persistence.job_dir.clone())
    };

    // workspace_root IS the .orbit dir; repo_root is its parent.
    let repo_root = workspace_root
        .parent()
        .unwrap_or(workspace_root)
        .to_path_buf();
    let paths = WorkspacePaths::new(
        repo_root,
        workspace_root.to_path_buf(),
        global_root.to_path_buf(),
    );
    let file_lock_store = Arc::new(open_file_lock_store(&paths.orbit_dir)?);

    let tool_store = tool_store_sqlite(store.clone());
    let audit_event_store = audit_event_store_sqlite(store.clone());

    let skill_catalog = if persistence.skill_dir != persistence.global_skill_dir {
        SkillCatalog::layered(
            persistence.skill_dir.clone(),
            persistence.global_skill_dir.clone(),
        )
    } else {
        SkillCatalog::new(persistence.skill_dir.clone())
    };
    skill_catalog.ensure_layout()?;

    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    load_external_tools(&store, &mut registry)?;

    let execution_env_policy = runtime_config.execution_env.clone();
    let codex_execution_policy = runtime_config.codex_execution.clone();
    let persistence = runtime_config.persistence.clone();
    let user_name = runtime_config.user_name.clone();
    let actor = ActorIdentity::from_env();
    let task_approval_required_for_agent = runtime_config.task_approval.required_for_agent;
    let task_delegate_approval = runtime_config.task_approval.delegate_approval;
    let scoring_enabled = runtime_config.scoring_enabled;

    Ok(OrbitContext::new(
        paths,
        file_lock_store,
        task_store,
        activity_store,
        job_store,
        tool_store,
        audit_event_store,
        PolicyEngine::new_local_default_allow(),
        Arc::new(registry),
        skill_catalog,
        execution_env_policy,
        codex_execution_policy,
        persistence,
        user_name,
        actor,
        task_approval_required_for_agent,
        task_delegate_approval,
        scoring_enabled,
    ))
}

pub(super) struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

static IN_MEMORY_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn build_context_in_memory() -> Result<(OrbitContext, TempDir), OrbitError> {
    let n = IN_MEMORY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let data_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new("."))
        .join("tmp")
        .join(format!("in-memory-{n}"));
    std::fs::create_dir_all(&data_root).map_err(|e| OrbitError::Io(e.to_string()))?;
    let guard = TempDir(data_root.clone());

    let context = build_context_from_roots(&data_root, &data_root)?;
    Ok((context, guard))
}

fn load_external_tools(store: &Store, registry: &mut ToolRegistry) -> Result<(), OrbitError> {
    let stored_tools = store.list_tools()?;
    for tool in stored_tools {
        if !tool.builtin && tool.enabled && !registry.has(&tool.name) {
            registry.register(ExternalTool {
                name: tool.name,
                path: tool.path,
                description: tool.description,
            });
        }
    }
    Ok(())
}

fn open_file_lock_store(orbit_dir: &Path) -> Result<FileLockStore, OrbitError> {
    std::fs::create_dir_all(orbit_dir).map_err(|error| OrbitError::Store(error.to_string()))?;
    let db_path = orbit_dir.join("file_locks.db");
    let conn = Connection::open(&db_path).map_err(|error| {
        OrbitError::Store(format!(
            "failed to open file lock database '{}': {error}",
            db_path.display()
        ))
    })?;
    apply_lock_schema(&conn)?;
    Ok(FileLockStore::new(Arc::new(Mutex::new(conn))))
}
