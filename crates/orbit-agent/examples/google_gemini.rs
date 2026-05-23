#![allow(missing_docs)]
// ORB-00013: Examples are user-facing smoke binaries that print progress and unwrap setup invariants.
#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]

//! 1-turn Gemini prompt demonstrating the HTTP agent loop.
//!
//! Requires `GEMINI_API_KEY`. If unset, the example exits cleanly to
//! keep CI usage safe in credential-free environments.

use std::env;
use std::time::Duration;

use orbit_agent::loop_engine::{AgentLoop, AgentLoopConfig, InMemorySink, Session};
use orbit_agent::providers::gemini_http::GeminiHttpTransport;
use orbit_tools::{ToolContext, ToolRegistry};

fn main() {
    let api_key = env::var("GEMINI_API_KEY").ok().filter(|v| !v.is_empty());

    if api_key.is_none() {
        eprintln!("[skip] GEMINI_API_KEY not set; example exits 0");
        return;
    }

    let model =
        env::var("ORBIT_EXAMPLE_GEMINI_MODEL").unwrap_or_else(|_| "gemini-1.5-flash".to_string());

    let transport = GeminiHttpTransport::new(api_key.unwrap(), &model, Some(2))
        .expect("build gemini transport")
        .with_timeout(Duration::from_secs(120))
        .expect("configure gemini transport timeout");

    let run_id = format!("google-gemini-{}", now_ms());
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
        "gemini_http",
        model,
        "You are a concise assistant. Reply in one short sentence.",
        Some("example:google_gemini".to_string()),
    );

    let outcome = AgentLoop::run(
        &mut session,
        &cfg,
        &transport,
        &registry,
        &tool_ctx,
        &sink,
        "In one short sentence, state what an API is.",
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
        Err(err) => panic!("google gemini example failed: {err}"),
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
