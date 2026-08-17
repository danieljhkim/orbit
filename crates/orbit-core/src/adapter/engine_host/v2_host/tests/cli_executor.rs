use orbit_engine::RuntimeHost;

use crate::OrbitRuntime;
use crate::adapter::engine_host::v2_host::test_support::seed_executor;

#[test]
fn cli_executor_resolution_preserves_registered_static_args() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    seed_executor(&runtime, "codex", None);

    let resolved = runtime
        .resolve_cli_executor("codex")
        .expect("resolve codex executor");

    assert_eq!(resolved.command, "codex");
    assert_eq!(resolved.args, ["exec", "--json"]);
}

/// Table-driven coverage of executor selection through the centralized
/// `Provider` surface (ORB-10091): every provider that ships a CLI runtime
/// resolves to a command named after its canonical id; a provider that
/// parses but is HTTP-only (`openai_compat`) and a provider that does not
/// parse both fail with a stable diagnostic and never fall back.
#[test]
fn cli_executor_selection_diagnostics_table() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    for provider in ["claude", "codex", "gemini", "grok"] {
        let resolved = runtime
            .resolve_cli_executor(provider)
            .unwrap_or_else(|err| panic!("{provider} should resolve: {err:?}"));
        assert_eq!(resolved.command, provider, "command for {provider}");
        assert!(resolved.args.is_empty(), "fallback args for {provider}");
    }

    for unsupported in ["ollama", "openai_compat"] {
        let error = runtime
            .resolve_cli_executor(unsupported)
            .expect_err("unsupported provider must not fall back to CLI");
        assert!(
            format!("{error:?}").contains("unsupported by the Orbit CLI entry point"),
            "unsupported diagnostic for {unsupported}: {error:?}"
        );
    }

    let unknown = runtime
        .resolve_cli_executor("bogus_provider")
        .expect_err("unknown provider must not resolve to a default runtime");
    let msg = format!("{unknown:?}");
    assert!(
        msg.contains("unknown provider"),
        "unknown diagnostic: {msg}"
    );
    assert!(
        msg.contains("no CLI runtime registered"),
        "unknown diagnostic: {msg}"
    );
}
