#[path = "agent/environment.rs"]
mod environment;
#[path = "agent/execution.rs"]
mod execution;
#[path = "agent/response.rs"]
mod response;

use orbit_types::ExecutorDef;

use super::ActivityExecutor;
use crate::context::{
    AttemptOutcome, ExecutionContext, ExecutorHost, execution_working_directory_with_task,
};

pub(crate) use environment::{
    inject_activity_tools, inject_actor_kind, inject_agent_identity, inject_proc_allowed_programs,
};
use execution::{execute_with_cwd, resolve_agent_execution};

pub struct AgentExecutor {
    bound_executor: Option<ExecutorDef>,
}

impl AgentExecutor {
    pub fn new() -> Self {
        Self {
            bound_executor: None,
        }
    }

    pub fn from_executor_def(def: ExecutorDef) -> Self {
        Self {
            bound_executor: Some(def),
        }
    }
}

impl ActivityExecutor for AgentExecutor {
    fn spec_type(&self) -> &str {
        "agent_invoke"
    }

    fn execute(&self, host: ExecutorHost<'_>, execution: &ExecutionContext) -> AttemptOutcome {
        let agent_host = host.agent();
        let working_dir = execution_working_directory_with_task(&agent_host, execution);
        let resolved =
            match resolve_agent_execution(&agent_host, execution, self.bound_executor.as_ref()) {
                Ok(resolved) => resolved,
                Err(error) => return response::invocation_failed_outcome(error),
            };
        execute_with_cwd(&agent_host, execution, working_dir, &resolved)
    }
}
