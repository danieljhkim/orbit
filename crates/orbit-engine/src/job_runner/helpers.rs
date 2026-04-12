use std::fs;
use std::path::{Path, PathBuf};

use orbit_agent::Agent;
use orbit_types::{
    InvocationTrace, JobRunState, JobStep, KnowledgeRunMetrics, OrbitError, StepCondition,
};
use serde_json::Value;
use tiktoken_rs::cl100k_base;
use tracing::info;

use crate::context::{
    EngineHost, ExecutionContext, JobRunHost, TaskAutomationUpdate,
    execution_working_directory_with_task,
};

pub(super) fn extract_task_id(input: &Value) -> Option<&str> {
    input
        .as_object()
        .and_then(|map| map.get("task_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn normalize_agent_label(agent_cli: &str) -> String {
    std::path::Path::new(agent_cli)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(agent_cli)
        .to_ascii_lowercase()
}

pub(super) fn json_value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub(super) fn merge_job_input(
    default_input: Option<&Value>,
    input: Value,
) -> Result<Value, OrbitError> {
    let mut merged = match default_input {
        None => serde_json::Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(other) => {
            return Err(OrbitError::InvalidInput(format!(
                "job default_input must be an object, got {}",
                json_value_type_name(other)
            )));
        }
    };

    let input_map = match input {
        Value::Object(map) => map,
        other => {
            return Err(OrbitError::InvalidInput(format!(
                "job run input must be an object, got {}",
                json_value_type_name(&other)
            )));
        }
    };

    for (key, value) in input_map {
        merged.insert(key, value);
    }

    Ok(Value::Object(merged))
}

pub(super) fn should_run_step(
    condition: StepCondition,
    previous_step_state: Option<JobRunState>,
) -> bool {
    match condition {
        StepCondition::Always => true,
        StepCondition::OnSuccess => {
            previous_step_state.is_none_or(|state| matches!(state, JobRunState::Success))
        }
        StepCondition::OnFailure => previous_step_state.is_some_and(step_state_records_failure),
        StepCondition::OnTimeout => {
            previous_step_state.is_some_and(|state| matches!(state, JobRunState::Timeout))
        }
    }
}

pub(super) fn step_state_records_failure(state: JobRunState) -> bool {
    matches!(
        state,
        JobRunState::Failed | JobRunState::Timeout | JobRunState::Cancelled
    )
}

pub(super) fn step_state_records_incident(state: JobRunState) -> bool {
    matches!(state, JobRunState::Failed | JobRunState::Timeout)
}

pub(super) fn run_was_cancelled<H: JobRunHost>(host: &H, run_id: &str) -> Result<bool, OrbitError> {
    Ok(host
        .get_job_run(run_id)?
        .is_some_and(|run| run.state == JobRunState::Cancelled))
}

/// Returns `true` if the accumulated input contains `"loop_exit": true`.
pub(super) fn check_loop_exit<H: crate::context::TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> bool {
    // Primary: check for explicit loop_exit signal in piped input.
    let explicit = input
        .as_object()
        .and_then(|map| map.get("loop_exit"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if explicit {
        return true;
    }

    // Fallback: if the agent persisted pr_status to the task but crashed before
    // returning structured output (with loop_exit), check the task directly.
    if let Some(task_id) = extract_task_id(input)
        && let Ok(task) = host.get_task(task_id)
        && let Some(ref pr_status) = task.pr_status
    {
        let normalized = crate::executor::automation::review::normalize_review_decision(pr_status);
        if normalized == "APPROVED" {
            return true;
        }
    }

    false
}

pub(super) fn log_step_completion(
    step_index: usize,
    iteration: u32,
    step: &JobStep,
    state: JobRunState,
    duration_ms: Option<u64>,
    error_code: Option<&str>,
    error_message: Option<&str>,
) {
    if step_state_records_incident(state) {
        info!(
            step_index,
            iteration,
            target_id = %step.target_id,
            target_type = %step.target_type,
            state = %state,
            duration_ms = ?duration_ms,
            error_code = error_code.unwrap_or(""),
            error_message = error_message.unwrap_or(""),
            "step failed"
        );
    } else {
        info!(
            step_index,
            iteration,
            target_id = %step.target_id,
            target_type = %step.target_type,
            state = %state,
            duration_ms = ?duration_ms,
            "step completed"
        );
    }
}

/// Resolve a step's effective `agent_cli` / `model` at execution time.
///
/// Precedence (highest first):
///
/// 1. An explicit, non-empty `step.agent_cli` — always wins.
/// 2. `step.agent_cli_from_input`: when set, the named key is looked up in the
///    current job input and used as the agent CLI. `model_from_input` is
///    consulted the same way to fill a missing `step.model`. This is the
///    general-purpose hook that lets workflows like `duel` randomize agent
///    assignment per run without patching the step at load time.
/// 3. The task's actor identity (original implementer). Used by review-loop
///    steps that should route fixes back to the agent that wrote the code.
///
/// Returns `None` if no override is needed (the caller should use `step` as-is).
pub(super) fn resolve_step_agent<H: crate::context::TaskHost + ?Sized>(
    host: &H,
    step: &JobStep,
    input: &Value,
) -> Option<JobStep> {
    if !step.agent_cli.trim().is_empty() {
        return None;
    }
    if let Some(resolved) = resolve_step_agent_from_input(step, input) {
        return Some(resolved);
    }
    resolve_step_agent_from_task(host, step, input)
}

/// Populate `step.agent_cli` / `step.model` from named keys in the current
/// job input, when the step declares `agent_cli_from_input` / `model_from_input`.
///
/// Returns `None` when the step has no `agent_cli_from_input` hook, when the
/// declared key is missing or not a string, or when the value is an empty
/// string (so the next precedence layer can try).
pub(super) fn resolve_step_agent_from_input(step: &JobStep, input: &Value) -> Option<JobStep> {
    if !step.agent_cli.trim().is_empty() {
        return None;
    }
    let key = step.agent_cli_from_input.as_deref()?;
    let map = input.as_object()?;
    let agent_cli = map
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    let mut resolved = step.clone();
    resolved.agent_cli = agent_cli.to_string();

    if resolved.model.is_none()
        && let Some(model_key) = step.model_from_input.as_deref()
        && let Some(model) = map
            .get(model_key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    {
        resolved.model = Some(model.to_string());
    }

    Some(resolved)
}

/// When a step's `agent_cli` is empty, try to resolve it from the task's
/// `agent` and `model` fields so the original implementer handles the step
/// (e.g. in a review-loop where the fix should go back to the same agent).
pub(super) fn resolve_step_agent_from_task<H: crate::context::TaskHost + ?Sized>(
    host: &H,
    step: &JobStep,
    input: &Value,
) -> Option<JobStep> {
    if !step.agent_cli.trim().is_empty() {
        return None;
    }
    let task_id = extract_task_id(input)?;
    let task = host.get_task(task_id).ok()?;
    let agent = task
        .actor_identity
        .agent_name()
        .filter(|a| !a.trim().is_empty())?;
    let mut resolved = step.clone();
    resolved.agent_cli = agent.to_string();
    if resolved.model.is_none() {
        resolved.model = task.actor_identity.agent_model().map(ToOwned::to_owned);
    }
    Some(resolved)
}

pub(super) fn record_task_agent_context<H: EngineHost>(
    host: &H,
    execution: &ExecutionContext,
) -> Result<(), OrbitError> {
    if execution.agent_cli.trim().is_empty() {
        return Ok(());
    }
    let Some(task_id) = extract_task_id(&execution.input) else {
        return Ok(());
    };

    host.apply_task_automation_update(
        task_id,
        TaskAutomationUpdate {
            agent: Some(normalize_agent_label(&execution.agent_cli)),
            model: resolved_model_name(host, execution),
            ..Default::default()
        },
    )
}

pub(super) fn resolved_model_name<H: EngineHost>(
    host: &H,
    execution: &ExecutionContext,
) -> Option<String> {
    let config = host
        .agent_config_for(&execution.agent_cli, execution.model.as_deref())
        .ok()?;
    let model_from_config = config.model.clone();
    let agent = Agent::new(&config).ok();
    let resolved = agent
        .and_then(|agent| agent.model_name().map(ToOwned::to_owned))
        .or(model_from_config);
    host.canonical_model_name(&execution.agent_cli, resolved.as_deref())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedImplementChangeMetrics {
    raw_read_token_baseline: u64,
}

pub(super) fn prepare_implement_change_metrics<H: crate::context::TaskHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
) -> Result<Option<PreparedImplementChangeMetrics>, OrbitError> {
    if execution.activity.id != "implement_change" {
        return Ok(None);
    }

    let Some(task_id) = extract_task_id(&execution.input) else {
        return Ok(None);
    };
    let task = host.get_task(task_id)?;
    let workspace_root = execution_working_directory_with_task(host, execution)
        .map(PathBuf::from)
        .ok_or_else(|| {
            OrbitError::Execution(
                "implement_change metrics require an effective workspace_path".to_string(),
            )
        })?;

    let mut raw_read_token_baseline = 0_u64;
    for context_file in task.context_files {
        let path = resolve_context_path(&workspace_root, &context_file);
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        raw_read_token_baseline = raw_read_token_baseline.saturating_add(tokenize_text(&content)?);
    }

    Ok(Some(PreparedImplementChangeMetrics {
        raw_read_token_baseline,
    }))
}

pub(super) fn build_knowledge_run_metrics(
    prepared: &PreparedImplementChangeMetrics,
    trace: &InvocationTrace,
) -> Result<KnowledgeRunMetrics, OrbitError> {
    let actual_fs_read_tokens_during_run = trace
        .tool_calls
        .iter()
        .filter(|call| call.tool_name == "fs.read")
        .try_fold(0_u64, |acc, call| {
            let payload_tokens = call
                .result_payload
                .as_ref()
                .map(tokenize_json_value)
                .transpose()?
                .unwrap_or(0);
            Ok::<u64, OrbitError>(acc.saturating_add(payload_tokens))
        })?;

    let pack_payload = trace
        .tool_calls
        .iter()
        .find(|call| call.tool_name == "orbit.graph.pack")
        .and_then(|call| call.result_payload.as_ref());

    let knowledge_unavailable = pack_payload
        .and_then(|payload| payload.get("kind"))
        .and_then(Value::as_str)
        == Some("knowledge_unavailable");

    let knowledge_pack_used = pack_payload.is_some() && !knowledge_unavailable;
    let knowledge_pack_tokens = if knowledge_pack_used {
        pack_payload.map(tokenize_json_value).transpose()?
    } else {
        None
    };
    let knowledge_pack_unresolved_count = if knowledge_pack_used {
        pack_payload
            .and_then(|payload| payload.get("unresolved_selectors"))
            .and_then(Value::as_array)
            .map(|items| items.len() as u32)
            .unwrap_or(0)
    } else {
        0
    };

    Ok(KnowledgeRunMetrics {
        raw_read_token_baseline: prepared.raw_read_token_baseline,
        knowledge_pack_tokens,
        compression_ratio: knowledge_pack_tokens
            .and_then(|tokens| safe_ratio(prepared.raw_read_token_baseline, tokens)),
        actual_fs_read_tokens_during_run,
        double_read_rate: safe_ratio(
            actual_fs_read_tokens_during_run,
            prepared.raw_read_token_baseline,
        ),
        knowledge_pack_used,
        knowledge_pack_unresolved_count,
        total_llm_input_tokens: trace
            .usage
            .input
            .saturating_add(trace.usage.cache_read)
            .saturating_add(trace.usage.cache_create),
    })
}

pub(super) fn tokenize_text(text: &str) -> Result<u64, OrbitError> {
    let encoder = cl100k_base()
        .map_err(|error| OrbitError::Execution(format!("load cl100k_base: {error}")))?;
    Ok(encoder.encode_with_special_tokens(text).len() as u64)
}

fn tokenize_json_value(value: &Value) -> Result<u64, OrbitError> {
    let serialized = serde_json::to_string(value)
        .map_err(|error| OrbitError::Execution(format!("serialize tool result: {error}")))?;
    tokenize_text(&serialized)
}

fn safe_ratio(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator != 0).then(|| numerator as f64 / denominator as f64)
}

fn resolve_context_path(workspace_root: &Path, context_file: &str) -> PathBuf {
    let path = Path::new(context_file);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

#[cfg(test)]
mod condition_tests {
    use orbit_types::{JobRunState, StepCondition};

    use super::should_run_step;

    #[test]
    fn on_success_steps_do_not_run_after_failed_merge_steps() {
        assert!(!should_run_step(
            StepCondition::OnSuccess,
            Some(JobRunState::Failed),
        ));
        assert!(!should_run_step(
            StepCondition::OnSuccess,
            Some(JobRunState::Timeout),
        ));
        assert!(!should_run_step(
            StepCondition::OnSuccess,
            Some(JobRunState::Cancelled),
        ));
        assert!(should_run_step(
            StepCondition::OnSuccess,
            Some(JobRunState::Success),
        ));
    }
}

#[cfg(test)]
mod resolve_step_agent_tests {
    //! Covers the three-layer precedence chain for step agent resolution:
    //!
    //! 1. Explicit `step.agent_cli` wins over every other source.
    //! 2. `step.agent_cli_from_input` reads from current job input.
    //! 3. Task actor identity fallback when neither of the above applies.

    use super::*;
    use chrono::Utc;
    use orbit_types::{
        ActorIdentity, JobStep, JobTargetType, OrbitError, Task, TaskPriority, TaskStatus, TaskType,
    };
    use serde_json::json;

    use crate::context::TaskHost;

    /// Minimal `TaskHost` used to drive the task-actor fallback case.
    struct StubTaskHost {
        task: Task,
    }

    impl TaskHost for StubTaskHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            if self.task.id == task_id {
                Ok(self.task.clone())
            } else {
                Err(OrbitError::TaskNotFound(task_id.to_string()))
            }
        }

        fn get_task_artifacts(
            &self,
            _task_id: &str,
        ) -> Result<Vec<orbit_types::TaskArtifact>, OrbitError> {
            Ok(Vec::new())
        }

        fn list_tasks_filtered(
            &self,
            _status: Option<TaskStatus>,
            _priority: Option<TaskPriority>,
            _parent_id: Option<&str>,
            _batch_id: Option<&str>,
        ) -> Result<Vec<Task>, OrbitError> {
            unimplemented!("not used in resolver tests")
        }

        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not used in resolver tests")
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not used in resolver tests")
        }

        fn apply_task_automation_update(
            &self,
            _task_id: &str,
            _update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            Ok(())
        }
    }

    fn sample_task(id: &str, actor: ActorIdentity) -> Task {
        let now = Utc::now();
        Task {
            id: id.to_string(),
            parent_id: None,
            title: "t".into(),
            description: "d".into(),
            acceptance_criteria: vec![],
            plan: String::new(),
            execution_summary: String::new(),
            context_files: vec![],
            workspace_path: None,
            repo_root: None,
            assigned_to: None,
            created_by: None,
            actor_identity: actor,
            status: TaskStatus::InProgress,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Feature,
            pr_number: None,
            pr_status: None,
            proposed_by: None,
            source_task_id: None,
            batch_id: None,
            comments: vec![],
            history: vec![],
            review_threads: vec![],
            created_at: now,
            updated_at: now,
        }
    }

    fn step_with(agent_cli: &str, from_input: Option<&str>) -> JobStep {
        JobStep {
            target_type: JobTargetType::Activity,
            target_id: "some_activity".into(),
            agent_cli: agent_cli.to_string(),
            agent_cli_from_input: from_input.map(str::to_string),
            model_from_input: from_input.map(|_| "duel_model".to_string()),
            timeout_seconds: 60,
            ..JobStep::default()
        }
    }

    #[test]
    fn explicit_step_agent_cli_wins_over_every_other_source() {
        // Step has an explicit agent_cli. Even though `agent_cli_from_input`
        // is set AND the task actor identity exists, neither should be
        // consulted — the explicit value must pass straight through.
        let host = StubTaskHost {
            task: sample_task("T1", ActorIdentity::agent("should_not_be_used", "m")),
        };
        let step = step_with("claude", Some("reviewer_agent_cli"));
        let input = json!({
            "task_id": "T1",
            "reviewer_agent_cli": "codex",
            "duel_model": "gpt-5.4",
        });

        let resolved = resolve_step_agent(&host, &step, &input);
        // None = caller uses the original step as-is; agent_cli stays "claude".
        assert!(
            resolved.is_none(),
            "explicit agent_cli must short-circuit resolution"
        );
    }

    #[test]
    fn empty_agent_cli_with_from_input_reads_from_current_input() {
        let host = StubTaskHost {
            task: sample_task(
                "T1",
                // Task actor is set but must be IGNORED — from_input has
                // higher precedence than the task-actor fallback.
                ActorIdentity::agent("wrong_agent", "wrong_model"),
            ),
        };
        let step = step_with("", Some("reviewer_agent_cli"));
        let input = json!({
            "task_id": "T1",
            "reviewer_agent_cli": "gemini",
            "duel_model": "gemini-3.1-pro-preview",
        });

        let resolved = resolve_step_agent(&host, &step, &input).expect("resolver should fire");
        assert_eq!(resolved.agent_cli, "gemini");
        assert_eq!(resolved.model.as_deref(), Some("gemini-3.1-pro-preview"));
    }

    #[test]
    fn empty_agent_cli_without_from_input_falls_back_to_task_actor() {
        let host = StubTaskHost {
            task: sample_task("T1", ActorIdentity::agent("claude", "opus")),
        };
        // No `agent_cli_from_input` hook on this step → resolver must
        // fall through to the task-actor fallback path.
        let step = step_with("", None);
        let input = json!({ "task_id": "T1" });

        let resolved = resolve_step_agent(&host, &step, &input).expect("resolver should fire");
        assert_eq!(resolved.agent_cli, "claude");
        assert_eq!(resolved.model.as_deref(), Some("opus"));
    }

    #[test]
    fn from_input_missing_key_falls_back_to_task_actor() {
        // Edge case: the step declares `agent_cli_from_input` but the key
        // is absent from current_input. Resolver must not stall — it should
        // fall through to the next precedence layer (task actor).
        let host = StubTaskHost {
            task: sample_task("T1", ActorIdentity::agent("claude", "opus")),
        };
        let step = step_with("", Some("reviewer_agent_cli"));
        let input = json!({ "task_id": "T1" }); // no reviewer_agent_cli key

        let resolved = resolve_step_agent(&host, &step, &input).expect("resolver should fire");
        assert_eq!(resolved.agent_cli, "claude");
    }

    #[test]
    fn from_input_empty_string_value_falls_back_to_task_actor() {
        // Another edge case: key present but value is an empty string.
        // Treat as "not set" and fall through to task actor.
        let host = StubTaskHost {
            task: sample_task("T1", ActorIdentity::agent("claude", "opus")),
        };
        let step = step_with("", Some("reviewer_agent_cli"));
        let input = json!({ "task_id": "T1", "reviewer_agent_cli": "" });

        let resolved = resolve_step_agent(&host, &step, &input).expect("resolver should fire");
        assert_eq!(resolved.agent_cli, "claude");
    }
}

#[cfg(test)]
mod knowledge_metrics_tests {
    use chrono::Utc;
    use orbit_types::{ActorIdentity, Task, TaskPriority, TaskStatus, TaskType, ToolCallTrace};
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;
    use crate::context::{TaskAutomationUpdate, TaskHost};

    struct MetricsTaskHost {
        task: Task,
    }

    impl TaskHost for MetricsTaskHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            if self.task.id == task_id {
                Ok(self.task.clone())
            } else {
                Err(OrbitError::TaskNotFound(task_id.to_string()))
            }
        }

        fn get_task_artifacts(
            &self,
            _task_id: &str,
        ) -> Result<Vec<orbit_types::TaskArtifact>, OrbitError> {
            Ok(Vec::new())
        }

        fn list_tasks_filtered(
            &self,
            _status: Option<TaskStatus>,
            _priority: Option<TaskPriority>,
            _parent_id: Option<&str>,
            _batch_id: Option<&str>,
        ) -> Result<Vec<Task>, OrbitError> {
            unimplemented!("not used in metrics tests")
        }

        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not used in metrics tests")
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not used in metrics tests")
        }

        fn apply_task_automation_update(
            &self,
            _task_id: &str,
            _update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            unimplemented!("not used in metrics tests")
        }
    }

    fn task(workspace_path: &str) -> Task {
        Task {
            id: "T1".to_string(),
            parent_id: None,
            title: "measure implement_change".to_string(),
            description: String::new(),
            acceptance_criteria: vec![],
            plan: "1. do work".to_string(),
            execution_summary: String::new(),
            context_files: vec!["src/lib.rs".to_string(), "README.md".to_string()],
            workspace_path: Some(workspace_path.to_string()),
            repo_root: None,
            assigned_to: None,
            created_by: None,
            actor_identity: ActorIdentity::default(),
            status: TaskStatus::InProgress,
            priority: TaskPriority::High,
            complexity: None,
            task_type: TaskType::Feature,
            pr_number: None,
            pr_status: None,
            proposed_by: None,
            source_task_id: None,
            batch_id: None,
            comments: vec![],
            history: vec![],
            review_threads: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn execution() -> ExecutionContext {
        ExecutionContext {
            activity: orbit_types::Activity {
                id: "implement_change".to_string(),
                spec_type: "agent_invoke".to_string(),
                description: String::new(),
                input_schema_json: json!({}),
                output_schema_json: json!({}),
                spec_config: json!({}),
                tools: vec![],
                proc_allowed_programs: vec![],
                workspace_path: None,
                created_by: None,
                is_active: true,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            job: None,
            agent_cli: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
            timeout_seconds: 60,
            env_extra: vec![],
            env_set: Default::default(),
            input: json!({"task_id": "T1"}),
            debug: false,
        }
    }

    #[test]
    fn tokenize_text_uses_cl100k_base() {
        assert_eq!(tokenize_text("hello world").expect("tokens"), 2);
    }

    #[test]
    fn prepare_implement_change_metrics_reads_context_file_baseline() {
        let dir = tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).expect("src dir");
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").expect("lib");
        std::fs::write(dir.path().join("README.md"), "workspace readme\n").expect("readme");

        let host = MetricsTaskHost {
            task: task(&dir.path().to_string_lossy()),
        };

        let prepared =
            prepare_implement_change_metrics(&host, &execution()).expect("prepared metrics");

        assert!(prepared.is_some());
        assert!(prepared.expect("metrics").raw_read_token_baseline > 0);
    }

    #[test]
    fn build_knowledge_run_metrics_records_pack_success_path() {
        let prepared = PreparedImplementChangeMetrics {
            raw_read_token_baseline: 120,
        };
        let trace = InvocationTrace {
            usage: orbit_types::TokenUsage {
                input: 900,
                cache_read: 30,
                cache_create: 20,
                ..Default::default()
            },
            tool_calls: vec![
                ToolCallTrace {
                    seq: 1,
                    tool_name: "orbit.graph.pack".to_string(),
                    result_bytes: 0,
                    result_payload: Some(json!({
                        "entries": [{"selector": "file:src/lib.rs"}],
                        "unresolved_selectors": ["file:README.md"]
                    })),
                },
                ToolCallTrace {
                    seq: 2,
                    tool_name: "fs.read".to_string(),
                    result_bytes: 0,
                    result_payload: Some(json!({"path": "README.md", "content": "hello world"})),
                },
            ],
            duration_ms: 0,
        };

        let metrics = build_knowledge_run_metrics(&prepared, &trace).expect("metrics");

        assert_eq!(metrics.raw_read_token_baseline, 120);
        assert!(metrics.knowledge_pack_used);
        assert_eq!(metrics.knowledge_pack_unresolved_count, 1);
        assert_eq!(metrics.total_llm_input_tokens, 950);
        assert!(metrics.knowledge_pack_tokens.is_some());
        assert!(metrics.compression_ratio.is_some());
        assert!(metrics.double_read_rate.is_some());
        assert!(metrics.actual_fs_read_tokens_during_run > 0);
    }

    #[test]
    fn build_knowledge_run_metrics_records_fallback_path() {
        let prepared = PreparedImplementChangeMetrics {
            raw_read_token_baseline: 120,
        };
        let trace = InvocationTrace {
            usage: orbit_types::TokenUsage {
                input: 700,
                ..Default::default()
            },
            tool_calls: vec![
                ToolCallTrace {
                    seq: 1,
                    tool_name: "orbit.graph.pack".to_string(),
                    result_bytes: 0,
                    result_payload: Some(json!({
                        "kind": "knowledge_unavailable",
                        "reason": "missing manifest"
                    })),
                },
                ToolCallTrace {
                    seq: 2,
                    tool_name: "fs.read".to_string(),
                    result_bytes: 0,
                    result_payload: Some(json!({"path": "src/lib.rs", "content": "fallback"})),
                },
            ],
            duration_ms: 0,
        };

        let metrics = build_knowledge_run_metrics(&prepared, &trace).expect("metrics");

        assert!(!metrics.knowledge_pack_used);
        assert!(metrics.knowledge_pack_tokens.is_none());
        assert!(metrics.compression_ratio.is_none());
        assert!(metrics.double_read_rate.is_some());
        assert_eq!(metrics.knowledge_pack_unresolved_count, 0);
        assert_eq!(metrics.total_llm_input_tokens, 700);
        assert!(metrics.actual_fs_read_tokens_during_run > 0);
    }
}
