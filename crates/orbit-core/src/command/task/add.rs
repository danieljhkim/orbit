use orbit_common::types::{
    OrbitError, OrbitEvent, Task, TaskStatus, TaskType, normalize_task_dependencies,
    normalize_task_tags, prune_missing_context_files, validate_task_dependencies,
};
use orbit_store::{TaskCreateParams as StoreTaskCreateParams, friction_bounty};

use crate::OrbitRuntime;
use crate::runtime::TaskRecordUpdateParams;

use super::helpers::{authored_role_value, build_task_comments, effective_actor_label};
use super::params::TaskAddParams;
use super::paths::{
    context_files_pruned_history_entry, context_workspace_root,
    emit_graph_unavailable_warning_if_needed, normalize_context_files_for_write,
    normalize_workspace_path,
};

impl OrbitRuntime {
    pub fn add_task(&self, params: TaskAddParams) -> Result<Task, OrbitError> {
        self.add_task_with_identity(params, None, None)
    }

    pub fn add_task_with_identity(
        &self,
        params: TaskAddParams,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Task, OrbitError> {
        let (canonical_agent, canonical_model) =
            self.try_canonical_agent_model_identity(agent.as_deref(), model.as_deref())?;
        let actor = self.actor().clone();
        let effective_label = effective_actor_label(
            &actor.label,
            canonical_agent.as_deref(),
            canonical_model.as_deref(),
        );
        let (task_type, initial_status) = infer_task_create_type_and_status(
            params.task_type,
            params.status,
            TaskStatus::Proposed,
        )?;
        let uses_system_identity = params.system_created;
        let create_label = if uses_system_identity {
            "system".to_string()
        } else {
            effective_label.clone()
        };
        let planned_by = authored_role_value(params.plan.as_str(), &create_label);
        let comments = build_task_comments(params.comment.clone(), create_label.as_str())?;
        let workspace_path =
            normalize_workspace_path(&self.paths().repo_root, params.workspace_path.as_deref())?;
        let dependencies = normalize_task_dependencies(params.dependencies.clone())?;
        validate_task_dependencies(&self.list_tasks()?, None, &dependencies)?;

        let prune_root = context_workspace_root(&self.paths().repo_root, workspace_path.as_deref());
        let normalized_context_files =
            normalize_context_files_for_write(params.context_files.clone(), &prune_root)?;
        emit_graph_unavailable_warning_if_needed(&normalized_context_files, self.data_root_path());
        let (kept_context_files, dropped_context_files) =
            prune_missing_context_files(&prune_root, normalized_context_files);

        let task = self.with_mutation(|| {
            let task = self.stores().tasks().create(StoreTaskCreateParams {
                actor: create_label.clone(),
                parent_id: params.parent_id.clone(),
                title: params.title.clone(),
                description: params.description.clone(),
                acceptance_criteria: params.acceptance_criteria.clone(),
                dependencies: dependencies.clone(),
                tags: normalize_task_tags(params.tags.clone()),
                plan: params.plan.clone(),
                execution_summary: String::new(),
                context_files: kept_context_files.clone(),
                workspace_path: workspace_path.clone(),
                repo_root: None,
                created_by: Some(create_label.clone()),
                planned_by,
                implemented_by: None,
                agent: canonical_agent.clone(),
                model: canonical_model.clone(),
                status: initial_status,
                priority: params.priority,
                complexity: params.complexity,
                task_type,
                external_refs: params.external_refs.clone(),
                source_task_id: params.source_task_id.clone(),
                comments: comments.clone(),
            })?;
            Ok((
                task.clone(),
                OrbitEvent::TaskAdded {
                    id: task.id.clone(),
                },
            ))
        })?;

        if self.scoring_enabled()
            && task_type.counts_toward_friction_bounty()
            && let Some(model) = &canonical_model
        {
            emit_friction_reported_trace(
                &task.id,
                canonical_agent
                    .as_deref()
                    .unwrap_or(effective_label.as_str()),
                model,
                &task.title,
            );
            let _ = friction_bounty::record_friction_reported(&self.paths().scoreboard_dir, model);
        }

        let task = if dropped_context_files.is_empty() {
            task
        } else {
            self.stores().tasks().update(
                &task.id,
                TaskRecordUpdateParams {
                    actor: create_label.clone(),
                    append_history: vec![context_files_pruned_history_entry(
                        &create_label,
                        &dropped_context_files,
                    )],
                    ..Default::default()
                },
            )?
        };

        Ok(task)
    }
}

fn infer_task_create_type_and_status(
    requested_type: Option<TaskType>,
    requested_status: Option<TaskStatus>,
    default_status: TaskStatus,
) -> Result<(TaskType, TaskStatus), OrbitError> {
    if requested_status == Some(TaskStatus::Archived) {
        return Err(OrbitError::InvalidInput(
            "status 'archived' cannot be set at task creation; use the archive command".to_string(),
        ));
    }

    if requested_type == Some(TaskType::Friction)
        && requested_status.is_some_and(|status| status != TaskStatus::Friction)
    {
        let status = requested_status.expect("checked Some");
        return Err(OrbitError::InvalidInput(format!(
            "conflicting friction task creation: type 'friction' requires status 'friction' (got status '{status}')"
        )));
    }

    if requested_status == Some(TaskStatus::Friction)
        && requested_type.is_some_and(|kind| kind != TaskType::Friction)
    {
        let kind = requested_type.expect("checked Some");
        return Err(OrbitError::InvalidInput(format!(
            "conflicting friction task creation: status 'friction' requires type 'friction' (got type '{kind}')"
        )));
    }

    if requested_type == Some(TaskType::Friction) || requested_status == Some(TaskStatus::Friction)
    {
        return Ok((TaskType::Friction, TaskStatus::Friction));
    }

    Ok((
        requested_type.unwrap_or(TaskType::Task),
        requested_status.unwrap_or(default_status),
    ))
}

fn emit_friction_reported_trace(task_id: &str, agent: &str, model: &str, summary: &str) {
    tracing::warn!(
        target: "orbit.friction.reported",
        task_id,
        agent,
        model,
        summary,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::{Arc, Mutex};

    use crate::command::task::TaskUpdateParams;
    use orbit_common::types::{TaskStatus, TaskType};
    use serde_json::Value;
    use tempfile::tempdir;
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    static FRICTION_TRACE_LOCK: Mutex<()> = Mutex::new(());

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

    #[test]
    fn task_add_enters_proposed_and_requires_approval_before_backlog() {
        let (_root, runtime) = test_runtime();

        let task = runtime
            .add_task(TaskAddParams {
                title: "Create orbit hello".to_string(),
                description: "Add a small hello file.".to_string(),
                acceptance_criteria: vec!["orbit-hello.txt exists.".to_string()],
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("human task add succeeds");

        assert_eq!(task.status, TaskStatus::Proposed);

        let approved = runtime
            .approve_task(&task.id, Some("LGTM".to_string()), None)
            .expect("proposed task can be approved into backlog");
        assert_eq!(approved.status, TaskStatus::Backlog);

        let started = runtime
            .start_task(&task.id, Some("start approved task".to_string()), None)
            .expect("backlog task starts directly");
        assert_eq!(started.status, TaskStatus::InProgress);
    }

    #[test]
    fn friction_creation_infers_type_and_status() {
        let (_root, runtime) = test_runtime();

        let by_type = runtime
            .add_task(TaskAddParams {
                title: "By type".to_string(),
                description: "Friction by type.".to_string(),
                task_type: Some(TaskType::Friction),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("type friction infers status");
        assert_eq!(by_type.task_type, TaskType::Friction);
        assert_eq!(by_type.status, TaskStatus::Friction);

        let by_status = runtime
            .add_task(TaskAddParams {
                title: "By status".to_string(),
                description: "Friction by status.".to_string(),
                status: Some(TaskStatus::Friction),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("status friction infers type");
        assert_eq!(by_status.task_type, TaskType::Friction);
        assert_eq!(by_status.status, TaskStatus::Friction);

        let redundant = runtime
            .add_task(TaskAddParams {
                title: "By both".to_string(),
                description: "Friction by both.".to_string(),
                task_type: Some(TaskType::Friction),
                status: Some(TaskStatus::Friction),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("redundant friction type/status is valid");
        assert_eq!(redundant.task_type, TaskType::Friction);
        assert_eq!(redundant.status, TaskStatus::Friction);
    }

    #[test]
    fn friction_creation_rejects_conflicting_type_and_status() {
        let (_root, runtime) = test_runtime();

        let err = runtime
            .add_task(TaskAddParams {
                title: "Conflict one".to_string(),
                description: "Conflicting status.".to_string(),
                task_type: Some(TaskType::Friction),
                status: Some(TaskStatus::Proposed),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect_err("type friction with proposed status should fail");
        assert!(
            err.to_string().contains(
                "conflicting friction task creation: type 'friction' requires status 'friction'"
            ),
            "{err}"
        );

        let err = runtime
            .add_task(TaskAddParams {
                title: "Conflict two".to_string(),
                description: "Conflicting type.".to_string(),
                task_type: Some(TaskType::Bug),
                status: Some(TaskStatus::Friction),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect_err("status friction with bug type should fail");
        assert!(
            err.to_string().contains(
                "conflicting friction task creation: status 'friction' requires type 'friction'"
            ),
            "{err}"
        );
    }

    #[test]
    fn friction_tasks_approve_reject_and_cannot_reenter_friction() {
        let (_root, runtime) = test_runtime();

        let accepted = runtime
            .add_task(TaskAddParams {
                title: "Accept friction".to_string(),
                description: "Accept this report.".to_string(),
                status: Some(TaskStatus::Friction),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("create friction task");
        let accepted = runtime
            .approve_task(&accepted.id, Some("accepted".to_string()), None)
            .expect("approve friction task");
        assert_eq!(accepted.status, TaskStatus::Backlog);
        assert_eq!(accepted.task_type, TaskType::Friction);

        let err = runtime
            .update_task(
                &accepted.id,
                TaskUpdateParams {
                    status: Some(TaskStatus::Friction),
                    ..Default::default()
                },
            )
            .expect_err("friction cannot be restored");
        assert!(err.to_string().contains("friction -> backlog"), "{err}");

        let rejected = runtime
            .add_task(TaskAddParams {
                title: "Reject friction".to_string(),
                description: "Reject this report.".to_string(),
                status: Some(TaskStatus::Friction),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("create friction task");
        let rejected = runtime
            .reject_task(&rejected.id, "invalid report".to_string(), None)
            .expect("reject friction task");
        assert_eq!(rejected.status, TaskStatus::Rejected);
        assert_eq!(rejected.task_type, TaskType::Friction);
    }

    #[test]
    fn scoreboard_refresh_counts_friction_status_history() {
        let _trace_guard = FRICTION_TRACE_LOCK.lock().expect("trace lock");
        let (_root, runtime) = test_runtime();
        let scoreboard_dir = runtime.data_root().join("state").join("scoreboard");
        fs::create_dir_all(&scoreboard_dir).expect("create scoreboard dir");
        fs::write(
            scoreboard_dir.join("friction_bounty.json"),
            r#"{"issues-reported":{"stale":99}}"#,
        )
        .expect("seed stale scoreboard");

        let mut tasks = Vec::new();
        for title in ["reported", "accepted", "rejected", "done"] {
            tasks.push(
                runtime
                    .add_task_with_identity(
                        TaskAddParams {
                            title: title.to_string(),
                            description: "Friction fixture.".to_string(),
                            task_type: Some(TaskType::Friction),
                            workspace_path: Some(".".to_string()),
                            ..Default::default()
                        },
                        Some("codex".to_string()),
                        Some("gpt-fixture".to_string()),
                    )
                    .expect("create friction task"),
            );
        }
        runtime
            .approve_task(&tasks[1].id, Some("accept".to_string()), None)
            .expect("approve one");
        runtime
            .reject_task(&tasks[2].id, "reject".to_string(), None)
            .expect("reject one");
        runtime
            .update_task(
                &tasks[3].id,
                TaskUpdateParams {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .expect("mark one done directly from friction");

        runtime
            .generate_scoreboard_summary()
            .expect("refresh scoreboard");
        let raw = fs::read_to_string(scoreboard_dir.join("friction_bounty.json"))
            .expect("read scoreboard");
        let json: Value = serde_json::from_str(&raw).expect("scoreboard json");
        assert_eq!(json["issues-reported"]["gpt-fixture"], 4);
        assert_eq!(json["issues-accepted"]["gpt-fixture"], 2);
        assert_eq!(json["issues-rejected"]["gpt-fixture"], 1);
        assert!(json["issues-reported"].get("stale").is_none());
    }

    #[test]
    fn legacy_proposed_friction_task_migrates_to_friction_state() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Legacy friction".to_string(),
                description: "Legacy proposed friction.".to_string(),
                status: Some(TaskStatus::Friction),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("create friction task");

        let tasks_dir = runtime.data_root().join("tasks");
        let friction_dir = tasks_dir.join("friction").join(&task.id);
        let proposed_dir = tasks_dir.join("proposed").join(&task.id);
        let task_yaml = friction_dir.join("task.yaml");
        let yaml = fs::read_to_string(&task_yaml).expect("read task yaml");
        fs::write(
            &task_yaml,
            yaml.replace("to_status: friction", "to_status: proposed"),
        )
        .expect("rewrite legacy history");
        fs::write(friction_dir.join("artifacts").join("proof.txt"), "kept")
            .expect("write artifact");
        fs::create_dir_all(proposed_dir.parent().expect("proposed parent"))
            .expect("create proposed dir");
        fs::rename(&friction_dir, &proposed_dir).expect("move to legacy proposed dir");

        let migrated = runtime.get_task(&task.id).expect("migrate and reload");
        assert_eq!(migrated.status, TaskStatus::Friction);
        assert!(tasks_dir.join("friction").join(&task.id).is_dir());
        assert!(!proposed_dir.exists());
        assert_eq!(
            fs::read_to_string(
                tasks_dir
                    .join("friction")
                    .join(&task.id)
                    .join("artifacts")
                    .join("proof.txt")
            )
            .expect("artifact preserved"),
            "kept"
        );
        let created = migrated
            .history
            .iter()
            .find(|entry| entry.event == "created")
            .expect("created history");
        assert_eq!(created.to_status, Some(TaskStatus::Friction));
    }

    #[test]
    fn friction_task_submission_emits_one_tracing_event() {
        let _trace_guard = FRICTION_TRACE_LOCK.lock().expect("trace lock");
        let (_root, runtime) = test_runtime();
        assert!(runtime.scoring_enabled());
        let subscriber = RecordingSubscriber::default();
        let recorder = subscriber.clone();
        let dispatch = tracing::Dispatch::new(subscriber);

        let task = tracing::dispatcher::with_default(&dispatch, || {
            tracing::callsite::rebuild_interest_cache();
            runtime
                .add_task_with_identity(
                    TaskAddParams {
                        title: "Friction reported on ORB-1011".to_string(),
                        description: "Tooling got stuck.".to_string(),
                        acceptance_criteria: vec!["Report is visible.".to_string()],
                        task_type: Some(TaskType::Friction),
                        workspace_path: Some(".".to_string()),
                        ..Default::default()
                    },
                    Some("codex".to_string()),
                    Some("gpt-5.5".to_string()),
                )
                .expect("friction task add succeeds")
        });
        assert_eq!(task.status, TaskStatus::Friction);
        assert_eq!(task.task_type, TaskType::Friction);
        assert_eq!(task.model.as_deref(), Some("gpt-5.5"));

        let events = recorder.events_for_target("orbit.friction.reported");
        assert_eq!(events.len(), 1, "expected exactly one friction event");
        let fields = &events[0].fields;
        assert_eq!(fields.get("task_id"), Some(&task.id));
        assert_eq!(fields.get("agent"), Some(&"codex".to_string()));
        assert_eq!(fields.get("model"), Some(&"gpt-5.5".to_string()));
        assert_eq!(
            fields.get("summary"),
            Some(&"Friction reported on ORB-1011".to_string())
        );
    }

    #[derive(Clone, Default)]
    struct RecordingSubscriber {
        events: Arc<Mutex<Vec<RecordedEvent>>>,
    }

    impl RecordingSubscriber {
        fn events_for_target(&self, target: &str) -> Vec<RecordedEvent> {
            self.events
                .lock()
                .expect("events lock")
                .iter()
                .filter(|event| event.target == target)
                .cloned()
                .collect()
        }
    }

    impl Subscriber for RecordingSubscriber {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut visitor = FieldRecorder::default();
            event.record(&mut visitor);
            self.events
                .lock()
                .expect("events lock")
                .push(RecordedEvent {
                    target: event.metadata().target().to_string(),
                    fields: visitor.fields,
                });
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    #[derive(Clone, Debug)]
    struct RecordedEvent {
        target: String,
        fields: BTreeMap<String, String>,
    }

    #[derive(Default)]
    struct FieldRecorder {
        fields: BTreeMap<String, String>,
    }

    impl Visit for FieldRecorder {
        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .insert(field.name().to_string(), format!("{value:?}"));
        }
    }
}
