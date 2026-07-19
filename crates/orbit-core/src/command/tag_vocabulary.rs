use std::collections::BTreeSet;

use orbit_common::types::OrbitError;
use orbit_store::sqlite::task_registry::read_workspace_config_optional;

use crate::OrbitRuntime;

pub(crate) fn validate_workspace_tags(
    runtime: &OrbitRuntime,
    artifact_kind: &str,
    tags: &[String],
) -> Result<(), OrbitError> {
    if tags.is_empty() {
        return Ok(());
    }
    let Some(config) = read_workspace_config_optional(&runtime.paths().orbit_dir)? else {
        return Err(OrbitError::InvalidInput(format!(
            "cannot validate {artifact_kind} tags because .orbit/config.yaml is missing"
        )));
    };
    validate_tags_against_vocabulary(artifact_kind, tags, &config.learnings.tag_vocabulary)
}

pub(crate) fn validate_tags_against_vocabulary(
    artifact_kind: &str,
    tags: &[String],
    vocabulary: &[String],
) -> Result<(), OrbitError> {
    let allowed = vocabulary
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let unknown = tags
        .iter()
        .filter(|tag| !allowed.contains(tag.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "{artifact_kind} tags are not in .orbit/config.yaml learnings.tag_vocabulary: {}",
        unknown.join(", ")
    )))
}
