//! [`ExecutorHost`] and the narrowed per-executor facades.
//!
//! Each facade exposes only the host traits its executor family needs and
//! delegates every call back to the full host passed to [`ExecutorHost::new`].

use orbit_common::types::activity_job::AgentRole;
use orbit_common::types::{
    ExecutorDef, ExternalRef, OrbitError, Task, TaskArtifact, TaskComment, TaskHistoryEntry,
    TaskPriority, TaskStatus,
};
use std::collections::HashMap;

use super::execution::ExecutionContext;
use super::hosts::{
    AgentProtocolHost, AgentRoleConfig, EnvironmentHost, ExecutorLookupHost, RuntimeHost, TaskHost,
    TaskReadHost,
};

#[derive(Clone, Copy)]
pub struct ExecutorHost<'a> {
    task_reader: &'a (dyn TaskReadHost + Sync),
    environment: &'a (dyn EnvironmentHost + Sync),
    agent_protocol: &'a (dyn AgentProtocolHost + Sync),
    executor_lookup: &'a (dyn ExecutorLookupHost + Sync),
}

impl<'a> ExecutorHost<'a> {
    pub fn new<H>(host: &'a H) -> Self
    where
        H: RuntimeHost + TaskHost + EnvironmentHost + AgentProtocolHost + ExecutorLookupHost + Sync,
    {
        Self {
            task_reader: host,
            environment: host,
            agent_protocol: host,
            executor_lookup: host,
        }
    }

    pub fn agent(self) -> AgentExecutorHost<'a> {
        AgentExecutorHost {
            task_reader: self.task_reader,
            environment: self.environment,
            agent_protocol: self.agent_protocol,
            executor_lookup: self.executor_lookup,
        }
    }

    pub fn cli(self) -> CliCommandExecutorHost<'a> {
        CliCommandExecutorHost {
            task_reader: self.task_reader,
            environment: self.environment,
        }
    }
}

#[derive(Clone, Copy)]
pub struct AgentExecutorHost<'a> {
    task_reader: &'a (dyn TaskReadHost + Sync),
    environment: &'a (dyn EnvironmentHost + Sync),
    agent_protocol: &'a (dyn AgentProtocolHost + Sync),
    executor_lookup: &'a (dyn ExecutorLookupHost + Sync),
}

impl TaskReadHost for AgentExecutorHost<'_> {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        self.task_reader.get_task(task_id)
    }

    fn get_task_artifacts(&self, task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
        self.task_reader.get_task_artifacts(task_id)
    }

    fn get_task_comments(&self, task_id: &str) -> Result<Vec<TaskComment>, OrbitError> {
        self.task_reader.get_task_comments(task_id)
    }

    fn get_task_history(&self, task_id: &str) -> Result<Vec<TaskHistoryEntry>, OrbitError> {
        self.task_reader.get_task_history(task_id)
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        job_run_id: Option<&str>,
        external_ref: Option<&ExternalRef>,
        has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.task_reader.list_tasks_filtered(
            status,
            priority,
            parent_id,
            job_run_id,
            external_ref,
            has_external_ref_system,
        )
    }
}

impl EnvironmentHost for AgentExecutorHost<'_> {
    fn agent_provider_config(&self) -> HashMap<String, String> {
        self.environment.agent_provider_config()
    }

    fn execution_env_inherit(&self) -> bool {
        self.environment.execution_env_inherit()
    }

    fn hydrated_env_allowlist(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.environment.hydrated_env_allowlist(env_extra)
    }

    fn orbit_root(&self) -> Option<String> {
        self.environment.orbit_root()
    }

    fn cli_command_environment(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.environment.cli_command_environment(env_extra)
    }

    fn missing_required_environment_vars(&self, required_env_vars: &[&str]) -> Vec<String> {
        self.environment
            .missing_required_environment_vars(required_env_vars)
    }

    fn agent_role_config(&self, role: AgentRole) -> Option<AgentRoleConfig> {
        self.environment.agent_role_config(role)
    }
}

impl AgentProtocolHost for AgentExecutorHost<'_> {
    fn build_agent_stdin_envelope_payload(
        &self,
        execution: &ExecutionContext,
    ) -> Result<Vec<u8>, OrbitError> {
        self.agent_protocol
            .build_agent_stdin_envelope_payload(execution)
    }
}

impl ExecutorLookupHost for AgentExecutorHost<'_> {
    fn get_executor_def(&self, name: &str) -> Result<Option<ExecutorDef>, OrbitError> {
        self.executor_lookup.get_executor_def(name)
    }
}

#[derive(Clone, Copy)]
pub struct CliCommandExecutorHost<'a> {
    task_reader: &'a (dyn TaskReadHost + Sync),
    environment: &'a (dyn EnvironmentHost + Sync),
}

impl TaskReadHost for CliCommandExecutorHost<'_> {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        self.task_reader.get_task(task_id)
    }

    fn get_task_artifacts(&self, task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
        self.task_reader.get_task_artifacts(task_id)
    }

    fn get_task_comments(&self, task_id: &str) -> Result<Vec<TaskComment>, OrbitError> {
        self.task_reader.get_task_comments(task_id)
    }

    fn get_task_history(&self, task_id: &str) -> Result<Vec<TaskHistoryEntry>, OrbitError> {
        self.task_reader.get_task_history(task_id)
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        job_run_id: Option<&str>,
        external_ref: Option<&ExternalRef>,
        has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.task_reader.list_tasks_filtered(
            status,
            priority,
            parent_id,
            job_run_id,
            external_ref,
            has_external_ref_system,
        )
    }
}

impl EnvironmentHost for CliCommandExecutorHost<'_> {
    fn agent_provider_config(&self) -> HashMap<String, String> {
        self.environment.agent_provider_config()
    }

    fn execution_env_inherit(&self) -> bool {
        self.environment.execution_env_inherit()
    }

    fn hydrated_env_allowlist(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.environment.hydrated_env_allowlist(env_extra)
    }

    fn orbit_root(&self) -> Option<String> {
        self.environment.orbit_root()
    }

    fn cli_command_environment(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.environment.cli_command_environment(env_extra)
    }

    fn missing_required_environment_vars(&self, required_env_vars: &[&str]) -> Vec<String> {
        self.environment
            .missing_required_environment_vars(required_env_vars)
    }

    fn agent_role_config(&self, role: AgentRole) -> Option<AgentRoleConfig> {
        self.environment.agent_role_config(role)
    }
}
