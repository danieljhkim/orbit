use orbit_types::{Job, JobRunState, JobStep, PipelineState};
use serde_json::{Value, json};
use std::collections::HashMap;

pub(crate) fn step_recovery_key(step: &JobStep) -> String {
    step.id.as_deref().unwrap_or(&step.target_id).to_string()
}

pub(crate) fn apply_pipeline_patch(current_input: &mut Value, patch: &Value) {
    if let (Some(input_map), Some(patch_map)) = (current_input.as_object_mut(), patch.as_object()) {
        for (key, value) in patch_map {
            input_map.insert(key.clone(), value.clone());
        }
    }
}

/// Build a structured step entry for the template context.
///
/// Each step is represented as:
/// ```json
/// {
///   "state": {"status": "success", "exit_code": 0, "duration_ms": 1234},
///   "output": { ... raw step output ... }
/// }
/// ```
pub(crate) fn wrap_step_entry(
    step_state: Option<JobRunState>,
    exit_code: Option<i32>,
    duration_ms: Option<u64>,
    raw_output: Option<&Value>,
) -> Value {
    json!({
        "state": {
            "status": step_state.map(|s| s.to_string()).unwrap_or_default(),
            "exit_code": exit_code.unwrap_or(-1),
            "duration_ms": duration_ms.unwrap_or(0),
        },
        "output": raw_output.cloned().unwrap_or(Value::Null),
    })
}

pub(crate) fn build_steps_template_outputs(
    job: &Job,
    state: &PipelineState,
    next_step_index: usize,
) -> HashMap<String, Value> {
    if job.steps.is_empty() {
        return HashMap::new();
    }
    let mut outputs = HashMap::new();
    for step_index in 0..next_step_index {
        let step = &job.steps[step_index % job.steps.len()];
        let raw_output = state.step_outputs.get(&(step_index as u32));
        let step_state = state.step_states.get(&(step_index as u32)).copied();
        let entry = wrap_step_entry(step_state, None, None, raw_output);
        outputs.insert(step_recovery_key(step), entry);
    }
    outputs
}

pub(crate) fn build_retry_pipeline_state(
    job: &Job,
    source_state: &PipelineState,
    retry_from_index: usize,
) -> PipelineState {
    let baseline = source_state.rebuild_pipeline_before(retry_from_index as u32);
    let mut recovered =
        PipelineState::new(source_state.run_id.clone(), job.job_id.clone(), baseline);
    recovered.step_outputs = source_state
        .step_outputs
        .range(..(retry_from_index as u32))
        .map(|(step_index, output)| (*step_index, output.clone()))
        .collect();
    recovered.step_states = source_state
        .step_states
        .range(..(retry_from_index as u32))
        .map(|(step_index, state)| (*step_index, *state))
        .collect();
    recovered.next_step_index = retry_from_index as u32;
    recovered.previous_step_state =
        source_state.previous_step_state_before(retry_from_index as u32);
    recovered.iteration = source_state.iteration;
    recovered
}

pub(crate) fn apply_output_map(mut output: Value, output_map: &HashMap<String, String>) -> Value {
    if let Some(obj) = output.as_object_mut() {
        for (source, target) in output_map {
            if let Some(value) = obj.remove(source) {
                obj.insert(target.clone(), value);
            }
        }
    }
    output
}

pub(crate) fn pipeline_patch_for_job_step(output: Option<&Value>) -> Option<Value> {
    match output {
        Some(Value::Object(map)) => Some(Value::Object(map.clone())),
        _ => None,
    }
}

pub(crate) fn final_state_from_pipeline_state(state: &PipelineState) -> JobRunState {
    match state.previous_step_state {
        Some(JobRunState::Failed) => JobRunState::Failed,
        Some(JobRunState::Timeout) => JobRunState::Timeout,
        Some(JobRunState::Cancelled) => JobRunState::Cancelled,
        _ => JobRunState::Success,
    }
}
