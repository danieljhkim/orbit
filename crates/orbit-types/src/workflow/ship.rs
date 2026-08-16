use crate::workspace::Workspace;

use super::WorkflowError;

/// Pipeline mode for shipping work: open a PR or apply locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShipMode {
    Pr,
    Local,
}

impl ShipMode {
    pub fn as_input_value(self) -> &'static str {
        match self {
            Self::Pr => "pr",
            Self::Local => "local",
        }
    }

    pub fn parse(value: &str) -> Result<Self, WorkflowError> {
        match value {
            "pr" => Ok(Self::Pr),
            "local" => Ok(Self::Local),
            other => Err(WorkflowError::Invalid(format!(
                "unknown ship mode '{other}' (expected 'pr' or 'local')"
            ))),
        }
    }
}

/// Resolve a workspace's effective ship mode, defaulting safely to PR mode.
pub fn resolved_ship_mode(workspace: &Workspace) -> ShipMode {
    workspace
        .ship_mode
        .as_deref()
        .and_then(|value| ShipMode::parse(value).ok())
        .unwrap_or(ShipMode::Pr)
}
