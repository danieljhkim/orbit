use std::path::Path;
use std::sync::Arc;

use orbit_policy::PolicyEngine;
use orbit_search::{EmbedWorker, VectorStore};
use orbit_store::Store;
use orbit_store::compose::{
    WorkspaceTaskBackends, audit_event_store_sqlite, global_executor_def_store,
    global_policy_def_store, layered_policy_def_store, task_reservation_store_sqlite,
    tool_store_sqlite, workspace_job_run_store, workspace_policy_def_store,
    workspace_task_backends,
};
use orbit_store::maintenance::task_registry::{
    BindWorkspaceParams, TaskRegistryStore, WorkspaceConfig, read_workspace_config_optional,
    task_registry_path, workspace_id_for_orbit_dir, write_workspace_config,
};

use orbit_common::OrbitError;
use orbit_config::ResolvedConfig;
use orbit_engine::PrConfig;
use orbit_tools::ToolRegistry;
use orbit_tools::external::ExternalTool;
use orbit_types::policy::DEFAULT_POLICY_NAME;
use orbit_types::workspace::WorkspacePaths;

use crate::context::OrbitContext;
use crate::context::{
    ActorIdentity, OrbitExecutionAssets, OrbitPolicyContext, OrbitRuntimeSettings, OrbitStores,
};
use crate::runtime::WorkspaceRuntimeBinding;
use crate::skill_catalog::SkillCatalog;

/// Runtime builder. Global root provides activities, jobs, executors, policies,
/// config, global skills, and SQLite. Shared root provides existing workspace
/// state. Local root is carried for per-worktree artifact phases.
pub(crate) fn build_context_from_roots(
    global_root: &Path,
    workspace_root: &Path,
    local_root: &Path,
    binding: Option<&WorkspaceRuntimeBinding>,
    runtime_config: &ResolvedConfig,
) -> Result<OrbitContext, OrbitError> {
    let persistence = &runtime_config.persistence;

    let store = Store::open(&persistence.audit_db)?;

    // workspace_root IS the .orbit dir. For custom roots outside the repo,
    // prefer the registry's workspace root over the parent-directory fallback.
    let repo_root = binding
        .map(|binding| binding.repo_root.clone())
        .unwrap_or_else(|| {
            workspace_root
                .parent()
                .unwrap_or(workspace_root)
                .to_path_buf()
        });
    let paths = WorkspacePaths::new_with_local(
        repo_root,
        workspace_root.to_path_buf(),
        local_root.to_path_buf(),
        global_root.to_path_buf(),
    );

    let task_backends = build_v2_task_backends(
        global_root,
        &paths,
        binding.map(|binding| binding.workspace_id.as_str()),
    )?;
    let workspace_id = workspace_id_for_orbit_dir(&paths.orbit_dir)?;
    let import_report = orbit_store::workflow::legacy_state::import_legacy_v2_state(
        &store,
        &paths.orbit_dir,
        &workspace_id,
    )?;
    if import_report.skipped_records() {
        tracing::warn!(
            workspace_id = %workspace_id,
            audit_events_skipped = import_report.audit_events_skipped,
            "skipped malformed legacy state records during SQLite import",
        );
    }
    let semantic_vector_store = Arc::new(VectorStore::open(&persistence.semantic_db)?);
    let semantic_worker = Arc::new(EmbedWorker::start((*semantic_vector_store).clone()));
    let job_run_store = workspace_job_run_store(store.clone(), workspace_id);

    // Executors and policies are global-only. Jobs always persist run state
    // under the workspace state directory.
    let tool_store = tool_store_sqlite(store.clone());
    let audit_event_store = audit_event_store_sqlite(store.clone());
    let task_reservation_store = task_reservation_store_sqlite(store.clone());
    let executor_def_store = global_executor_def_store(persistence.executor_dir.clone());
    let global_policy_store = global_policy_def_store(persistence.policy_dir.clone());
    let workspace_policy_store = workspace_policy_def_store(paths.policies_dir.clone());
    let policy_def_store = layered_policy_def_store(workspace_policy_store, global_policy_store);
    let active_policy = policy_def_store
        .get_policy_def(DEFAULT_POLICY_NAME)?
        .ok_or_else(|| {
            OrbitError::Execution(format!(
                "default policy `{DEFAULT_POLICY_NAME}` was not found after seeding"
            ))
        })?;

    let skill_catalog =
        SkillCatalog::layered(persistence.skill_dir.clone(), global_root.join("skills"));
    skill_catalog.ensure_layout()?;

    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    load_external_tools(&store, &mut registry)?;

    let execution_env_policy = runtime_config.execution_env.clone();
    let codex_execution_policy = runtime_config.codex_execution.clone();
    let persistence = runtime_config.persistence.clone();
    let actor = ActorIdentity::from_env();
    let scoring_enabled = runtime_config.scoring_enabled;
    // Config owns PR settings as plain data; Core is the composition layer
    // that translates them into the execution engine's shape.
    let pr_config = PrConfig {
        task_url_template: runtime_config.pr.task_url_template.clone(),
    };
    let workflow_base_branch = runtime_config.workflow_base_branch.clone();
    let workflow_auto_ship = runtime_config.workflow_auto_ship;
    let routines_source = runtime_config.routines_source;
    let crews = runtime_config.crews.clone();
    let default_crew = runtime_config.default_crew.clone();
    let system_crew = runtime_config.system_crew.clone();

    Ok(OrbitContext::new(
        paths,
        OrbitStores::new(
            task_backends.task,
            task_backends.document,
            task_backends.history,
            task_backends.artifact,
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
            scoring_enabled,
            pr_config,
            workflow_base_branch,
            workflow_auto_ship,
            routines_source,
            crews,
            default_crew,
            system_crew,
        ),
    ))
}

fn build_v2_task_backends(
    global_root: &Path,
    paths: &WorkspacePaths,
    workspace_id_hint: Option<&str>,
) -> Result<WorkspaceTaskBackends, OrbitError> {
    let registry = TaskRegistryStore::open(&task_registry_path(global_root))?;
    let config = read_workspace_config_optional(&paths.orbit_dir)?;
    let workspace_id = if let Some(config) = &config {
        if let Some(hint) = workspace_id_hint
            && config.workspace_id != hint
        {
            return Err(OrbitError::WorkspaceError(format!(
                "workspace binding id '{}' does not match configured workspace id '{}'",
                hint, config.workspace_id
            )));
        }
        Some(config.workspace_id.clone())
    } else if let Some(hint) = workspace_id_hint {
        Some(hint.to_string())
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

pub(crate) type TempDir = tempfile::TempDir;

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
