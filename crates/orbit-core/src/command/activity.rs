use std::collections::BTreeSet;
use std::path::Path;

use orbit_store::ActivityCreateParams as StoreWorkCreateParams;
use orbit_store::ActivityUpdateParams as StoreActivityUpdateParams;
use orbit_types::{
    Activity, ActivityResource, JobRunState, OrbitError, OrbitEvent, RESOURCE_SCHEMA_VERSION,
    ResourceKind,
};
use serde_json::Value;

use crate::OrbitRuntime;
use crate::paths::ORBIT_ROOT_TOKEN;

pub(crate) const DEFAULT_ACTIVITY_FILES: &[(&str, &str)] = &[
    // ---- retained internal activities (still referenced by jobs) ----
    (
        "update_task",
        include_str!("../../assets/activities/automation/update_task.yaml"),
    ),
    (
        "parallel_dispatch_tasks",
        include_str!("../../assets/activities/automation/parallel_dispatch_tasks.yaml"),
    ),
    (
        "select_duel_roles",
        include_str!("../../assets/activities/automation/select_duel_roles.yaml"),
    ),
    (
        "run_planning_duel",
        include_str!("../../assets/activities/automation/run_planning_duel.yaml"),
    ),
    (
        "record_duel_scores",
        include_str!("../../assets/activities/automation/record_duel_scores.yaml"),
    ),
    // ---- generic built-in automation activities ----
    (
        "git_commit",
        include_str!("../../assets/activities/automation/git_commit.yaml"),
    ),
    (
        "git_push",
        include_str!("../../assets/activities/automation/git_push.yaml"),
    ),
    (
        "git_pull",
        include_str!("../../assets/activities/automation/git_pull.yaml"),
    ),
    (
        "git_merge",
        include_str!("../../assets/activities/automation/git_merge.yaml"),
    ),
    (
        "worktree_setup",
        include_str!("../../assets/activities/automation/worktree_setup.yaml"),
    ),
    (
        "worktree_cleanup",
        include_str!("../../assets/activities/automation/worktree_cleanup.yaml"),
    ),
    (
        "pr_open",
        include_str!("../../assets/activities/automation/pr_open.yaml"),
    ),
    (
        "pr_sync_reviews",
        include_str!("../../assets/activities/automation/pr_sync_reviews.yaml"),
    ),
    (
        "check_task_value",
        include_str!("../../assets/activities/automation/check_task_value.yaml"),
    ),
    (
        "dispatch_batch",
        include_str!("../../assets/activities/automation/dispatch_batch.yaml"),
    ),
    (
        "run_command",
        include_str!("../../assets/activities/automation/run_command.yaml"),
    ),
    // ---- new generic agent activities ----
    (
        "agent_implement",
        include_str!("../../assets/activities/agent_invoke/agent_implement.yaml"),
    ),
    (
        "agent_implement_fix",
        include_str!("../../assets/activities/agent_invoke/agent_implement_fix.yaml"),
    ),
    (
        "finalize_tasks",
        include_str!("../../assets/activities/agent_invoke/finalize_tasks.yaml"),
    ),
    (
        "agent_plan",
        include_str!("../../assets/activities/agent_invoke/agent_plan.yaml"),
    ),
    (
        "agent_review",
        include_str!("../../assets/activities/agent_invoke/agent_review.yaml"),
    ),
    (
        "agent_batch_review",
        include_str!("../../assets/activities/agent_invoke/agent_batch_review.yaml"),
    ),
    (
        "agent_arbitrate",
        include_str!("../../assets/activities/agent_invoke/agent_arbitrate.yaml"),
    ),
];

const VALID_ACTIVITY_SPEC_TYPES: &[&str] = &["agent_invoke", "cli_command", "automation"];

