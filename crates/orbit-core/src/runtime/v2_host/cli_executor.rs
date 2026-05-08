use orbit_common::types::ExecutorType;
use orbit_engine::activity_job::{DispatchError, ResolvedCliExecutor};

use crate::OrbitRuntime;

/// Map a v2 provider name to the CLI executor that dispatches it. Env-var
/// overrides (`ORBIT_V2_CLI_<PROVIDER>`) let smokes substitute a fixture
/// binary for the real provider CLI; production normally comes from the
/// registered executor def, falling back to the provider name itself
/// (`claude`, `codex`, `gemini`, `ollama`) when no executor is registered.
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

    match provider {
        "claude" | "codex" | "gemini" | "ollama" => Ok(ResolvedCliExecutor {
            command: provider.to_string(),
            args: Vec::new(),
        }),
        "openai_compat" => Err(DispatchError::CliInvocationFailed(
            "provider openai_compat has no CLI runtime (HTTP-only)".to_string(),
        )),
        other => Err(DispatchError::CliInvocationFailed(format!(
            "unknown provider `{other}` — no CLI runtime registered"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use orbit_engine::activity_job::V2RuntimeHost;

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
}
