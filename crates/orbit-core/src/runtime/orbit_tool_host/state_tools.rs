use orbit_common::types::OrbitError;
use orbit_store::state_io;
use orbit_tools::OrbitTaskScope;
use serde_json::{Value, json};

use super::input::{resolve_state_dir, resolve_state_payload, resolve_step_index};
use orbit_common::types::optional_string;

pub(super) fn get(task_scope: &OrbitTaskScope, input: Value) -> Result<Value, OrbitError> {
    let state_dir = resolve_state_dir(task_scope, &input)?;
    let pipeline = state_io::read_pipeline(&state_dir)?;
    match optional_string(&input, "key")? {
        Some(key) => Ok(pipeline
            .as_object()
            .and_then(|map| map.get(&key))
            .cloned()
            .unwrap_or(Value::Null)),
        None => Ok(pipeline),
    }
}

pub(super) fn set(task_scope: &OrbitTaskScope, input: Value) -> Result<Value, OrbitError> {
    let state_dir = resolve_state_dir(task_scope, &input)?;
    let step_index = resolve_step_index(&input)?;
    let payload = resolve_state_payload(&input)?;
    state_io::write_step_output(&state_dir, step_index, &payload)?;
    Ok(json!({
        "state_dir": state_dir.display().to_string(),
        "step_index": step_index,
        "written": payload,
    }))
}
