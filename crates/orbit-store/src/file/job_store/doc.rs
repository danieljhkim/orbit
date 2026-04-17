use orbit_types::{JobRun, JobRunStep};
use serde::{Deserialize, Serialize};

/// Serialized to jrun.yaml — contains run-level fields only.
/// Step-level fields (exit_code, agent_response_json, etc.) live in steps/*.yaml.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JobRunFileDocument {
    pub(super) schema_version: u8,
    pub(super) run: JobRun,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JobRunStepFileDocument {
    pub(super) schema_version: u8,
    pub(super) step: JobRunStep,
}
