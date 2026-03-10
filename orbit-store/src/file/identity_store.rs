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

    pub fn list(&self) -> Result<Vec<ResolvedIdentity>, OrbitError> {
        self.list_filtered(None)
    }

    /// List identities, optionally filtering by role.
    /// Ordering is deterministic (sorted by ID) both before and after filtering.
    pub fn list_filtered(
        &self,
        role: Option<IdentityRole>,
    ) -> Result<Vec<ResolvedIdentity>, OrbitError> {
        let all: Vec<ResolvedIdentity> = self
            .list_ids()?
            .into_iter()
            .map(|identity_id| self.resolve(&identity_id))
            .collect::<Result<_, _>>()?;
        match role {
            None => Ok(all),
            Some(r) => Ok(all.into_iter().filter(|i| i.role == r).collect()),
        }
    }

    fn list_ids(&self) -> Result<Vec<String>, OrbitError> {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use orbit_types::{IdentityRole, OrbitError};

    use super::IdentityCatalog;

    #[test]
    fn list_filtered_by_role_returns_only_matching_identities() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("alice.yaml"),
            "identity:\n  name: Alice\n  role: engineer\n",
        )
        .expect("write alice");
        std::fs::write(
            dir.path().join("bob.yaml"),
            "identity:\n  name: Bob\n  role: member\n",
        )
        .expect("write bob");
        std::fs::write(
            dir.path().join("carol.yaml"),
            "identity:\n  name: Carol\n  role: engineer\n",
        )
        .expect("write carol");

        let catalog = IdentityCatalog::new(dir.path().to_path_buf(), BTreeMap::new());

        let engineers = catalog
            .list_filtered(Some(IdentityRole::Engineer))
            .expect("list engineers");
        assert_eq!(engineers.len(), 2);
        assert!(engineers.iter().all(|i| i.role == IdentityRole::Engineer));
        // deterministic ordering preserved
        assert_eq!(engineers[0].id, "alice");
        assert_eq!(engineers[1].id, "carol");

        let members = catalog
            .list_filtered(Some(IdentityRole::Member))
            .expect("list members");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].id, "bob");
    }

    #[test]
    fn list_filtered_with_none_returns_all_identities() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("alice.yaml"),
            "identity:\n  name: Alice\n  role: engineer\n",
        )
        .expect("write alice");
        std::fs::write(
            dir.path().join("bob.yaml"),
            "identity:\n  name: Bob\n  role: member\n",
        )
        .expect("write bob");

        let catalog = IdentityCatalog::new(dir.path().to_path_buf(), BTreeMap::new());

        let all = catalog.list_filtered(None).expect("list all");
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn list_filtered_empty_match_returns_empty_vec() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("alice.yaml"),
            "identity:\n  name: Alice\n  role: engineer\n",
        )
        .expect("write alice");

        let catalog = IdentityCatalog::new(dir.path().to_path_buf(), BTreeMap::new());

        let leaders = catalog
            .list_filtered(Some(IdentityRole::Leader))
            .expect("list leaders");
        assert!(leaders.is_empty());
    }

    #[test]
    fn list_returns_sorted_resolved_identities() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("zara.yaml"),
            "identity:\n  name: Zara\n  role: member\n",
        )
        .expect("write zara");
        std::fs::write(
            dir.path().join("alice.yaml"),
            "identity:\n  name: Alice\n  role: engineer\n",
        )
        .expect("write alice");

        let catalog = IdentityCatalog::new(dir.path().to_path_buf(), BTreeMap::new());

        let identities = catalog.list().expect("list identities");

        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].id, "alice");
        assert_eq!(identities[0].name, "Alice");
        assert_eq!(identities[0].role, IdentityRole::Engineer);
        assert_eq!(identities[1].id, "zara");
    }

    #[test]
    fn list_surfaces_malformed_identity_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("broken.yaml"),
            "identity:\n  name: Broken\n  role: [not-valid\n",
        )
        .expect("write broken");

        let catalog = IdentityCatalog::new(dir.path().to_path_buf(), BTreeMap::new());

        let error = catalog.list().expect_err("malformed identity should fail");

        match error {
            OrbitError::IdentityValidation(message) => {
                assert!(message.contains("broken.yaml"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
