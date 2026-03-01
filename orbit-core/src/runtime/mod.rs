pub mod audit;
pub mod event_bus;
pub mod execute;
pub mod mutation;
pub mod pipeline;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use orbit_policy::PolicyEngine;
use orbit_tools::ToolRegistry;
use orbit_tools::external::ExternalTool;
use orbit_types::{Audit, Job};

use crate::config::RuntimeConfig;
use crate::skill_catalog::SkillCatalog;
use crate::task_file_store::TaskFileStore;
use crate::{OrbitContext, OrbitError};

#[derive(Clone)]
pub struct OrbitRuntime {
    pub(crate) context: OrbitContext,
    pub event_bus: event_bus::EventBus,
}

impl OrbitRuntime {
    pub fn initialize() -> Result<Self, OrbitError> {
        let data_root = Self::default_data_root();
        Self::from_data_root(&data_root)
    }

    pub fn from_data_root(data_root: &Path) -> Result<Self, OrbitError> {
        let runtime_config = RuntimeConfig::load_from_data_root(data_root)?;
        let db_path = data_root.join("orbit.db");
        let store = orbit_store::Store::open(&db_path)?;
        let task_root = Self::task_root_path(data_root);
        let task_store = TaskFileStore::new(task_root);
        task_store.ensure_layout()?;
        let legacy_tasks = store.list_tasks()?;
        let _ = task_store.migrate_from_sqlite_tasks(&legacy_tasks)?;
        let skill_root = Self::skill_root_path(data_root);
        let skill_catalog = SkillCatalog::new(skill_root);
        skill_catalog.ensure_layout()?;
        Self::export_legacy_skills_to_files(&store, &skill_catalog)?;

        let mut registry = ToolRegistry::new();
        registry.register_builtins();
        Self::load_external_tools(&store, &mut registry)?;

        Ok(Self {
            context: OrbitContext {
                store,
                policy: PolicyEngine::new_local_default_allow(),
                registry: Arc::new(registry),
                task_store,
                skill_catalog,
                execution_env_policy: runtime_config.execution_env,
            },
            event_bus: event_bus::EventBus::default(),
        })
    }

    pub fn in_memory() -> Result<Self, OrbitError> {
        let store = orbit_store::Store::open_in_memory()?;
        let task_root = std::env::temp_dir().join(format!(
            "orbit-task-store-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let task_store = TaskFileStore::new(task_root);
        task_store.ensure_layout()?;
        let skill_root = std::env::temp_dir().join(format!(
            "orbit-skill-store-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let skill_catalog = SkillCatalog::new(skill_root);
        skill_catalog.ensure_layout()?;
        let mut registry = ToolRegistry::new();
        registry.register_builtins();
        Self::load_external_tools(&store, &mut registry)?;
        let runtime_config = RuntimeConfig::default();

        Ok(Self {
            context: OrbitContext {
                store,
                policy: PolicyEngine::new_local_default_allow(),
                registry: Arc::new(registry),
                task_store,
                skill_catalog,
                execution_env_policy: runtime_config.execution_env,
            },
            event_bus: event_bus::EventBus::default(),
        })
    }

    fn load_external_tools(
        store: &orbit_store::Store,
        registry: &mut ToolRegistry,
    ) -> Result<(), OrbitError> {
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

    pub fn with_policy(mut self, policy: PolicyEngine) -> Self {
        self.context.policy = policy;
        self
    }

    pub fn list_audits(&self, limit: usize) -> Result<Vec<Audit>, OrbitError> {
        self.context.store.list_audits(limit)
    }

    pub fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.context.store.get_job(job_id)
    }

    pub fn execution_env_config(&self) -> (bool, Vec<String>) {
        (
            self.context.execution_env_policy.inherit(),
            self.context.execution_env_policy.pass().to_vec(),
        )
    }

    pub fn run_jobs(&self) -> Result<usize, OrbitError> {
        self.run_due_jobs(Utc::now())
    }

    pub fn trigger_watch_once(&self, path: &str) -> Result<(), OrbitError> {
        self.trigger_watch_path(path)
    }

    pub fn default_data_root() -> PathBuf {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".orbit")
    }

    fn task_root_path(data_root: &Path) -> PathBuf {
        if let Ok(value) = std::env::var("ORBIT_TASK_ROOT")
            && !value.trim().is_empty()
        {
            return PathBuf::from(value);
        }
        data_root.join("tasks")
    }

    fn skill_root_path(data_root: &Path) -> PathBuf {
        if let Ok(value) = std::env::var("ORBIT_SKILL_ROOT")
            && !value.trim().is_empty()
        {
            return PathBuf::from(value);
        }
        data_root.join("skills")
    }

    fn export_legacy_skills_to_files(
        store: &orbit_store::Store,
        skill_catalog: &SkillCatalog,
    ) -> Result<(), OrbitError> {
        let legacy_skills = store.list_skills()?;
        for skill in legacy_skills {
            let skill_dir = skill_catalog.root().join(&skill.name);
            if skill_dir.join("SKILL.md").exists() {
                continue;
            }
            std::fs::create_dir_all(&skill_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
            let purpose = skill
                .description
                .clone()
                .unwrap_or_else(|| format!("Migrated legacy skill '{}'", skill.name));
            let description = purpose
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let description = description.replace('\\', "\\\\").replace('"', "\\\"");
            let content = format!(
                "---\nname: {id}\ndescription: {description}\n---\n\n# {id}\n\n## Purpose\n{purpose}\n\n## Behavioral Constraints\n{instructions}\n\n## Output Requirements\n- Return structured output that matches the execution contract.\n",
                id = skill.name,
                description = format!("\"{description}\""),
                instructions = skill.instructions.trim(),
            );
            std::fs::write(skill_dir.join("SKILL.md"), content)
                .map_err(|e| OrbitError::Io(e.to_string()))?;
        }
        Ok(())
    }
}
