use std::collections::btree_map::Entry;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{OrbitError, PipelineState};
use serde_json::Value;

use orbit_common::utility::fs::atomic_write_text_volatile as write_atomic;

pub fn resolve_active_run_state_dir(
    orbit_root: &Path,
    run_id: &str,
) -> Result<Option<PathBuf>, OrbitError> {
    let runs_root = orbit_root.join("state").join("job-runs");
    if runs_root.exists() {
        for entry in fs::read_dir(&runs_root).map_err(|error| OrbitError::Io(error.to_string()))? {
            let entry = entry.map_err(|error| OrbitError::Io(error.to_string()))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(job_id) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if job_id == "archived" {
                continue;
            }
            let run_dir = path.join(run_id);
            if run_dir.is_dir() {
                return Ok(Some(run_dir));
            }
        }
    }
    Ok(None)
}

pub fn read_pipeline(state_dir: &Path) -> Result<Value, OrbitError> {
    Ok(read_state_file(state_dir)?.pipeline)
}

pub fn read_step_output(state_dir: &Path, step_index: u32) -> Result<Option<Value>, OrbitError> {
    Ok(read_state_file(state_dir)?
        .step_outputs
        .get(&step_index)
        .cloned())
}

pub fn write_step_output(
    state_dir: &Path,
    step_index: u32,
    data: &Value,
) -> Result<(), OrbitError> {
    let incoming = data
        .as_object()
        .ok_or_else(|| OrbitError::InvalidInput("step output must be a JSON object".to_string()))?;
    let mut state = read_state_file(state_dir)?;
    match state.step_outputs.entry(step_index) {
        Entry::Occupied(mut entry) => {
            let mut merged = match entry.get() {
                Value::Object(existing) => existing.clone(),
                _ => serde_json::Map::new(),
            };
            for (key, value) in incoming {
                merged.insert(key.clone(), value.clone());
            }
            entry.insert(Value::Object(merged));
        }
        Entry::Vacant(entry) => {
            entry.insert(Value::Object(incoming.clone()));
        }
    }
    state.updated_at = Utc::now();
    write_state_file(state_dir, &state)
}

fn read_state_file(state_dir: &Path) -> Result<PipelineState, OrbitError> {
    let state_path = state_path(state_dir);
    let raw = fs::read_to_string(&state_path).map_err(|error| {
        OrbitError::Io(format!(
            "failed to read state.json '{}': {error}",
            state_path.display()
        ))
    })?;
    serde_json::from_str(&raw).map_err(|error| {
        OrbitError::Store(format!(
            "invalid state.json '{}': {error}",
            state_path.display()
        ))
    })
}

fn write_state_file(state_dir: &Path, state: &PipelineState) -> Result<(), OrbitError> {
    let content = serde_json::to_string_pretty(state)
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    write_atomic(&state_path(state_dir), &content).map_err(Into::into)
}

fn state_path(state_dir: &Path) -> PathBuf {
    state_dir.join("state.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    fn test_state() -> PipelineState {
        PipelineState::new("jrun-test".to_string(), "job-test".to_string(), json!({}))
    }

    #[test]
    fn pipeline_state_waiting_reasons_round_trip_populated() {
        let mut state = test_state();
        state.set_waiting_reasons(
            Some(vec!["ORB-1".to_string(), "ORB-2".to_string()]),
            Some(vec!["file:src/lib.rs".to_string()]),
        );

        let encoded = serde_json::to_value(&state).expect("serialize state");
        assert_eq!(encoded["waiting_on_deps"], json!(["ORB-1", "ORB-2"]));
        assert_eq!(encoded["waiting_on_locks"], json!(["file:src/lib.rs"]));

        let decoded: PipelineState = serde_json::from_value(encoded).expect("deserialize state");
        assert_eq!(
            decoded.waiting_on_deps,
            Some(vec!["ORB-1".to_string(), "ORB-2".to_string()])
        );
        assert_eq!(
            decoded.waiting_on_locks,
            Some(vec!["file:src/lib.rs".to_string()])
        );
    }

    #[test]
    fn pipeline_state_waiting_reasons_round_trip_empty_arrays() {
        let mut state = test_state();
        state.set_waiting_reasons(Some(Vec::new()), Some(Vec::new()));

        let encoded = serde_json::to_value(&state).expect("serialize state");
        assert_eq!(encoded["waiting_on_deps"], json!([]));
        assert_eq!(encoded["waiting_on_locks"], json!([]));

        let decoded: PipelineState = serde_json::from_value(encoded).expect("deserialize state");
        assert_eq!(decoded.waiting_on_deps, Some(Vec::new()));
        assert_eq!(decoded.waiting_on_locks, Some(Vec::new()));
    }

    #[test]
    fn pipeline_state_waiting_reasons_round_trip_none_absent() {
        let state = test_state();

        let encoded = serde_json::to_value(&state).expect("serialize state");
        let object = encoded.as_object().expect("state object");
        assert!(!object.contains_key("waiting_on_deps"));
        assert!(!object.contains_key("waiting_on_locks"));

        let decoded: PipelineState = serde_json::from_value(encoded).expect("deserialize state");
        assert_eq!(decoded.waiting_on_deps, None);
        assert_eq!(decoded.waiting_on_locks, None);
    }
}
