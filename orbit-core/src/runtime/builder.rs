use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use orbit_policy::PolicyEngine;
use orbit_store::{
    Store, activity_store_file, agent_session_store_sqlite, audit_event_store_sqlite,
    job_store_file, lock_store_memory, task_store_file, tool_store_sqlite,
};

use orbit_tools::ToolRegistry;
use orbit_tools::external::ExternalTool;
use orbit_types::OrbitError;

use crate::OrbitContext;
use crate::config::RuntimeConfig;
use crate::identity_catalog::IdentityCatalog;
use crate::paths;
use crate::skill_catalog::SkillCatalog;

pub(crate) fn build_context_from_data_root(
    data_root: &Path,
    orbit_home: &Path,
) -> Result<OrbitContext, OrbitError> {
    let runtime_config = RuntimeConfig::load_from_data_root(data_root, orbit_home)?;
    let db_path = runtime_config.persistence.audit.path.clone();
    let store = Store::open(&db_path)?;

    let task_store = task_store_file(runtime_config.persistence.task.clone())?;
    let activity_store = activity_store_file(runtime_config.persistence.activity.path.clone())?;
    let job_store = job_store_file(runtime_config.persistence.job.path.clone())?;

    build_context_common(
        store,
        data_root.to_path_buf(),
        orbit_home.to_path_buf(),
        runtime_config,
        task_store,
        activity_store,
        job_store,
    )
}

pub(crate) fn build_context_in_memory() -> Result<OrbitContext, OrbitError> {
    let store = Store::open_in_memory()?;
    let temp_root = std::env::temp_dir().join(format!(
        "orbit-runtime-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let task_store = task_store_file(temp_root.join("tasks"))?;
    let activity_store = activity_store_file(temp_root.join("activities"))?;
    let job_store = job_store_file(temp_root.join("jobs"))?;
    let orbit_home = paths::orbit_home_root();
    let runtime_config = RuntimeConfig::default_for_roots(&orbit_home, &orbit_home);
    let data_root = runtime_config
        .persistence
        .task
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    build_context_common(
        store,
        data_root,
        orbit_home,
        runtime_config,
        task_store,
        activity_store,
        job_store,
    )
}

fn build_context_common(
    store: Store,
    data_root: PathBuf,
    orbit_home: PathBuf,
    runtime_config: RuntimeConfig,
    task_store: Arc<dyn orbit_store::TaskStoreBackend>,
    activity_store: Arc<dyn orbit_store::ActivityStoreBackend>,
    job_store: Arc<dyn orbit_store::JobStoreBackend>,
) -> Result<OrbitContext, OrbitError> {
    let tool_store = tool_store_sqlite(store.clone());
    let audit_event_store = audit_event_store_sqlite(store.clone());
    let agent_session_store = agent_session_store_sqlite(store.clone());
    let lock_store = lock_store_memory();

    let skill_root = runtime_config.persistence.skill.clone();
    let skill_catalog = SkillCatalog::new(skill_root);
    skill_catalog.ensure_layout()?;

    let identity_catalog = IdentityCatalog::new(
        runtime_config.identity.root.clone(),
        runtime_config.identity.role_overrides.clone(),
    );

    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    load_external_tools(&store, &mut registry)?;

    let execution_env_policy = runtime_config.execution_env.clone();
    let codex_execution_policy = runtime_config.codex_execution.clone();
    let persistence = runtime_config.persistence.clone();
    let task_approval_required_for_agent = runtime_config.task_approval.required_for_agent;
    let task_delegate_approval = runtime_config.task_approval.delegate_approval;

    Ok(OrbitContext {
        data_root,
        orbit_home,
        task_store,
        activity_store,
        job_store,
        tool_store,
        audit_event_store,
        agent_session_store,
        lock_store,
        policy: PolicyEngine::new_local_default_allow(),
        registry: Arc::new(registry),
        skill_catalog,
        identity_catalog,
        execution_env_policy,
        codex_execution_policy,
        persistence,
        task_approval_required_for_agent,
        task_delegate_approval,
    })
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
