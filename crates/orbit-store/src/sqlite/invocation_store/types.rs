use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use orbit_common::types::{InvocationTrace, RoleSlot};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct InvocationQuery {
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub job_run_id: Option<String>,
    pub activity_id: Option<String>,
    pub task_id: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub slot: Option<RoleSlot>,
    pub tool_name: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InvocationInsertParams {
    pub job_run_id: String,
    pub activity_id: String,
    pub agent: String,
    /// The model string the invocation was dispatched with — an exact provider
    /// model string, or the unversioned crew alias it was dispatched through.
    /// [ORB-10354] The store resolves an alias before insert, so the persisted
    /// `model` column always carries an exact string (or NULL) and the alias
    /// lands in `model_alias`. Callers do not pre-resolve.
    pub model: Option<String>,
    pub slot: Option<RoleSlot>,
    pub task_ids: Vec<String>,
    pub trace: InvocationTrace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InvocationToolCallRecord {
    pub invocation_id: i64,
    pub seq: u64,
    pub tool_name: String,
    pub result_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InvocationRecord {
    pub id: i64,
    pub ts: DateTime<Utc>,
    pub job_run_id: String,
    pub activity_id: String,
    pub agent: String,
    /// Exact provider model string, or `None` when the invocation was
    /// dispatched through an alias Orbit could not resolve. [ORB-10354] Never a
    /// crew alias — the price table is keyed by exact strings only, so an alias
    /// here would derive no cost and split per-model aggregates.
    pub model: Option<String>,
    /// The unversioned crew alias the invocation was dispatched through
    /// (`opus`, `sonnet`, `fable`, `pro`), when it was dispatched through one.
    /// Provenance metadata: `None` whenever the caller supplied an exact model
    /// string.
    pub model_alias: Option<String>,
    pub slot: Option<RoleSlot>,
    pub duration_ms: u64,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_create_tokens: u64,
    /// Premium 1-hour-TTL cache-creation tokens (`TokenUsage::cache_create_1h`).
    pub cache_create_1h_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub tool_call_count: u64,
    pub task_ids: Vec<String>,
    pub tool_calls: Vec<InvocationToolCallRecord>,
    /// Provider-reported total cost in USD, persisted verbatim from
    /// [`InvocationInsertParams::trace`] for monthly manual reconciliation.
    /// Never overwritten by `derived_cost_usd`.
    pub provider_cost_usd: Option<f64>,
    /// Normalized cost in USD derived at query time from `model`, `ts`, and
    /// the token splits against the versioned price table
    /// (`orbit_common::types::pricing`). `None` when no price row covers
    /// this model/date.
    pub derived_cost_usd: Option<f64>,
}

/// One observed `invocations.model` string that no price row covers
/// ([ORB-10354]). Produced by [`crate::Store::list_unpriced_invocation_models`]
/// — the live-store form of the curated fleet-coverage guard.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnpricedModelRow {
    pub model: String,
    pub invocation_count: u64,
    /// RFC3339 timestamps, verbatim from the store.
    pub first_seen: String,
    pub last_seen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityInvocationMetrics {
    pub activity_id: String,
    pub agent: String,
    pub model: Option<String>,
    pub invocation_count: u64,
    pub total_input_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_create_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub avg_tokens: f64,
    pub p50_tokens: u64,
    pub p95_tokens: u64,
    pub total_tool_calls: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentInvocationMetrics {
    pub agent: String,
    pub model: Option<String>,
    pub invocation_count: u64,
    pub total_input_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_create_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub avg_tokens: f64,
    pub p50_tokens: u64,
    pub p95_tokens: u64,
    pub total_tool_calls: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskInvocationMetrics {
    pub task_id: String,
    pub invocation_count: u64,
    pub total_input_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_create_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub total_tool_calls: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolInvocationMetrics {
    pub activity_id: String,
    pub tool_name: String,
    pub call_count: u64,
    pub avg_result_bytes: f64,
    pub total_result_bytes: u64,
}
