//! Tracing subscriber setup.
//!
//! One canonical initializer for any Orbit binary. Libraries should emit
//! via `tracing::{info, warn, error, debug, trace}` and never touch the
//! subscriber.
//!
//! Replacement guidance: the 13+ stray `eprintln!`/`println!` calls in
//! library crates (orbit-core, orbit-engine, orbit-store, orbit-knowledge)
//! should become `tracing::warn!` / `tracing::error!` with structured
//! fields. A workspace-level `deny(clippy::print_stderr, clippy::print_stdout)`
//! would enforce this but is left to follow-up.
//!
//! Redaction integration: today this module exposes [`redact_event_text`]
//! for manual pre-emission scrubbing. A proper `tracing::Layer` that applies
//! redaction to recorded fields is a TODO — the right shape is a
//! `tracing_subscriber::Layer` with a `Visit`-implementing formatter that
//! routes each value through [`crate::redaction::redact_all`]. Landing that
//! layer is the follow-up after migrating call sites off `eprintln!`.

use tracing_subscriber::EnvFilter;

use crate::redaction;

/// Install the default fmt + env-filter subscriber. Safe to call multiple
/// times — subsequent calls are no-ops (mirrors the current behaviour in
/// `orbit-cli/src/main.rs`).
///
/// `default_filter` is applied when `RUST_LOG` is unset (e.g. `"warn"`,
/// `"orbit=debug"`).
pub fn init_default_subscriber(default_filter: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .try_init();
}

/// Pre-emission scrubber for callers that need to sanitise a message before
/// handing it to `tracing::*!`. Applies env-value redaction plus the default
/// HTTP header/JSON patterns.
///
/// Prefer emitting structured fields and letting a future `RedactionLayer`
/// handle scrubbing uniformly; this helper exists so call sites can adopt
/// redaction today without blocking on the layer work.
pub fn redact_event_text(message: &str) -> String {
    redaction::redact_all(message)
}
