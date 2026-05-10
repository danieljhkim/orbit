use std::collections::BTreeMap;

use orbit_common::types::OrbitError;
use serde_json::{Value, json};

use super::{artifacts, metrics, roles};
use crate::context::{ActivityInvocationResult, RuntimeHost, TaskHost};
use crate::executor::automation::input::{input_string_field, required_input_string};

fn join_activity_result(
    result: std::thread::Result<Result<ActivityInvocationResult, OrbitError>>,
    label: &str,
) -> Result<ActivityInvocationResult, OrbitError> {
    match result {
        Ok(inner) => inner,
        Err(_) => Err(OrbitError::Execution(format!(
            "{label} activity thread panicked"
        ))),
    }
}

pub(crate) fn run_planning_duel<H: RuntimeHost + TaskHost + Sync + ?Sized>(
    host: &H,
    input: &Value,
    debug: bool,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let job_run_id = input_string_field(input, "job_run_id")
        .or_else(|| input_string_field(input, "run_id"))
        .ok_or_else(|| OrbitError::InvalidInput("missing required input.run_id".to_string()))?;

    let _ = host.get_task(task_id)?;

    artifacts::cleanup_stale_planning_duel_artifacts(host, task_id)?;

    let roles_output = roles::select_planning_duel_roles(host, &json!({ "task_id": task_id }))?;
    let planning_roles = roles::parse_planning_duel_roles(&roles_output)?;

    let planner_activity = roles::planner_activity();
    let planner_a_input = roles::planner_input(task_id);
    let planner_b_input = roles::planner_input(task_id);
    let (planner_a_result, planner_b_result) = std::thread::scope(|scope| {
        let planner_a = planning_roles.planner_a.clone();
        let planner_b = planning_roles.planner_b.clone();
        let planner_activity_a = planner_activity.clone();
        let planner_activity_b = planner_activity.clone();
        let handle_a = scope.spawn(move || {
            host.invoke_activity(
                planner_activity_a,
                &planner_a.agent,
                Some(planner_a.model.as_str()),
                planner_a_input,
                roles::PLANNER_TIMEOUT_SECONDS,
                debug,
            )
        });
        let handle_b = scope.spawn(move || {
            host.invoke_activity(
                planner_activity_b,
                &planner_b.agent,
                Some(planner_b.model.as_str()),
                planner_b_input,
                roles::PLANNER_TIMEOUT_SECONDS,
                debug,
            )
        });
        (
            join_activity_result(handle_a.join(), "planner_a"),
            join_activity_result(handle_b.join(), "planner_b"),
        )
    });
    let planner_a_result = planner_a_result?;
    let planner_b_result = planner_b_result?;

    let planner_artifacts = host.get_task_artifacts(task_id)?;
    let plan_artifacts = artifacts::planning_duel_plan_artifacts(&planner_artifacts)?;
    let _ = artifacts::plan_artifact_for_assignment(&plan_artifacts, &planning_roles.planner_a)?;
    let _ = artifacts::plan_artifact_for_assignment(&plan_artifacts, &planning_roles.planner_b)?;

    let arbiter_result = host.invoke_activity(
        roles::arbiter_activity(),
        &planning_roles.arbiter.agent,
        Some(planning_roles.arbiter.model.as_str()),
        roles::arbiter_input(task_id),
        roles::ARBITER_TIMEOUT_SECONDS,
        debug,
    )?;

    let artifacts_after_arbiter = host.get_task_artifacts(task_id)?;
    let winner =
        artifacts::winner_artifact_from_artifacts(&artifacts_after_arbiter, Some(&planning_roles))?;

    let role_metrics = BTreeMap::from([
        (
            "planner_a".to_string(),
            metrics::role_metrics_from_invocation(
                &planning_roles.planner_a,
                roles::PLANNER_ACTIVITY_ID,
                &planner_a_result,
            ),
        ),
        (
            "planner_b".to_string(),
            metrics::role_metrics_from_invocation(
                &planning_roles.planner_b,
                roles::PLANNER_ACTIVITY_ID,
                &planner_b_result,
            ),
        ),
        (
            "arbiter".to_string(),
            metrics::role_metrics_from_invocation(
                &planning_roles.arbiter,
                roles::ARBITER_ACTIVITY_ID,
                &arbiter_result,
            ),
        ),
    ]);

    let writeback = artifacts::writeback_planning_duel_task(
        host,
        &json!({
            "task_id": task_id,
            "planning_duel_roles": roles_output["planning_duel_roles"].clone(),
        }),
    )?;
    let _ = metrics::record_planning_duel_scores(
        host,
        &json!({
            "task_id": task_id,
            "job_run_id": job_run_id,
            "roles": role_metrics,
        }),
    )?;

    Ok(json!({
        "task_id": task_id,
        "run_id": job_run_id,
        "task_status": writeback["task_status"].clone(),
        "winner_agent_cli": winner.winner_agent_cli,
        "winner_model": winner.winner_model,
        "recorded": true,
    }))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, OnceLock};

    use chrono::Utc;
    use orbit_common::types::{
        Activity, InvocationTrace, Job, JobTargetType, OrbitEvent, PlanningRoleAssignment, Role,
        Task, TaskArtifact, TaskComment, TaskPriority, TaskStatus, TaskType,
    };
    use orbit_store::{InvocationQuery, InvocationRecord};
    use orbit_tools::ToolContext;
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use crate::context::{
        ActivityInvocationResult, JobRunResult, RuntimeHost, TaskAutomationUpdate, TaskReadHost,
        TaskWriteHost,
    };
    use crate::executor::registry::ActivityExecutorRegistry;

    use super::run_planning_duel;

    struct PlanningDuelHost {
        task: Mutex<Task>,
        artifacts: Mutex<Vec<TaskArtifact>>,
        data_root: PathBuf,
        scoreboard_dir: PathBuf,
        _tempdir: TempDir,
        workflow_admissions: AtomicUsize,
        task_starts: AtomicUsize,
        last_automation_update: Mutex<Option<TaskAutomationUpdate>>,
    }

    impl PlanningDuelHost {
        fn new(status: TaskStatus) -> Self {
            let tempdir = tempfile::tempdir().expect("tempdir");
            let data_root = tempdir.path().join(".orbit");
            let scoreboard_dir = data_root.join("state").join("scoreboard");
            std::fs::create_dir_all(&scoreboard_dir).expect("scoreboard dir");

            Self {
                task: Mutex::new(task_with_status(status)),
                artifacts: Mutex::new(Vec::new()),
                data_root,
                scoreboard_dir,
                _tempdir: tempdir,
                workflow_admissions: AtomicUsize::new(0),
                task_starts: AtomicUsize::new(0),
                last_automation_update: Mutex::new(None),
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
    }

    impl TaskReadHost for PlanningDuelHost {
        fn get_task(&self, task_id: &str) -> Result<Task, orbit_common::types::OrbitError> {
            let task = self.task.lock().expect("task lock").clone();
            if task.id == task_id {
                Ok(task)
            } else {
                Err(orbit_common::types::OrbitError::TaskNotFound(
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
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
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
            task.comments.extend(update.append_comments);
            task.agent = update.agent;
            task.model = update.model;
            task.updated_at = Utc::now();
            Ok(())
        }
    }

    impl RuntimeHost for PlanningDuelHost {
        fn record_event(&self, _event: OrbitEvent) -> Result<(), orbit_common::types::OrbitError> {
            Ok(())
        }

        fn repo_root(&self) -> Result<String, orbit_common::types::OrbitError> {
            Ok(self.data_root.display().to_string())
        }

        fn data_root(&self) -> &Path {
            &self.data_root
        }

        fn activity_executor_registry(&self) -> &ActivityExecutorRegistry {
            static REGISTRY: OnceLock<ActivityExecutorRegistry> = OnceLock::new();
            REGISTRY.get_or_init(ActivityExecutorRegistry::new)
        }

        fn run_job_now_with_input_debug(
            &self,
            _job_id: &str,
            _input: Value,
            _debug: bool,
        ) -> Result<JobRunResult, orbit_common::types::OrbitError> {
            Err(orbit_common::types::OrbitError::Execution(
                "not used by planning duel test".to_string(),
            ))
        }

        fn validate_activity_target_exists(
            &self,
            _target_type: JobTargetType,
            _target_id: &str,
        ) -> Result<Activity, orbit_common::types::OrbitError> {
            Err(orbit_common::types::OrbitError::Execution(
                "not used by planning duel test".to_string(),
            ))
        }

        fn get_job(&self, _job_id: &str) -> Result<Option<Job>, orbit_common::types::OrbitError> {
            Ok(None)
        }

        fn invocation_records(
            &self,
            _query: InvocationQuery,
        ) -> Result<Vec<InvocationRecord>, orbit_common::types::OrbitError> {
            Ok(Vec::new())
        }

        fn run_tool_with_context_and_role(
            &self,
            _name: &str,
            _input: Value,
            _role: Role,
            _tool_context: ToolContext,
        ) -> Result<Value, orbit_common::types::OrbitError> {
            Err(orbit_common::types::OrbitError::Execution(
                "stale artifact cleanup should not run tools in this test".to_string(),
            ))
        }

        fn invoke_activity(
            &self,
            activity: Activity,
            agent_cli: &str,
            model: Option<&str>,
            _input: Value,
            _timeout_seconds: u64,
            _debug: bool,
        ) -> Result<ActivityInvocationResult, orbit_common::types::OrbitError> {
            let model = model.unwrap_or("unknown-model");
            match activity.id.as_str() {
                "propose_duel_plan" => {
                    self.artifacts
                        .lock()
                        .expect("artifacts lock")
                        .push(TaskArtifact {
                            path: format!("planning-duel/{agent_cli}-{model}.md"),
                            content: format!(
                                "*authored by: {agent_cli} / {model}*\n## Plan\nPreserve task status.\n"
                            ),
                        });
                }
                "arbitrate_duel_plan" => {
                    let winner =
                        first_planner_assignment(&self.artifacts.lock().expect("artifacts lock"))?;
                    self.artifacts
                        .lock()
                        .expect("artifacts lock")
                        .push(TaskArtifact {
                            path: "planning-duel/winner.json".to_string(),
                            content: json!({
                                "winner_agent_cli": winner.agent,
                                "winner_model": winner.model,
                                "arbiter_rationale": "Preserves lifecycle state."
                            })
                            .to_string(),
                        });
                }
                other => {
                    return Err(orbit_common::types::OrbitError::Execution(format!(
                        "unexpected activity '{other}'"
                    )));
                }
            }

            Ok(ActivityInvocationResult {
                response_json: Some(json!({})),
                invocation_trace: InvocationTrace::default(),
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
        ) -> Result<(), orbit_common::types::OrbitError> {
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
    ) -> Result<PlanningRoleAssignment, orbit_common::types::OrbitError> {
        let artifact = artifacts
            .iter()
            .find(|artifact| {
                artifact.path.starts_with("planning-duel/") && artifact.path.ends_with(".md")
            })
            .ok_or_else(|| {
                orbit_common::types::OrbitError::Execution("missing planner artifact".to_string())
            })?;
        let name = artifact
            .path
            .strip_prefix("planning-duel/")
            .and_then(|value| value.strip_suffix(".md"))
            .ok_or_else(|| {
                orbit_common::types::OrbitError::Execution(
                    "invalid planner artifact path".to_string(),
                )
            })?;
        let (agent, model) = name.split_once('-').ok_or_else(|| {
            orbit_common::types::OrbitError::Execution(
                "invalid planner artifact signature".to_string(),
            )
        })?;
        Ok(PlanningRoleAssignment {
            agent: agent.to_string(),
            model: model.to_string(),
        })
    }

    fn task_with_status(status: TaskStatus) -> Task {
        let now = Utc::now();
        Task {
            id: "T20260430-STATUS".to_string(),
            parent_id: None,
            title: "Planning duel status preservation".to_string(),
            description: "Exercise planning duel without lifecycle admission.".to_string(),
            acceptance_criteria: Vec::new(),
            dependencies: Vec::new(),
            tags: Vec::new(),
            plan: String::new(),
            execution_summary: String::new(),
            context_files: Vec::new(),
            workspace_path: None,
            repo_root: None,
            created_by: Some("test".to_string()),
            planned_by: None,
            implemented_by: None,
            agent: None,
            model: None,
            status,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Bug,
            pr_status: None,
            external_refs: Vec::new(),
            source_task_id: None,
            batch_id: None,
            comments: Vec::<TaskComment>::new(),
            history: Vec::new(),
            review_threads: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn run_planning_duel_preserves_existing_task_status() {
        for status in [
            TaskStatus::Proposed,
            TaskStatus::Friction,
            TaskStatus::Backlog,
            TaskStatus::Rejected,
            TaskStatus::Archived,
            TaskStatus::InProgress,
        ] {
            let host = PlanningDuelHost::new(status);
            let output = run_planning_duel(
                &host,
                &json!({
                    "task_id": "T20260430-STATUS",
                    "run_id": format!("jrun-{status}")
                }),
                false,
            )
            .expect("planning duel succeeds without lifecycle admission");

            let expected_status = status.to_string();
            let task = host
                .get_task("T20260430-STATUS")
                .expect("task remains readable");
            let comment = task.comments.last().expect("planning duel comment");
            assert_eq!(host.task_status(), status, "{status}");
            assert_eq!(
                output["task_status"].as_str(),
                Some(expected_status.as_str()),
                "{status}"
            );
            assert!(
                comment
                    .message
                    .contains(&format!("Task status remains {expected_status}.")),
                "{status}: {}",
                comment.message
            );
            assert!(
                !comment
                    .message
                    .contains("Task status is in-progress for workflow execution."),
                "{status}: {}",
                comment.message
            );
            assert_eq!(host.admission_count(), 0, "{status}");
            assert_eq!(host.start_count(), 0, "{status}");
        }
    }

    fn install_planning_duel_artifacts(host: &PlanningDuelHost, plan_body: &str) {
        let mut artifacts = host.artifacts.lock().expect("artifacts lock");
        artifacts.clear();
        artifacts.push(TaskArtifact {
            path: "planning-duel/codex-gpt-5.5.md".to_string(),
            content: "*authored by: codex / gpt-5.5*\n## Plan\nLoser plan.\n".to_string(),
        });
        artifacts.push(TaskArtifact {
            path: "planning-duel/claude-claude-opus-4-7.md".to_string(),
            content: format!("*authored by: claude / claude-opus-4-7*\n{plan_body}"),
        });
        artifacts.push(TaskArtifact {
            path: "planning-duel/winner.json".to_string(),
            content: json!({
                "winner_agent_cli": "claude",
                "winner_model": "claude-opus-4-7",
                "arbiter_rationale": "Claude provided more detail."
            })
            .to_string(),
        });
    }

    fn run_writeback(host: &PlanningDuelHost) -> serde_json::Value {
        super::artifacts::writeback_planning_duel_task(
            host,
            &json!({
                "task_id": "T20260430-STATUS",
                "planning_duel_roles": {
                    "planner_a": { "agent": "codex", "model": "gpt-5.5" },
                    "planner_b": { "agent": "claude", "model": "claude-opus-4-7" },
                    "arbiter":   { "agent": "gemini", "model": "gemini-3.1-pro" }
                }
            }),
        )
        .expect("writeback succeeds")
    }

    #[test]
    fn writeback_populates_context_files_when_section_present() {
        let host = PlanningDuelHost::new(TaskStatus::InProgress);
        install_planning_duel_artifacts(
            &host,
            "## Plan\nDo it.\n\n## Context Files\n\n- `file:src/a.rs`\n- crates/foo/\n",
        );

        let _ = run_writeback(&host);

        assert_eq!(
            host.last_context_files_update(),
            Some(Some(vec![
                "file:src/a.rs".to_string(),
                "dir:crates/foo".to_string(),
            ])),
            "writeback should set context_files to canonical entries"
        );

        let task = host.get_task("T20260430-STATUS").expect("task readable");
        assert_eq!(
            task.context_files,
            vec!["file:src/a.rs".to_string(), "dir:crates/foo".to_string()]
        );
    }

    #[test]
    fn writeback_preserves_context_files_when_section_absent() {
        let host = PlanningDuelHost::new(TaskStatus::InProgress);
        // Pre-populate the task's context_files with curated state.
        host.task.lock().expect("task lock").context_files =
            vec!["file:pre-existing.rs".to_string()];
        install_planning_duel_artifacts(&host, "## Plan\nNo Context Files section here.\n");

        let _ = run_writeback(&host);

        assert_eq!(
            host.last_context_files_update(),
            Some(None),
            "writeback must leave context_files untouched when no section is present"
        );

        let task = host.get_task("T20260430-STATUS").expect("task readable");
        assert_eq!(
            task.context_files,
            vec!["file:pre-existing.rs".to_string()],
            "pre-existing context_files should be preserved"
        );
    }

    #[test]
    fn writeback_preserves_context_files_when_section_recognized_but_empty() {
        let host = PlanningDuelHost::new(TaskStatus::InProgress);
        host.task.lock().expect("task lock").context_files =
            vec!["file:pre-existing.rs".to_string()];
        install_planning_duel_artifacts(
            &host,
            "## Plan\nDo it.\n\n## Context Files\n\n## Risks\n- something.\n",
        );

        let _ = run_writeback(&host);

        assert_eq!(
            host.last_context_files_update(),
            Some(None),
            "an empty Context Files section should not clear the field"
        );
    }

    #[test]
    fn writeback_is_idempotent_across_two_resolves() {
        let host = PlanningDuelHost::new(TaskStatus::InProgress);
        install_planning_duel_artifacts(
            &host,
            "## Plan\nDo it.\n\n## Context Files\n- `file:src/a.rs`\n- `dir:src`\n",
        );

        let _ = run_writeback(&host);
        let first = host.last_context_files_update();

        let _ = run_writeback(&host);
        let second = host.last_context_files_update();

        assert!(first.is_some());
        assert_eq!(
            first, second,
            "two consecutive resolves must produce identical context_files"
        );
    }
}
