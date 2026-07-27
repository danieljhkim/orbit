use chrono::Utc;
use orbit_common::types::{RoleSlot, TokenUsage};
use orbit_store::InvocationRecord;

use super::super::metrics::aggregate_token_usage;
use super::super::types::{PlanningDuelEfficiency, into_efficiency_metrics};

fn invocation_record(
    input_tokens: u64,
    cache_read_tokens: u64,
    cache_create_tokens: u64,
    output_tokens: u64,
) -> InvocationRecord {
    InvocationRecord {
        id: 1,
        ts: Utc::now(),
        job_run_id: "jrun-1".to_string(),
        activity_id: "propose_duel_plan".to_string(),
        agent: "codex".to_string(),
        model: Some("gpt-5.6".to_string()),
        slot: Some(RoleSlot::PlannerA),
        duration_ms: 10,
        input_tokens,
        cache_read_tokens,
        cache_create_tokens,
        cache_create_1h_tokens: 0,
        output_tokens,
        total_tokens: input_tokens
            .saturating_add(cache_read_tokens)
            .saturating_add(cache_create_tokens)
            .saturating_add(output_tokens),
        tool_call_count: 0,
        task_ids: vec!["ORB-10339".to_string()],
        tool_calls: Vec::new(),
        provider_cost_usd: None,
        derived_cost_usd: None,
    }
}

#[test]
fn aggregate_token_usage_preserves_all_telemetry_splits() {
    let usage = aggregate_token_usage(&[
        invocation_record(100, 20, 30, 40),
        invocation_record(7, 8, 9, 10),
    ]);

    assert_eq!(usage.input, 107);
    assert_eq!(usage.cache_read, 28);
    assert_eq!(usage.cache_create, 39);
    assert_eq!(usage.output, 50);
}

#[test]
fn efficiency_metrics_preserve_reported_zero_token_usage() {
    let token_usage = TokenUsage::default();
    let metrics = into_efficiency_metrics(PlanningDuelEfficiency {
        token_usage: Some(token_usage.clone()),
        byte_proxy_total: 42,
        ..PlanningDuelEfficiency::default()
    });

    assert_eq!(metrics.token_usage, Some(token_usage));
    assert_eq!(metrics.byte_proxy_total, None);
}
