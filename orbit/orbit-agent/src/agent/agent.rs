use std::collections::HashMap;

use orbit_types::OrbitError;

use crate::providers::AgentProvider;
use crate::runtime::{AgentRuntime, RuntimeBackend, resolve_runtime};
use crate::types::{AgentRequest, AgentResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderOptions {
    Claude,
    Codex {
        sandbox: String,
        approval_policy: Option<String>,
        writable_dirs: Vec<String>,
    },
    Gemini,
    Mock,
}

impl ProviderOptions {
    /// Build `ProviderOptions` for a given agent CLI binary, using the supplied
    /// provider-agnostic config map.  Provider-specific keys are extracted
    /// inside each provider's match arm:
    ///
    /// - Codex reads `"sandbox"` (defaults to `"workspace-write"`) and
    ///   `"approval_policy"` (optional).
    ///
    /// Callers that do not need non-default settings can use `AgentConfig::cli()`.
    pub fn for_agent_cli(
        agent_cli: &str,
        config: &HashMap<String, String>,
    ) -> Result<Self, OrbitError> {
        match AgentProvider::detect_from_cli(agent_cli)? {
            AgentProvider::Codex => {
                let sandbox = config
                    .get("sandbox")
                    .cloned()
                    .unwrap_or_else(|| "workspace-write".to_string());
                let approval_policy = config.get("approval_policy").cloned();
                let writable_dirs = config
                    .get("writable_dirs_json")
                    .map(|raw| {
                        serde_json::from_str::<Vec<String>>(raw).map_err(|err| {
                            OrbitError::InvalidInput(format!(
                                "invalid codex writable_dirs_json provider option: {err}"
                            ))
                        })
                    })
                    .transpose()?
                    .unwrap_or_default();
                Ok(Self::Codex {
                    sandbox,
                    approval_policy,
                    writable_dirs,
                })
            }
            AgentProvider::Claude => Ok(Self::Claude),
            AgentProvider::Gemini => Ok(Self::Gemini),
            AgentProvider::MockAgent => Ok(Self::Mock),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    pub command: String,
    pub model: Option<String>,
    pub provider_options: ProviderOptions,
}

impl AgentConfig {
    /// Construct an `AgentConfig` from a CLI binary name, detecting the
    /// provider automatically.  Codex defaults to `workspace-write` sandbox
    /// and no approval-policy override; use `ProviderOptions::for_agent_cli`
    /// directly when non-default Codex settings are required.
    pub fn cli(command: impl Into<String>) -> Result<Self, OrbitError> {
        let command = command.into();
        let provider_options = ProviderOptions::for_agent_cli(&command, &HashMap::new())?;
        Ok(Self {
            command,
            model: None,
            provider_options,
        })
    }

    pub fn with_model(mut self, model: Option<&str>) -> Self {
        self.model = model.map(ToString::to_string);
        self
    }
}

pub struct Agent {
    runtime: RuntimeBackend,
}

impl Agent {
    pub fn new(cfg: &AgentConfig) -> Result<Self, OrbitError> {
        Ok(Self {
            runtime: resolve_runtime(cfg)?,
        })
    }

    pub fn invoke(&self, req: AgentRequest) -> Result<AgentResponse, OrbitError> {
        self.runtime.invoke(req)
    }

    pub fn model_name(&self) -> Option<&str> {
        self.runtime.model_name()
    }
}
