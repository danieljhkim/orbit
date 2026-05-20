use std::str::FromStr;

use crate::command::docs::DocType;
use orbit_common::types::{OrbitError, optional_string, required_string};
use serde_json::{Value, json};

use crate::OrbitRuntime;

use super::input::optional_bool_alias;

pub(super) fn list(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let doc_type = optional_string(&input, "type")?
        .map(|raw| DocType::from_str(&raw).map_err(OrbitError::InvalidInput))
        .transpose()?;
    let tag = optional_string(&input, "tag")?;
    to_json(runtime.list_docs(doc_type, tag.as_deref())?)
}

pub(super) fn show(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let path = required_string(&input, &["path"], "path")?;
    to_json(runtime.show_doc(&path)?)
}

pub(super) fn add(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let path = required_string(&input, &["path"], "path")?;
    to_json(runtime.add_docs_root(&path)?)
}

pub(super) fn reindex(runtime: &OrbitRuntime, _input: Value) -> Result<Value, OrbitError> {
    Ok(json!({ "message": runtime.reindex_docs()? }))
}

pub(super) fn migrate(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let dry_run = optional_bool_alias(&input, &["dry_run", "dryRun"])?.unwrap_or(false);
    to_json(runtime.migrate_docs(dry_run)?)
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, OrbitError> {
    serde_json::to_value(value)
        .map_err(|error| OrbitError::Execution(format!("serialize docs tool output: {error}")))
}
