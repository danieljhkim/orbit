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
        "dispatch_task",
        include_str!("../../assets/activities/dispatch_task.yaml"),
    ),
    (
        "implement_change",
        include_str!("../../assets/activities/implement_change.yaml"),
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
    pub identity_id: Option<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ActivityUpdateParams {
    pub description: Option<String>,
    pub input_schema_json: Option<Value>,
    pub output_schema_json: Option<Value>,
    pub spec_config: Option<Value>,
    pub workspace_path: Option<Option<String>>,
    pub identity_id: Option<Option<String>>,
    pub created_by: Option<Option<String>>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct ActivityFileEnvelope {
    schema_version: u8,
    #[serde(default)]
    created_by: Option<String>,
    #[serde(default)]
    identity_id: Option<String>,
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
        let identity_id = params.identity_id.clone();
        let mut created_by = params.created_by.clone();
        if let Some(id) = identity_id.as_ref() {
            let resolved = self.resolve_identity(id)?;
            if created_by.is_none() {
                created_by = Some(resolved.name);
            }
        }

        let activity = self
            .context
            .activity_store
            .add_activity(StoreWorkCreateParams {
                id: params.id,
                spec_type: params.spec_type,
                description: params.description,
                input_schema_json: params.input_schema_json,
                output_schema_json: params.output_schema_json,
                spec_config: params.spec_config,
                workspace_path: params.workspace_path,
                identity_id,
                created_by,
            })?;
        self.record_event(OrbitEvent::ActivityAdded {
            id: activity.id.clone(),
        })?;
        Ok(activity)
    }

    pub fn list_activities(&self, include_inactive: bool) -> Result<Vec<Activity>, OrbitError> {
        self.context
            .activity_store
            .list_activities(include_inactive)
    }

    pub fn show_activity(&self, id: &str) -> Result<Activity, OrbitError> {
        self.context
            .activity_store
            .get_activity(id)?
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

        let activity = self.context.activity_store.update_activity(
            id,
            StoreActivityUpdateParams {
                description: params.description,
                input_schema_json: params.input_schema_json,
                output_schema_json: params.output_schema_json,
                spec_config: params.spec_config,
                workspace_path: params.workspace_path,
                identity_id: params.identity_id,
                created_by: params.created_by,
                is_active: params.is_active,
            },
        )?;
        self.record_event(OrbitEvent::ActivityUpdated {
            id: activity.id.clone(),
        })?;
        Ok(activity)
    }

    pub fn delete_activity(&self, id: &str) -> Result<(), OrbitError> {
        let changed = self.context.activity_store.disable_activity(id)?;
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
    overwrite: bool,
) -> Result<usize, OrbitError> {
    let orbit_root = runtime.data_root();
    let specs = load_default_activity_specs(DEFAULT_ACTIVITY_FILES, Some(&orbit_root))?;
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
            identity_id: spec.identity_id,
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
                    identity_id: Some(spec.identity_id.clone()),
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{DEFAULT_ACTIVITY_FILES, load_default_activity_specs};

    #[test]
    fn parse_rejects_duplicate_ids() {
        let specs = [
            (
                "duplicate",
                r#"
schema_version: 1
activity:
  id: duplicate
  spec_type: agent_invoke
  description: first
  input_schema_json: {}
  output_schema_json: {}
"#,
            ),
            (
                "duplicate",
                r#"
schema_version: 1
activity:
  id: duplicate
  spec_type: agent_invoke
  description: second
  input_schema_json: {}
  output_schema_json: {}
"#,
            ),
        ];
        let err = load_default_activity_specs(&specs, None).expect_err("must fail");
        assert!(err.to_string().contains("duplicate activity id"));
    }

    #[test]
    fn parse_rejects_empty_ids() {
        let specs = [(
            "empty-id",
            r#"
schema_version: 1
activity:
  id: "  "
  spec_type: agent_invoke
  description: empty id
  input_schema_json: {}
  output_schema_json: {}
"#,
        )];
        let err = load_default_activity_specs(&specs, None).expect_err("must fail");
        assert!(err.to_string().contains("empty activity id"));
    }

    #[test]
    fn parse_rejects_mismatched_file_key_and_id() {
        let specs = [(
            "expected-id",
            r#"
schema_version: 1
activity:
  id: actual-id
  spec_type: agent_invoke
  description: mismatch
  input_schema_json: {}
  output_schema_json: {}
"#,
        )];
        let err = load_default_activity_specs(&specs, None).expect_err("must fail");
        assert!(err.to_string().contains("does not match spec id"));
    }

    #[test]
    fn parse_replaces_orbit_root_token_when_provided() {
        let specs = [(
            "tokenized",
            r#"
schema_version: 1
activity:
  id: tokenized
  spec_type: agent_invoke
  description: "{{ORBIT_ROOT}}/agents/executions/{{date}}-tokenized.md"
  input_schema_json: {}
  output_schema_json: {}
"#,
        )];
        let parsed =
            load_default_activity_specs(&specs, Some(Path::new("/tmp/orbit"))).expect("must parse");
        assert_eq!(
            parsed[0].description,
            "/tmp/orbit/agents/executions/{{date}}-tokenized.md"
        );
    }

    #[test]
    fn bundled_default_activity_specs_parse_successfully() {
        let parsed =
            load_default_activity_specs(DEFAULT_ACTIVITY_FILES, Some(Path::new("/tmp/orbit")))
                .expect("bundled default activities must parse");

        assert_eq!(parsed.len(), DEFAULT_ACTIVITY_FILES.len());
        let ids = parsed
            .iter()
            .map(|spec| spec.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "checkout_branch",
                "commit_changes",
                "create_branch",
                "dispatch_task",
                "implement_change",
                "open_pr",
                "review_tasks",
                "oversee_orbit_operations",
                "perform_maintenance",
                "review_pr",
                "run_tests",
            ]
        );
    }
}
