use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use orbit_common::types::{
    FrictionStatus, NotFoundKind, OrbitError, Task, TaskComment, TaskPriority, TaskStatus,
    TaskType, ToolSessionContext, is_valid_friction_id, normalize_optional_attribution_label,
    normalize_task_dependencies, normalize_task_tags, optional_csv_or_string_list_alias,
    optional_raw_string, optional_string, optional_string_alias, required_string,
    resolve_task_dependencies, task_dependencies_ready, task_matches_tags,
    validate_task_dependencies,
};
use orbit_common::utility::redaction::redact_all;
use orbit_common::utility::selector::canonical_selector;
use orbit_store::friction_store::{
    FrictionAddParams, FrictionUpdateParams, add_friction, friction_tags,
    prepare_hub_friction_root, readable_hub_friction_root, resolve_friction_by_task,
    update_friction,
};
use orbit_store::sqlite::task_registry::{
    RegisterWorkspaceParams, TaskRegistryStore, task_registry_path,
};
use orbit_store::{
    TaskArtifactUpdateParams, TaskCreateParams, TaskDocumentUpdateParams, TaskHistoryUpdateParams,
    WorkspaceTaskBackends, coordination_task_backends,
};
use orbit_tools::{
    OrbitBuiltinAction, OrbitTaskScope, OrbitToolHost, ReservationOwnerContext, ToolContext,
    ToolRegistry,
};
use serde_json::{Map, Value, json};

use crate::OrbitRuntime;

pub(crate) fn build_orbit_tool_host(
    runtime: &OrbitRuntime,
    task_id: Option<String>,
    run_id: Option<String>,
) -> Arc<dyn OrbitToolHost> {
    Arc::new(RuntimeOrbitToolHost {
        runtime: runtime.clone(),
        task_scope: OrbitTaskScope {
            orbit_root: Some(runtime.data_root_path().to_path_buf()),
            task_id,
            run_id: run_id.or_else(trusted_env_run_id),
        },
    })
}

#[derive(Clone)]
struct RuntimeOrbitToolHost {
    runtime: OrbitRuntime,
    task_scope: OrbitTaskScope,
}

/// Checkout-independent executor for coordination-authoritative hub tools.
///
/// It deliberately has no `OrbitRuntime`, `WorkspacePaths`, local configuration,
/// owner store, or fabricated checkout. The stable logical workspace ID is the
/// sole task/friction partition key.
#[derive(Clone)]
pub struct HubCoordinationExecutor {
    inner: Arc<HubCoordinationState>,
}

struct HubCoordinationState {
    global_root: PathBuf,
    workspace_id: String,
    legacy_friction_root: Option<PathBuf>,
    tasks: WorkspaceTaskBackends,
}

impl HubCoordinationExecutor {
    /// Registers the path-free task-registry partition for a logical workspace.
    /// Workspace initialization calls this once; identical repeats are safe.
    pub fn register_workspace(
        global_root: &Path,
        workspace_id: impl Into<String>,
        slug: impl Into<String>,
    ) -> Result<(), OrbitError> {
        let registry = TaskRegistryStore::open(&task_registry_path(global_root))?;
        registry.register_workspace(RegisterWorkspaceParams {
            workspace_id: workspace_id.into(),
            slug: slug.into(),
            repo_fingerprint: None,
        })?;
        Ok(())
    }

    pub fn new(
        global_root: &Path,
        workspace_id: impl Into<String>,
        legacy_friction_root: Option<PathBuf>,
    ) -> Result<Self, OrbitError> {
        let workspace_id = workspace_id.into();
        let registry = TaskRegistryStore::open(&task_registry_path(global_root))?;
        let tasks = coordination_task_backends(registry, workspace_id.clone());
        Ok(Self {
            inner: Arc::new(HubCoordinationState {
                global_root: global_root.to_path_buf(),
                workspace_id,
                legacy_friction_root,
                tasks,
            }),
        })
    }

