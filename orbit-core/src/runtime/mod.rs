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
        let db_path = data_root.join("orbit.db");
        let store = orbit_store::Store::open(&db_path)?;

        let mut registry = ToolRegistry::new();
        registry.register_builtins();
        Self::load_external_tools(&store, &mut registry)?;

        Ok(Self {
            context: OrbitContext {
                store,
                policy: PolicyEngine::new_local_default_allow(),
                registry: Arc::new(registry),
            },
            event_bus: event_bus::EventBus::default(),
        })
    }

    pub fn in_memory() -> Result<Self, OrbitError> {
        let store = orbit_store::Store::open_in_memory()?;
        let mut registry = ToolRegistry::new();
        registry.register_builtins();
        Self::load_external_tools(&store, &mut registry)?;

        Ok(Self {
            context: OrbitContext {
                store,
                policy: PolicyEngine::new_local_default_allow(),
                registry: Arc::new(registry),
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
}
