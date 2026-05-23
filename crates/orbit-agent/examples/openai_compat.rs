#![allow(missing_docs)]
// ORB-00013: Examples are user-facing smoke binaries that print progress and unwrap setup invariants.
#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]

//! 1-turn OpenAI-compatible prompt demonstrating the HTTP agent loop.
//!
//! Hosted OpenAI requires `OPENAI_API_KEY`; local OpenAI-compatible endpoints
//! can be exercised by pointing `OPENAI_BASE_URL` at `localhost`. An
//! unreachable localhost endpoint is treated as a clean skip so the example
//! remains safe in environments without a local model server.

use std::env;
use std::time::Duration;

use orbit_agent::loop_engine::{
    AgentLoop, AgentLoopConfig, AgentLoopError, InMemorySink, Session, TransportError,
};
use orbit_agent::providers::openai_compat::OpenAiCompatTransport;
use orbit_tools::{ToolContext, ToolRegistry};

fn main() {
    let base_url = env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com".into());
    let model =
        env::var("ORBIT_EXAMPLE_OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    let api_key = env::var("OPENAI_API_KEY").ok().filter(|v| !v.is_empty());
    let local_endpoint = is_local_endpoint(&base_url);

    if api_key.is_none() && !local_endpoint {
        eprintln!("[skip] OPENAI_API_KEY not set; example exits 0");
        return;
    }

    let transport = OpenAiCompatTransport::new(
        base_url.clone(),
        api_key.clone().unwrap_or_default(),
        &model,
        Vec::new(),
    )
    .expect("build openai-compatible transport")
    .with_bearer_auth(api_key.is_some())
    .with_timeout(Duration::from_secs(120))
    .expect("configure openai-compatible transport timeout");

    let run_id = format!("openai-compat-{}", now_ms());
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
        "openai_compat",
        model,
        "You are a concise assistant. Reply in one short sentence.",
        Some("example:openai_compat".to_string()),
    );

    let outcome = AgentLoop::run(
        &mut session,
        &cfg,
        &transport,
        &registry,
        &tool_ctx,
        &sink,
        "In one short sentence, state what HTTP is.",
    );

    match outcome {
        Ok(outcome) => {
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
        Err(AgentLoopError::Transport(TransportError::Network(message))) if local_endpoint => {
            eprintln!(
                "[skip] local OpenAI-compatible endpoint {} not reachable: {}",
                base_url, message
            );
            session.drop_quiet();
        }
        Err(err) => panic!("openai-compatible example failed: {err}"),
    }
}

fn is_local_endpoint(base_url: &str) -> bool {
    let lowered = base_url.to_ascii_lowercase();
    lowered.contains("://localhost")
        || lowered.contains("://127.0.0.1")
        || lowered.contains("://[::1]")
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
