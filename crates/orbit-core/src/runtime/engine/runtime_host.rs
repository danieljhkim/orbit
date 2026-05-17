use std::collections::HashMap;

use orbit_common::types::{
    Activity, AgentModelPair, InvocationTrace, JobRunState, JobTargetType, OrbitError, OrbitEvent,
    Role, RoleSlot,
};
use orbit_engine::{ActivityInvocationResult, ExecutionContext, ExecutorHost, RuntimeHost};
use orbit_store::{InvocationInsertParams, InvocationQuery, InvocationRecord, token_scoreboard};
use orbit_tools::ToolContext;
use serde_json::Value;

use super::identity::normalize_agent_name;
use super::invocation::{associated_task_ids, open_invocation_store};
use super::paths::current_repo_root;
use crate::OrbitRuntime;

impl RuntimeHost for OrbitRuntime {
    fn record_event(&self, event: OrbitEvent) -> Result<(), OrbitError> {
        OrbitRuntime::record_event(self, event)
    }

    fn repo_root(&self) -> Result<String, OrbitError> {
        current_repo_root(self)
    }

    fn data_root(&self) -> &std::path::Path {
        self.context.data_root()
    }

    fn activity_executor_registry(&self) -> &orbit_engine::ActivityExecutorRegistry {
        self.activity_executor_registry()
    }

    fn run_job_now_with_input_debug(
        &self,
        job_id: &str,
        _input: Value,
        _debug: bool,
    ) -> Result<orbit_engine::JobRunResult, OrbitError> {
        Err(OrbitError::Execution(format!(
            "v1 job dispatch is retired; refusing to run job '{job_id}' via RuntimeHost"
        )))
    }

    fn cancel_job_run(&self, run_id: &str) -> Result<(), OrbitError> {
        OrbitRuntime::cancel_job_run(self, run_id).map(|_| ())
    }

    fn validate_activity_target_exists(
        &self,
        _target_type: JobTargetType,
        target_id: &str,
    ) -> Result<Activity, OrbitError> {
        Err(OrbitError::Execution(format!(
            "v1 activity lookup is retired; refusing to resolve activity '{target_id}'"
        )))
    }

    fn get_job(&self, job_id: &str) -> Result<Option<orbit_common::types::Job>, OrbitError> {
        OrbitRuntime::get_job(self, job_id)
    }

    fn resolved_agent_model_pair(&self, agent_cli: &str) -> Option<AgentModelPair> {
        self.configured_agent_model_pair(agent_cli)
    }

    fn duel_candidate_families(&self) -> Vec<String> {
        self.duel_config().candidates.clone()
    }

    fn duel_orchestrator_model(&self, family: &str) -> Option<String> {
        let family = family.trim().to_ascii_lowercase();
        self.duel_config().models.get(&family).cloned()
    }

    fn canonical_model_name(&self, agent_cli: &str, model: Option<&str>) -> Option<String> {
        let _ = agent_cli;
        model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    fn invocation_records(
        &self,
        query: InvocationQuery,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        OrbitRuntime::invocation_records(self, query)
    }

    fn activity_implementer_identity(
        &self,
        input: &Value,
    ) -> Result<(Option<String>, Option<String>), OrbitError> {
        self.implementer_identity_for_activity_input(input)
    }

    fn run_tool_with_context_and_role(
        &self,
        name: &str,
        input: Value,
        role: Role,
        tool_context: ToolContext,
    ) -> Result<Value, OrbitError> {
        OrbitRuntime::run_tool_with_context_and_role(self, name, input, role, tool_context)
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
        if !is_planning_duel_agent_activity(&activity.id) {
            return Err(retired_invoke_activity_error(&activity.id));
        }

        let executor = self
            .activity_executor_registry()
            .get(agent_cli)
            .ok_or_else(|| {
                OrbitError::Execution(format!(
                    "planning duel activity '{}' requires direct-agent executor '{}'",
                    activity.id, agent_cli
                ))
            })?;
        let execution = ExecutionContext {
            activity,
            job: None,
            agent_cli: agent_cli.to_string(),
            model: model.map(ToOwned::to_owned),
            timeout_seconds,
            env_extra: Vec::new(),
            env_set: HashMap::new(),
            input,
            debug,
            steps_outputs: HashMap::new(),
            run_id: None,
            step_index: None,
            state_dir: None,
        };
        let outcome = executor.execute(ExecutorHost::new(self), &execution);
        if outcome.state != JobRunState::Success {
            return Err(OrbitError::Execution(outcome.error_message.unwrap_or_else(
                || {
                    format!(
                        "planning duel activity '{}' failed with state {}",
                        execution.activity.id, outcome.state
                    )
                },
            )));
        }
        Ok(ActivityInvocationResult {
            response_json: outcome.response_json,
            invocation_trace: outcome.invocation_trace.clone(),
            exit_code: outcome.exit_code,
            duration_ms: outcome
                .duration_ms
                .unwrap_or(outcome.invocation_trace.duration_ms),
        })
    }

    fn maybe_create_failure_task(
        &self,
        _job_id: &str,
        _run_id: &str,
        _error_code: &str,
        _error_message: &str,
        _agent: Option<&str>,
        _model: Option<&str>,
    ) -> Result<(), OrbitError> {
        Ok(())
    }

    fn scoring_enabled(&self) -> bool {
        self.context.scoring_enabled()
    }

    fn graph_editing(&self) -> bool {
        self.context.graph_editing()
    }

    fn actor_model_identity(&self) -> Option<String> {
        matches!(self.actor().kind, crate::context::ActorKind::Agent)
            .then(|| self.actor_label().trim())
            .filter(|label| !label.is_empty())
            .map(ToOwned::to_owned)
    }

    fn pr_config(&self) -> orbit_engine::PrConfig {
        OrbitRuntime::pr_config(self).clone()
    }

    fn scoreboard_dir(&self) -> &std::path::Path {
        &self.context.paths().scoreboard_dir
    }

    fn persist_invocation_trace(
        &self,
        job_run_id: &str,
        execution: &ExecutionContext,
        trace: &InvocationTrace,
    ) -> Result<(), OrbitError> {
        let requested_model = execution.model.as_deref();
        let (agent, model) =
            self.canonical_agent_model_identity(Some(&execution.agent_cli), requested_model);
        let store = open_invocation_store(self)?;
        store.insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: job_run_id.to_string(),
            activity_id: execution.activity.id.clone(),
            agent: agent.unwrap_or_else(|| normalize_agent_name(&execution.agent_cli)),
            model,
            slot: role_slot_from_input(&execution.input),
            task_ids: associated_task_ids(&execution.input),
            trace: trace.clone(),
        })?;

