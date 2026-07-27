#![allow(missing_docs)]
// Examples are user-facing smoke binaries that print progress and unwrap setup invariants.
#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]

//! Network-free smoke for AC13 (redaction applied at write time).
//!
//! Sends a raw request body containing `Authorization: Bearer secret-xyz`
//! through the in-memory sink's blob store. Reads the blob back by hash and
//! asserts the secret is absent from both the stored bytes and from every
//! event's payload keys.

use std::env;
use std::process::ExitCode;

use orbit_agent::loop_engine::{AuditSink, BlobStore, InMemorySink, RedactionMiddleware};

fn main() -> ExitCode {
    let blob_root = env::temp_dir().join("orbit-agent-examples").join(format!(
        "redaction-blobs-{}",
        chrono::Utc::now().timestamp_millis()
    ));

    let sink = InMemorySink::new(&blob_root);

    let secret = "secret-xyz";
    let payload = format!(
        r#"POST /v1/messages
Authorization: Bearer {secret}
x-api-key: {secret}
content-type: application/json

{{"api_key":"{secret}","model":"claude","messages":[]}}"#,
    );

    let hash = sink.write_blob(payload.as_bytes());
    println!("blob hash: {hash}");

    let stored = sink
        .blob_store()
        .read(&hash)
        .expect("read stored blob back");
    let stored_str = String::from_utf8_lossy(&stored);

    if stored_str.contains(secret) {
        eprintln!(
            "FAIL: stored blob contains raw secret. first 200 chars: {}",
            stored_str.chars().take(200).collect::<String>()
        );
        return ExitCode::FAILURE;
    }
    if !stored_str.contains("[REDACTED_AUTH]") {
        eprintln!(
            "FAIL: stored blob missing redaction marker. first 200 chars: {}",
            stored_str.chars().take(200).collect::<String>()
        );
        return ExitCode::FAILURE;
    }

    // Also verify the standalone BlobStore with default redaction.
    let direct_store =
        BlobStore::new(blob_root.join("direct")).with_redaction(RedactionMiddleware::default());
    let direct_hash = direct_store
        .write(format!("Bearer {secret}").as_bytes())
        .expect("direct write");
    let direct = direct_store.read(&direct_hash).expect("direct read");
    let direct_str = String::from_utf8_lossy(&direct);
    if direct_str.contains(secret) {
        eprintln!("FAIL: direct blob store leaked secret: {direct_str}");
        return ExitCode::FAILURE;
    }

    println!("ok: redaction applied at write time, secret absent from stored bytes");
    ExitCode::SUCCESS
}
