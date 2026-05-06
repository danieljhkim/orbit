use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use orbit_policy::PolicyEngine;
use orbit_store::{
    Store, audit_event_store_sqlite, global_executor_def_store, global_policy_def_store,
    layered_policy_def_store, task_reservation_store_sqlite, tool_store_sqlite,
    workspace_job_run_store, workspace_policy_def_store, workspace_task_backends,
};

use orbit_common::types::{DEFAULT_POLICY_NAME, OrbitError, WorkspacePaths};
use orbit_tools::ToolRegistry;
use orbit_tools::external::ExternalTool;

use crate::OrbitContext;
use crate::command::init::global_skills_dir;
use crate::command::policy::seed_default_policies;
use crate::config::RuntimeConfig;
use crate::context::{
    ActorIdentity, OrbitExecutionAssets, OrbitPolicyContext, OrbitRuntimeSettings, OrbitStores,
};
use crate::skill_catalog::SkillCatalog;
use crate::workspace_registry;

/// Legacy single-root builder. Treats data_root as both global and workspace root.
pub(crate) fn build_context_from_data_root(data_root: &Path) -> Result<OrbitContext, OrbitError> {
    build_context_from_roots(data_root, data_root)
}

/// Two-root builder. Global root provides activities, jobs, executors, policies,
/// config, global skills, and SQLite. Workspace root provides tasks,
/// optional skill overrides, and runtime state.
pub(crate) fn build_context_from_roots(
    global_root: &Path,
    workspace_root: &Path,
) -> Result<OrbitContext, OrbitError> {
    let runtime_config = RuntimeConfig::load_layered(global_root, workspace_root)?;
    let persistence = &runtime_config.persistence;

    let store = Store::open(&persistence.audit_db)?;

    // workspace_root IS the .orbit dir. For custom roots outside the repo,
    // prefer the registry's workspace root over the parent-directory fallback.
    let repo_root = registered_repo_root(global_root, workspace_root).unwrap_or_else(|| {
        workspace_root
            .parent()
            .unwrap_or(workspace_root)
            .to_path_buf()
    });
    let paths = WorkspacePaths::new(
        repo_root,
        workspace_root.to_path_buf(),
        global_root.to_path_buf(),
    );

    let task_backends = workspace_task_backends(persistence.task_dir.clone());
    let job_run_store = workspace_job_run_store(paths.jobs_dir.clone());

    // Executors and policies are global-only. Jobs always persist run state
    // under the workspace state directory.
    let tool_store = tool_store_sqlite(store.clone());
    let audit_event_store = audit_event_store_sqlite(store.clone());
    let task_reservation_store = task_reservation_store_sqlite(store.clone());
    let executor_def_store = global_executor_def_store(persistence.executor_dir.clone());
    let global_policy_store = global_policy_def_store(persistence.policy_dir.clone());
    seed_default_policies(global_policy_store.as_ref(), false)?;
    let workspace_policy_store = workspace_policy_def_store(paths.policies_dir.clone());
    let policy_def_store = layered_policy_def_store(workspace_policy_store, global_policy_store);
    let active_policy = policy_def_store
        .get_policy_def(DEFAULT_POLICY_NAME)?
        .ok_or_else(|| {
            OrbitError::Execution(format!(
                "default policy `{DEFAULT_POLICY_NAME}` was not found after seeding"
            ))
        })?;

    let skill_catalog = SkillCatalog::layered(
        persistence.skill_dir.clone(),
        global_skills_dir(global_root),
    );
    skill_catalog.ensure_layout()?;

    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    load_external_tools(&store, &mut registry)?;

    let execution_env_policy = runtime_config.execution_env.clone();
    let codex_execution_policy = runtime_config.codex_execution.clone();
    let persistence = runtime_config.persistence.clone();
    let actor = ActorIdentity::from_env();
    let task_approval_required_for_agent = runtime_config.task_approval.required_for_agent;
    let task_delegate_approval = runtime_config.task_approval.delegate_approval;
    let scoring_enabled = runtime_config.scoring_enabled;
    let graph_editing = runtime_config.graph_editing;
    let v2_backend = runtime_config.v2_backend().map(ToString::to_string);
    let agent_roles = runtime_config.agent_roles.clone();

    Ok(OrbitContext::new(
        paths,
        OrbitStores::new(
            task_backends.task,
            task_backends.document,
            task_backends.history,
            task_backends.review,
            task_backends.artifact,
            task_reservation_store,
            job_run_store,
            tool_store,
            audit_event_store,
            executor_def_store,
            policy_def_store,
        ),
        OrbitExecutionAssets::new(Arc::new(registry), skill_catalog),
        OrbitPolicyContext::new(
            PolicyEngine::from_def(&active_policy)?,
            execution_env_policy,
            codex_execution_policy,
        ),
        OrbitRuntimeSettings::new(
            persistence,
            actor,
            task_approval_required_for_agent,
            task_delegate_approval,
            scoring_enabled,
            graph_editing,
            v2_backend,
            agent_roles,
        ),
    ))
}

fn registered_repo_root(global_root: &Path, workspace_root: &Path) -> Option<PathBuf> {
    let registry_path = workspace_registry::registry_path_for(global_root);
    let registry = workspace_registry::load_registry_from(&registry_path).ok()?;
    let workspace_root_canonical =
        std::fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    registry.workspaces.iter().find_map(|workspace| {
        let orbit_dir_canonical = std::fs::canonicalize(&workspace.orbit_dir)
            .unwrap_or_else(|_| workspace.orbit_dir.clone());
        (orbit_dir_canonical == workspace_root_canonical).then(|| workspace.root.clone())
    })
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
                parameters: tool.parameters,
            });
        }
    }
    Ok(())
}
