//! Four precedence smokes for `Backend::Auto` resolution per §3.1.
//!
//! AC #2 of T20260419-0104: verify each tier of the flag → env → config →
//! default chain with a separate scenario. Uses the pure
//! `resolve_backend_precedence` function so no OrbitRuntime / workspace
//! scaffolding is required.

use orbit_core::command::backend_resolver::{BackendSource, resolve_backend_precedence};
use orbit_types::v2::Backend;

fn main() {
    println!("v2 backend precedence smoke — T20260419-0104 AC #2");

    // Flag wins over all lower tiers.
    let r = resolve_backend_precedence(Some(Backend::Cli), Some("http"), Some("http"));
    assert_eq!(r.backend, Backend::Cli);
    assert_eq!(r.source, BackendSource::Flag);
    println!(
        "  1) flag tier — resolved={} source={:?}",
        r.backend.as_str(),
        r.source
    );

    // Env is consulted when flag is absent.
    let r = resolve_backend_precedence(None, Some("cli"), Some("http"));
    assert_eq!(r.backend, Backend::Cli);
    assert_eq!(r.source, BackendSource::Env);
    println!(
        "  2) env tier — resolved={} source={:?}",
        r.backend.as_str(),
        r.source
    );

    // Config is consulted when flag + env both absent.
    let r = resolve_backend_precedence(None, None, Some("cli"));
    assert_eq!(r.backend, Backend::Cli);
    assert_eq!(r.source, BackendSource::Config);
    println!(
        "  3) config tier — resolved={} source={:?}",
        r.backend.as_str(),
        r.source
    );

    // Default falls through to `http` when nothing is set.
    let r = resolve_backend_precedence(None, None, None);
    assert_eq!(r.backend, Backend::Http);
    assert_eq!(r.source, BackendSource::Default);
    println!(
        "  4) default tier — resolved={} source={:?}",
        r.backend.as_str(),
        r.source
    );

    // `auto` at any lower tier folds to the hard-coded `http` fallback so the
    // dispatcher never sees `Auto` (§3.1 — resolution is one-shot at load).
    let r = resolve_backend_precedence(None, Some("auto"), Some("cli"));
    assert_eq!(
        r.backend,
        Backend::Http,
        "env=auto must fold to http, not cascade to config"
    );
    assert_eq!(r.source, BackendSource::Env);
    println!("  5) env=auto folds to http (no cascade)");

    // Invalid env value is ignored (caller didn't know about it) and we fall
    // through to config.
    let r = resolve_backend_precedence(None, Some("invalid-backend-xyz"), Some("cli"));
    assert_eq!(r.backend, Backend::Cli);
    assert_eq!(r.source, BackendSource::Config);
    println!("  6) unrecognized env value → cascade to config");

    println!("OK — all precedence scenarios passed");
}
