use std::collections::BTreeSet;
use std::path::Path;

use orbit_store::ActivityCreateParams as StoreWorkCreateParams;
use orbit_store::ActivityUpdateParams as StoreActivityUpdateParams;
use orbit_types::{Activity, JobRunState, OrbitError, OrbitEvent};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::OrbitRuntime;
use crate::paths::ORBIT_ROOT_TOKEN;

pub(crate) const DEFAULT_ACTIVITY_FILES: &[(&str, &str)] = &[
    (
        "checkout_branch",
        include_str!("../../assets/activities/checkout_branch.yaml"),
    ),
    (
        "commit_changes",
        include_str!("../../assets/activities/commit_changes.yaml"),
    ),
    (
        "create_branch",
        include_str!("../../assets/activities/create_branch.yaml"),
    ),
    (
        "update_task",
        include_str!("../../assets/activities/update_task.yaml"),
    ),
    (
        "dispatch_task",
        include_str!("../../assets/activities/dispatch_task.yaml"),
    ),
    (
        "dispatch_parallel_task",
        include_str!("../../assets/activities/dispatch_parallel_task.yaml"),
    ),
    (
        "parallel_dispatch_tasks",
        include_str!("../../assets/activities/parallel_dispatch_tasks.yaml"),
    ),
    (
        "plan_task",
        include_str!("../../assets/activities/plan_task.yaml"),
    ),
    (
        "implement_change",
        include_str!("../../assets/activities/implement_change.yaml"),
    ),
    (
        "merge_pr",
        include_str!("../../assets/activities/merge_pr.yaml"),
    ),
    (
        "open_pr",
        include_str!("../../assets/activities/open_pr.yaml"),
    ),
    (
        "review_tasks",
        include_str!("../../assets/activities/review_tasks.yaml"),
    ),
    (
        "oversee_orbit_operations",
        include_str!("../../assets/activities/oversee_orbit_operations.yaml"),
    ),
    (
        "perform_maintenance",
        include_str!("../../assets/activities/perform_maintenance.yaml"),
    ),
    (
        "review_pr",
        include_str!("../../assets/activities/review_pr.yaml"),
    ),
    (
        "run_tests",
        include_str!("../../assets/activities/run_tests.yaml"),
    ),
    (
        "check_review_decision",
        include_str!("../../assets/activities/check_review_decision.yaml"),
    ),
    (
        "load_pr_comments",
        include_str!("../../assets/activities/load_pr_comments.yaml"),
    ),
    (
        "implement_fix",
        include_str!("../../assets/activities/implement_fix.yaml"),
    ),
    (
        "push_changes",
        include_str!("../../assets/activities/push_changes.yaml"),
    ),
    (
        "sync_review_to_github",
        include_str!("../../assets/activities/sync_review_to_github.yaml"),
    ),
];

const VALID_ACTIVITY_SPEC_TYPES: &[&str] = &["agent_invoke", "cli_command", "api", "automation"];

#[derive(Debug, Clone)]
pub struct ActivityAddParams {
    pub id: String,
    pub spec_type: String,
    pub description: String,
    pub input_schema_json: Value,
    pub output_schema_json: Value,
    pub spec_config: Value,
    pub workspace_path: Option<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ActivityUpdateParams {
    pub description: Option<String>,
    pub input_schema_json: Option<Value>,
    pub output_schema_json: Option<Value>,
    pub spec_config: Option<Value>,
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
            workspace_path: p.workspace_path,
            created_by: p.created_by,
            is_active: p.is_active,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ActivityFileEnvelope {
    schema_version: u8,
    #[serde(default)]
    created_by: Option<String>,
    #[allow(dead_code)]
    #[serde(default, rename = "identity_id")]
    legacy_identity_id: Option<String>,
    activity: ActivityFileSpec,
}

#[derive(Debug, Clone, Deserialize)]
struct ActivityFileSpec {
    id: String,
    spec_type: String,
    description: String,
    #[serde(default)]
    input_schema_json: Value,
    #[serde(default)]
    output_schema_json: Value,
    #[serde(default)]
    workspace_path: Option<String>,
    #[serde(flatten)]
    spec_config: Map<String, Value>,
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
        let activity = self.add_activity_record(params.into())?;
        self.record_event(OrbitEvent::ActivityAdded {
            id: activity.id.clone(),
        })?;
        Ok(activity)
    }

    pub fn list_activities(&self, include_inactive: bool) -> Result<Vec<Activity>, OrbitError> {
        self.list_activity_records(include_inactive)
    }

    pub fn show_activity(&self, id: &str) -> Result<Activity, OrbitError> {
        self.get_activity_record(id)?
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

        let activity = self.update_activity_record(id, params.into())?;
        self.record_event(OrbitEvent::ActivityUpdated {
            id: activity.id.clone(),
        })?;
        Ok(activity)
    }

    pub fn delete_activity(&self, id: &str) -> Result<(), OrbitError> {
        let changed = self.disable_activity_record(id)?;
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
        if activity_requires_agent_cli(&activity.spec_type) && params.agent_cli.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "agent_cli must not be empty for agent_invoke activities".to_string(),
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
) -> Result<Vec<ActivityAddParams>, OrbitError> {
    let mut specs = Vec::with_capacity(raw_specs.len());
    let mut ids = BTreeSet::new();
    for (expected_id, raw) in raw_specs {
        let rendered = match orbit_root {
            Some(root) => inject_activity_template_tokens(raw, root),
            None => (*raw).to_string(),
        };
        let spec = serde_yaml::from_str::<ActivityFileEnvelope>(&rendered).map_err(|err| {
            OrbitError::InvalidInput(format!(
                "invalid default activity spec '{}': {err}",
                expected_id
            ))
        })?;
        if spec.schema_version != 1 {
            return Err(OrbitError::InvalidInput(format!(
                "default activity spec '{}' uses unsupported schema_version {}",
                expected_id, spec.schema_version
            )));
        }
        let id = spec.activity.id.trim();
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
        specs.push(ActivityAddParams {
            id: spec.activity.id,
            spec_type: spec.activity.spec_type,
            description: spec.activity.description,
            input_schema_json: spec.activity.input_schema_json,
            output_schema_json: spec.activity.output_schema_json,
            spec_config: Value::Object(spec.activity.spec_config),
            workspace_path: spec.activity.workspace_path,
            created_by: spec.created_by,
        });
    }
    Ok(specs)
}

fn inject_activity_template_tokens(raw: &str, orbit_root: &Path) -> String {
    let orbit_root_value = orbit_root.to_string_lossy();
    raw.replace(ORBIT_ROOT_TOKEN, orbit_root_value.as_ref())
}

fn seed_default_activities_from_specs(
    runtime: &OrbitRuntime,
    specs: &[ActivityAddParams],
    overwrite: bool,
) -> Result<usize, OrbitError> {
    let mut created = 0usize;
    for spec in specs {
        if runtime.show_activity(&spec.id).is_ok() {
            if !overwrite {
                continue;
            }
            runtime.update_activity(
                &spec.id,
                ActivityUpdateParams {
                    description: Some(spec.description.clone()),
                    input_schema_json: Some(spec.input_schema_json.clone()),
                    output_schema_json: Some(spec.output_schema_json.clone()),
                    spec_config: Some(spec.spec_config.clone()),
                    workspace_path: Some(spec.workspace_path.clone()),
                    created_by: Some(spec.created_by.clone()),
                    is_active: Some(true),
                },
            )?;
            created += 1;
            continue;
        }
        runtime.add_activity(spec.clone())?;
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
