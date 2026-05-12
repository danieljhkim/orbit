use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use orbit_embed::vector::{EmbedWorker, VectorStore};
use orbit_policy::PolicyEngine;
use orbit_store::sqlite::task_registry::{
    BindWorkspaceParams, TaskRegistryStore, WorkspaceConfig, read_workspace_config_optional,
    task_registry_path, write_workspace_config,
};
use orbit_store::{
    Store, audit_event_store_sqlite, global_executor_def_store, global_policy_def_store,
    layered_policy_def_store, task_reservation_store_sqlite, tool_store_sqlite,
    workspace_adr_backends, workspace_job_run_store, workspace_learning_backend,
    workspace_policy_def_store, workspace_task_backends,
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

    let task_backends = build_v2_task_backends(global_root, &paths)?;
    let adr_store = workspace_adr_backends(persistence.adr_dir.clone(), store.clone());
    let learning_store =
        workspace_learning_backend(persistence.learning_dir.clone(), store.clone());
    let semantic_vector_store = Arc::new(VectorStore::open(&persistence.semantic_db)?);
    let semantic_worker = Arc::new(EmbedWorker::start((*semantic_vector_store).clone()));
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
    let pr_config = runtime_config.pr_config().clone();
    let v2_backend = runtime_config.v2_backend().map(ToString::to_string);
    let workflow_base_branch = runtime_config.workflow_base_branch().to_string();
    let agent_roles = runtime_config.agent_roles.clone();

    Ok(OrbitContext::new(
        paths,
        OrbitStores::new(
            task_backends.task,
            task_backends.document,
            task_backends.history,
            task_backends.review,
            task_backends.artifact,
            adr_store,
            learning_store,
            semantic_vector_store,
            semantic_worker,
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
            pr_config,
            v2_backend,
            workflow_base_branch,
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

fn build_v2_task_backends(
    global_root: &Path,
    paths: &WorkspacePaths,
) -> Result<orbit_store::WorkspaceTaskBackends, OrbitError> {
    let registry = TaskRegistryStore::open(&task_registry_path(global_root))?;
    let config = read_workspace_config_optional(&paths.orbit_dir)?;
    let workspace_id = if let Some(config) = &config {
        Some(config.workspace_id.clone())
    } else {
        rebind_candidate_workspace_id(&registry, paths)?
    };
    let binding = registry.bind_workspace(BindWorkspaceParams {
        workspace_id,
        slug: workspace_slug(&paths.repo_root),
        repo_root: paths.repo_root.clone(),
        workspace_path: paths.repo_root.clone(),
        orbit_dir: paths.orbit_dir.clone(),
        repo_fingerprint: None,
    })?;
    if config
        .as_ref()
        .is_none_or(|config| config.workspace_id != binding.workspace_id)
    {
        write_workspace_config(
            &paths.orbit_dir,
            &WorkspaceConfig {
                schema_version: 1,
                workspace_id: binding.workspace_id.clone(),
            },
        )?;
    }

    Ok(workspace_task_backends(
        registry,
        binding.workspace_id,
        paths.orbit_dir.clone(),
        Some(binding.workspace_path.to_string_lossy().into_owned()),
        Some(binding.repo_root.to_string_lossy().into_owned()),
    ))
}

fn rebind_candidate_workspace_id(
    registry: &TaskRegistryStore,
    paths: &WorkspacePaths,
) -> Result<Option<String>, OrbitError> {
    let candidates =
        registry.find_rebind_candidates(&paths.repo_root, &paths.repo_root, &paths.orbit_dir)?;
    match candidates.as_slice() {
        [] => Ok(None),
        [candidate] => Ok(Some(candidate.workspace_id.clone())),
        _ => Err(OrbitError::WorkspaceError(format!(
            "workspace config is missing and multiple task artifact bindings match '{}'; restore .orbit/config.yaml or choose a workspace binding",
            paths.orbit_dir.display()
        ))),
    }
}

fn workspace_slug(repo_root: &Path) -> String {
    repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace")
        .to_string()
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

#[cfg(test)]
mod tests {
    use orbit_common::types::TaskStatus;
    use tempfile::tempdir;

    use super::*;
    use crate::OrbitRuntime;
    use crate::command::task::{TaskAddParams, TaskUpdateParams};

    fn v2_runtime() -> (tempfile::TempDir, PathBuf, PathBuf, OrbitRuntime) {
        let root = tempdir().expect("tempdir");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("create global root");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build runtime");
        (root, global_root, workspace_root, runtime)
    }

    #[test]
    fn v2_task_backend_wires_through_runtime_add_show_list_and_update() {
        let (_root, _global_root, workspace_root, runtime) = v2_runtime();

        let task = runtime
            .add_task(TaskAddParams {
                title: "Runtime v2 task".to_string(),
                description: "Created through OrbitRuntime".to_string(),
                plan: "1. Start it".to_string(),
                status: Some(TaskStatus::Backlog),
                ..Default::default()
            })
            .expect("create task");
        assert_eq!(task.id, "ORB-00000");
        assert!(!workspace_root.join("tasks/backlog").exists());
        assert!(workspace_root.join("tasks/ORB-00000").exists());

        let started = runtime
            .start_task(&task.id, Some("start".to_string()), None)
            .expect("start task");
        assert_eq!(started.status, TaskStatus::InProgress);

        let updated = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    comment: Some("Runtime comment".to_string()),
                    execution_summary: Some("Finished the runtime smoke".to_string()),
                    status: Some(TaskStatus::Review),
                    ..Default::default()
                },
            )
            .expect("update task");
        assert_eq!(updated.status, TaskStatus::Review);
        assert!(
            runtime
                .get_task_comments(&task.id)
                .expect("read task comments")
                .iter()
                .any(|comment| comment.message == "Runtime comment")
        );
        assert_eq!(runtime.list_tasks().expect("list tasks").len(), 1);
        assert_eq!(
            runtime
                .search_tasks("runtime smoke")
                .expect("search tasks")
                .len(),
            1
        );

        runtime
            .delete_task_guarded(&updated.id, true)
            .expect("delete v2 task");
        assert!(matches!(
            runtime.get_task(&updated.id),
            Err(OrbitError::TaskNotFound(_))
        ));
    }

    #[test]
    fn v2_task_backend_persists_workspace_binding_across_runtime_rebuild() {
        let (_root, global_root, workspace_root, runtime) = v2_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Persistent v2 task".to_string(),
                description: "Survives runtime reconstruction".to_string(),
                status: Some(TaskStatus::Backlog),
                ..Default::default()
            })
            .expect("create task");
        let workspace_config =
            read_workspace_config_optional(&workspace_root).expect("read workspace config");
        let workspace_id = workspace_config
            .as_ref()
            .map(|config| config.workspace_id.as_str())
            .expect("workspace id");
        assert!(workspace_id.starts_with("repo-"), "{workspace_id}");
        assert_eq!(workspace_id.len(), "repo-000000".len());

        let rebuilt =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("rebuild runtime");
        let fetched = rebuilt.get_task(&task.id).expect("get task after rebuild");
        assert_eq!(fetched.title, "Persistent v2 task");
        assert_eq!(
            read_workspace_config_optional(&workspace_root)
                .expect("read workspace config")
                .map(|config| config.workspace_id),
            workspace_config.map(|config| config.workspace_id)
        );
    }

    #[test]
    fn v2_task_backend_rebinds_when_workspace_config_is_missing() {
        let (_root, global_root, workspace_root, runtime) = v2_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Rebind v2 task".to_string(),
                description: "Survives missing workspace config".to_string(),
                status: Some(TaskStatus::Backlog),
                ..Default::default()
            })
            .expect("create task");
        let original_config =
            read_workspace_config_optional(&workspace_root).expect("read workspace config");
        std::fs::remove_file(workspace_root.join("config.yaml")).expect("remove workspace config");

        let rebuilt =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("rebuild runtime");
        let fetched = rebuilt.get_task(&task.id).expect("get task after rebind");

        assert_eq!(fetched.title, "Rebind v2 task");
        assert_eq!(
            read_workspace_config_optional(&workspace_root)
                .expect("read rewritten workspace config")
                .map(|config| config.workspace_id),
            original_config.map(|config| config.workspace_id)
        );
    }
}
