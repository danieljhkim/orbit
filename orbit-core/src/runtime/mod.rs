pub mod audit;
pub mod event_bus;
pub mod execute;
pub mod mutation;
pub mod pipeline;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use orbit_policy::PolicyEngine;
use orbit_tools::ToolRegistry;
use orbit_types::{Audit, Job, JobStatus, OrbitEvent, Task};

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

        Ok(Self {
            context: OrbitContext {
                store,
                policy: PolicyEngine::new_local_default_allow(),
                registry: Arc::new(registry),
            },
            event_bus: event_bus::EventBus::default(),
        })
    }

    pub fn with_policy(mut self, policy: PolicyEngine) -> Self {
        self.context.policy = policy;
        self
    }

    pub fn add_task(&self, title: &str) -> Result<Task, OrbitError> {
        self.with_mutation(|tx| {
            let task = tx.insert_task(title)?;
            Ok((
                task.clone(),
                OrbitEvent::TaskAdded {
                    id: task.id.clone(),
                },
            ))
        })
    }

    pub fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        self.context.store.list_tasks()
    }

    pub fn list_audits(&self, limit: usize) -> Result<Vec<Audit>, OrbitError> {
        self.context.store.list_audits(limit)
    }

    pub fn schedule_job(
        &self,
        name: &str,
        command: &str,
        next_run_at: DateTime<Utc>,
    ) -> Result<Job, OrbitError> {
        self.with_mutation(|tx| {
            let job = tx.insert_job(name, command, next_run_at)?;
            Ok((job.clone(), OrbitEvent::JobStarted { id: job.id.clone() }))
        })
    }

    pub fn job_status(&self, id: &str) -> Result<Option<JobStatus>, OrbitError> {
        self.context.store.get_job_status(id)
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
