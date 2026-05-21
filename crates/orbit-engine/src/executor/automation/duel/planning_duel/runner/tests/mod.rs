#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use orbit_common::types::{
    Activity, InvocationTrace, Job, JobTargetType, NotFoundKind, OrbitError, OrbitEvent,
    PlanningRoleAssignment, Role, Task, TaskArtifact, TaskComment, TaskPriority, TaskStatus,
    TaskType,
};
use orbit_store::{InvocationQuery, InvocationRecord};
use orbit_tools::ToolContext;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::context::{
    ActivityInvocationResult, JobRunResult, RuntimeHost, TaskActivityUpdate, TaskAutomationUpdate,
    TaskReadHost, TaskWriteHost,
};
use crate::executor::registry::ActivityExecutorRegistry;

use crate::executor::automation::duel::planning_duel::artifacts;
use super::super::run_planning_duel;

struct PlanningDuelHost {
    task: Mutex<Task>,
    comments: Mutex<Vec<TaskComment>>,
    artifacts: Mutex<Vec<TaskArtifact>>,
    data_root: PathBuf,
    scoreboard_dir: PathBuf,
    _tempdir: TempDir,
    workflow_admissions: AtomicUsize,
    task_starts: AtomicUsize,
    last_automation_update: Mutex<Option<TaskAutomationUpdate>>,
    omit_planner_artifacts: AtomicUsize,
}

impl PlanningDuelHost {
    fn new(status: TaskStatus) -> Self {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let data_root = tempdir.path().join(".orbit");
        let scoreboard_dir = data_root.join("state").join("scoreboard");
        std::fs::create_dir_all(&scoreboard_dir).expect("scoreboard dir");

        Self {
            task: Mutex::new(task_with_status(status)),
            comments: Mutex::new(Vec::new()),
            artifacts: Mutex::new(Vec::new()),
            data_root,
            scoreboard_dir,
            _tempdir: tempdir,
            workflow_admissions: AtomicUsize::new(0),
            task_starts: AtomicUsize::new(0),
            last_automation_update: Mutex::new(None),
            omit_planner_artifacts: AtomicUsize::new(0),
        }
    }

    fn task_status(&self) -> TaskStatus {
        self.task.lock().expect("task lock").status
    }

    fn last_context_files_update(&self) -> Option<Option<Vec<String>>> {
        self.last_automation_update
            .lock()
            .expect("update lock")
            .as_ref()
            .map(|update| update.context_files.clone())
    }

    fn admission_count(&self) -> usize {
        self.workflow_admissions.load(Ordering::SeqCst)
    }

    fn start_count(&self) -> usize {
        self.task_starts.load(Ordering::SeqCst)
    }

    fn omit_planner_artifacts(&self) {
        self.omit_planner_artifacts.store(1, Ordering::SeqCst);
    }
}

impl TaskReadHost for PlanningDuelHost {
    fn get_task(&self, task_id: &str) -> Result<Task, orbit_common::types::OrbitError> {
        let task = self.task.lock().expect("task lock").clone();
        if task.id == task_id {
            Ok(task)
        } else {
            Err(OrbitError::not_found(
                NotFoundKind::Task,
                task_id.to_string(),
            ))
        }
    }

    fn get_task_artifacts(
        &self,
        _task_id: &str,
    ) -> Result<Vec<TaskArtifact>, orbit_common::types::OrbitError> {
        Ok(self.artifacts.lock().expect("artifacts lock").clone())
    }

    fn get_task_comments(
        &self,
        _task_id: &str,
    ) -> Result<Vec<TaskComment>, orbit_common::types::OrbitError> {
        Ok(self.comments.lock().expect("comments lock").clone())
    }

    fn list_tasks_filtered(
        &self,
        _status: Option<TaskStatus>,
        _priority: Option<TaskPriority>,
        _parent_id: Option<&str>,
        _batch_id: Option<&str>,
        _external_ref: Option<&orbit_common::types::ExternalRef>,
        _has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, orbit_common::types::OrbitError> {
        Ok(vec![self.task.lock().expect("task lock").clone()])
    }
}

impl TaskWriteHost for PlanningDuelHost {
    fn start_task(
        &self,
        _task_id: &str,
        _note: Option<String>,
        _comment: Option<String>,
    ) -> Result<Task, orbit_common::types::OrbitError> {
        self.task_starts.fetch_add(1, Ordering::SeqCst);
        Err(orbit_common::types::OrbitError::Execution(
            "planning duel must not start tasks".to_string(),
        ))
    }

