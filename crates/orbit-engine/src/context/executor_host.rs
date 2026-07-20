//! [`ExecutorHost`] and the narrowed per-executor facades.
//!
//! Each facade exposes only the host traits its executor family needs and
//! delegates every call back to the full host passed to [`ExecutorHost::new`].

use crate::executor::registry::ActivityExecutorRegistry;
use orbit_common::types::activity_job::AgentRole;
use orbit_common::types::{
    Activity, AgentModelPair, ExecutorDef, ExternalRef, InvocationTrace, Job, JobTargetType,
    OrbitError, OrbitEvent, Role, Task, TaskArtifact, TaskComment, TaskHistoryEntry, TaskPriority,
    TaskStatus,
};
use orbit_store::{InvocationQuery, InvocationRecord};
use orbit_tools::ToolContext;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

use super::execution::ExecutionContext;
use super::hosts::{
    AgentProtocolHost, AgentRoleConfig, EnvironmentHost, ExecutorLookupHost, RuntimeHost,
    TaskActivityUpdate, TaskAutomationUpdate, TaskHost, TaskReadHost, TaskWriteHost,
};
use super::outcome::{ActivityInvocationResult, JobRunResult};

#[derive(Clone, Copy)]
pub struct ExecutorHost<'a> {
    runtime: &'a (dyn RuntimeHost + Sync),
    task_reader: &'a (dyn TaskReadHost + Sync),
    task_writer: &'a (dyn TaskWriteHost + Sync),
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
            runtime: host,
            task_reader: host,
            task_writer: host,
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

    pub fn automation(self) -> AutomationExecutorHost<'a> {
        AutomationExecutorHost {
            runtime: self.runtime,
            task_reader: self.task_reader,
            task_writer: self.task_writer,
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

#[derive(Clone, Copy)]
pub struct AutomationExecutorHost<'a> {
    runtime: &'a (dyn RuntimeHost + Sync),
    task_reader: &'a (dyn TaskReadHost + Sync),
    task_writer: &'a (dyn TaskWriteHost + Sync),
    environment: &'a (dyn EnvironmentHost + Sync),
}

impl TaskReadHost for AutomationExecutorHost<'_> {
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

impl TaskWriteHost for AutomationExecutorHost<'_> {
    fn start_task(
        &self,
        task_id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.task_writer.start_task(task_id, note, comment)
    }

    fn admit_task_for_workflow(&self, task_id: &str, workflow: &str) -> Result<Task, OrbitError> {
        self.task_writer.admit_task_for_workflow(task_id, workflow)
    }

    fn update_task_from_activity(
        &self,
        task_id: &str,
        update: TaskActivityUpdate,
    ) -> Result<Task, OrbitError> {
        self.task_writer.update_task_from_activity(task_id, update)
    }

    fn apply_task_automation_update(
        &self,
        task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        self.task_writer
            .apply_task_automation_update(task_id, update)
    }
}

impl EnvironmentHost for AutomationExecutorHost<'_> {
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

impl RuntimeHost for AutomationExecutorHost<'_> {
    fn record_event(&self, event: OrbitEvent) -> Result<(), OrbitError> {
        self.runtime.record_event(event)
    }

    fn repo_root(&self) -> Result<String, OrbitError> {
        self.runtime.repo_root()
    }

    fn data_root(&self) -> &Path {
        self.runtime.data_root()
    }

    fn activity_executor_registry(&self) -> &ActivityExecutorRegistry {
        self.runtime.activity_executor_registry()
    }

    fn run_job_now_with_input_debug(
        &self,
        job_id: &str,
        input: Value,
        debug: bool,
    ) -> Result<JobRunResult, OrbitError> {
        self.runtime
            .run_job_now_with_input_debug(job_id, input, debug)
    }

    fn cancel_job_run(&self, run_id: &str) -> Result<(), OrbitError> {
        self.runtime.cancel_job_run(run_id)
    }

    fn validate_activity_target_exists(
        &self,
        target_type: JobTargetType,
        target_id: &str,
    ) -> Result<Activity, OrbitError> {
        self.runtime
            .validate_activity_target_exists(target_type, target_id)
    }

    fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.runtime.get_job(job_id)
    }

    fn invocation_records(
        &self,
        query: InvocationQuery,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        self.runtime.invocation_records(query)
    }

    fn activity_implementer_identity(
        &self,
        input: &Value,
    ) -> Result<(Option<String>, Option<String>), OrbitError> {
        self.runtime.activity_implementer_identity(input)
    }

    fn run_tool_with_context_and_role(
        &self,
        name: &str,
        input: Value,
        role: Role,
        tool_context: ToolContext,
    ) -> Result<Value, OrbitError> {
        self.runtime
            .run_tool_with_context_and_role(name, input, role, tool_context)
    }

    fn invoke_activity(
        &self,
        activity: Activity,
        agent_cli: &str,
        model: Option<&str>,
        input: Value,
        timeout_seconds: u64,
        debug: bool,
    ) -> Result<ActivityInvocationResult, OrbitError> {
        self.runtime
            .invoke_activity(activity, agent_cli, model, input, timeout_seconds, debug)
    }

    fn maybe_create_failure_task(
        &self,
        job_id: &str,
        run_id: &str,
        error_code: &str,
        error_message: &str,
        agent: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), OrbitError> {
        self.runtime.maybe_create_failure_task(
            job_id,
            run_id,
            error_code,
            error_message,
            agent,
            model,
        )
    }

    fn resolved_agent_model_pair(&self, agent_cli: &str) -> Option<AgentModelPair> {
        self.runtime.resolved_agent_model_pair(agent_cli)
    }

    fn duel_candidate_families(&self) -> Vec<String> {
        self.runtime.duel_candidate_families()
    }

    fn duel_orchestrator_model(&self, family: &str) -> Option<String> {
        self.runtime.duel_orchestrator_model(family)
    }

    fn canonical_model_name(&self, agent_cli: &str, model: Option<&str>) -> Option<String> {
        self.runtime.canonical_model_name(agent_cli, model)
    }

    fn scoring_enabled(&self) -> bool {
        self.runtime.scoring_enabled()
    }

    fn graph_editing(&self) -> bool {
        self.runtime.graph_editing()
    }

    fn actor_model_identity(&self) -> Option<String> {
        self.runtime.actor_model_identity()
    }

    fn scoreboard_dir(&self) -> &Path {
        self.runtime.scoreboard_dir()
    }

    fn persist_invocation_trace(
        &self,
        job_run_id: &str,
        execution: &ExecutionContext,
        trace: &InvocationTrace,
    ) -> Result<(), OrbitError> {
        self.runtime
            .persist_invocation_trace(job_run_id, execution, trace)
    }
}
