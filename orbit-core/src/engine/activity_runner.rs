use orbit_types::{Activity, JobRunState, OrbitError};
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::command::activity::activity_skill_refs_from_spec_config;
use crate::engine::{
    ACTIVITY_EXECUTION_FAILED, AttemptOutcome, DirectActivityRunOutcome, ExecutionContext,
    input_workspace_path, redact_attempt_outcome,
};
use crate::executor::{agent, api, automation, cli_command};
use crate::json_schema::validate_instance_against_schema;
use crate::template::TemplateContext;

impl OrbitRuntime {
    pub(crate) fn run_activity_direct(
        &self,
        activity: &Activity,
        agent_cli: &str,
        timeout_seconds: u64,
    ) -> Result<DirectActivityRunOutcome, OrbitError> {
        let execution = ExecutionContext {
            activity: activity.clone(),
            job: None,
            agent_cli: agent_cli.to_string(),
            timeout_seconds,
            env_extra: vec![],
            input: json!({}),
        };
        let outcome = self.execute_single_attempt(&execution);
        Ok(DirectActivityRunOutcome {
            state: outcome.state,
            duration_ms: outcome.duration_ms,
            error_code: outcome.error_code,
            error_message: outcome.error_message,
            protocol_violation: outcome.protocol_violation,
        })
    }

    pub(crate) fn build_execution_context_for_step(
        &self,
        job: &orbit_types::Job,
        step: &orbit_types::JobStep,
        input: Value,
    ) -> Result<ExecutionContext, OrbitError> {
        let activity = self.show_activity(&step.target_id)?;
        self.validate_activity_input_schema(&activity, &input)?;
        Ok(ExecutionContext {
            activity,
            job: Some(job.clone()),
            agent_cli: step.agent_cli.clone(),
            timeout_seconds: step.timeout_seconds,
            env_extra: step.env_extra.clone(),
            input,
        })
    }

