#![allow(missing_docs)]
// ORB-00013: Examples are user-facing smoke binaries that print progress and unwrap setup invariants.
#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]

//! 1-turn Anthropic prompt demonstrating the HTTP agent loop.
//!
//! Skips cleanly (exit 0, printed notice) when `ANTHROPIC_API_KEY` is unset so
//! that `cargo build --examples` and CI without secrets still work.

use std::env;
use std::time::Duration;

use orbit_agent::loop_engine::{AgentLoop, AgentLoopConfig, InMemorySink, Session};
use orbit_agent::providers::anthropic::AnthropicMessagesTransport;
use orbit_tools::{ToolContext, ToolRegistry};

fn main() {
    let Some(api_key) = env::var("ANTHROPIC_API_KEY").ok().filter(|v| !v.is_empty()) else {
        eprintln!("[skip] ANTHROPIC_API_KEY not set; example exits 0");
        return;
    };

    let model = env::var("ORBIT_EXAMPLE_ANTHROPIC_MODEL")
        .unwrap_or_else(|_| "claude-haiku-4-5-20251001".to_string());

    let transport =
        AnthropicMessagesTransport::new(api_key, &model).expect("build anthropic transport");

    let run_id = format!("anthropic-messages-{}", now_ms());
    let audit_blob_root = env::temp_dir()
        .join("orbit-agent-examples")
        .join("audit-blobs")
        .join(&run_id);
    let sink = InMemorySink::new(&audit_blob_root);

    let registry = ToolRegistry::new();
    let tool_ctx = ToolContext::default();

    let cfg = AgentLoopConfig::new_for_run(&run_id)
        .with_max_iterations(2)
        .with_max_total_tokens(50_000)
        .with_wall_clock_timeout(Duration::from_secs(120))
        .with_max_response_tokens(256);

    let mut session = Session::new(
        "anthropic",
        model,
        "You are a concise assistant. Reply in one short sentence.",
        Some("example:anthropic_messages".to_string()),
    );

    let outcome = AgentLoop::run(
        &mut session,
        &cfg,
        &transport,
        &registry,
        &tool_ctx,
        &sink,
        "In one short sentence, state what HTTP is.",
    )
    .expect("loop run");

    println!("reply: {}", outcome.final_message);
    println!(
        "usage: input={} output={} cache_read={} cache_create={}",
        outcome.usage.input_tokens,
        outcome.usage.output_tokens,
        outcome.usage.cache_read_input_tokens,
        outcome.usage.cache_creation_input_tokens,
    );
    println!("terminate: {:?}", outcome.terminate_reason);
    println!("audit blobs: {}", sink.blob_store().root().display());

    session.close(&run_id, &sink);
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
