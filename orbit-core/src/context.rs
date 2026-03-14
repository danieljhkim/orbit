use std::path::PathBuf;
use std::sync::Arc;

use orbit_policy::PolicyEngine;
use orbit_store::{
    ActivityStoreBackend, AgentSessionStoreBackend, AuditEventStoreBackend, JobStoreBackend,
    LockStoreBackend, TaskStoreBackend, ToolStoreBackend,
};
use orbit_tools::ToolRegistry;

use crate::config::{CodexExecutionPolicy, ExecutionEnvPolicy, PersistenceConfig};
use crate::identity_catalog::IdentityCatalog;
use crate::skill_catalog::SkillCatalog;

#[derive(Clone)]
pub struct OrbitContext {
    pub(crate) data_root: PathBuf,
    pub(crate) orbit_home: PathBuf,
    pub(crate) task_store: Arc<dyn TaskStoreBackend>,
    pub(crate) activity_store: Arc<dyn ActivityStoreBackend>,
    pub(crate) job_store: Arc<dyn JobStoreBackend>,
    pub(crate) tool_store: Arc<dyn ToolStoreBackend>,
    pub(crate) audit_event_store: Arc<dyn AuditEventStoreBackend>,
    pub(crate) agent_session_store: Arc<dyn AgentSessionStoreBackend>,
    pub(crate) lock_store: Arc<dyn LockStoreBackend>,
    pub(crate) policy: PolicyEngine,
    pub(crate) registry: Arc<ToolRegistry>,
    pub(crate) skill_catalog: SkillCatalog,
    pub(crate) identity_catalog: IdentityCatalog,
    pub(crate) execution_env_policy: ExecutionEnvPolicy,
    pub(crate) codex_execution_policy: CodexExecutionPolicy,
    pub(crate) persistence: PersistenceConfig,
    pub(crate) task_approval_required_for_agent: bool,
    pub(crate) task_delegate_approval: bool,
}