    pub fn execute_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        let mut registry = ToolRegistry::new();
        registry.register_builtins();
        let context = ToolContext {
            cwd: std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().into_owned()),
            session_context,
            orbit_host: Some(Arc::new(self.clone())),
            ..Default::default()
        };
        registry.execute(name, &context, input)
    }

    fn actor(agent: Option<&str>, model: Option<&str>) -> String {
        normalize_optional_attribution_label(model.or(agent), model)
            .unwrap_or_else(|| "unknown".to_string())
    }

    fn task(&self, id: &str) -> Result<Task, OrbitError> {
        self.inner
            .tasks
            .task
            .get_task(id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, id.to_string()))
    }

    fn task_json(&self, task: &Task) -> Result<Value, OrbitError> {
        let status = self.inner.tasks.task.task_status_index()?;
        let mut value = super::json::task_to_json(task, &status);
        let object = value.as_object_mut().ok_or_else(|| {
            OrbitError::Execution("task JSON projection did not produce an object".to_string())
        })?;
        object.insert(
            "comments".to_string(),
            serde_json::to_value(
                self.inner
                    .tasks
                    .history
                    .get_task_comments(&task.id)?
                    .unwrap_or_default(),
            )
            .map_err(|error| OrbitError::Execution(format!("serialize comments: {error}")))?,
        );
        object.insert(
            "history".to_string(),
            serde_json::to_value(
                self.inner
                    .tasks
                    .history
                    .get_task_history(&task.id)?
                    .unwrap_or_default(),
            )
            .map_err(|error| OrbitError::Execution(format!("serialize history: {error}")))?,
        );
        Ok(value)
    }

    fn add_task(
        &self,
        input: Value,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Value, OrbitError> {
        let actor = Self::actor(agent.as_deref(), model.as_deref());
        let context_files = optional_csv_or_string_list_alias(&input, &["context_files"])?
            .unwrap_or_default()
            .into_iter()
            .map(|entry| {
                canonical_selector(&entry)
                    .map_err(|error| OrbitError::InvalidInput(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let relations = super::input::parse_relations(&input)?.unwrap_or_default();
        let task_type = optional_string_alias(&input, &["type", "task_type", "taskType"])?
            .map(|value| super::input::parse_task_type("type", &value))
            .transpose()?
            .unwrap_or(TaskType::Feature);
        let title = redact_all(&required_string(&input, &["title"], "title")?);
        let description = redact_all(&required_string(&input, &["description"], "description")?);
        let acceptance_criteria = optional_csv_or_string_list_alias(
            &input,
            &[
                "acceptance_criteria",
                "acceptanceCriteria",
                "acceptance-criteria",
            ],
        )?
        .unwrap_or_default()
        .into_iter()
        .map(|criterion| redact_all(&criterion))
        .collect();
        let task = self.inner.tasks.task.create_task(TaskCreateParams {
            actor: actor.clone(),
            parent_id: None,
            title,
            description,
            acceptance_criteria,
            dependencies: Vec::new(),
            relations,
            tags: normalize_task_tags(
                optional_csv_or_string_list_alias(&input, &["tags", "tag"])?.unwrap_or_default(),
            ),
            plan: String::new(),
            execution_summary: String::new(),
            context_files,
            workspace_path: None,
            repo_root: None,
            created_by: Some(actor),
            planned_by: None,
            implemented_by: None,
            status: TaskStatus::Proposed,
            priority: optional_string(&input, "priority")?
                .map(|value| super::input::parse_task_priority("priority", &value))
                .transpose()?
                .unwrap_or(TaskPriority::Medium),
            complexity: optional_string(&input, "complexity")?
                .map(|value| super::input::parse_task_complexity("complexity", &value))
                .transpose()?,
            task_type,
            external_refs: Vec::new(),
            source_task_id: None,
            crew: optional_string(&input, "crew")?,
            comments: Vec::new(),
        })?;
        self.task_json(&task)
    }

    fn update_task(
        &self,
        input: Value,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Value, OrbitError> {
        let id = required_string(&input, &["id"], "id")?;
        let current = self.task(&id)?;
        let actor = Self::actor(agent.as_deref(), model.as_deref());
        let status = optional_string(&input, "status")?
            .map(|value| super::input::parse_task_status("status", &value))
            .transpose()?;
        let has_non_status_mutation = [
            "title",
            "description",
            "acceptance_criteria",
            "dependencies",
            "relations",
            "tags",
            "plan",
            "execution_summary",
            "type",
            "source_task_id",
            "planned_by",
            "implemented_by",
            "pr_status",
            "job_run_id",
            "crew",
            "context_files",
            "context",
            "artifacts",
            "artifact",
        ]
        .iter()
        .any(|field| input.get(*field).is_some());
        let unarchiving = current.status == TaskStatus::Archived
            && status == Some(TaskStatus::Backlog)
            && !has_non_status_mutation;
        if current.status == TaskStatus::Archived && !unarchiving {
            return Err(OrbitError::InvalidInput(format!(
                "task {id} is archived and cannot be modified; restore it to backlog first"
            )));
        }
        if current.status == TaskStatus::Done && (status.is_some() || has_non_status_mutation) {
            return Err(OrbitError::InvalidInput(format!(
                "task {id} is done and cannot be modified; done is terminal"
            )));
        }
        if let Some(target) = status {
            current
                .status
                .validate_transition(target)
                .map_err(OrbitError::TaskStatusTransition)?;
            if target == TaskStatus::InProgress
                && current.status != TaskStatus::InProgress
                && input
                    .get("plan")
                    .and_then(Value::as_str)
                    .unwrap_or(&current.plan)
                    .trim()
                    .is_empty()
            {
                return Err(OrbitError::InvalidInput(format!(
                    "task '{id}' requires a non-empty plan before entering in-progress"
                )));
            }
            if current.status == TaskStatus::InProgress
                && target == TaskStatus::Review
                && optional_raw_string(&input, "execution_summary")?
                    .as_deref()
                    .unwrap_or(&current.execution_summary)
                    .trim()
                    .is_empty()
            {
                return Err(OrbitError::InvalidInput(format!(
                    "task '{id}' requires non-empty execution_summary before transitioning in-progress -> review"
                )));
            }
        }
        let dependencies = optional_csv_or_string_list_alias(&input, &["dependencies"])?
            .map(normalize_task_dependencies)
            .transpose()?;
        if let Some(dependencies) = dependencies.as_ref() {
            validate_task_dependencies(
                &self.inner.tasks.task.list_tasks()?,
                Some(&id),
                dependencies,
            )?;
        }
        let context_files =
            optional_csv_or_string_list_alias(&input, &["context_files", "context"])?
                .map(|entries| {
                    entries
                        .into_iter()
                        .map(|entry| {
                            canonical_selector(&entry)
                                .map_err(|error| OrbitError::InvalidInput(error.to_string()))
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?;
        let artifacts = super::input::parse_artifacts(&input)?;
        let relations = super::input::parse_relations(&input)?;
        let comment = optional_string(&input, "comment")?;
        let task_type = optional_string_alias(&input, &["type", "task_type", "taskType"])?
            .map(|value| super::input::parse_task_type("type", &value))
            .transpose()?;
        let description = input
            .get("description")
            .map(|value| {
                value.as_str().map(redact_all).ok_or_else(|| {
                    OrbitError::InvalidInput("`description` must be a string".to_string())
                })
            })
            .transpose()?;
        let plan = input
            .get("plan")
            .map(|value| {
                value
                    .as_str()
                    .map(redact_all)
                    .ok_or_else(|| OrbitError::InvalidInput("`plan` must be a string".to_string()))
            })
            .transpose()?;
        let explicit_planned_by = raw_clearable(&input, "planned_by")?;
        let explicit_implemented_by = raw_clearable(&input, "implemented_by")?;
        self.inner.tasks.document.update_task_document(
            &id,
            TaskDocumentUpdateParams {
                actor: actor.clone(),
                title: optional_string(&input, "title")?.map(|value| redact_all(&value)),
                description,
                acceptance_criteria: optional_csv_or_string_list_alias(
                    &input,
                    &[
                        "acceptance_criteria",
                        "acceptanceCriteria",
                        "acceptance-criteria",
                    ],
                )?
                .map(|values| values.into_iter().map(|value| redact_all(&value)).collect()),
                dependencies,
                relations,
                tags: optional_csv_or_string_list_alias(&input, &["tags", "tag"])?
                    .map(normalize_task_tags),
                plan,
                execution_summary: optional_raw_string(&input, "execution_summary")?
                    .map(|value| redact_all(&value)),
                context_files,
                planned_by: explicit_planned_by
                    .or_else(|| input.get("plan").is_some().then(|| Some(actor.clone()))),
                implemented_by: explicit_implemented_by.or_else(|| {
                    status
                        .is_some_and(|value| matches!(value, TaskStatus::Review | TaskStatus::Done))
                        .then(|| Some(actor.clone()))
                }),
                priority: None,
                complexity: None,
                task_type,
                external_refs: None,
                pr_status: raw_clearable(&input, "pr_status")?,
                source_task_id: raw_clearable(&input, "source_task_id")?,
                job_run_id: raw_clearable(&input, "job_run_id")?,
                crew: raw_clearable(&input, "crew")?,
                created_by: None,
            },
        )?;
        if !artifacts.is_empty() {
            self.inner.tasks.artifact.upsert_task_artifacts(
                &id,
                TaskArtifactUpdateParams {
                    actor: actor.clone(),
                    upsert_artifacts: artifacts,
                },
            )?;
        }
        if status.is_some() || comment.is_some() {
            let append_comments = comment
                .map(|message| TaskComment {
                    at: Utc::now(),
                    by: actor.clone(),
                    message: redact_all(message.trim()),
                })
                .into_iter()
                .collect();
            self.inner.tasks.history.update_task_history(
                &id,
                TaskHistoryUpdateParams {
                    actor,
                    status,
                    status_event: status.map(|_| "updated".to_string()),
                    status_note: None,
                    append_history: Vec::new(),
                    append_comments,
                },
            )?;
        }
        let updated = self.task(&id)?;
        if updated.status == TaskStatus::Done
            && current.status != TaskStatus::Done
            && let Ok(root) = self.friction_root()
        {
            for relation in &updated.relations {
                if relation.relation_type == orbit_common::types::TaskRelationType::Resolves
                    && is_valid_friction_id(&relation.target)
                    && let Err(error) =
                        resolve_friction_by_task(&root, &relation.target, &id, Utc::now())
                {
                    tracing::warn!(
                        task_id = id,
                        friction_id = relation.target,
                        error = %error,
                        "checkoutless task completion could not apply friction side effect"
                    );
                }
            }
        }
        self.task_json(&updated)
    }

    fn transition(
        &self,
        input: Value,
        agent: Option<String>,
        model: Option<String>,
        start: bool,
    ) -> Result<Value, OrbitError> {
        let id = required_string(&input, &["id"], "id")?;
        let task = self.task(&id)?;
        let target = if start {
            if !matches!(
                task.status,
                TaskStatus::Proposed
                    | TaskStatus::Backlog
                    | TaskStatus::Someday
                    | TaskStatus::Blocked
            ) {
                return Err(OrbitError::InvalidInput(format!(
                    "task '{id}' is in status '{}'; start requires proposed, backlog, someday, or blocked",
                    task.status
                )));
            }
            if task.plan.trim().is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "task '{id}' requires a non-empty plan before entering in-progress"
                )));
            }
            TaskStatus::InProgress
        } else {
            match task.status {
                TaskStatus::Proposed => TaskStatus::Backlog,
                TaskStatus::Review => TaskStatus::Done,
                other => {
                    return Err(OrbitError::InvalidInput(format!(
                        "task '{id}' is in status '{other}'; approve requires proposed or review"
                    )));
                }
            }
        };
        let mut update = Map::new();
        update.insert("id".to_string(), Value::String(id));
        update.insert("status".to_string(), Value::String(target.to_string()));
        if let Some(note) = optional_string(&input, "note")? {
            update.insert("comment".to_string(), Value::String(note));
        } else if let Some(comment) = optional_string(&input, "comment")? {
            update.insert("comment".to_string(), Value::String(comment));
        }
        if let Some(crew) = optional_string(&input, "crew")? {
            update.insert("crew".to_string(), Value::String(crew));
        }
        self.update_task(Value::Object(update), agent, model)
    }

    fn show_task(&self, input: Value) -> Result<Value, OrbitError> {
        if input
            .get("with_context")
            .or_else(|| input.get("withContext"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(OrbitError::InvalidInput(
                "checkoutless hub execution cannot provide `with_context` local-derived enrichment"
                    .to_string(),
            ));
        }
        let id = required_string(&input, &["id"], "id")?;
        let task = self.task(&id)?;
        let Some(fields) = optional_csv_or_string_list_alias(&input, &["fields", "field"])? else {
            return self.task_json(&task);
        };
        let status = self.inner.tasks.task.task_status_index()?;
        let one = |field: &str| -> Result<Value, OrbitError> {
            match field {
                "comments" => serde_json::to_value(
                    self.inner
                        .tasks
                        .history
                        .get_task_comments(&id)?
                        .unwrap_or_default(),
                )
                .map_err(|error| OrbitError::Execution(error.to_string())),
                "history" => serde_json::to_value(
                    self.inner
                        .tasks
                        .history
                        .get_task_history(&id)?
                        .unwrap_or_default(),
                )
                .map_err(|error| OrbitError::Execution(error.to_string())),
                "artifacts" => Ok(super::json::serialize_task_artifacts(
                    &self
                        .inner
                        .tasks
                        .artifact
                        .get_task_artifacts(&id)?
                        .unwrap_or_default(),
                )),
                "plan" => Ok(json!(task.plan)),
                "execution_summary" => Ok(json!(task.execution_summary)),
                "description" => Ok(json!(task.description)),
                "acceptance_criteria" => Ok(json!(task.acceptance_criteria)),
                "dependencies" => Ok(json!(task.dependencies())),
                "resolved_dependencies" => Ok(json!(
                    resolve_task_dependencies(&task, &status)
                        .into_iter()
                        .map(|entry| entry.label())
                        .collect::<Vec<_>>()
                )),
                "tags" => Ok(json!(task.tags)),
                "context_files" => Ok(json!(task.context_files)),
                other => Err(OrbitError::InvalidInput(format!(
                    "unknown field selector `{other}`"
                ))),
            }
        };
        if fields.len() == 1 {
            return one(&fields[0]);
        }
        let mut object = Map::new();
        for field in fields {
            object.insert(field.clone(), one(&field)?);
        }
        Ok(Value::Object(object))
    }

    fn list_tasks(&self, input: Value) -> Result<Value, OrbitError> {
        let status_filter = optional_string(&input, "status")?
            .map(|value| super::input::parse_task_status("status", &value))
            .transpose()?;
        let type_filter = optional_string_alias(&input, &["type", "task_type", "taskType"])?
            .map(|value| super::input::parse_task_type("type", &value))
            .transpose()?;
        let tags = optional_csv_or_string_list_alias(&input, &["tags", "tag"])?.unwrap_or_default();
        let ready = super::input::optional_bool_alias(&input, &["ready"])?;
        let limit = super::input::task_list_limit(&input)?;
        let status = self.inner.tasks.task.task_status_index()?;
        // `list_tasks()` returns tasks newest-first (`created_at DESC`, task ID
        // ascending for ties); the filters preserve that order, so `take(limit)`
        // yields the newest matching tasks (ORB-10310).
        let tasks = self
            .inner
            .tasks
            .task
            .list_tasks()?
            .into_iter()
            .filter(|task| status_filter.is_none_or(|value| task.status == value))
            .filter(|task| type_filter.is_none_or(|value| task.task_type == value))
            .filter(|task| task_matches_tags(task, &tags))
            .filter(|task| ready != Some(true) || task_dependencies_ready(task, &status))
            .take(limit)
            .map(|task| super::json::task_to_json(&task, &status))
            .collect();
        Ok(Value::Array(tasks))
    }

    fn friction_root(&self) -> Result<PathBuf, OrbitError> {
        prepare_hub_friction_root(
            &self.inner.global_root,
            &self.inner.workspace_id,
            self.inner.legacy_friction_root.as_deref(),
        )
    }

    fn readable_friction_root(&self) -> Result<PathBuf, OrbitError> {
        readable_hub_friction_root(
            &self.inner.global_root,
            &self.inner.workspace_id,
            self.inner.legacy_friction_root.as_deref(),
        )
    }

    fn friction(
        &self,
        action: OrbitBuiltinAction,
        input: Value,
        model: Option<String>,
    ) -> Result<Value, OrbitError> {
        match action {
            OrbitBuiltinAction::FrictionList => {
                let root = self.readable_friction_root()?;
                let mut value = super::friction_tools::list_at_root(&root, input)?;
                strip_private_friction_paths(&mut value);
                Ok(value)
            }
            OrbitBuiltinAction::FrictionShow => {
                let root = self.readable_friction_root()?;
                let mut value = super::friction_tools::show_at_root(&root, input)?;
                strip_private_friction_paths(&mut value);
                Ok(value)
            }
            OrbitBuiltinAction::FrictionTags => {
                let root = self.readable_friction_root()?;
                Ok(json!(friction_tags(&root)?))
            }
            OrbitBuiltinAction::FrictionAdd => {
                let root = self.friction_root()?;
                let model = model
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| {
                        OrbitError::InvalidInput("orbit.friction.add requires `model`".to_string())
                    })?;
                let stored = add_friction(
                    &root,
                    FrictionAddParams {
                        model,
                        body: redact_all(&required_string(
                            &input,
                            &["body", "description"],
                            "body",
                        )?),
                        tags: optional_csv_or_string_list_alias(&input, &["tags", "tag"])?
                            .unwrap_or_default(),
                        during_task: optional_string(&input, "during_task")?
                            .or(optional_string(&input, "task_id")?),
                        created_at: Utc::now(),
                    },
                )?;
                friction_json(stored)
            }
            OrbitBuiltinAction::FrictionUpdate => {
                let root = self.friction_root()?;
                let id = required_string(&input, &["id"], "id")?;
                let status = optional_string(&input, "status")?
                    .map(|value| {
                        FrictionStatus::from_str(&value)
                            .map_err(|error| OrbitError::InvalidInput(format!("`status` {error}")))
                    })
                    .transpose()?;
                let tags = optional_csv_or_string_list_alias(&input, &["tags", "tag"])?;
                let body = optional_string(&input, "body")?.map(|value| redact_all(&value));
                if status.is_none() && tags.is_none() && body.is_none() {
                    return Err(OrbitError::InvalidInput(
                        "orbit.friction.update requires `status`, `tags`, or `body`".to_string(),
                    ));
                }
                friction_json(update_friction(
                    &root,
                    &id,
                    FrictionUpdateParams {
                        status,
                        tags,
                        body,
                        resolved_by_task: None,
                        updated_at: Utc::now(),
                    },
                )?)
            }
            _ => unreachable!(),
        }
    }
}

impl OrbitToolHost for HubCoordinationExecutor {
    fn execute(
        &self,
        action: OrbitBuiltinAction,
        input: Value,
        agent: Option<String>,
        model: Option<String>,
        _reservation_owner: Option<ReservationOwnerContext>,
    ) -> Result<Value, OrbitError> {
        let (input, _redaction_report) =
            super::artifact_redaction::sanitize_tool_input(action, input)?;
        match action {
            OrbitBuiltinAction::TaskAdd => self.add_task(input, agent, model),
            OrbitBuiltinAction::TaskApprove => self.transition(input, agent, model, false),
            OrbitBuiltinAction::TaskStart => self.transition(input, agent, model, true),
            OrbitBuiltinAction::TaskShow => self.show_task(input),
            OrbitBuiltinAction::TaskList => self.list_tasks(input),
            OrbitBuiltinAction::TaskUpdate => self.update_task(input, agent, model),
            OrbitBuiltinAction::FrictionAdd
            | OrbitBuiltinAction::FrictionList
            | OrbitBuiltinAction::FrictionShow
            | OrbitBuiltinAction::FrictionTags
            | OrbitBuiltinAction::FrictionUpdate => self.friction(action, input, model),
            _ => Err(OrbitError::InvalidInput(format!(
                "action {action:?} is outside the checkoutless hub coordination executor"
            ))),
        }
    }

    fn task_scope(&self) -> OrbitTaskScope {
        OrbitTaskScope::default()
    }
}

fn raw_clearable(input: &Value, field: &str) -> Result<Option<Option<String>>, OrbitError> {
    Ok(optional_raw_string(input, field)?.map(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    }))
}

fn friction_json(
    stored: orbit_store::friction_store::StoredFrictionRecord,
) -> Result<Value, OrbitError> {
    serde_json::to_value(&stored.record)
        .map_err(|error| OrbitError::Store(format!("serialize friction record: {error}")))
}

fn strip_private_friction_paths(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(strip_private_friction_paths),
        Value::Object(object) => {
            object.remove("path");
        }
        _ => {}
    }
}

impl OrbitToolHost for RuntimeOrbitToolHost {
    fn execute(
        &self,
        action: OrbitBuiltinAction,
        input: Value,
        agent: Option<String>,
        model: Option<String>,
        reservation_owner: Option<ReservationOwnerContext>,
    ) -> Result<Value, OrbitError> {
        let (agent, model) = self
            .runtime
            .try_canonical_agent_model_identity(agent.as_deref(), model.as_deref())?;
        super::dispatch::execute(
            &self.runtime,
            &self.task_scope,
            action,
            input,
            agent,
            model,
            reservation_owner,
        )
    }

    fn task_scope(&self) -> OrbitTaskScope {
        self.task_scope.clone()
    }
}

fn trusted_env_run_id() -> Option<String> {
    let managed = std::env::var("ORBIT_MANAGED_RUN_CONTEXT")
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE"));
    if !managed {
        return None;
    }
    std::env::var("ORBIT_RUN_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod checkoutless_hub_tests {
    use super::*;

    fn executor() -> (
        tempfile::TempDir,
        HubCoordinationExecutor,
        ToolSessionContext,
    ) {
        let root = tempfile::tempdir().expect("global root");
        HubCoordinationExecutor::register_workspace(root.path(), "ws_checkoutless", "checkoutless")
            .expect("register workspace");
        let executor = HubCoordinationExecutor::new(root.path(), "ws_checkoutless", None)
            .expect("coordination executor");
        let context = ToolSessionContext::trusted_local(
            Some("ws_checkoutless".to_string()),
            Some("hm_hub".to_string()),
            Some("hub".to_string()),
        );
        (root, executor, context)
    }

    #[test]
    fn task_artifact_review_and_verdict_persist_without_checkout() {
        let (root, executor, context) = executor();
        let created = executor
            .execute_tool(
                "orbit.task.add",
                json!({
                    "workspace": "ws_checkoutless",
                    "title": "Coordinate from hub",
                    "description": "No checkout required",
                    "context_files": ["future/new.rs", "symbol:src/lib.rs#Host:struct"],
                    "model": "codex"
                }),
                context.clone(),
            )
            .expect("add checkoutless task");
        let id = created["id"].as_str().expect("task id");
        assert_eq!(created["context_files"][0], "file:future/new.rs");
        assert!(created.get("resolved_crew").is_none());

        executor
            .execute_tool(
                "orbit.task.update",
                json!({
                    "id": id,
                    "plan": "1. Execute",
                    "status": "in_progress",
                    "execution_summary": "Outcome: success",
                    "model": "codex"
                }),
                context.clone(),
            )
            .expect("persist verdict fields");

        let source = root.path().join("caller.txt");
        std::fs::write(&source, "payload").expect("caller artifact");
        let with_artifact = executor
            .execute_tool(
                "orbit.task.artifact.put",
                json!({
                    "id": id,
                    "source_path": source,
                    "path": "reports/result.txt",
                    "model": "codex"
                }),
                context.clone(),
            )
            .expect("put artifact bytes");
        assert_eq!(with_artifact["execution_summary"], "Outcome: success");

        let artifact = executor
            .execute_tool(
                "orbit.task.show",
                json!({"id": id, "fields": "artifacts"}),
                context,
            )
            .expect("show artifact");
        assert_eq!(artifact[0]["path"], "reports/result.txt");
        assert_eq!(artifact[0]["content"], "payload");
        assert!(
            !root.path().join(".orbit").exists(),
            "no fabricated checkout"
        );
    }

    #[test]
    fn friction_writes_use_canonical_workspace_partition() {
        let (root, executor, context) = executor();
        let result = executor
            .execute_tool(
                "orbit.friction.add",
                json!({"body": "Hub friction", "tags": ["tooling"], "model": "codex"}),
                context,
            )
            .expect("add friction");
        assert!(result.get("path").is_none(), "hub responses are path-free");
        let month = Utc::now().format("%Y-%m").to_string();
        let directory = root
            .path()
            .join("frictions/workspaces/ws_checkoutless")
            .join(month);
        assert!(
            std::fs::read_dir(directory)
                .expect("workspace friction month")
                .flatten()
                .any(|entry| entry.path().extension().is_some_and(|ext| ext == "md")),
            "friction persisted in the canonical workspace partition"
        );
    }
}
