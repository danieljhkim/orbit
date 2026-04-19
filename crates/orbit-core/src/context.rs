use std::path::Path;
use std::sync::Arc;

use orbit_policy::PolicyEngine;
use orbit_store::{
    ActivityStoreBackend, AuditEventStoreBackend, ExecutorDefStoreBackend,
    JobDefinitionStoreBackend, JobRunStoreBackend, PolicyDefStoreBackend, TaskArtifactStoreBackend,
    TaskDocumentStoreBackend, TaskHistoryStoreBackend, TaskReservationStoreBackend,
    TaskReviewStoreBackend, TaskStoreBackend, ToolStoreBackend,
};
use orbit_tools::ToolRegistry;
use orbit_types::WorkspacePaths;

use crate::config::{CodexExecutionPolicy, ExecutionEnvPolicy, PersistenceConfig};
use crate::skill_catalog::SkillCatalog;

const ORBIT_AGENT_NAME: &str = "ORBIT_AGENT_NAME";
const ORBIT_AGENT_MODEL: &str = "ORBIT_AGENT_MODEL";

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
        let actor_label = std::env::var(ORBIT_AGENT_MODEL)
            .ok()
            .or_else(|| std::env::var(ORBIT_AGENT_NAME).ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        actor_label.map(Self::agent).unwrap_or_default()
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
    stores: OrbitStores,
    execution: OrbitExecutionAssets,
    policy: OrbitPolicyContext,
    runtime: OrbitRuntimeSettings,
}

#[derive(Clone)]
pub(crate) struct OrbitStores {
    pub(crate) task: Arc<dyn TaskStoreBackend>,
    pub(crate) task_document: Arc<dyn TaskDocumentStoreBackend>,
    pub(crate) task_history: Arc<dyn TaskHistoryStoreBackend>,
    pub(crate) task_review: Arc<dyn TaskReviewStoreBackend>,
    pub(crate) task_artifact: Arc<dyn TaskArtifactStoreBackend>,
    pub(crate) task_reservation: Arc<dyn TaskReservationStoreBackend>,
    pub(crate) activity: Arc<dyn ActivityStoreBackend>,
    pub(crate) job_definition: Arc<dyn JobDefinitionStoreBackend>,
    pub(crate) job_run: Arc<dyn JobRunStoreBackend>,
    pub(crate) tool: Arc<dyn ToolStoreBackend>,
    pub(crate) audit_event: Arc<dyn AuditEventStoreBackend>,
    pub(crate) executor_def: Arc<dyn ExecutorDefStoreBackend>,
    pub(crate) policy_def: Arc<dyn PolicyDefStoreBackend>,
}

impl OrbitStores {
    pub(crate) fn new(
        task: Arc<dyn TaskStoreBackend>,
        task_document: Arc<dyn TaskDocumentStoreBackend>,
        task_history: Arc<dyn TaskHistoryStoreBackend>,
        task_review: Arc<dyn TaskReviewStoreBackend>,
        task_artifact: Arc<dyn TaskArtifactStoreBackend>,
        task_reservation: Arc<dyn TaskReservationStoreBackend>,
        activity: Arc<dyn ActivityStoreBackend>,
        job_definition: Arc<dyn JobDefinitionStoreBackend>,
        job_run: Arc<dyn JobRunStoreBackend>,
        tool: Arc<dyn ToolStoreBackend>,
        audit_event: Arc<dyn AuditEventStoreBackend>,
        executor_def: Arc<dyn ExecutorDefStoreBackend>,
        policy_def: Arc<dyn PolicyDefStoreBackend>,
    ) -> Self {
        Self {
            task,
            task_document,
            task_history,
            task_review,
            task_artifact,
            task_reservation,
            activity,
            job_definition,
            job_run,
            tool,
            audit_event,
            executor_def,
            policy_def,
        }
    }
}

#[derive(Clone)]
pub(crate) struct OrbitExecutionAssets {
    registry: Arc<ToolRegistry>,
    skill_catalog: SkillCatalog,
}

