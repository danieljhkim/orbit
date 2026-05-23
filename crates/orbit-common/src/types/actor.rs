use std::fmt::{Display, Formatter};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::types::agent_pair::{agent_family_from_cli, all_agent_families};

/// Typed identity for attribution across all Orbit artifacts.
///
/// Orbit stores model-only agent attribution. The agent family is derived from
/// the model string at render or routing time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ActorIdentity {
    /// The Orbit system itself (automated processes with no specific agent).
    #[default]
    System,
    /// An AI agent model identifier.
    Agent { model: String },
    /// A human operator.
    Human { label: String },
}

impl ActorIdentity {
    /// Construct an `Agent` variant, trimming whitespace.
    pub fn agent(model: impl Into<String>) -> Self {
        Self::Agent {
            model: model.into().trim().to_string(),
        }
    }

    /// Construct a `Human` variant, trimming whitespace.
    pub fn human(label: impl Into<String>) -> Self {
        Self::Human {
            label: label.into().trim().to_string(),
        }
    }

    /// Build from a legacy `(Option<String>, Option<String>)` agent/model pair.
    /// The model wins because the agent family is derivable from it.
    pub fn from_legacy(agent: Option<&str>, model: Option<&str>) -> Self {
        if let Some(model) = model.map(str::trim).filter(|s| !s.is_empty()) {
            return Self::Agent {
                model: model.to_string(),
            };
        }
        if let Some(agent) = agent.map(str::trim).filter(|s| !s.is_empty()) {
            return Self::Agent {
                model: agent.to_string(),
            };
        }
        Self::System
    }

    /// Returns the derived agent family if this is a known `Agent` model.
    pub fn agent_name(&self) -> Option<&str> {
        match self {
            Self::Agent { model } => agent_from_model(model),
            _ => None,
        }
    }

    /// Returns the model name if this is an `Agent` variant.
    pub fn agent_model(&self) -> Option<&str> {
        match self {
            Self::Agent { model } if !model.is_empty() => Some(model),
            _ => None,
        }
    }

    /// Returns a display label suitable for use in `created_by`, `assigned_to`, etc.
    pub fn label(&self) -> String {
        match self {
            Self::System => "system".to_string(),
            Self::Agent { model } => model.clone(),
            Self::Human { label } => label.clone(),
        }
    }

    /// Returns `true` if this is an `Agent` variant.
    pub fn is_agent(&self) -> bool {
        matches!(self, Self::Agent { .. })
    }

    /// Returns `true` if this is the `System` variant.
    pub fn is_system(&self) -> bool {
        matches!(self, Self::System)
    }

    /// Returns `true` if this is a `Human` variant.
    pub fn is_human(&self) -> bool {
        matches!(self, Self::Human { .. })
    }

    /// Decompose into legacy `(Option<String>, Option<String>)` for backward
    /// compatibility with code that still expects separate agent/model fields.
    pub fn to_legacy(&self) -> (Option<String>, Option<String>) {
        match self {
            Self::System => (None, None),
            Self::Agent { model } if model.is_empty() => (None, None),
            Self::Agent { model } => (
                agent_from_model(model).map(ToOwned::to_owned),
                Some(model.clone()),
            ),
            Self::Human { .. } => (None, None),
        }
    }
}

impl Display for ActorIdentity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Infer Orbit's agent family from a model identifier.
pub fn agent_from_model(model: &str) -> Option<&'static str> {
    let model = model.trim().to_ascii_lowercase();
    if model.is_empty() {
        return None;
    }

    if model.starts_with("claude-") || model.starts_with("opus") || model.starts_with("sonnet") {
        return Some("claude");
    }
    if model.starts_with("gpt-") || model.starts_with("o1") || model.starts_with("o3") {
        return Some("codex");
    }
    if model.starts_with("gemini-") {
        return Some("gemini");
    }
    if model.starts_with("ollama:") {
        return Some("ollama");
    }
    // Grok (xAI) family — covers grok-4, grok-3, and grok3* short names
    if model.starts_with("grok-") || model.starts_with("grok3") {
        return Some("grok");
    }

    None
}

