use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::workflow::JobRunState;
use crate::workflow::child_dispatch::{
    ChildCancellation, ChildCancellationPolicy, ChildDispatch, ChildDispatchPhase,
};

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
    /// Child Runs this run dispatched, in submission order.
    ///
    /// Written the moment `orbit.pipeline.invoke` returns a durable child run
    /// id — before a blocking parent enters its wait — so parent/child lineage
    /// is observable for the whole life of the dispatch rather than only after
    /// the step's output is finally persisted. Unlike the waiting reasons
    /// above, this survives terminalization: a cancelled parent must still
    /// name the child it left behind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub child_dispatches: Vec<ChildDispatch>,
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
            child_dispatches: Vec::new(),
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

    /// Record a child dispatch, keyed by the child's run id.
    ///
    /// Upsert rather than push: a resumed or retried parent re-executing the
    /// same dispatch step must not accumulate duplicate rows for one child.
    /// A re-record keeps the original `submitted_at` so the observable
    /// submission instant does not drift.
    pub fn record_child_dispatch(&mut self, dispatch: ChildDispatch) {
        match self
            .child_dispatches
            .iter_mut()
            .find(|existing| existing.child_run_id == dispatch.child_run_id)
        {
            Some(existing) => {
                let submitted_at = existing.submitted_at;
                *existing = dispatch;
                existing.submitted_at = submitted_at;
            }
            None => self.child_dispatches.push(dispatch),
        }
        self.updated_at = Utc::now();
    }

    /// Advance a recorded child dispatch. Returns false when no dispatch with
    /// that child run id is recorded, so a caller can tell a lost checkpoint
    /// from a successful update instead of silently succeeding.
    pub fn advance_child_dispatch(
        &mut self,
        child_run_id: &str,
        phase: ChildDispatchPhase,
        child_status: Option<String>,
        error: Option<String>,
    ) -> bool {
        let Some(dispatch) = self
            .child_dispatches
            .iter_mut()
            .find(|dispatch| dispatch.child_run_id == child_run_id)
        else {
            return false;
        };
        dispatch.phase = phase;
        if child_status.is_some() {
            dispatch.child_status = child_status;
        }
        if error.is_some() {
            dispatch.error = error;
        }
        dispatch.updated_at = Utc::now();
        self.updated_at = Utc::now();
        true
    }

    /// Every child dispatch the parent still considers open.
    pub fn open_child_dispatches(&self) -> impl Iterator<Item = &ChildDispatch> {
        self.child_dispatches
            .iter()
            .filter(|dispatch| dispatch.phase.is_open())
    }

    /// Close an open dispatch because the parent itself terminalized, and
    /// record which cancellation policy was applied to the child.
    ///
    /// The linkage itself is never dropped: an operator who cancels a parent
    /// mid-wait still needs the child run id, which is the only handle on the
    /// work that outlived (or was stopped with) the parent.
    pub fn terminalize_child_dispatch(
        &mut self,
        child_run_id: &str,
        cancellation: ChildCancellation,
    ) -> bool {
        let Some(dispatch) = self
            .child_dispatches
            .iter_mut()
            .find(|dispatch| dispatch.child_run_id == child_run_id)
        else {
            return false;
        };
        dispatch.phase = ChildDispatchPhase::Terminal;
        dispatch.cancellation = Some(cancellation);
        dispatch.updated_at = Utc::now();
        self.updated_at = Utc::now();
        true
    }

    /// The child run ids a terminalizing parent must cancel, per each
    /// dispatch's own [`ChildCancellationPolicy`].
    pub fn cascade_cancellation_targets(&self) -> Vec<String> {
        self.open_child_dispatches()
            .filter(|dispatch| dispatch.cancellation_policy() == ChildCancellationPolicy::Cascade)
            .map(|dispatch| dispatch.child_run_id.clone())
            .collect()
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