    fn admit_task_for_workflow(
        &self,
        _task_id: &str,
        _workflow: &str,
    ) -> Result<Task, orbit_common::types::OrbitError> {
        self.workflow_admissions.fetch_add(1, Ordering::SeqCst);
        Err(orbit_common::types::OrbitError::Execution(
            "planning duel must not admit tasks for workflow execution".to_string(),
        ))
    }

    fn update_task_from_activity(
        &self,
        _task_id: &str,
        _update: TaskActivityUpdate,
    ) -> Result<Task, orbit_common::types::OrbitError> {
        Err(orbit_common::types::OrbitError::Execution(
            "planning duel must not update task status from activity".to_string(),
        ))
    }

    fn apply_task_automation_update(
        &self,
        _task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), orbit_common::types::OrbitError> {
        if update.status.is_some() {
            return Err(orbit_common::types::OrbitError::Execution(
                "planning duel writeback must not include a status update".to_string(),
            ));
        }
        *self.last_automation_update.lock().expect("update lock") = Some(update.clone());
        let mut task = self.task.lock().expect("task lock");
        if let Some(plan) = update.plan {
            task.plan = plan;
        }
        if let Some(context_files) = update.context_files {
            task.context_files = context_files;
        }
        self.comments
            .lock()
            .expect("comments lock")
            .extend(update.append_comments);
        task.updated_at = Utc::now();
        Ok(())
    }
}

impl RuntimeHost for PlanningDuelHost {
    fn record_event(&self, _event: OrbitEvent) -> Result<(), orbit_common::types::OrbitError> {
        Ok(())
    }

    fn repo_root(&self) -> Result<String, orbit_common::types::OrbitError> {
        Ok(self.data_root.to_string_lossy().to_string())
    }

    fn data_root(&self) -> &Path {
        &self.data_root
    }

    fn activity_executor_registry(&self) -> &ActivityExecutorRegistry {
        // reuse the one from context if needed, but for runner tests a default suffices
        static REG: OnceLock<ActivityExecutorRegistry> = OnceLock::new();
        REG.get_or_init(ActivityExecutorRegistry::default)
    }

    fn run_job_now_with_input_debug(
        &self,
        _job_id: &str,
        _input: Value,
        _debug: bool,
    ) -> Result<JobRunResult, OrbitError> {
        unimplemented!("not needed by runner tests")
    }

    fn validate_activity_target_exists(
        &self,
        _target_type: JobTargetType,
        _target_id: &str,
    ) -> Result<Activity, OrbitError> {
        unimplemented!("not needed by runner tests")
    }

    fn get_job(&self, _job_id: &str) -> Result<Option<Job>, OrbitError> {
        Ok(None)
    }

    fn invocation_records(
        &self,
        _query: InvocationQuery,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        Ok(Vec::new())
    }

    fn run_tool_with_context_and_role(
        &self,
        _name: &str,
        _input: Value,
        _role: Role,
        _tool_context: ToolContext,
    ) -> Result<Value, OrbitError> {
        Err(OrbitError::Execution(
            "stale artifact cleanup should not run tools in this test".to_string(),
        ))
    }

    fn invoke_activity(
        &self,
        activity: Activity,
        agent_cli: &str,
        model: Option<&str>,
        input: Value,
        _timeout_seconds: u64,
        _debug: bool,
    ) -> Result<ActivityInvocationResult, OrbitError> {
        let _model = model.unwrap_or("unknown-model");
        match activity.id.as_str() {
            "propose_duel_plan" => {
                let slot = input
                    .get("planning_duel_slot")
                    .and_then(Value::as_str)
                    .unwrap_or("planner_a");
                let should_omit = self
                    .omit_planner_artifacts
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                        remaining.checked_sub(1)
                    })
                    .is_ok();
                if !should_omit {
                    self.artifacts
                        .lock()
                        .expect("artifacts lock")
                        .push(TaskArtifact::from_text(
                            format!("planning-duel/{slot}.md"),
                            format!(
                                "*authored by: {agent_cli} / {slot}*\n## Plan\nPreserve task status.\n"
                            ),
                        ));
                }
            }
            "arbitrate_duel_plan" => {
                let _winner =
                    first_planner_assignment(&self.artifacts.lock().expect("artifacts lock"))?;
                self.artifacts
                    .lock()
                    .expect("artifacts lock")
                    .push(TaskArtifact::from_text(
                        "planning-duel/winner.json",
                        json!({
                            "winner_slot": "planner_a",
                            "arbiter_rationale": "Preserves lifecycle state."
                        })
                        .to_string(),
                    ));
            }
            other => {
                return Err(OrbitError::Execution(format!(
                    "unexpected activity '{other}'"
                )));
            }
        }