/// Infer a provider family from a model identifier.
pub fn provider_from_model(model: &str) -> Option<&'static str> {
    match agent_from_model(model)? {
        "claude" => Some("anthropic"),
        "codex" => Some("openai"),
        "gemini" => Some("google"),
        "ollama" => Some("ollama"),
        "grok" => Some("xai"),
        _ => None,
    }
}

/// Normalize legacy attribution labels to the model-only convention when a
/// concrete model is available.
pub fn normalize_attribution_label(label: &str, model_hint: Option<&str>) -> String {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Some((family, model)) = trimmed.rsplit_once(" / ") {
        let family = family.trim();
        let model = model.trim();
        let is_known_family = family.eq_ignore_ascii_case("agent")
            || all_agent_families()
                .iter()
                .any(|known| family.eq_ignore_ascii_case(known));
        if is_known_family && !model.is_empty() {
            return model.to_string();
        }
    }

    if let Some(model_hint) = model_hint.map(str::trim).filter(|value| !value.is_empty()) {
        let family = agent_family_from_cli(trimmed);
        let is_known_family = all_agent_families()
            .iter()
            .any(|known| family == *known && trimmed.eq_ignore_ascii_case(known));
        if trimmed.eq_ignore_ascii_case("agent") || is_known_family {
            return model_hint.to_string();
        }
    }

    trimmed.to_string()
}

pub fn normalize_optional_attribution_label(
    label: Option<&str>,
    model_hint: Option<&str>,
) -> Option<String> {
    label
        .map(|value| normalize_attribution_label(value, model_hint))
        .filter(|value| !value.is_empty())
}

/// Custom serialization: emits the flat display label string.
///
/// - `System` → `"system"`
/// - `Agent { model }` → `"model"`
/// - `Human { label }` → `"label"`
impl Serialize for ActorIdentity {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.label())
    }
}

#[derive(Serialize, Deserialize)]
struct AgentFields {
    #[serde(default)]
    name: String,
    #[serde(default)]
    model: String,
}

impl<'de> Deserialize<'de> for ActorIdentity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de;

        struct ActorVisitor;

        impl<'de> de::Visitor<'de> for ActorVisitor {
            type Value = ActorIdentity;

            fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
                formatter.write_str(
                    r#""system", a model string, {"agent": {"model": "..."}}, or {"human": "..."}"#,
                )
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<ActorIdentity, E> {
                match value {
                    "system" => Ok(ActorIdentity::System),
                    other => {
                        // Legacy: bare string like "claude / opus-4.6" or "human"
                        if other.contains(" / ") {
                            let parts: Vec<&str> = other.splitn(2, " / ").collect();
                            Ok(ActorIdentity::Agent {
                                model: parts[1].trim().to_string(),
                            })
                        } else if other == "human" {
                            Ok(ActorIdentity::Human {
                                label: "human".to_string(),
                            })
                        } else {
                            Ok(ActorIdentity::Agent {
                                model: other.to_string(),
                            })
                        }
                    }
                }
            }

            fn visit_map<M: de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<ActorIdentity, M::Error> {
                let key: String = map
                    .next_key()?
                    .ok_or_else(|| de::Error::custom("expected 'agent' or 'human' key"))?;
                match key.as_str() {
                    "agent" => {
                        let fields: AgentFields = map.next_value()?;
                        let model = if fields.model.trim().is_empty() {
                            fields.name
                        } else {
                            fields.model
                        };
                        Ok(ActorIdentity::Agent {
                            model: model.trim().to_string(),
                        })
                    }
                    "human" => {
                        let label: String = map.next_value()?;
                        Ok(ActorIdentity::Human { label })
                    }
                    other => Err(de::Error::unknown_field(other, &["agent", "human"])),
                }
            }
        }

        deserializer.deserialize_any(ActorVisitor)
    }
}
