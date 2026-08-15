use orbit_common::types::{InvocationTrace, KnowledgeRunMetrics};

use super::summary::ratio;

/// Fold a new invocation into persisted [`KnowledgeRunMetrics`].
///
/// The v1 pack-compression path was removed in ORB-00391. ORB-10828 then
/// retired the last builtin whose result tokens fed the read-token counters,
/// so a fresh trace no longer creates knowledge metrics. Historical rows keep
/// their stored `actual_fs_read_tokens_during_run` / `double_read_rate` values;
/// later traces only accumulate LLM input tokens onto those rows.
pub(crate) fn merge_invocation_trace(
    existing: Option<&KnowledgeRunMetrics>,
    trace: &InvocationTrace,
) -> Option<KnowledgeRunMetrics> {
    let mut metrics = existing.cloned()?;
    metrics.total_llm_input_tokens = metrics.total_llm_input_tokens.saturating_add(
        trace
            .usage
            .input
            .saturating_add(trace.usage.cache_read)
            .saturating_add(trace.usage.cache_create)
            .saturating_add(trace.usage.cache_create_1h),
    );

    metrics.compression_ratio = metrics
        .knowledge_pack_used
        .then(|| {
            ratio(
                metrics.raw_read_token_baseline,
                metrics.knowledge_pack_tokens.unwrap_or(0),
            )
        })
        .flatten();

    Some(metrics)
}
