use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::JobRunState;

/// Persistent pipeline state for a job run.
///
/// Stored as `state.json` in the run bundle directory. Steps read accumulated
/// state from `pipeline` and write their recovery metadata back so retry and
/// reconcile can resume from the persisted snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineState {
    pub run_id: String,
    pub job_id: String,
    /// Merged job defaults + run input. Immutable after creation.
    pub initial_input: Value,
    /// Accumulated pipeline state — each step's output is merged here.
    /// This replaces the in-memory `current_input` blob.
    pub pipeline: Value,
    /// Raw per-step outputs keyed by global step index.
    /// These are used to rebuild `steps.*` template context during recovery.
    #[serde(default)]
    pub step_outputs: BTreeMap<u32, Value>,
    /// Per-step pipeline patches keyed by global step index.
    /// Successful steps merge these patches into `pipeline`.
    #[serde(default)]
    pub pipeline_patches: BTreeMap<u32, Value>,
    /// Per-step states keyed by global step index.
    #[serde(default)]
    pub step_states: BTreeMap<u32, JobRunState>,
    /// Next global step index the engine should execute.
    #[serde(default)]
    pub next_step_index: u32,
    /// Last non-skipped step state observed by the run.
    #[serde(default)]
    pub previous_step_state: Option<JobRunState>,
    /// Current loop iteration (0-based). Updated at each loop boundary.
    #[serde(default)]
    pub iteration: u32,
    /// Task dependencies currently blocking this run, when the run is parked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_on_deps: Option<Vec<String>>,
    /// Task lock resource identifiers currently blocking this run, when parked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_on_locks: Option<Vec<String>>,
    pub updated_at: DateTime<Utc>,
}

impl PipelineState {
    /// Create a new pipeline state from initial inputs.
    pub fn new(run_id: String, job_id: String, initial_input: Value) -> Self {
        Self {
            run_id,
            job_id,
            pipeline: initial_input.clone(),
            initial_input,
            step_outputs: BTreeMap::new(),
            pipeline_patches: BTreeMap::new(),
            step_states: BTreeMap::new(),
            next_step_index: 0,
            previous_step_state: None,
            iteration: 0,
            waiting_on_deps: None,
            waiting_on_locks: None,
            updated_at: Utc::now(),
        }
    }

    /// Record step recovery metadata and advance the resume cursor.
    pub fn record_step(
        &mut self,
        step_index: u32,
        step_state: JobRunState,
        raw_output: Option<Value>,
        pipeline_patch: Option<Value>,
    ) {
        if let Some(output) = raw_output {
            self.step_outputs.insert(step_index, output);
        }
        if step_state == JobRunState::Success
            && let Some(patch) = pipeline_patch
        {
            merge_pipeline_patch(&mut self.pipeline, &patch);
            self.pipeline_patches.insert(step_index, patch);
        }
        self.step_states.insert(step_index, step_state);
        if step_state != JobRunState::Skipped {
            self.previous_step_state = Some(step_state);
        }
        self.next_step_index = step_index.saturating_add(1);
        self.updated_at = Utc::now();
    }

    /// Replace the accumulated pipeline snapshot directly.
    pub fn sync_pipeline(&mut self, pipeline: Value) {
        self.pipeline = pipeline;
        self.updated_at = Utc::now();
    }

    pub fn set_iteration(&mut self, iteration: u32) {
        self.iteration = iteration;
        self.updated_at = Utc::now();
    }

    pub fn set_waiting_reasons(
        &mut self,
        waiting_on_deps: Option<Vec<String>>,
        waiting_on_locks: Option<Vec<String>>,
    ) {
        self.waiting_on_deps = waiting_on_deps;
        self.waiting_on_locks = waiting_on_locks;
        self.updated_at = Utc::now();
    }

    pub fn clear_waiting_reasons(&mut self) {
        self.waiting_on_deps = None;
        self.waiting_on_locks = None;
        self.updated_at = Utc::now();
    }

    /// Rebuild the pipeline snapshot just before `step_index` executes.
    pub fn rebuild_pipeline_before(&self, step_index: u32) -> Value {
        let mut pipeline = self.initial_input.clone();
        for (_, patch) in self.pipeline_patches.range(..step_index) {
            merge_pipeline_patch(&mut pipeline, patch);
        }
        pipeline
    }

    /// Recover the last non-skipped step state before `step_index`.
    pub fn previous_step_state_before(&self, step_index: u32) -> Option<JobRunState> {
        self.step_states
            .range(..step_index)
            .rev()
            .map(|(_, state)| *state)
            .find(|state| *state != JobRunState::Skipped)
    }
}

fn merge_pipeline_patch(pipeline: &mut Value, patch: &Value) {
    if let (Some(pipeline_map), Some(patch_map)) = (pipeline.as_object_mut(), patch.as_object()) {
        for (key, value) in patch_map {
            pipeline_map.insert(key.clone(), value.clone());
        }
    }
}
