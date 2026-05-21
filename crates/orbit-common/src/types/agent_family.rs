use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::OrbitError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentFamily {
    Codex,
    Claude,
    Gemini,
    Grok,
}

impl AgentFamily {
    pub const ALL: [Self; 4] = [Self::Codex, Self::Claude, Self::Gemini, Self::Grok];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::Grok => "grok",
        }
    }

    pub fn parse(value: &str) -> Result<Self, OrbitError> {
        value.parse()
    }
}

impl Display for AgentFamily {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AgentFamily {
    type Err = OrbitError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            "gemini" => Ok(Self::Gemini),
            "grok" => Ok(Self::Grok),
            other => Err(OrbitError::InvalidInput(format!(
                "unknown agent family '{other}'; expected one of codex, claude, gemini, grok"
            ))),
        }
    }
}

#[cfg(test)]
mod tests;
