use orbit_common::types::{
    ExternalRef, OrbitError, OrbitEvent, ReviewThread, Task, TaskComment, TaskHistoryEntry,
    TaskPriority, TaskStatus, normalize_optional_attribution_label, push_external_ref_if_missing,
};
use orbit_engine::{RuntimeHost, TaskAutomationUpdate, TaskReadHost, TaskWriteHost};

use crate::OrbitRuntime;
use crate::command::task::SYSTEM_ACTOR_LABEL;
use crate::runtime::TaskRecordUpdateParams as StoreTaskUpdateParams;

impl TaskReadHost for OrbitRuntime {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        OrbitRuntime::get_task(self, task_id)
    }

    fn get_task_artifacts(
        &self,
        task_id: &str,
    ) -> Result<Vec<orbit_common::types::TaskArtifact>, OrbitError> {
        OrbitRuntime::get_task_artifacts(self, task_id)
    }

    fn get_task_comments(&self, task_id: &str) -> Result<Vec<TaskComment>, OrbitError> {
        OrbitRuntime::get_task_comments(self, task_id)
    }

    fn get_task_history(&self, task_id: &str) -> Result<Vec<TaskHistoryEntry>, OrbitError> {
        OrbitRuntime::get_task_history(self, task_id)
    }

    fn get_task_review_threads(&self, task_id: &str) -> Result<Vec<ReviewThread>, OrbitError> {
        OrbitRuntime::get_task_review_threads(self, task_id)
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
        OrbitRuntime::list_tasks_filtered(
            self,
            status,
            priority,
            parent_id,
            job_run_id,
            external_ref,
            has_external_ref_system,
        )
    }
}

impl TaskWriteHost for OrbitRuntime {
    fn start_task(
        &self,
        task_id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        OrbitRuntime::start_task_as_system(self, task_id, note, comment)
    }

    fn admit_task_for_workflow(&self, task_id: &str, workflow: &str) -> Result<Task, OrbitError> {
        OrbitRuntime::admit_task_for_workflow_as_system(self, task_id, workflow)
    }

