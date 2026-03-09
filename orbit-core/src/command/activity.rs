use std::collections::BTreeSet;
use std::path::Path;

use orbit_store::ActivityCreateParams as StoreWorkCreateParams;
use orbit_types::{Activity, JobRunState, OrbitError, OrbitEvent};
use serde::Deserialize;
use serde_json::Value;

use crate::OrbitRuntime;

const DEFAULT_JOB_FILES: [(&str, &str); 4] = [
    (
        "approve-task-leader",
        include_str!("../../assets/activities/approve-task-leader.yaml"),
    ),
    (
        "oversee-orbit-operations",
        include_str!("../../assets/activities/oversee-orbit-operations.yaml"),
    ),
    (
        "perform-maintenance",
        include_str!("../../assets/activities/perform-maintenance.yaml"),
    ),
    (
        "resolve-backlogged-task",
        include_str!("../../assets/activities/resolve-backlogged-task.yaml"),
    ),
];
use crate::paths::ORBIT_ROOT_TOKEN;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityAddParams {
    pub id: String,
    pub spec_type: String,
    pub description: String,
    #[serde(default)]
    pub instruction: String,
    pub input_schema_json: Value,
    pub output_schema_json: Value,
    pub artifact_path_template: Option<String>,
    #[serde(default)]
    pub skill_refs: Vec<String>,
    pub identity_id: Option<String>,
    pub assigned_to: Option<String>,
    pub created_by: Option<String>,
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
    pub fn add_activity(&self, params: ActivityAddParams) -> Result<Activity, OrbitError> {
        validate_activity_params(&params)?;
        let _ = self.resolve_activity_skill_refs(&params.skill_refs)?;
        let identity_id = params.identity_id.clone();
        let mut assigned_to = params.assigned_to.clone();
        let mut created_by = params.created_by.clone();
        if let Some(id) = identity_id.as_ref() {
            let resolved = self.resolve_identity(id)?;
            if assigned_to.is_none() {
                assigned_to = Some(resolved.name.clone());
            }
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
                instruction: params.instruction,
                input_schema_json: params.input_schema_json,
                output_schema_json: params.output_schema_json,
                artifact_path_template: params.artifact_path_template,
                skill_refs: params.skill_refs,
                identity_id,
                assigned_to,
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
        if params.agent_cli.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "agent_cli must not be empty".to_string(),
            ));
        }

        let activity = self.show_activity(&params.activity_id)?;
        self.record_event(OrbitEvent::ActivityRunStarted {
            id: activity.id.clone(),
        })?;
        let outcome =
            self.run_activity_direct(&activity, &params.agent_cli, params.timeout_seconds)?;
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

pub(crate) fn seed_default_activities(runtime: &OrbitRuntime) -> Result<usize, OrbitError> {
    let orbit_root = runtime.data_root();
    let specs = load_default_activity_specs(&DEFAULT_JOB_FILES, Some(&orbit_root))?;
    seed_default_activities_from_specs(runtime, &specs)
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
        let spec = serde_yaml::from_str::<ActivityAddParams>(&rendered).map_err(|err| {
            OrbitError::InvalidInput(format!(
                "invalid default activity spec '{}': {err}",
                expected_id
            ))
        })?;
        let id = spec.id.trim();
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
        specs.push(spec);
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
) -> Result<usize, OrbitError> {
    let mut created = 0usize;
    for spec in specs {
        if runtime.show_activity(&spec.id).is_ok() {
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
    if params.skill_refs.iter().any(|v| v.trim().is_empty()) {
        return Err(OrbitError::InvalidInput(
            "skill_refs must not contain empty values".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{DEFAULT_JOB_FILES, load_default_activity_specs};

    #[test]
    fn parse_rejects_duplicate_ids() {
        let specs = [
            (
                "duplicate",
                r#"
id: duplicate
specType: task
description: first
inputSchemaJson: {}
outputSchemaJson: {}
"#,
            ),
            (
                "duplicate",
                r#"
id: duplicate
specType: task
description: second
inputSchemaJson: {}
outputSchemaJson: {}
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
id: "  "
specType: task
description: empty id
inputSchemaJson: {}
outputSchemaJson: {}
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
id: actual-id
specType: task
description: mismatch
inputSchemaJson: {}
outputSchemaJson: {}
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
id: tokenized
specType: task
description: token replacement
inputSchemaJson: {}
outputSchemaJson: {}
artifactPathTemplate: "{{ORBIT_ROOT}}/agents/executions/{{date}}-tokenized.md"
"#,
        )];
        let parsed =
            load_default_activity_specs(&specs, Some(Path::new("/tmp/orbit"))).expect("must parse");
        assert_eq!(
            parsed[0].artifact_path_template.as_deref(),
            Some("/tmp/orbit/agents/executions/{{date}}-tokenized.md")
        );
    }

    #[test]
    fn bundled_default_activity_specs_parse_successfully() {
        let parsed = load_default_activity_specs(&DEFAULT_JOB_FILES, Some(Path::new("/tmp/orbit")))
            .expect("bundled default activities must parse");

        assert_eq!(parsed.len(), DEFAULT_JOB_FILES.len());
    }
}
