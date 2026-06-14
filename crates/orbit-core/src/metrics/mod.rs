//! Tool-invocation metrics derived from job-run traces.
//!
//! Migrated out of the decommissioned `orbit-knowledge` crate in ORB-00391.
//! The persisted [`orbit_common::types::KnowledgeRunMetrics`] type and the
//! `JobRun::knowledge_metrics` field stay in `orbit-common`; this module owns
//! the pure computation over invocation traces ([`merge_invocation_trace`])
//! and the cross-run aggregation rendered by the dashboard ([`aggregate`]).
//!
//! The v1 `orbit.graph.pack` compression path was dropped with the tool
//! (ORB-00388); only `fs.read` token accounting remains, which reproduces the
//! behavior the prior implementation produced for pack-less runs.

mod ingest;
mod summary;

#[cfg(test)]
mod tests;

pub(crate) use ingest::merge_invocation_trace;
pub use summary::{
    DoubleReadSummary, KnowledgeStatsSummary, RatioSummary, TokenInputSummary, aggregate,
};