        Ok(ActivityInvocationResult {
            response_json: Some(json!({
                "provider": agent_cli,
                "stdout_blob_ref": "stdout-digest",
                "stderr_blob_ref": "stderr-digest",
                "stdout_text": "orbit.duel.plan.add failed: store_error: attempt to write a readonly database",
            })),
            invocation_trace: InvocationTrace {
                tool_calls: vec![orbit_common::types::ToolCallTrace {
                    seq: 1,
                    tool_name: "orbit.duel.plan.add".to_string(),
                    result_bytes: 91,
                    result_payload: None,
                }],
                ..InvocationTrace::default()
            },
            exit_code: Some(0),
            duration_ms: 1,
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
        true
    }

    fn graph_editing(&self) -> bool {
        false
    }

    fn scoreboard_dir(&self) -> &Path {
        &self.scoreboard_dir
    }
}

fn first_planner_assignment(
    artifacts: &[TaskArtifact],
) -> Result<PlanningRoleAssignment, OrbitError> {
    let artifact = artifacts
        .iter()
        .find(|artifact| {
            artifact.path.starts_with("planning-duel/") && artifact.path.ends_with(".md")
        })
        .ok_or_else(|| OrbitError::Execution("missing planner artifact".to_string()))?;
    let content = artifact.text_content().ok_or_else(|| {
        OrbitError::Execution("planner artifact content must be utf-8".to_string())
    })?;
    let signature = artifacts::parse_planning_duel_signature(content)?;
    Ok(PlanningRoleAssignment {
        family: signature.family,
    })
}

fn task_with_status(status: TaskStatus) -> Task {
    let now = Utc::now();
    Task {
        id: "T20260430-STATUS".to_string(),
        title: "Planning duel status preservation".to_string(),
        description: "Exercise planning duel without lifecycle admission.".to_string(),
        acceptance_criteria: Vec::new(),
        tags: Vec::new(),
        plan: String::new(),
        execution_summary: String::new(),
        context_files: Vec::new(),
        created_by: Some("test".to_string()),
        planned_by: None,
        implemented_by: None,
        status,
        priority: TaskPriority::Medium,
        complexity: None,
        task_type: TaskType::Bug,
        pr_status: None,
        external_refs: Vec::new(),
        relations: Vec::new(),
        job_run_id: None,
        crew: None,
        created_at: now,
        updated_at: now,
    }
}

fn install_planning_duel_artifacts(host: &PlanningDuelHost, plan_body: &str) {
    let mut artifacts = host.artifacts.lock().expect("artifacts lock");
    artifacts.clear();
    artifacts.push(TaskArtifact::from_text(
        "planning-duel/planner_a.md",
        "*authored by: codex / planner_a*\n## Plan\nLoser plan.\n",
    ));
    artifacts.push(TaskArtifact::from_text(
        "planning-duel/planner_b.md",
        format!("*authored by: claude / planner_b*\n{plan_body}"),
    ));
    artifacts.push(TaskArtifact::from_text(
        "planning-duel/winner.json",
        json!({
            "winner_slot": "planner_b",
            "arbiter_rationale": "Claude provided more detail."
        })
        .to_string(),
    ));
}

fn run_writeback(host: &PlanningDuelHost) -> serde_json::Value {
    artifacts::writeback_planning_duel_task(
        host,
        &json!({
            "task_id": "T20260430-STATUS",
            "planning_duel_roles": {
                "planner_a": { "family": "codex" },
                "planner_b": { "family": "claude" },
                "arbiter":   { "family": "gemini" }
            }
        }),
    )
    .expect("writeback succeeds")
}

mod run;
mod writeback;
