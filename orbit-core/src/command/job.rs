use std::collections::BTreeSet;

use orbit_store::JobCreateParams as StoreWorkCreateParams;
use orbit_types::{Job, OrbitError, OrbitEvent};
use serde::Deserialize;
use serde_json::Value;

use crate::OrbitRuntime;

const DEFAULT_JOB_FILES: [(&str, &str); 1] = [(
    "approve-task-leader",
    include_str!("../../assets/jobs/approve-task-leader.yaml"),
)];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobAddParams {
    pub id: String,
    pub spec_type: String,
    pub description: String,
    pub input_schema_json: Value,
    pub output_schema_json: Value,
    pub artifact_path_template: Option<String>,
    #[serde(default)]
    pub skill_refs: Vec<String>,
    pub identity_id: Option<String>,
    pub assigned_to: Option<String>,
    pub created_by: Option<String>,
}

impl OrbitRuntime {
    pub fn add_job(&self, params: JobAddParams) -> Result<Job, OrbitError> {
        validate_job_params(&params)?;
        let _ = self.resolve_job_skill_refs(&params.skill_refs)?;
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

        let job = self.context.job_store.add_job(StoreWorkCreateParams {
            id: params.id,
            spec_type: params.spec_type,
            description: params.description,
            input_schema_json: params.input_schema_json,
            output_schema_json: params.output_schema_json,
            artifact_path_template: params.artifact_path_template,
            skill_refs: params.skill_refs,
            identity_id,
            assigned_to,
            created_by,
        })?;
        self.record_event(OrbitEvent::JobAdded { id: job.id.clone() })?;
        Ok(job)
    }

    pub fn list_jobs(&self, include_inactive: bool) -> Result<Vec<Job>, OrbitError> {
        self.context.job_store.list_jobs(include_inactive)
    }

    pub fn show_job(&self, id: &str) -> Result<Job, OrbitError> {
        self.context
            .job_store
            .get_job(id)?
            .ok_or_else(|| OrbitError::JobNotFound(id.to_string()))
    }

    pub fn delete_job(&self, id: &str) -> Result<(), OrbitError> {
        let changed = self.context.job_store.disable_job(id)?;
        if !changed {
            return Err(OrbitError::JobNotFound(id.to_string()));
        }
        self.record_event(OrbitEvent::JobDisabled { id: id.to_string() })
    }
}

pub(crate) fn seed_default_jobs(runtime: &OrbitRuntime) -> Result<usize, OrbitError> {
    let specs = load_default_job_specs(&DEFAULT_JOB_FILES)?;
    seed_default_jobs_from_specs(runtime, &specs)
}

fn load_default_job_specs(raw_specs: &[(&str, &str)]) -> Result<Vec<JobAddParams>, OrbitError> {
    let mut specs = Vec::with_capacity(raw_specs.len());
    let mut ids = BTreeSet::new();
    for (expected_id, raw) in raw_specs {
        let spec = serde_yaml::from_str::<JobAddParams>(raw).map_err(|err| {
            OrbitError::InvalidInput(format!("invalid default job spec '{}': {err}", expected_id))
        })?;
        let id = spec.id.trim();
        if id.is_empty() {
            return Err(OrbitError::InvalidInput(format!(
                "default job spec '{}' contains empty job id",
                expected_id
            )));
        }
        if id != *expected_id {
            return Err(OrbitError::InvalidInput(format!(
                "default job file key '{}' does not match spec id '{}'",
                expected_id, id
            )));
        }
        if !ids.insert(id.to_string()) {
            return Err(OrbitError::InvalidInput(format!(
                "default job set contains duplicate job id '{id}'"
            )));
        }
        specs.push(spec);
    }
    Ok(specs)
}

fn seed_default_jobs_from_specs(
    runtime: &OrbitRuntime,
    specs: &[JobAddParams],
) -> Result<usize, OrbitError> {
    let mut created = 0usize;
    for spec in specs {
        if runtime.show_job(&spec.id).is_ok() {
            continue;
        }
        runtime.add_job(spec.clone())?;
        created += 1;
    }
    Ok(created)
}

fn validate_job_params(params: &JobAddParams) -> Result<(), OrbitError> {
    if params.id.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "job id must not be empty".to_string(),
        ));
    }
    if params.spec_type.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "job type must not be empty".to_string(),
        ));
    }
    if params.description.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "job description must not be empty".to_string(),
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
    use super::load_default_job_specs;

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
        let err = load_default_job_specs(&specs).expect_err("must fail");
        assert!(err.to_string().contains("duplicate job id"));
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
        let err = load_default_job_specs(&specs).expect_err("must fail");
        assert!(err.to_string().contains("empty job id"));
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
        let err = load_default_job_specs(&specs).expect_err("must fail");
        assert!(err.to_string().contains("does not match spec id"));
    }
}