impl OrbitExecutionAssets {
    pub(crate) fn new(registry: Arc<ToolRegistry>, skill_catalog: SkillCatalog) -> Self {
        Self {
            registry,
            skill_catalog,
        }
    }
}

#[derive(Clone)]
pub(crate) struct OrbitPolicyContext {
    policy: PolicyEngine,
    execution_env_policy: ExecutionEnvPolicy,
    codex_execution_policy: CodexExecutionPolicy,
}

impl OrbitPolicyContext {
    pub(crate) fn new(
        policy: PolicyEngine,
        execution_env_policy: ExecutionEnvPolicy,
        codex_execution_policy: CodexExecutionPolicy,
    ) -> Self {
        Self {
            policy,
            execution_env_policy,
            codex_execution_policy,
        }
    }
}

#[derive(Clone)]
pub(crate) struct OrbitRuntimeSettings {
    persistence: PersistenceConfig,
    actor: ActorIdentity,
    task_approval_required_for_agent: bool,
    task_delegate_approval: bool,
    scoring_enabled: bool,
    graph_editing: bool,
    /// Persisted default for the v2 `agent_loop` execution backend (§3.1).
    v2_backend: Option<String>,
}

impl OrbitRuntimeSettings {
    pub(crate) fn new(
        persistence: PersistenceConfig,
        actor: ActorIdentity,
        task_approval_required_for_agent: bool,
        task_delegate_approval: bool,
        scoring_enabled: bool,
        graph_editing: bool,
        v2_backend: Option<String>,
    ) -> Self {
        Self {
            persistence,
            actor,
            task_approval_required_for_agent,
            task_delegate_approval,
            scoring_enabled,
            graph_editing,
            v2_backend,
        }
    }

    pub(crate) fn v2_backend(&self) -> Option<&str> {
        self.v2_backend.as_deref()
    }
}

impl OrbitContext {
    pub(crate) fn new(
        paths: WorkspacePaths,
        stores: OrbitStores,
        execution: OrbitExecutionAssets,
        policy: OrbitPolicyContext,
        runtime: OrbitRuntimeSettings,
    ) -> Self {
        Self {
            paths,
            stores,
            execution,
            policy,
            runtime,
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

    pub(crate) fn stores(&self) -> &OrbitStores {
        &self.stores
    }

    pub(crate) fn policy(&self) -> &PolicyEngine {
        &self.policy.policy
    }

    pub(crate) fn set_policy(&mut self, policy: PolicyEngine) {
        self.policy.policy = policy;
    }

    pub(crate) fn registry(&self) -> &ToolRegistry {
        self.execution.registry.as_ref()
    }

    pub(crate) fn skill_catalog(&self) -> &SkillCatalog {
        &self.execution.skill_catalog
    }

    pub(crate) fn execution_env_policy(&self) -> &ExecutionEnvPolicy {
        &self.policy.execution_env_policy
    }

    pub(crate) fn codex_execution_policy(&self) -> &CodexExecutionPolicy {
        &self.policy.codex_execution_policy
    }

    pub(crate) fn persistence(&self) -> &PersistenceConfig {
        &self.runtime.persistence
    }

    pub(crate) fn actor(&self) -> &ActorIdentity {
        &self.runtime.actor
    }

    pub(crate) fn set_actor(&mut self, actor: ActorIdentity) {
        self.runtime.actor = actor;
    }

    pub(crate) fn task_approval_required_for_agent(&self) -> bool {
        self.runtime.task_approval_required_for_agent
    }

    pub(crate) fn task_delegate_approval(&self) -> bool {
        self.runtime.task_delegate_approval
    }

    pub(crate) fn scoring_enabled(&self) -> bool {
        self.runtime.scoring_enabled
    }

    pub(crate) fn graph_editing(&self) -> bool {
        self.runtime.graph_editing
    }

    /// Persisted default for the v2 `agent_loop` execution backend (§3.1
    /// resolution precedence step 3). `None` means "not configured".
    pub(crate) fn v2_backend(&self) -> Option<&str> {
        self.runtime.v2_backend()
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
