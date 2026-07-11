use orbit_common::types::ExecutorType;
use orbit_common::types::activity_job::{Provider, ProviderEntryPoint};
use orbit_engine::{DispatchError, ResolvedCliExecutor};

use crate::OrbitRuntime;

/// Map a v2 provider name to the CLI executor that dispatches it. Env-var
/// overrides (`ORBIT_V2_CLI_<PROVIDER>`) let smokes substitute a fixture
/// binary for the real provider CLI; production normally comes from the
/// registered executor def, falling back to the provider name itself
/// (`claude`, `codex`, `gemini`, `grok`, `ollama`) when no executor is registered.
pub(super) fn resolve_cli_executor(
    runtime: &OrbitRuntime,
    provider: &str,
) -> Result<ResolvedCliExecutor, DispatchError> {
    let env_key = format!("ORBIT_V2_CLI_{}", provider.to_ascii_uppercase());
    let env_command = std::env::var(&env_key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if let Some(def) = runtime.get_executor_def(provider).map_err(|err| {
        DispatchError::CliInvocationFailed(format!("load executor `{provider}`: {err}"))
    })? {
        if !matches!(
            def.executor_type,
            ExecutorType::DirectAgent | ExecutorType::AgentCli
        ) {
            return Err(DispatchError::CliInvocationFailed(format!(
                "executor `{provider}` has type `{}`; backend: cli requires a direct_agent or agent_cli executor",
                def.executor_type
            )));
        }

        let command = env_command
            .or_else(|| {
                def.command
                    .as_ref()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
            .ok_or_else(|| {
                DispatchError::CliInvocationFailed(format!(
                    "executor `{provider}` is missing a command"
                ))
            })?;

        return Ok(ResolvedCliExecutor {
            command,
            args: def.args,
        });
    }

    if let Some(command) = env_command {
        return Ok(ResolvedCliExecutor {
            command,
            args: Vec::new(),
        });
    }

    // Canonical provider identity + capability live on the orbit-common
    // `Provider` surface (ORB-10091). Selecting a provider that parses but has
    // no CLI runtime (`openai_compat`, HTTP-only) or does not parse at all
    // fails with a stable diagnostic and never silently falls back to a
    // different runtime.
    match Provider::parse(provider) {
        Ok(parsed)
            if Provider::capabilities(ProviderEntryPoint::Orbit).contains(&parsed)
                && parsed.has_cli_runtime() =>
        {
            Ok(ResolvedCliExecutor {
                command: parsed.as_str().to_string(),
                args: Vec::new(),
            })
        }
        Ok(Provider::OpenaiCompat) => Err(DispatchError::CliInvocationFailed(
            "provider openai_compat is unsupported by the Orbit CLI entry point (HTTP-only)"
                .to_string(),
        )),
        Ok(parsed) => Err(DispatchError::CliInvocationFailed(format!(
            "provider {parsed} is unsupported by the Orbit CLI entry point"
        ))),
        Err(err) => Err(DispatchError::CliInvocationFailed(format!(
            "{err} — no CLI runtime registered"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use orbit_engine::V2RuntimeHost;

    use crate::OrbitRuntime;
    use crate::runtime::v2_host::test_support::seed_executor;

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
}