    pub(crate) fn execute_single_attempt(&self, execution: &ExecutionContext) -> AttemptOutcome {
        let outcome = match execution.activity.spec_type.as_str() {
            "agent_invoke" => agent::execute(self, execution),
            "cli_command" => self.execute_cli_command_attempt(execution),
            "api" => self.execute_api_attempt(execution),
            "automation" => self.execute_automation_attempt(execution),
            other => AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: Some(1),
                duration_ms: None,
                response_json: None,
                error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                error_message: Some(format!("unsupported activity spec_type '{other}'")),
                protocol_violation: false,
            },
        };
        redact_attempt_outcome(outcome)
    }

    fn execute_cli_command_attempt(&self, execution: &ExecutionContext) -> AttemptOutcome {
        let template_context = self.execution_template_context(execution);
        match cli_command::execute(
            &execution.activity.spec_config,
            &template_context,
            execution.timeout_seconds,
        ) {
            Ok((result, duration_ms, exit_code)) => {
                if let Err(err) = self.validate_activity_output_schema(&execution.activity, &result)
                {
                    return AttemptOutcome {
                        state: JobRunState::Failed,
                        exit_code,
                        duration_ms: Some(duration_ms),
                        response_json: Some(result),
                        error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                        error_message: Some(err.to_string()),
                        protocol_violation: false,
                    };
                }
                AttemptOutcome {
                    state: JobRunState::Success,
                    exit_code,
                    duration_ms: Some(duration_ms),
                    response_json: Some(result),
                    error_code: None,
                    error_message: None,
                    protocol_violation: false,
                }
            }
            Err(err) => AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: Some(1),
                duration_ms: None,
                response_json: None,
                error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                error_message: Some(err.to_string()),
                protocol_violation: false,
            },
        }
    }

    fn execute_api_attempt(&self, execution: &ExecutionContext) -> AttemptOutcome {
        let template_context = self.execution_template_context(execution);
        match api::execute(
            &execution.activity.spec_config,
            &template_context,
            execution.timeout_seconds,
        ) {
            Ok(result) => {
                if let Err(err) = self.validate_activity_output_schema(&execution.activity, &result)
                {
                    return AttemptOutcome {
                        state: JobRunState::Failed,
                        exit_code: Some(0),
                        duration_ms: None,
                        response_json: Some(result),
                        error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                        error_message: Some(err.to_string()),
                        protocol_violation: false,
                    };
                }
                AttemptOutcome {
                    state: JobRunState::Success,
                    exit_code: Some(0),
                    duration_ms: None,
                    response_json: Some(result),
                    error_code: None,
                    error_message: None,
                    protocol_violation: false,
                }
            }
            Err(err) => AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: Some(1),
                duration_ms: None,
                response_json: None,
                error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                error_message: Some(err.to_string()),
                protocol_violation: false,
            },
        }
    }

    fn execute_automation_attempt(&self, execution: &ExecutionContext) -> AttemptOutcome {
        match automation::execute(self, &execution.activity, &execution.input) {
            Ok(result) => {
                if let Err(err) = self.validate_activity_output_schema(&execution.activity, &result)
                {
                    return AttemptOutcome {
                        state: JobRunState::Failed,
                        exit_code: Some(0),
                        duration_ms: None,
                        response_json: Some(result),
                        error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                        error_message: Some(err.to_string()),
                        protocol_violation: false,
                    };
                }
                AttemptOutcome {
                    state: JobRunState::Success,
                    exit_code: Some(0),
                    duration_ms: None,
                    response_json: Some(result),
                    error_code: None,
                    error_message: None,
                    protocol_violation: false,
                }
            }
            Err(err) => AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: Some(1),
                duration_ms: None,
                response_json: None,
                error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                error_message: Some(err.to_string()),
                protocol_violation: false,
            },
        }
    }

    pub(crate) fn execution_template_context(
        &self,
        execution: &ExecutionContext,
    ) -> TemplateContext {
        let mut env = std::env::vars().collect::<std::collections::HashMap<_, _>>();
        env.insert("ORBIT_TASK_ACTOR_KIND".to_string(), "agent".to_string());
        if let Some(identity_id) = execution.activity.identity_id.as_ref() {
            env.insert(
                "ORBIT_TASK_ACTOR_IDENTITY_ID".to_string(),
                identity_id.clone(),
            );
        }

        TemplateContext {
            input: execution.input.clone(),
            env,
            workspace_path: execution
                .activity
                .workspace_path
                .clone()
                .or_else(|| input_workspace_path(&execution.input)),
        }
    }

    pub(crate) fn validate_activity_input_schema(
        &self,
        activity: &Activity,
        input: &Value,
    ) -> Result<(), OrbitError> {
        let context = format!(
            "job run input does not match activity '{}' input schema",
            activity.id
        );
        match validate_instance_against_schema(&activity.input_schema_json, input, &context) {
            Ok(()) => Ok(()),
            Err(OrbitError::AgentProtocolViolation(message)) => {
                Err(OrbitError::InvalidInput(message))
            }
            Err(other) => Err(other),
        }
    }

    pub(crate) fn validate_activity_output_schema(
        &self,
        activity: &Activity,
        output: &Value,
    ) -> Result<(), OrbitError> {
        let context = format!(
            "activity '{}' output does not match output schema",
            activity.id
        );
        validate_instance_against_schema(&activity.output_schema_json, output, &context)
    }

    pub(crate) fn validate_activity_target_exists(
        &self,
        target_type: orbit_types::JobTargetType,
        target_id: &str,
    ) -> Result<Activity, OrbitError> {
        let _ = target_type;
        let activity = self.show_activity(target_id)?;
        let skill_refs = activity_skill_refs_from_spec_config(&activity.spec_config)?;
        let _ = self.resolve_activity_skill_refs(&skill_refs)?;
        Ok(activity)
    }
}