#[derive(Debug, Clone)]
pub struct ActivityAddParams {
    pub id: String,
    pub spec_type: String,
    pub description: String,
    pub input_schema_json: Value,
    pub output_schema_json: Value,
    pub spec_config: Value,
    pub executor: Option<String>,
    pub workspace_path: Option<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ActivityUpdateParams {
    pub description: Option<String>,
    pub input_schema_json: Option<Value>,
    pub output_schema_json: Option<Value>,
    pub spec_config: Option<Value>,
    pub executor: Option<Option<String>>,
    pub workspace_path: Option<Option<String>>,
    pub created_by: Option<Option<String>>,
    pub is_active: Option<bool>,
}

impl From<ActivityAddParams> for StoreWorkCreateParams {
    fn from(p: ActivityAddParams) -> Self {
        Self {
            id: p.id,
            spec_type: p.spec_type,
            description: p.description,
            input_schema_json: p.input_schema_json,
            output_schema_json: p.output_schema_json,
            spec_config: p.spec_config,
            executor: p.executor,
            workspace_path: p.workspace_path,
            created_by: p.created_by,
        }
    }
}

impl From<ActivityUpdateParams> for StoreActivityUpdateParams {
    fn from(p: ActivityUpdateParams) -> Self {
        Self {
            description: p.description,
            input_schema_json: p.input_schema_json,
            output_schema_json: p.output_schema_json,
            spec_config: p.spec_config,
            executor: p.executor,
            workspace_path: p.workspace_path,
            created_by: p.created_by,
            is_active: p.is_active,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActivityRunParams {
    pub activity_id: String,
    pub agent_cli: String,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct ActivityRunResult {
    pub activity_id: String,
    pub state: JobRunState,
    pub duration_ms: Option<u64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

impl OrbitRuntime {
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

    pub fn add_activity(&self, params: ActivityAddParams) -> Result<Activity, OrbitError> {
        validate_activity_params(&params)?;
        let skill_refs = activity_skill_refs_from_spec_config(&params.spec_config)?;
        let _ = self.resolve_activity_skill_refs(&skill_refs)?;
        let activity = self.stores().activities().add(params.into())?;
        self.record_event(OrbitEvent::ActivityAdded {
            id: activity.id.clone(),
        })?;
        Ok(activity)
    }

    pub fn list_activities(&self, include_inactive: bool) -> Result<Vec<Activity>, OrbitError> {
        self.stores().activities().list(include_inactive)
    }

    pub fn show_activity(&self, id: &str) -> Result<Activity, OrbitError> {
        self.stores()
            .activities()
            .get(id)?
            .ok_or_else(|| OrbitError::ActivityNotFound(id.to_string()))
    }

    pub fn update_activity(
        &self,
        id: &str,
        params: ActivityUpdateParams,
    ) -> Result<Activity, OrbitError> {
        if let Some(spec_config) = params.spec_config.as_ref() {
            ensure_spec_config_object(spec_config)?;
            let skill_refs = activity_skill_refs_from_spec_config(spec_config)?;
            let _ = self.resolve_activity_skill_refs(&skill_refs)?;
        }

        let activity = self.stores().activities().update(id, params.into())?;
        self.record_event(OrbitEvent::ActivityUpdated {
            id: activity.id.clone(),
        })?;
        Ok(activity)
    }

    pub fn delete_activity(&self, id: &str) -> Result<(), OrbitError> {
        let changed = self.stores().activities().disable(id)?;
        if !changed {
            return Err(OrbitError::ActivityNotFound(id.to_string()));
        }
        self.record_event(OrbitEvent::ActivityDisabled { id: id.to_string() })
    }

    pub fn run_activity_now(
        &self,
        params: ActivityRunParams,
    ) -> Result<ActivityRunResult, OrbitError> {
        if params.activity_id.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "activity id must not be empty".to_string(),
            ));
        }

        let activity = self.show_activity(&params.activity_id)?;
        validate_activity_executor_for_run(self, &activity)?;
        if activity_requires_agent_cli(&activity.spec_type)
            && params.agent_cli.trim().is_empty()
            && !activity_has_executor(&activity)
        {
            return Err(OrbitError::InvalidInput(
                "agent_cli must not be empty for agent_invoke activities without an explicit executor"
                    .to_string(),
            ));
        }
        self.record_event(OrbitEvent::ActivityRunStarted {
            id: activity.id.clone(),
        })?;
        let outcome = orbit_engine::run_activity_direct(
            self,
            &activity,
            &params.agent_cli,
            params.timeout_seconds,
            false,
        )?;
        if outcome.protocol_violation {
            self.record_event(OrbitEvent::ActivityProtocolViolation {
                id: activity.id.clone(),
                message: outcome
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "agent protocol violation".to_string()),
            })?;
        }
        self.record_event(OrbitEvent::ActivityRunCompleted {
            id: activity.id.clone(),
            state: outcome.state.to_string(),
        })?;

        Ok(ActivityRunResult {
            activity_id: activity.id,
            state: outcome.state,
            duration_ms: outcome.duration_ms,
            error_code: outcome.error_code,
            error_message: outcome.error_message,
        })
    }
}

pub(crate) fn seed_default_activities(
    runtime: &OrbitRuntime,
    global_root: &Path,
    overwrite: bool,
) -> Result<usize, OrbitError> {
    let specs = load_default_activity_specs(DEFAULT_ACTIVITY_FILES, Some(global_root))?;
    seed_default_activities_from_specs(runtime, &specs, overwrite)
}

fn load_default_activity_specs(
    raw_specs: &[(&str, &str)],
    orbit_root: Option<&Path>,
) -> Result<Vec<ActivityResource>, OrbitError> {
    let mut specs = Vec::with_capacity(raw_specs.len());
    let mut ids = BTreeSet::new();
    for (expected_id, raw) in raw_specs {
        let rendered = match orbit_root {
            Some(root) => inject_activity_template_tokens(raw, root),
            None => (*raw).to_string(),
        };
        let resource = serde_yaml::from_str::<ActivityResource>(&rendered).map_err(|err| {
            OrbitError::InvalidInput(format!(
                "invalid default activity spec '{}': {err}",
                expected_id
            ))
        })?;
        if resource.schema_version != RESOURCE_SCHEMA_VERSION {
            return Err(OrbitError::InvalidInput(format!(
                "default activity spec '{}' uses unsupported schemaVersion {}",
                expected_id, resource.schema_version
            )));
        }
        if resource.kind != ResourceKind::Activity {
            return Err(OrbitError::InvalidInput(format!(
                "default activity spec '{}' has unexpected kind {}",
                expected_id, resource.kind
            )));
        }
        let id = resource.metadata.name.trim();
        if id.is_empty() {
            return Err(OrbitError::InvalidInput(format!(
                "default activity spec '{}' contains empty activity id",
                expected_id
            )));
        }
        if id != *expected_id {
            return Err(OrbitError::InvalidInput(format!(
                "default activity file key '{}' does not match spec id '{}'",
                expected_id, id
            )));
        }
        if !ids.insert(id.to_string()) {
            return Err(OrbitError::InvalidInput(format!(
                "default activity set contains duplicate activity id '{id}'"
            )));
        }
        specs.push(resource);
    }
    Ok(specs)
}

fn inject_activity_template_tokens(raw: &str, orbit_root: &Path) -> String {
    let orbit_root_value = orbit_root.to_string_lossy();
    raw.replace(ORBIT_ROOT_TOKEN, orbit_root_value.as_ref())
}

fn seed_default_activities_from_specs(
    runtime: &OrbitRuntime,
    specs: &[ActivityResource],
    overwrite: bool,
) -> Result<usize, OrbitError> {
    let mut created = 0usize;
    for resource in specs {
        let activity_id = resource.metadata.name.clone();
        let spec = &resource.spec;
        let add_params = ActivityAddParams {
            id: activity_id.clone(),
            spec_type: spec.spec_type.clone(),
            description: spec.description.clone(),
            input_schema_json: spec.input_schema_json.clone(),
            output_schema_json: spec.output_schema_json.clone(),
            spec_config: Value::Object(spec.spec_config.clone()),
            executor: spec.executor.clone(),
            workspace_path: spec.workspace_path.clone(),
            created_by: spec.created_by.clone(),
        };
        validate_activity_params(&add_params)?;
        if runtime.show_activity(&activity_id).is_ok() {
            if !overwrite {
                continue;
            }
            let update_params: StoreActivityUpdateParams = ActivityUpdateParams {
                description: Some(spec.description.clone()),
                input_schema_json: Some(spec.input_schema_json.clone()),
                output_schema_json: Some(spec.output_schema_json.clone()),
                spec_config: Some(Value::Object(spec.spec_config.clone())),
                executor: Some(spec.executor.clone()),
                workspace_path: Some(spec.workspace_path.clone()),
                created_by: Some(spec.created_by.clone()),
                is_active: Some(spec.is_active),
            }
            .into();
            runtime
                .stores()
                .activities()
                .update(&activity_id, update_params)?;
            created += 1;
            continue;
        }
        runtime.stores().activities().add(add_params.into())?;
        if !spec.is_active {
            let disable_params: StoreActivityUpdateParams = ActivityUpdateParams {
                is_active: Some(false),
                ..Default::default()
            }
            .into();
            runtime
                .stores()
                .activities()
                .update(&activity_id, disable_params)?;
        }
        created += 1;
    }
    Ok(created)
}

fn validate_activity_params(params: &ActivityAddParams) -> Result<(), OrbitError> {
    if params.id.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "activity id must not be empty".to_string(),
        ));
    }
    if params.spec_type.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "activity type must not be empty".to_string(),
        ));
    }
    if !VALID_ACTIVITY_SPEC_TYPES.contains(&params.spec_type.as_str()) {
        return Err(OrbitError::InvalidInput(format!(
            "activity type '{}' is unsupported; valid values: {}",
            params.spec_type,
            VALID_ACTIVITY_SPEC_TYPES.join(", ")
        )));
    }
    if params.description.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "activity description must not be empty".to_string(),
        ));
    }
    if !params.input_schema_json.is_object() {
        return Err(OrbitError::InvalidInput(
            "input schema must be a JSON object".to_string(),
        ));
    }
    if !params.output_schema_json.is_object() {
        return Err(OrbitError::InvalidInput(
            "output schema must be a JSON object".to_string(),
        ));
    }
    ensure_spec_config_object(&params.spec_config)?;
    if activity_skill_refs_from_spec_config(&params.spec_config)?
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err(OrbitError::InvalidInput(
            "skill_refs must not contain empty values".to_string(),
        ));
    }

    Ok(())
}

