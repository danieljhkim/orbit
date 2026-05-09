//! v2 `agent_loop` backend resolution per §3.1.
//!
//! Resolves `Backend::Auto` to a concrete `Backend::{Http, Cli}` using the
//! precedence chain:
//!   1. `--backend=<value>` CLI flag (explicit invocation-level override).
//!   2. `ORBIT_BACKEND` env var.
//!   3. `[runtime] backend = "<value>"` in `config.toml`.
//!   4. Hard-coded fallback: `Cli`.
//!
//! Called once per run at load time by direct activity helpers and
//! `orbit job run` entry points. The resolved value is then applied to the
//! parsed asset via `orbit_common::types::activity_job::resolve_*_backends` and the §3.2
//! loader-rejection validator runs against the concrete backends.

use orbit_common::types::activity_job::Backend;

use crate::OrbitRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendSource {
    Flag,
    Env,
    Config,
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedBackend {
    pub backend: Backend,
    pub source: BackendSource,
}

impl OrbitRuntime {
    /// Resolve `Auto` to a concrete backend per §3.1 precedence. Uses the
    /// current process env for `ORBIT_BACKEND` and the workspace config for
    /// `[runtime] backend`.
    pub fn resolve_v2_backend(&self, flag_override: Option<Backend>) -> ResolvedBackend {
        resolve_backend_precedence(
            flag_override,
            std::env::var("ORBIT_BACKEND").ok().as_deref(),
            self.v2_backend_config(),
        )
    }
}

/// Pure precedence logic (flag → env → config → default) — exposed so smokes
/// and callers without an `OrbitRuntime` can verify each tier deterministically.
pub fn resolve_backend_precedence(
    flag_override: Option<Backend>,
    env_value: Option<&str>,
    config_value: Option<&str>,
) -> ResolvedBackend {
    if let Some(backend) = flag_override {
        return ResolvedBackend {
            backend: concretize(backend, Backend::Cli),
            source: BackendSource::Flag,
        };
    }
    if let Some(raw) = env_value
        && let Some(backend) = Backend::parse(raw)
    {
        return ResolvedBackend {
            backend: concretize(backend, Backend::Cli),
            source: BackendSource::Env,
        };
    }
    if let Some(raw) = config_value
        && let Some(backend) = Backend::parse(raw)
    {
        return ResolvedBackend {
            backend: concretize(backend, Backend::Cli),
            source: BackendSource::Config,
        };
    }
    ResolvedBackend {
        backend: Backend::Cli,
        source: BackendSource::Default,
    }
}

/// Parse a raw `--backend` flag value into a `Backend`. Returns `Ok(None)`
/// when the user didn't pass the flag; `Err(...)` on an unknown value so the
/// CLI can reject it before dispatch.
pub fn parse_backend_flag(raw: Option<&str>) -> Result<Option<Backend>, String> {
    match raw {
        None => Ok(None),
        Some(value) => Backend::parse(value).map(Some).ok_or_else(|| {
            format!("invalid --backend value `{value}`; expected one of: http, cli, auto")
        }),
    }
}

/// §3.1 guarantees the resolver produces a concrete `Http` or `Cli`. When an
/// intermediate precedence tier literally stores `auto`, we fold it to the
/// hard-coded fallback so downstream code never observes `Auto`.
fn concretize(backend: Backend, fallback: Backend) -> Backend {
    match backend {
        Backend::Auto => fallback,
        Backend::Http => Backend::Http,
        Backend::Cli => Backend::Cli,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_uses_cli_fallback() {
        let resolved = resolve_backend_precedence(None, None, None);

        assert_eq!(resolved.backend, Backend::Cli);
        assert_eq!(resolved.source, BackendSource::Default);
    }

    #[test]
    fn auto_config_folds_to_cli_fallback() {
        let resolved = resolve_backend_precedence(None, None, Some("auto"));

        assert_eq!(resolved.backend, Backend::Cli);
        assert_eq!(resolved.source, BackendSource::Config);
    }

    #[test]
    fn explicit_flag_overrides_env_and_config() {
        let resolved = resolve_backend_precedence(Some(Backend::Http), Some("cli"), Some("cli"));

        assert_eq!(resolved.backend, Backend::Http);
        assert_eq!(resolved.source, BackendSource::Flag);
    }

    #[test]
    fn explicit_env_overrides_config() {
        let resolved = resolve_backend_precedence(None, Some("http"), Some("cli"));

        assert_eq!(resolved.backend, Backend::Http);
        assert_eq!(resolved.source, BackendSource::Env);
    }

    #[test]
    fn auto_flag_folds_to_cli_fallback() {
        let resolved = resolve_backend_precedence(Some(Backend::Auto), Some("http"), Some("http"));

        assert_eq!(resolved.backend, Backend::Cli);
        assert_eq!(resolved.source, BackendSource::Flag);
    }
}
