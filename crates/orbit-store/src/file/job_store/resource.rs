use std::path::Path;
#[cfg(unix)]
use std::process::Command;

use chrono::{DateTime, Utc};
use orbit_types::{
    Job, JobResource, JobResourceSpec, OrbitError, RESOURCE_SCHEMA_VERSION, ResourceKind,
};

use super::JobFileStore;
use crate::file::layout::list_yaml_files;
use crate::file::yaml_doc::{read_yaml, write_yaml_atomic};

impl JobFileStore {
    pub(super) fn read_all_activities(&self) -> Result<Vec<Job>, OrbitError> {
        self.ensure_layout()?;
        let mut paths = list_yaml_files(&self.jobs_dir())?;
        paths.extend(list_yaml_files(&self.disabled_jobs_dir())?);
        paths.sort();
        let mut jobs = Vec::new();
        for path in paths {
            jobs.push(self.read_activity_at(&path)?);
        }
        Ok(jobs)
    }

    pub(super) fn read_activity_at(&self, path: &Path) -> Result<Job, OrbitError> {
        let doc = read_yaml::<JobResource>(path, "job file")?;
        job_from_resource(doc, path)
    }

    pub(super) fn write_activity(&self, job: &Job) -> Result<(), OrbitError> {
        self.ensure_layout()?;
        let doc = job_to_resource(job);
        write_yaml_atomic(&self.job_path(&job.job_id), &doc)
    }
}

fn job_from_resource(doc: JobResource, path: &Path) -> Result<Job, OrbitError> {
    if doc.kind != ResourceKind::Job {
        return Err(OrbitError::Store(format!(
            "invalid job file '{}': expected kind Job, found {}",
            path.display(),
            doc.kind
        )));
    }
    if doc.schema_version != RESOURCE_SCHEMA_VERSION {
        return Err(OrbitError::Store(format!(
            "invalid job file '{}': unsupported schemaVersion {}",
            path.display(),
            doc.schema_version
        )));
    }

    let created_at = parse_timestamp_from_job_id(&doc.metadata.name);
    Ok(Job {
        job_id: doc.metadata.name,
        state: doc.spec.state,
        default_input: doc.spec.default_input,
        max_active_runs: validate_max_active_runs(doc.spec.max_active_runs)?,
        max_iterations: doc.spec.max_iterations,
        steps: doc.spec.steps,
        policy: doc.spec.policy,
        created_at,
        updated_at: created_at,
    })
}

pub(super) fn job_to_resource(job: &Job) -> JobResource {
    JobResource::new(
        ResourceKind::Job,
        job.job_id.clone(),
        JobResourceSpec {
            state: job.state,
            default_input: job.default_input.clone(),
            max_active_runs: job.max_active_runs,
            max_iterations: job.max_iterations,
            steps: job.steps.clone(),
            policy: job.policy.clone(),
        },
    )
}

pub(super) fn validate_max_active_runs(max_active_runs: u32) -> Result<u32, OrbitError> {
    if max_active_runs == 0 {
        return Err(OrbitError::JobValidation(
            "job max_active_runs must be at least 1".to_string(),
        ));
    }
    Ok(max_active_runs)
}

/// Derive a UTC timestamp from a job ID of the form `job-YYYYMMDD-HHMM[-N]` (new)
/// or `job-YYYYMMDD-HHMMSS[-mmm][-N]` (legacy). Falls back to `Utc::now()` for IDs
/// that don't embed a parseable timestamp.
fn parse_timestamp_from_job_id(job_id: &str) -> DateTime<Utc> {
    let rest = job_id.strip_prefix("job-").unwrap_or(job_id);
    let mut parts = rest.splitn(3, '-');
    let date = parts.next().unwrap_or("");
    let time = parts.next().unwrap_or("");
    if date.len() == 8 {
        let padded_time = if time.len() == 4 {
            format!("{time}00")
        } else if time.len() == 6 {
            time.to_string()
        } else {
            return Utc::now();
        };
        let s = format!("{date}{padded_time}");
        if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(&s, "%Y%m%d%H%M%S") {
            return ndt.and_utc();
        }
    }
    Utc::now()
}

#[cfg(unix)]
pub(super) fn process_start_time_token(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!token.is_empty()).then_some(token)
}

#[cfg(not(unix))]
pub(super) fn process_start_time_token(_pid: u32) -> Option<String> {
    None
}
