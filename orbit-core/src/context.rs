use std::path::Path;
use std::sync::Arc;

use orbit_lock::FileLockStore;
use orbit_policy::PolicyEngine;
use orbit_store::{
    ActivityStoreBackend, AuditEventStoreBackend, JobStoreBackend, TaskStoreBackend,
    ToolStoreBackend,
};
use orbit_tools::ToolRegistry;
use orbit_types::WorkspacePaths;

use crate::config::{CodexExecutionPolicy, ExecutionEnvPolicy, PersistenceConfig};
use crate::skill_catalog::SkillCatalog;

const ORBIT_TASK_ACTOR_KIND: &str = "ORBIT_TASK_ACTOR_KIND";
const ORBIT_TASK_ACTOR_LABEL: &str = "ORBIT_TASK_ACTOR_LABEL";
const LEGACY_ORBIT_TASK_ACTOR_IDENTITY_ID: &str = "ORBIT_TASK_ACTOR_IDENTITY_ID";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
    Human,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorIdentity {
    pub kind: ActorKind,
    pub label: String,
}

impl ActorIdentity {
    pub fn human(label: impl Into<String>) -> Self {
        Self {
            kind: ActorKind::Human,
            label: normalize_actor_label(label.into(), "human"),
        }
    }

    pub fn agent(label: impl Into<String>) -> Self {
        Self {
            kind: ActorKind::Agent,
            label: normalize_actor_label(label.into(), "agent"),
        }
    }

    pub(crate) fn from_env() -> Self {
        let kind_raw = std::env::var(ORBIT_TASK_ACTOR_KIND).ok();
        let actor_label = std::env::var(ORBIT_TASK_ACTOR_LABEL)
            .ok()
            .or_else(|| std::env::var(LEGACY_ORBIT_TASK_ACTOR_IDENTITY_ID).ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        match kind_raw.as_deref() {
            Some("agent") => actor_label
                .map(Self::agent)
                .unwrap_or_else(|| Self::agent("agent")),
            _ if actor_label.is_some() => Self::agent(actor_label.unwrap_or_default()),
            _ => Self::default(),
        }
    }
}

impl Default for ActorIdentity {
    fn default() -> Self {
        Self::human("human")
    }
}

#[derive(Clone)]
pub struct OrbitContext {
    paths: WorkspacePaths,
    file_lock_store: Arc<FileLockStore>,
    task_store: Arc<dyn TaskStoreBackend>,
    activity_store: Arc<dyn ActivityStoreBackend>,
    job_store: Arc<dyn JobStoreBackend>,
    tool_store: Arc<dyn ToolStoreBackend>,
    audit_event_store: Arc<dyn AuditEventStoreBackend>,
    policy: PolicyEngine,
    registry: Arc<ToolRegistry>,
    skill_catalog: SkillCatalog,
    execution_env_policy: ExecutionEnvPolicy,
    codex_execution_policy: CodexExecutionPolicy,
    persistence: PersistenceConfig,
    user_name: String,
    actor: ActorIdentity,
    task_approval_required_for_agent: bool,
    task_delegate_approval: bool,
    scoring_enabled: bool,
}

impl OrbitContext {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        paths: WorkspacePaths,
        file_lock_store: Arc<FileLockStore>,
        task_store: Arc<dyn TaskStoreBackend>,
        activity_store: Arc<dyn ActivityStoreBackend>,
        job_store: Arc<dyn JobStoreBackend>,
        tool_store: Arc<dyn ToolStoreBackend>,
        audit_event_store: Arc<dyn AuditEventStoreBackend>,
        policy: PolicyEngine,
        registry: Arc<ToolRegistry>,
        skill_catalog: SkillCatalog,
        execution_env_policy: ExecutionEnvPolicy,
        codex_execution_policy: CodexExecutionPolicy,
        persistence: PersistenceConfig,
        user_name: String,
        actor: ActorIdentity,
        task_approval_required_for_agent: bool,
        task_delegate_approval: bool,
        scoring_enabled: bool,
    ) -> Self {
        Self {
            paths,
            file_lock_store,
            task_store,
            activity_store,
            job_store,
            tool_store,
            audit_event_store,
            policy,
            registry,
            skill_catalog,
            execution_env_policy,
            codex_execution_policy,
            persistence,
            user_name,
            actor,
            task_approval_required_for_agent,
            task_delegate_approval,
            scoring_enabled,
        }
    }

    /// Returns the .orbit/ data directory (backward-compatible alias).
    pub(crate) fn data_root(&self) -> &Path {
        &self.paths.orbit_dir
    }

    pub(crate) fn global_root(&self) -> &Path {
        &self.paths.global_dir
    }

    pub(crate) fn paths(&self) -> &WorkspacePaths {
        &self.paths
    }

    pub(crate) fn task_store(&self) -> &Arc<dyn TaskStoreBackend> {
        &self.task_store
    }

    pub(crate) fn file_lock_store(&self) -> &Arc<FileLockStore> {
        &self.file_lock_store
    }

    pub(crate) fn activity_store(&self) -> &Arc<dyn ActivityStoreBackend> {
        &self.activity_store
    }

    pub(crate) fn job_store(&self) -> &Arc<dyn JobStoreBackend> {
        &self.job_store
    }

    pub(crate) fn tool_store(&self) -> &Arc<dyn ToolStoreBackend> {
        &self.tool_store
    }

    pub(crate) fn audit_event_store(&self) -> &Arc<dyn AuditEventStoreBackend> {
        &self.audit_event_store
    }

    pub(crate) fn policy(&self) -> &PolicyEngine {
        &self.policy
    }

    pub(crate) fn set_policy(&mut self, policy: PolicyEngine) {
        self.policy = policy;
    }

    pub(crate) fn registry(&self) -> &ToolRegistry {
        self.registry.as_ref()
    }

    pub(crate) fn skill_catalog(&self) -> &SkillCatalog {
        &self.skill_catalog
    }

    pub(crate) fn execution_env_policy(&self) -> &ExecutionEnvPolicy {
        &self.execution_env_policy
    }

    pub(crate) fn codex_execution_policy(&self) -> &CodexExecutionPolicy {
        &self.codex_execution_policy
    }

    pub(crate) fn persistence(&self) -> &PersistenceConfig {
        &self.persistence
    }

    pub(crate) fn user_name(&self) -> &str {
        &self.user_name
    }

    pub(crate) fn actor(&self) -> &ActorIdentity {
        &self.actor
    }

    pub(crate) fn set_actor(&mut self, actor: ActorIdentity) {
        self.actor = actor;
    }

    pub(crate) fn task_approval_required_for_agent(&self) -> bool {
        self.task_approval_required_for_agent
    }

    pub(crate) fn task_delegate_approval(&self) -> bool {
        self.task_delegate_approval
    }

    pub(crate) fn scoring_enabled(&self) -> bool {
        self.scoring_enabled
    }
}

fn normalize_actor_label(label: String, default_label: &str) -> String {
    let label = label.trim();
    if label.is_empty() {
        default_label.to_string()
    } else {
        label.to_string()
    }
}
