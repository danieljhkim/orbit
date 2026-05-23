use std::fs;
use std::path::PathBuf;

use orbit_common::types::{
    OrbitError, POLICY_RESOURCE_SCHEMA_VERSION, PolicyDef, PolicyResource, PolicyResourceSpec,
    ResourceKind, ResourceMetadata, parse_policy_resource, validate_resource_name,
};

use orbit_common::utility::fs::atomic_write_text_volatile as write_atomic;

pub(crate) struct PolicyDefFileStore {
    root: PathBuf,
}

impl PolicyDefFileStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn policies_dir(&self) -> PathBuf {
        self.root.clone()
    }

    fn policy_path(&self, name: &str) -> Result<PathBuf, OrbitError> {
        validate_resource_name(name)?;
        Ok(self.policies_dir().join(format!("{name}.yaml")))
    }

    pub(crate) fn ensure_layout(&self) -> Result<(), OrbitError> {
        fs::create_dir_all(self.policies_dir()).map_err(|e| OrbitError::Io(e.to_string()))?;
        Ok(())
    }

    pub(crate) fn list_policy_defs(&self) -> Result<Vec<PolicyDef>, OrbitError> {
        let dir = self.policies_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut defs = Vec::new();
        let entries = fs::read_dir(&dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
            {
                let content =
                    fs::read_to_string(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
                let def = parse_policy_def(&content, path.display().to_string())?;
                defs.push(def);
            }
        }
        defs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(defs)
    }

    pub(crate) fn get_policy_def(&self, name: &str) -> Result<Option<PolicyDef>, OrbitError> {
        let path = self.policy_path(name)?;
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
        let def = parse_policy_def(&content, format!("policy {name}"))?;
        Ok(Some(def))
    }

    pub(crate) fn upsert_policy_def(&self, def: &PolicyDef) -> Result<(), OrbitError> {
        def.validate()?;
        let path = self.policy_path(&def.name)?;
        self.ensure_layout()?;
        let content = serde_yaml::to_string(&PolicyResource {
            schema_version: POLICY_RESOURCE_SCHEMA_VERSION,
            kind: ResourceKind::Policy,
            metadata: ResourceMetadata::named(def.name.clone()),
            spec: PolicyResourceSpec {
                description: def.description.clone(),
                deny_read: def.deny_read.clone(),
                deny_modify: def.deny_modify.clone(),
                fs_profiles: def.fs_profiles.clone(),
                created_at: def.created_at,
                updated_at: def.updated_at,
            },
        })
        .map_err(|e| {
            OrbitError::InvalidInput(format!("failed to serialize policy {}: {e}", def.name))
        })?;
        write_atomic(&path, &content).map_err(Into::into)
    }
}

fn parse_policy_def(content: &str, label: String) -> Result<PolicyDef, OrbitError> {
    let doc = parse_policy_resource(content, &label)?;
    let def = PolicyDef {
        name: doc.metadata.name,
        description: doc.spec.description,
        deny_read: doc.spec.deny_read,
        deny_modify: doc.spec.deny_modify,
        fs_profiles: doc.spec.fs_profiles,
        created_at: doc.spec.created_at,
        updated_at: doc.spec.updated_at,
    };
    def.validate()?;
    Ok(def)
}

#[cfg(test)]
#[cfg(test)]
mod tests;