fn ensure_spec_config_object(spec_config: &Value) -> Result<(), OrbitError> {
    if spec_config.is_object() {
        Ok(())
    } else {
        Err(OrbitError::InvalidInput(
            "spec_config must be a JSON object".to_string(),
        ))
    }
}

pub(crate) fn activity_requires_agent_cli(spec_type: &str) -> bool {
    spec_type == "agent_invoke"
}

fn validate_activity_executor_for_run(
    runtime: &OrbitRuntime,
    activity: &Activity,
) -> Result<(), OrbitError> {
    let Some(executor_name) = activity
        .executor
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };

    let executor = runtime
        .stores()
        .executors()
        .get(executor_name)?
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "activity executor '{}' does not exist",
                executor_name
            ))
        })?;

    if activity.spec_type == "agent_invoke"
        && !matches!(
            executor.executor_type.as_str(),
            "agent_cli" | "direct_agent"
        )
    {
        return Err(OrbitError::InvalidInput(format!(
            "activity executor '{}' has unsupported type '{}' for agent_invoke activities",
            executor_name, executor.executor_type
        )));
    }

    Ok(())
}

fn activity_has_executor(activity: &Activity) -> bool {
    activity
        .executor
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

pub(crate) fn activity_skill_refs_from_spec_config(
    spec_config: &Value,
) -> Result<Vec<String>, OrbitError> {
    ensure_spec_config_object(spec_config)?;
    let Some(raw_refs) = spec_config.get("skill_refs") else {
        return Ok(Vec::new());
    };
    serde_json::from_value(raw_refs.clone()).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "activity spec_config.skill_refs must be an array of strings: {error}"
        ))
    })
}
