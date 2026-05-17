use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ExecutorResourceSpec is the persisted wire shape; ExecutorDef is the runtime shape.
use super::resource::ExecutorResourceSpec;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorType {
    AgentCli,
    DirectAgent,
    CliCommand,
}

impl ExecutorType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentCli => "agent_cli",
            Self::DirectAgent => "direct_agent",
            Self::CliCommand => "cli_command",
        }
    }
}

impl fmt::Display for ExecutorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Sandbox primitive applied to a CLI-backend agent invocation. The variant
/// names a concrete OS primitive; `orbit-exec` selects the implementation.
///
/// Today only `macos-sandbox-exec` is wired; a future Linux variant
/// (`linux-bwrap` or similar) can land alongside without changing the
/// schema shape.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutorSandboxKind {
    MacosSandboxExec,
}

impl ExecutorSandboxKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MacosSandboxExec => "macos-sandbox-exec",
        }
    }
}

impl fmt::Display for ExecutorSandboxKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StdoutFormat {
    Envelope,
    Json,
    Text,
}

impl StdoutFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Envelope => "envelope",
            Self::Json => "json",
            Self::Text => "text",
        }
    }
}

impl fmt::Display for StdoutFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutorDef {
    pub name: String,
    /// Executor family, serialized as "agent_cli", "direct_agent", or "cli_command".
    pub executor_type: ExecutorType,
    /// For agent_cli: the CLI command (e.g., "claude", "codex")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Expected stdout format, serialized as "envelope", "json", or "text".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_format: Option<StdoutFormat>,
    /// Overrides the agent family's default `AgentModelPair` resolution for audit
    /// canonicalization, envelope rendering, and review attribution.
    ///
    /// Does NOT control which model the subprocess actually runs; operators
    /// should encode runtime model selection in `args`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_pair_override: Option<ModelPairOverride>,
    /// CLI flag name used to pass `JobStep.model` to a direct-agent subprocess.
    ///
    /// Carries only the flag name, for example `"-m"` or `"--model"`. At
    /// invocation time, when both `model_flag` and the step's runtime model are
    /// present, `direct_agent` appends `[model_flag, step.model]` after the
    /// operator-declared `args`. Orbit does not inspect `args` for duplicates;
    /// the CLI's own last-wins behavior resolves repeated model flags. When
    /// either field is absent, nothing is injected, so operators can still
    /// hardcode fixed model arguments such as `--model X` in `args`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_flag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// OS sandbox primitive to wrap the CLI invocation in. When `None`, the
    /// CLI is spawned bare (today's behavior). When `Some`, `orbit-exec`
    /// translates the activity's `FsProfile` into a sandbox payload and
    /// wraps the spawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<ExecutorSandboxKind>,
    /// When `sandbox` is set but the platform's trusted sandbox primitive is
    /// unavailable (e.g. `/usr/bin/sandbox-exec` is missing), should the runner
    /// degrade to bare exec? Default `false` (fail-closed).
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_fallback: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Override for an agent family's strong/weak `AgentModelPair`.
///
/// Controls how Orbit canonicalizes the agent's model for audit trail,
/// envelope rendering, and review automation attribution.
///
/// Does NOT control which model the subprocess actually runs. Operators must
/// encode the runtime model in `args`, and may set `ORBIT_AGENT_MODEL` via
/// `env:` for explicit audit attribution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct ModelPairOverride {
    pub strong: String,
    pub weak: String,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl ExecutorDef {
    pub fn from_resource_spec(
        name: String,
        spec: ExecutorResourceSpec,
        source_label: &str,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        let ExecutorResourceSpec {
            executor_type,
            command,
            args,
            stdout_format,
            model_pair_override,
            legacy_models,
            model_flag,
            timeout_seconds,
            env,
            sandbox,
            allow_fallback,
            created_at: _,
            updated_at: _,
        } = spec;

        if legacy_models.is_some() {
            tracing::warn!(
                target: "orbit.executor.def",
                source = %source_label,
                "deprecated `models` key on executor def; rename to `model_pair_override` for AgentModelPair audit/envelope/review overrides. Runtime model selection is not controlled by this field; encode it in `args`, and set `ORBIT_AGENT_MODEL` via `env:` for explicit audit attribution."
            );
        }

        Self {
            name,
            executor_type,
            command,
            args,
            stdout_format,
            model_pair_override: model_pair_override.or(legacy_models),
            model_flag,
            timeout_seconds,
            env,
            sandbox,
            allow_fallback,
            created_at,
            updated_at,
        }
    }

    pub fn model_pair_override(&self) -> Option<&ModelPairOverride> {
        self.model_pair_override.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt::MakeWriter;

    use super::*;
    use crate::types::ExecutorResource;

    #[derive(Clone)]
    struct CaptureMakeWriter(Arc<Mutex<Vec<u8>>>);

    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl<'a> MakeWriter<'a> for CaptureMakeWriter {
        type Writer = CaptureWriter;

        fn make_writer(&'a self) -> Self::Writer {
            CaptureWriter(Arc::clone(&self.0))
        }
    }

    impl Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("capture writer lock")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn capture_warnings<F, T>(f: F) -> (T, String)
    where
        F: FnOnce() -> T,
    {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(CaptureMakeWriter(Arc::clone(&buffer)))
            .with_max_level(LevelFilter::WARN)
            .with_target(true)
            .with_ansi(false)
            .without_time()
            .finish();
        let result = tracing::subscriber::with_default(subscriber, f);
        let logs = String::from_utf8(buffer.lock().expect("capture buffer lock").clone())
            .expect("captured logs are utf8");
        (result, logs)
    }

    fn def_from_yaml(yaml: &str, source_label: &str) -> ExecutorDef {
        let resource: ExecutorResource = serde_yaml::from_str(yaml).expect("parse executor yaml");
        ExecutorDef::from_resource_spec(
            resource.metadata.name,
            resource.spec.clone(),
            source_label,
            resource.spec.created_at,
            resource.spec.updated_at,
        )
    }

    #[test]
    fn roundtrips_model_pair_override_without_legacy_models_key() {
        let def = def_from_yaml(
            r#"
schemaVersion: 2
kind: Executor
metadata:
  name: gemini
spec:
  executor_type: direct_agent
  command: gemini
  args:
    - -m
    - gemini-3.1-pro
  model_pair_override:
    strong: gemini-3.1-pro
    weak: gemini-3-flash
  model_flag: "-m"
"#,
            "roundtrip",
        );

        assert_eq!(
            def.model_pair_override(),
            Some(&ModelPairOverride {
                strong: "gemini-3.1-pro".to_string(),
                weak: "gemini-3-flash".to_string(),
            })
        );
        assert_eq!(def.model_flag.as_deref(), Some("-m"));

        let serialized = serde_yaml::to_string(&def).expect("serialize executor def");
        assert!(
            serialized.contains("model_pair_override:"),
            "serialized executor def should use new key: {serialized}"
        );
        assert!(
            serialized.contains("model_flag: -m"),
            "serialized executor def should include model flag: {serialized}"
        );
        assert!(
            !serialized.contains("models:"),
            "serialized executor def should not use legacy key: {serialized}"
        );
    }

    #[test]
    fn legacy_models_deserializes_with_deprecation_warning() {
        let (def, logs) = capture_warnings(|| {
            def_from_yaml(
                r#"
schemaVersion: 2
kind: Executor
metadata:
  name: gemini
spec:
  executor_type: direct_agent
  command: gemini
  models:
    strong: gemini-3.1-pro
    weak: gemini-3-flash
"#,
                "legacy",
            )
        });

        assert_eq!(
            def.model_pair_override(),
            Some(&ModelPairOverride {
                strong: "gemini-3.1-pro".to_string(),
                weak: "gemini-3-flash".to_string(),
            })
        );
        assert_eq!(
            logs.matches("deprecated `models` key").count(),
            1,
            "expected one deprecation warning, got: {logs}"
        );
        assert!(
            logs.contains("orbit.executor.def"),
            "warning should use executor def target: {logs}"
        );
        assert!(logs.contains("model_pair_override"), "{logs}");
        assert!(logs.contains("AgentModelPair"), "{logs}");
        assert!(logs.contains("args"), "{logs}");
        assert!(logs.contains("env:"), "{logs}");
        assert!(logs.contains("ORBIT_AGENT_MODEL"), "{logs}");
    }

    #[test]
    fn legacy_models_rejects_unknown_keys() {
        let err = serde_yaml::from_str::<ExecutorResource>(
            r#"
schemaVersion: 2
kind: Executor
metadata:
  name: gemini
spec:
  executor_type: direct_agent
  command: gemini
  models:
    strong: gemini-3.1-pro
    weak: gemini-3-flash
    extra: unsupported
"#,
        )
        .expect_err("unknown model-pair keys should be rejected");

        assert!(
            err.to_string().contains("extra"),
            "error should name the unsupported key: {err}"
        );
    }
}
