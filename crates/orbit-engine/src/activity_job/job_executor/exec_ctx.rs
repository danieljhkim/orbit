// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use super::*;

pub(super) const DEFAULT_MODEL_FOR_SESSION: &str = "claude-sonnet-4-5";

pub(super) struct ExecCtx<'a> {
    pub(super) run_id: String,
    pub(super) audit: Arc<V2AuditWriter>,
    pub(super) host: &'a dyn V2RuntimeHost,
    pub(super) input: Value,
    pub(super) pipeline: Arc<Mutex<HashMap<String, Value>>>,
    pub(super) sessions: Arc<Mutex<HashMap<String, Session>>>,
    pub(super) recovery_activity: Option<ResolvedRecoveryActivity>,
    /// `Some(value)` inside a fan-out worker. Rendered into template context
    /// as `{{ item }}`.
    pub(super) item: Option<Value>,
    pub(super) iteration: Option<u32>,
}

impl ExecCtx<'_> {
    /// Resolved task id for the activity input, if any. Threaded onto every
    /// job-lifecycle tracing emission so subprocess and step events correlate.
    pub(super) fn task_id(&self) -> Option<&str> {
        super::super::cli_runner::task_id_from_input(&self.input)
    }

    pub(super) fn template_ctx(&self) -> TemplateContext {
        let pipeline = self.pipeline.lock().expect("pipeline poisoned").clone();
        let mut steps: HashMap<String, Value> = HashMap::new();
        for (k, v) in &pipeline {
            steps.insert(k.clone(), wrap_step_output(v));
        }
        let mut input = self.input.clone();
        if let Some(item) = &self.item {
            // Expose item under input.item for template resolution. v1's
            // template engine only splits paths under a named namespace; we
            // reuse the `input.*` namespace to keep the resolver unchanged.
            if let Value::Object(map) = &mut input {
                map.insert("item".to_string(), item.clone());
            }
        }
        if let Some(iteration) = self.iteration
            && let Value::Object(map) = &mut input
        {
            map.insert("iteration".to_string(), Value::from(iteration));
        }
        TemplateContext {
            input,
            env: Default::default(),
            workspace_path: None,
            item: self.item.clone(),
            iteration: self.iteration,
            steps,
        }
    }
}

/// Pipeline outputs are stored raw but the template engine expects
/// `{{ steps.<id>.output.<field> }}`. Wrap them accordingly so callers read
/// with the same `.output.` prefix they would against a v1 step.
pub(super) fn wrap_step_output(raw: &Value) -> Value {
    serde_json::json!({ "output": raw })
}

/// Result of running a single step.
pub(super) struct StepOutcome {
    pub(super) success: bool,
    pub(super) output: Value,
    pub(super) error_code: Option<String>,
    pub(super) message: Option<String>,
    /// `true` when `when:` returned false and the step did not run. Kept for
    /// future callers that need to distinguish a skipped-but-successful step
    /// from one that actually executed.
    #[allow(dead_code)]
    pub(super) skipped: bool,
}

pub(super) fn record_pipeline(ctx: &ExecCtx<'_>, key: &str, v: Value) {
    ctx.pipeline
        .lock()
        .expect("pipeline poisoned")
        .insert(key.to_string(), v);
}

// ---------------------------------------------------------------------------
// Parallel
// ---------------------------------------------------------------------------