    fn update_task_from_activity(
        &self,
        task_id: &str,
        status: TaskStatus,
        execution_summary: Option<String>,
        comment: Option<String>,
        note: Option<String>,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Task, OrbitError> {
        OrbitRuntime::update_task_from_activity(
            self,
            task_id,
            status,
            execution_summary,
            comment,
            note,
            agent,
            model,
        )
    }

    fn apply_task_automation_update(
        &self,
        task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        let existing_task = self.get_task(task_id)?;
        if update.status == Some(TaskStatus::Friction)
            && existing_task.status != TaskStatus::Friction
        {
            return Err(OrbitError::InvalidInput(format!(
                "status 'friction' can only be set at creation; task '{task_id}' is currently '{}'",
                existing_task.status
            )));
        }
        if update.status == Some(TaskStatus::InProgress)
            && crate::command::task::in_progress_transition_requires_plan(existing_task.status)
        {
            crate::command::task::ensure_task_has_execution_plan(
                task_id,
                existing_task.plan.as_str(),
            )?;
        }
        let (agent, model) = self
            .try_canonical_agent_model_identity(update.agent.as_deref(), update.model.as_deref())?;
        let runtime_model_identity = <Self as RuntimeHost>::actor_model_identity(self);
        let _ = self.with_mutation(|| {
            let actor_label = SYSTEM_ACTOR_LABEL.to_string();
            let explicit_attribution_label = normalize_optional_attribution_label(
                update
                    .model
                    .as_deref()
                    .or(model.as_deref())
                    .or(update.agent.as_deref())
                    .or(agent.as_deref()),
                model.as_deref(),
            );
            let planned_by = update.plan.as_ref().map(|_| {
                Some(
                    explicit_attribution_label
                        .clone()
                        .or_else(|| runtime_model_identity.clone())
                        .unwrap_or_else(|| actor_label.clone()),
                )
            });
            let implemented_by = if let Some(existing) = existing_task.implemented_by.as_deref() {
                normalize_optional_attribution_label(Some(existing), None)
            } else {
                normalize_optional_attribution_label(
                    model
                        .as_deref()
                        .or(explicit_attribution_label.as_deref())
                        .or(runtime_model_identity.as_deref())
                        .or(Some(actor_label.as_str())),
                    model.as_deref(),
                )
            };
            let external_refs = if update.external_refs.is_empty() {
                None
            } else {
                let mut refs = existing_task.external_refs.clone();
                for external_ref in update.external_refs.clone() {
                    push_external_ref_if_missing(&mut refs, external_ref);
                }
                Some(refs)
            };
            let task = self.stores().tasks().update(
                task_id,
                StoreTaskUpdateParams {
                    actor: actor_label.clone(),
                    execution_summary: update.execution_summary.clone(),
                    plan: update.plan.clone(),
                    context_files: update.context_files.clone(),
                    planned_by,
                    implemented_by: if matches!(
                        update.status,
                        Some(TaskStatus::Review | TaskStatus::Done)
                    ) {
                        implemented_by.clone().map(Some)
                    } else {
                        None
                    },
                    status: update.status,
                    external_refs,
                    job_run_id: update.job_run_id.clone().map(Some),
                    status_event: update.status_event.clone(),
                    status_note: update.status_note.clone(),
                    append_comments: update.append_comments.clone(),
                    replace_review_threads: update.review_threads.clone(),
                    ..Default::default()
                },
            )?;
            Ok((
                task.clone(),
                OrbitEvent::TaskUpdated {
                    id: task_id.to_string(),
                },
            ))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use crate::command::task::{TaskAddParams, TaskUpdateParams};
    use chrono::Utc;
    use orbit_common::types::activity_job::{ActivityV2Spec, DeterministicSpec};
    use orbit_engine::{V2AuditWriter, V2DispatchInput};
    use serde_json::{Value, json};
    use tempfile::tempdir;

    fn test_runtime() -> (tempfile::TempDir, OrbitRuntime) {
        let root = tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("create global root");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
        (root, runtime)
    }

    fn test_runtime_with_config(config: &str) -> (tempfile::TempDir, OrbitRuntime) {
        let root = tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("create global root");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::write(workspace_root.join("config.toml"), config).expect("write test config");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
        (root, runtime)
    }

    fn attribution_test_config() -> &'static str {
        r#"
[workflow]
default_crew = "all-claude"

[crews.all-claude]
planner = { model = "claude-opus-4-7", provider = "claude", backend = "cli" }
implementer = { model = "claude-sonnet-4-6", provider = "claude", backend = "cli" }
reviewer = { model = "claude-opus-4-7", provider = "claude", backend = "cli" }

[crews.all-codex]
planner = { model = "gpt-5.5", provider = "codex", backend = "cli" }
implementer = { model = "gpt-5.5", provider = "codex", backend = "cli" }
reviewer = { model = "gpt-5.5", provider = "codex", backend = "cli" }
"#
    }

    fn insert_run_with_resolved_crew(runtime: &OrbitRuntime, job_id: &str, input: Value) -> String {
        let run = runtime
            .stores()
            .jobs()
            .insert_run(job_id, 1, Utc::now(), Some(input.clone()), None)
            .expect("insert job run");
        runtime
            .record_run_crew_from_input(&run.run_id, &input)
            .expect("record run crew");
        run.run_id
    }

    fn run_update_task_v2_activity(runtime: &OrbitRuntime, run_id: &str, input: Value) -> Value {
        let audit_dir = tempdir().expect("audit tempdir");
        let audit = V2AuditWriter::with_disk_sinks(
            audit_dir.path(),
            run_id,
            "test:update_task".to_string(),
            None,
        )
        .expect("audit writer");
        let spec = ActivityV2Spec::Deterministic(DeterministicSpec {
            action: "update_task".to_string(),
            config: json!({}),
        });

        orbit_engine::dispatch_v2_activity(V2DispatchInput {
            activity_name: "update_task",
            spec: &spec,
            fs_profile: None,
            input,
            audit,
            run_id,
            host: Some(runtime),
        })
        .expect("dispatch update_task activity")
        .output
    }

    fn approve_for_execution(runtime: &OrbitRuntime, task: &Task) -> Task {
        runtime
            .approve_task(
                &task.id,
                Some("approve for test execution".to_string()),
                None,
            )
            .expect("approve task")
    }

    fn init_git_repo(repo: &Path) {
        git(repo, &["init"]);
        git(repo, &["checkout", "-b", "main"]);
        git(repo, &["config", "user.name", "Orbit Test"]);
        git(repo, &["config", "user.email", "orbit-test@example.com"]);
        fs::write(repo.join("README.md"), "test repo\n").expect("write README");
        git(repo, &["add", "README.md"]);
        git(repo, &["commit", "-m", "initial commit"]);
    }

    fn git(current_dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(current_dir)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {} failed in {}:\nstdout: {}\nstderr: {}",
            args.join(" "),
            current_dir.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn run_worktree_setup(runtime: &OrbitRuntime, task_ids: &[String], run_id: &str) -> Value {
        orbit_engine::execute_deterministic_action(
            runtime,
            "worktree_setup",
            &json!({
                "run_id": run_id,
                "task_ids": task_ids,
                "base": "main",
                "base_sync": "local",
                "branch_prefix": "orbit-test"
            }),
            false,
            &HashMap::new(),
            None,
        )
        .expect("run worktree setup")
    }

    #[test]
    fn apply_task_automation_update_does_not_touch_context_files_when_unset() {
        let (root, runtime) = test_runtime();
        // Pre-create files in the workspace so add_task does not prune the
        // selectors as missing.
        let repo = root.path().join("repo");
        fs::write(repo.join("README.md"), "test\n").expect("write README.md");
        fs::write(repo.join("CLAUDE.md"), "test\n").expect("write CLAUDE.md");
        let task = runtime
            .add_task(TaskAddParams {
                title: "Preserve context_files".to_string(),
                description: "Exercise context_files preservation across automation updates."
                    .to_string(),
                workspace_path: Some(".".to_string()),
                context_files: vec!["README.md".to_string(), "CLAUDE.md".to_string()],
                ..Default::default()
            })
            .expect("add task");

        let initial_context_files = runtime
            .get_task(&task.id)
            .expect("reload task")
            .context_files;
        assert!(
            !initial_context_files.is_empty(),
            "test precondition: add_task should keep existing context_files"
        );

        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    plan: Some("## Plan\nSome plan body.\n".to_string()),
                    context_files: None,
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("automation update without context_files set");

        let updated = runtime.get_task(&task.id).expect("reload task");
        assert_eq!(
            updated.context_files, initial_context_files,
            "context_files: None should leave the field untouched"
        );
    }

    #[test]
    fn apply_task_automation_update_replaces_context_files_when_set() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Replace context_files".to_string(),
                description: "Exercise context_files replacement via automation update."
                    .to_string(),
                workspace_path: Some(".".to_string()),
                context_files: vec!["Cargo.toml".to_string()],
                ..Default::default()
            })
            .expect("add task");

        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    context_files: Some(vec![
                        "file:src/new.rs".to_string(),
                        "dir:src/new_dir".to_string(),
                    ]),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("automation update with new context_files");

        let updated = runtime.get_task(&task.id).expect("reload task");
        assert_eq!(
            updated.context_files,
            vec!["file:src/new.rs".to_string(), "dir:src/new_dir".to_string()]
        );
    }

    #[test]
    fn automation_can_restamp_in_progress_task_without_plan() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Restamp task metadata".to_string(),
                description: "Exercise idempotent in-progress automation updates.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");
        let task = approve_for_execution(&runtime, &task);