        if let Err(error) =
            token_scoreboard::write_token_scoreboard(&self.paths().scoreboard_dir, &store)
        {
            tracing::warn!(
                target: "orbit.core.scoreboard",
                error = %error,
                "failed to refresh tokens scoreboard",
            );
        }

        Ok(())
    }
}

fn role_slot_from_input(input: &Value) -> Option<RoleSlot> {
    input
        .get("planning_duel_slot")
        .or_else(|| input.get("role_slot"))
        .or_else(|| input.get("slot"))
        .and_then(Value::as_str)
        .and_then(|value| value.parse().ok())
}

fn is_planning_duel_agent_activity(activity_id: &str) -> bool {
    matches!(activity_id, "propose_duel_plan" | "arbitrate_duel_plan")
}

fn retired_invoke_activity_error(activity_id: &str) -> OrbitError {
    OrbitError::Execution(format!(
        "v1 invoke_activity is retired; activity '{activity_id}' cannot be dispatched via RuntimeHost"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use orbit_common::types::{ExecutorDef, ExecutorType};
    use serde_json::json;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn planning_duel_invoke_activity_uses_direct_agent_bridge() {
        let seed_runtime = OrbitRuntime::in_memory().expect("build runtime");
        let root = seed_runtime.data_root();
        let fake_agent = root.join("fake-agent.sh");
        std::fs::write(
            &fake_agent,
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{\"ok\":true},\"error\":null}'\n",
        )
        .expect("write fake agent");
        #[cfg(unix)]
        {
            let mut permissions = std::fs::metadata(&fake_agent)
                .expect("fake agent metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&fake_agent, permissions).expect("chmod fake agent");
        }

        let now = Utc::now();
        seed_runtime
            .upsert_executor_def(&ExecutorDef {
                name: "codex".to_string(),
                executor_type: ExecutorType::DirectAgent,
                command: Some(fake_agent.display().to_string()),
                args: Vec::new(),
                stdout_format: None,
                model_pair_override: None,
                model_flag: None,
                timeout_seconds: None,
                env: HashMap::new(),
                sandbox: None,
                allow_fallback: false,
                created_at: now,
                updated_at: now,
            })
            .expect("seed fake direct-agent executor");
        let runtime = OrbitRuntime::from_roots(&root, &root).expect("reload runtime");

        let result = runtime
            .invoke_activity(
                planning_duel_activity("propose_duel_plan"),
                "codex",
                Some("test-model"),
                json!({}),
                5,
                false,
            )
            .expect("planning duel activity should invoke through bridge");

        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.response_json, None);
    }

    #[test]
    fn invoke_activity_still_rejects_non_planning_duel_v1_activities() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let err = runtime
            .invoke_activity(
                planning_duel_activity("legacy_activity"),
                "codex",
                None,
                json!({}),
                5,
                false,
            )
            .expect_err("unrelated v1 activity should remain retired");

        assert!(
            err.to_string().contains("v1 invoke_activity is retired"),
            "unexpected error: {err}"
        );
    }

    fn planning_duel_activity(id: &str) -> Activity {
        let now = Utc::now();
        Activity {
            id: id.to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "test planning duel activity".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            spec_config: json!({
                "instruction": "Return a success envelope."
            }),
            tools: Vec::new(),
            proc_allowed_programs: Vec::new(),
            executor: None,
            workspace_path: None,
            created_by: Some("test".to_string()),
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }
}
