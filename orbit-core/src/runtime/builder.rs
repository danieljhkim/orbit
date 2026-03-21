use std::path::{Path, PathBuf};
use std::sync::Arc;

use orbit_policy::PolicyEngine;
use orbit_store::{
    Store, activity_store_layered, activity_store_memory, audit_event_store_sqlite,
    job_store_layered, job_store_memory, task_store_file, task_store_memory, tool_store_sqlite,
};

use orbit_tools::ToolRegistry;
use orbit_tools::external::ExternalTool;
use orbit_types::OrbitError;

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
    let db_path = runtime_config.persistence.audit.path.clone();
    let store = Store::open(&db_path)?;

    let task_store = task_store_file(runtime_config.persistence.task.clone())?;

    let ws_activities_dir = workspace_root.join("activities");
    let ws_activities = if ws_activities_dir.is_dir() {
        Some(ws_activities_dir)
    } else {
        None
    };
    let activity_store = activity_store_layered(
        runtime_config.persistence.activity.path.clone(),
        ws_activities,
    )?;

    let ws_jobs_dir = workspace_root.join("jobs");
    let ws_jobs = if ws_jobs_dir.is_dir() {
        Some(ws_jobs_dir)
    } else {
        None
    };
    let job_store = job_store_layered(
        runtime_config.persistence.job.path.clone(),
        ws_jobs,
    )?;

    build_context_common(
        store,
        global_root.to_path_buf(),
        workspace_root.to_path_buf(),
        runtime_config,
        task_store,
        activity_store,
        job_store,
    )
}

pub(crate) fn build_context_in_memory() -> Result<OrbitContext, OrbitError> {
    let store = Store::open_in_memory()?;
    let task_store = task_store_memory();
    let activity_store = activity_store_memory();
    let job_store = job_store_memory();
    let runtime_config = RuntimeConfig::default_for_data_root(Path::new(".orbit"));
    let data_root = Path::new(".").to_path_buf();

    build_context_common(
        store,
        data_root.clone(),
        data_root,
        runtime_config,
        task_store,
        activity_store,
        job_store,
    )
}

fn build_context_common(
    store: Store,
    global_root: PathBuf,
    workspace_root: PathBuf,
    runtime_config: RuntimeConfig,
    task_store: Arc<dyn orbit_store::TaskStoreBackend>,
    activity_store: Arc<dyn orbit_store::ActivityStoreBackend>,
    job_store: Arc<dyn orbit_store::JobStoreBackend>,
) -> Result<OrbitContext, OrbitError> {
    let tool_store = tool_store_sqlite(store.clone());
    let audit_event_store = audit_event_store_sqlite(store.clone());

    let skill_root = runtime_config.persistence.skill.clone();
    let skill_catalog = SkillCatalog::new(skill_root);
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

    Ok(OrbitContext::new(
        global_root,
        workspace_root,
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
    ))
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