        assert!(task.plan.is_empty());
        let started = runtime
            .start_task(&task.id, Some("start approved task".to_string()), None)
            .expect("start backlog task without plan");
        assert_eq!(started.status, TaskStatus::InProgress);

        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    job_run_id: Some("jrun-test".to_string()),
                    status: Some(TaskStatus::InProgress),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("restamp in-progress task metadata");

        let updated = runtime.get_task(&task.id).expect("reload task");
        assert_eq!(updated.status, TaskStatus::InProgress);
        assert_eq!(updated.job_run_id.as_deref(), Some("jrun-test"));
    }

    #[test]
    fn worktree_setup_admits_unplanned_workflow_statuses() {
        let (root, runtime) = test_runtime();
        let repo = root.path().join("repo");
        init_git_repo(&repo);
        let proposed = runtime
            .add_task(TaskAddParams {
                title: "Proposed workflow task".to_string(),
                description: "Starts from proposed without a plan.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("create proposed task");
        let backlog = approve_for_execution(
            &runtime,
            &runtime
                .add_task(TaskAddParams {
                    title: "Backlog workflow task".to_string(),
                    description: "Starts from backlog without a plan.".to_string(),
                    workspace_path: Some(".".to_string()),
                    ..Default::default()
                })
                .expect("create backlog candidate"),
        );
        let rejected = runtime
            .add_task(TaskAddParams {
                title: "Rejected workflow task".to_string(),
                description: "Starts from rejected without a plan.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("create rejected candidate");
        let rejected = runtime
            .reject_task(
                &rejected.id,
                "exercise workflow admission".to_string(),
                None,
            )
            .expect("reject task");
        let archived = runtime
            .add_task(TaskAddParams {
                title: "Archived workflow task".to_string(),
                description: "Starts from archived without a plan.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("create archived candidate");
        runtime.archive_task(&archived.id).expect("archive task");

        let task_ids = vec![
            proposed.id.clone(),
            backlog.id.clone(),
            rejected.id.clone(),
            archived.id.clone(),
        ];
        let output = run_worktree_setup(&runtime, &task_ids, "jrun-admit");
        let workspace_path = output["workspace_path"]
            .as_str()
            .expect("workspace path output")
            .to_string();
        assert!(!workspace_path.is_empty());

        for task_id in &task_ids {
            let task = runtime.get_task(task_id).expect("reload admitted task");
            assert_eq!(task.status, TaskStatus::InProgress, "{task_id}");
            assert_eq!(task.job_run_id.as_deref(), Some("jrun-admit"));
        }

        let admitted_again = runtime
            .admit_task_for_workflow_as_system(&proposed.id, "worktree_setup")
            .expect("idempotent workflow admission");
        assert_eq!(admitted_again.status, TaskStatus::InProgress);
    }

    #[test]
    fn direct_update_to_in_progress_still_requires_plan_for_unapproved_statuses() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Direct update remains gated".to_string(),
                description: "A direct update is not workflow admission.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("create proposed task");

        let err = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .expect_err("direct update should still require a plan");
        assert!(
            err.to_string()
                .contains("requires a non-empty execution plan"),
            "{err}"
        );
    }

    #[test]
    fn generic_automation_update_does_not_unarchive_empty_plan_tasks() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Archived generic automation".to_string(),
                description: "Generic metadata stamping is not workflow admission.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("create task");
        runtime.archive_task(&task.id).expect("archive task");

        let err = runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    status: Some(TaskStatus::InProgress),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect_err("generic automation update should not admit archived task");
        assert!(
            err.to_string()
                .contains("requires a non-empty execution plan"),
            "{err}"
        );
    }

    #[test]
    fn update_task_automation_records_status_history_as_system() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Review automated update".to_string(),
                description: "Exercise update_task automation attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");
        let task = approve_for_execution(&runtime, &task);
        runtime
            .start_task(&task.id, Some("human starts work".to_string()), None)
            .expect("start task");
        runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    execution_summary: Some("Implemented and validated.".to_string()),
                    ..Default::default()
                },
            )
            .expect("set execution summary");

        orbit_engine::execute_deterministic_action(
            &runtime,
            "update_task",
            &json!({
                "task_id": task.id.clone(),
                "status": "review"
            }),
            false,
            &HashMap::new(),
            None,
        )
        .expect("run update_task automation");

        let history = runtime.get_task_history(&task.id).expect("reload history");
        let status_entry = history
            .iter()
            .rev()
            .find(|entry| {
                entry.event == "status_changed" && entry.to_status == Some(TaskStatus::Review)
            })
            .expect("review transition history");
        assert_eq!(status_entry.by, SYSTEM_ACTOR_LABEL);
        assert_eq!(
            status_entry.note.as_deref(),
            Some("automation: update_task \u{2192} review")
        );
    }

    #[test]
    fn v2_update_task_activity_uses_resolved_crew_implementer_identity() {
        let (_root, runtime) = test_runtime_with_config(attribution_test_config());
        let task = runtime
            .add_task(TaskAddParams {
                title: "Review attributed update".to_string(),
                description: "Exercise update_task activity implementer attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                crew: Some("all-claude".to_string()),
                ..Default::default()
            })
            .expect("add task");
        let task = approve_for_execution(&runtime, &task);
        runtime
            .start_task(&task.id, Some("start task".to_string()), None)
            .expect("start task");
        runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    execution_summary: Some(
                        "Agent persisted intermediate work without changing status.".to_string(),
                    ),
                    ..Default::default()
                },
            )
            .expect("intermediate task update");
        let run_id = insert_run_with_resolved_crew(
            &runtime,
            "task_pipeline",
            json!({ "task_id": task.id.clone() }),
        );
        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    job_run_id: Some(run_id.clone()),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("stamp job run");

        run_update_task_v2_activity(
            &runtime,
            &run_id,
            json!({
                "task_id": task.id.clone(),
                "status": "review"
            }),
        );

        let updated = runtime.get_task(&task.id).expect("reload task");
        assert_eq!(updated.status, TaskStatus::Review);
        assert_eq!(updated.implemented_by.as_deref(), Some("claude"));
    }

    #[test]
    fn v2_update_task_activity_preserves_existing_implemented_by() {
        let (_root, runtime) = test_runtime_with_config(attribution_test_config());
        let task = runtime
            .add_task(TaskAddParams {
                title: "Preserve existing implementer".to_string(),
                description: "Exercise review to done automation attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                crew: Some("all-codex".to_string()),
                ..Default::default()
            })
            .expect("add task");
        let task = approve_for_execution(&runtime, &task);
        runtime
            .start_task(&task.id, Some("start task".to_string()), None)
            .expect("start task");
        runtime
            .update_task_with_identity(
                &task.id,
                TaskUpdateParams {
                    status: Some(TaskStatus::Review),
                    execution_summary: Some("Agent moved the task to review.".to_string()),
                    ..Default::default()
                },
                Some("claude".to_string()),
                Some("claude".to_string()),
            )
            .expect("agent review update");
        let reviewed = runtime.get_task(&task.id).expect("reload reviewed task");
        assert_eq!(reviewed.implemented_by.as_deref(), Some("claude"));

        let run_id = insert_run_with_resolved_crew(
            &runtime,
            "task_pipeline",
            json!({ "task_id": task.id.clone() }),
        );
        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    job_run_id: Some(run_id.clone()),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("stamp job run");

        run_update_task_v2_activity(
            &runtime,
            &run_id,
            json!({
                "task_id": task.id.clone(),
                "status": "done"
            }),
        );

        let done = runtime.get_task(&task.id).expect("reload done task");
        assert_eq!(done.status, TaskStatus::Done);
        assert_eq!(done.implemented_by.as_deref(), Some("claude"));
    }

    #[test]
    fn review_transition_still_requires_execution_summary() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Review guard".to_string(),
                description: "Exercise review summary requirement.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");
        let task = approve_for_execution(&runtime, &task);
        runtime
            .start_task(&task.id, Some("start task".to_string()), None)
            .expect("start task");

        let err = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    status: Some(TaskStatus::Review),
                    ..Default::default()
                },
            )
            .expect_err("review without execution summary should fail");
        assert!(
            err.to_string()
                .contains("requires non-empty execution_summary"),
            "{err}"
        );
    }

    #[test]
    fn activity_update_comment_records_comment_as_system() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Activity comment".to_string(),
                description: "Exercise activity comment attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");
        let task = approve_for_execution(&runtime, &task);

        runtime
            .update_task_from_activity(
                &task.id,
                TaskStatus::InProgress,
                None,
                Some("Automation left a note.".to_string()),
                Some("automation start".to_string()),
                None,
                None,
            )
            .expect("activity update");

        let comments = runtime
            .get_task_comments(&task.id)
            .expect("reload comments");
        let comment = comments.last().expect("activity comment");
        assert_eq!(comment.by, SYSTEM_ACTOR_LABEL);
        let history = runtime.get_task_history(&task.id).expect("reload history");
        let comment_history = history
            .iter()
            .find(|entry| entry.event == "commented")
            .expect("comment history");
        assert_eq!(comment_history.by, SYSTEM_ACTOR_LABEL);
    }

    #[test]
    fn automation_update_under_agent_runtime_uses_model_identity_for_attribution() {
        let (_root, runtime) = test_runtime();
        let runtime = runtime.with_actor(crate::ActorIdentity::agent("gpt-agent-test"));
        let task = runtime
            .add_task(TaskAddParams {
                title: "Agent-driven automation".to_string(),
                description: "Exercise agent runtime attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");
        let task = approve_for_execution(&runtime, &task);
        runtime
            .start_task(&task.id, Some("start task".to_string()), None)
            .expect("start task");

        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    plan: Some("Implement the task through workflow automation.".to_string()),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("automation plan update");
        let planned = runtime.get_task(&task.id).expect("reload planned task");
        assert_eq!(planned.planned_by.as_deref(), Some("gpt-agent-test"));

        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    status: Some(TaskStatus::Review),
                    execution_summary: Some("Implemented through automation.".to_string()),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("automation review update");

        let updated = runtime.get_task(&task.id).expect("reload task");
        assert_eq!(updated.implemented_by.as_deref(), Some("gpt-agent-test"));
    }

    #[test]
    fn automation_update_without_agent_runtime_falls_back_to_system_attribution() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "System automation".to_string(),
                description: "Exercise non-agent workflow attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");
        let task = approve_for_execution(&runtime, &task);
        runtime
            .start_task(&task.id, Some("start task".to_string()), None)
            .expect("start task");

        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    status: Some(TaskStatus::Review),
                    execution_summary: Some("Implemented through system automation.".to_string()),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("automation review update");

        let updated = runtime.get_task(&task.id).expect("reload task");
        assert_eq!(updated.implemented_by.as_deref(), Some(SYSTEM_ACTOR_LABEL));
    }

    #[test]
    fn automation_review_done_transitions_preserve_existing_implemented_by_without_model() {
        let (_root, runtime) = test_runtime();
        let runtime = runtime.with_actor(crate::ActorIdentity::agent("gpt-agent-test"));
        let task = runtime
            .add_task(TaskAddParams {
                title: "Preserve implementer".to_string(),
                description: "Exercise existing implementer preservation.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");
        let task = approve_for_execution(&runtime, &task);
        runtime
            .start_task(&task.id, Some("start task".to_string()), None)
            .expect("start task");
        runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    execution_summary: Some("Existing implementation summary.".to_string()),
                    implemented_by: Some(Some("claude-opus-4-7".to_string())),
                    ..Default::default()
                },
            )
            .expect("seed existing implementation attribution");

        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    status: Some(TaskStatus::Review),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("automation review update");
        let reviewed = runtime.get_task(&task.id).expect("reload reviewed task");
        assert_eq!(reviewed.implemented_by.as_deref(), Some("claude-opus-4-7"));

        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    status: Some(TaskStatus::Done),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("automation done update");
        let done = runtime.get_task(&task.id).expect("reload done task");
        assert_eq!(done.implemented_by.as_deref(), Some("claude-opus-4-7"));
    }

    #[test]
    fn generic_automation_status_update_uses_system_history_and_preserves_implementer() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Generic automation".to_string(),
                description: "Exercise TaskAutomationUpdate attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");
        let task = approve_for_execution(&runtime, &task);
        runtime
            .start_task(&task.id, Some("human starts work".to_string()), None)
            .expect("start task");

        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    status: Some(TaskStatus::Review),
                    execution_summary: Some("Automated handoff complete.".to_string()),
                    agent: Some("codex".to_string()),
                    model: Some("gpt-test".to_string()),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("automation update");

        let updated = runtime.get_task(&task.id).expect("reload task");
        let history = runtime.get_task_history(&task.id).expect("reload history");
        let status_entry = history
            .iter()
            .rev()
            .find(|entry| {
                entry.event == "status_changed" && entry.to_status == Some(TaskStatus::Review)
            })
            .expect("review transition history");
        assert_eq!(status_entry.by, SYSTEM_ACTOR_LABEL);
        assert_eq!(updated.implemented_by.as_deref(), Some("gpt-test"));
    }

    #[test]
    fn direct_update_task_keeps_default_human_attribution() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Human comment".to_string(),
                description: "Exercise direct update attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");

        runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    comment: Some("Human-visible note.".to_string()),
                    ..Default::default()
                },
            )
            .expect("update task");

        let comments = runtime
            .get_task_comments(&task.id)
            .expect("reload comments");
        let comment = comments.last().expect("human comment");
        assert_eq!(comment.by, "human");
        let history = runtime.get_task_history(&task.id).expect("reload history");
        let comment_history = history
            .iter()
            .find(|entry| entry.event == "commented")
            .expect("comment history");
        assert_eq!(comment_history.by, "human");
    }

    #[test]
    fn dispatch_batch_claim_records_start_and_comment_as_system() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Claim in batch".to_string(),
                description: "Exercise dispatch_batch attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");
        let task = approve_for_execution(&runtime, &task);

        orbit_engine::execute_deterministic_action(
            &runtime,
            "dispatch_batch",
            &json!({
                "run_id": "jrun-test",
                "parallelism": 1,
                "task_ids": [task.id.clone()]
            }),
            false,
            &HashMap::new(),
            None,
        )
        .expect("dispatch batch");

        let history = runtime.get_task_history(&task.id).expect("reload history");
        let start_entry = history
            .iter()
            .find(|entry| entry.event == "started")
            .expect("start history");
        assert_eq!(start_entry.by, SYSTEM_ACTOR_LABEL);
        let comments = runtime
            .get_task_comments(&task.id)
            .expect("reload comments");
        let batch_comment = comments
            .iter()
            .find(|comment| comment.message.starts_with("Batch dispatched:"))
            .expect("batch dispatch comment");
        assert_eq!(batch_comment.by, SYSTEM_ACTOR_LABEL);
    }
}
