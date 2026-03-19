use std::path::PathBuf;
use std::sync::Arc;

use orbit_policy::PolicyEngine;
use orbit_store::{
    ActivityStoreBackend, AuditEventStoreBackend, JobStoreBackend, TaskStoreBackend,
    ToolStoreBackend,
};
use orbit_tools::ToolRegistry;

use crate::config::{CodexExecutionPolicy, ExecutionEnvPolicy, PersistenceConfig};
use crate::skill_catalog::SkillCatalog;

#[derive(Clone)]
pub struct OrbitContext {
    pub(crate) data_root: PathBuf,
    pub(crate) task_store: Arc<dyn TaskStoreBackend>,
    pub(crate) activity_store: Arc<dyn ActivityStoreBackend>,
    pub(crate) job_store: Arc<dyn JobStoreBackend>,
    pub(crate) tool_store: Arc<dyn ToolStoreBackend>,
    pub(crate) audit_event_store: Arc<dyn AuditEventStoreBackend>,
    pub(crate) policy: PolicyEngine,
    pub(crate) registry: Arc<ToolRegistry>,
    pub(crate) skill_catalog: SkillCatalog,
    pub(crate) execution_env_policy: ExecutionEnvPolicy,
    pub(crate) codex_execution_policy: CodexExecutionPolicy,
    pub(crate) persistence: PersistenceConfig,
    pub(crate) user_name: String,
    pub(crate) task_approval_required_for_agent: bool,
    pub(crate) task_delegate_approval: bool,
}
