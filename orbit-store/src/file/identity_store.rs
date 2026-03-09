use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use orbit_types::{IdentityRole, OrbitError, ResolvedIdentity};
use serde::Deserialize;
use serde_yaml::Value;

#[derive(Debug, Clone)]
pub struct IdentityCatalog {
    root: PathBuf,
    role_overrides: BTreeMap<String, IdentityRole>,
}

impl IdentityCatalog {
    pub fn new(root: PathBuf, role_overrides: BTreeMap<String, IdentityRole>) -> Self {
        Self {
            root,
            role_overrides,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ensure_layout(&self) -> Result<(), OrbitError> {
        fs::create_dir_all(&self.root).map_err(|e| OrbitError::Io(e.to_string()))
    }

    pub fn role_overrides(&self) -> &BTreeMap<String, IdentityRole> {
        &self.role_overrides
    }

    pub fn list(&self) -> Result<Vec<String>, OrbitError> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let entries = fs::read_dir(&self.root).map_err(|e| {
            OrbitError::Io(format!(
                "failed to read identity root '{}': {e}",
                self.root.display()
            ))
        })?;
        let mut ids: Vec<String> = entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension()?.to_str()? == "yaml" {
                    path.file_stem()?.to_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect();
        ids.sort();
        Ok(ids)
    }

    pub fn resolve(&self, identity_id: &str) -> Result<ResolvedIdentity, OrbitError> {
        let identity_id = identity_id.trim();
        if identity_id.is_empty() {
            return Err(OrbitError::IdentityValidation(
                "identity id must not be empty".to_string(),
            ));
        }

        let path = self.root.join(format!("{identity_id}.yaml"));
        if !path.exists() {
            return Err(OrbitError::IdentityNotFound(identity_id.to_string()));
        }

        let raw = fs::read_to_string(&path).map_err(|e| {
            OrbitError::Io(format!("failed to read identity '{}': {e}", path.display()))
        })?;
        let parsed: IdentityYaml = serde_yaml::from_str(&raw).map_err(|e| {
            OrbitError::IdentityValidation(format!(
                "invalid identity file '{}': {e}",
                path.display()
            ))
        })?;

        let file_name = parsed.identity.name.trim().to_string();
        if file_name.is_empty() {
            return Err(OrbitError::IdentityValidation(format!(
                "identity file '{}' is missing identity.name",
                path.display()
            )));
        }

        let yaml_role = parsed
            .identity
            .role
            .as_deref()
            .map(parse_role)
            .transpose()?;
        let role = self
            .role_overrides
            .get(identity_id)
            .copied()
            .or(yaml_role)
            .unwrap_or(IdentityRole::Member);

        Ok(ResolvedIdentity {
            id: identity_id.to_string(),
            name: parsed
                .identity
                .display_name
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or(file_name),
            role,
            personality: normalize_map(parsed.personality.unwrap_or_default()),
            behavior: normalize_map(parsed.behavior.unwrap_or_default()),
        })
    }
}

pub fn compile_identity_block(identity: &ResolvedIdentity) -> String {
    let mut lines = vec![
        "<agent_identity>".to_string(),
        format!("Name: {}", identity.name),
        format!("Role: {}", identity.role),
        String::new(),
        "personality:".to_string(),
    ];

    if identity.personality.is_empty() {
        lines.push("- none".to_string());
    } else {
        for (key, value) in &identity.personality {
            lines.push(format!("- {key}: {value}"));
        }
    }

    lines.push(String::new());
    lines.push("behavior:".to_string());
    if identity.behavior.is_empty() {
        lines.push("- none".to_string());
    } else {
        for (key, value) in &identity.behavior {
            lines.push(format!("- {key}: {value}"));
        }
    }
    lines.push("</agent_identity>".to_string());
    lines.join("\n")
}

fn parse_role(raw: &str) -> Result<IdentityRole, OrbitError> {
    raw.parse::<IdentityRole>()
        .map_err(|e| OrbitError::IdentityValidation(e.to_string()))
}

fn normalize_map(values: BTreeMap<String, Value>) -> Vec<(String, String)> {
    values
        .into_iter()
        .map(|(k, v)| (k, value_to_string(v)))
        .collect()
}

fn value_to_string(value: Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v,
        Value::Sequence(v) => {
            let values = v.into_iter().map(value_to_string).collect::<Vec<_>>();
            format!("[{}]", values.join(", "))
        }
        Value::Mapping(v) => {
            let mut pairs = Vec::new();
            for (k, val) in v {
                let key = match k {
                    Value::String(s) => s,
                    other => value_to_string(other),
                };
                pairs.push(format!("{key}: {}", value_to_string(val)));
            }
            format!("{{{}}}", pairs.join(", "))
        }
        Value::Tagged(v) => value_to_string(v.value),
    }
}

#[derive(Debug, Deserialize)]
struct IdentityYaml {
    identity: IdentityHeader,
    #[serde(default)]
    personality: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    behavior: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Deserialize)]
struct IdentityHeader {
    name: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    role: Option<String>,
}
