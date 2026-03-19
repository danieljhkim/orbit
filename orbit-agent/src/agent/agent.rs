use orbit_types::OrbitError;

use crate::runtime::{AgentRuntime, RuntimeBackend, resolve_runtime};
use crate::types::{AgentRequest, AgentResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    pub command: String,
    pub model: Option<String>,
    pub codex_sandbox: Option<String>,
    pub codex_approval_policy: Option<String>,
}

impl AgentConfig {
    pub fn cli(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            model: None,
            codex_sandbox: None,
            codex_approval_policy: None,
        }
    }

    pub fn with_model(mut self, model: Option<&str>) -> Self {
        self.model = model.map(ToString::to_string);
        self
    }

    pub fn with_codex_execution(
        mut self,
        sandbox: impl Into<String>,
        approval_policy: Option<&str>,
    ) -> Self {
        self.codex_sandbox = Some(sandbox.into());
        self.codex_approval_policy = approval_policy.map(ToString::to_string);
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
}
