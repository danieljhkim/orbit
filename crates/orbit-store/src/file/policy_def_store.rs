use std::fs;
use std::path::PathBuf;

use orbit_types::{
    OrbitError, PolicyDef, PolicyResource, PolicyResourceSpec, RESOURCE_SCHEMA_VERSION,
    ResourceKind,
};

use crate::file::fs_utils::write_atomic;

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
        let path = self.policies_dir().join(format!("{name}.yaml"));
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
        let def = parse_policy_def(&content, format!("policy {name}"))?;
        Ok(Some(def))
    }

    pub(crate) fn upsert_policy_def(&self, def: &PolicyDef) -> Result<(), OrbitError> {
        self.ensure_layout()?;
        let path = self.policies_dir().join(format!("{}.yaml", def.name));
        let content = serde_yaml::to_string(&PolicyResource::new(
            ResourceKind::Policy,
            def.name.clone(),
            PolicyResourceSpec {
                description: def.description.clone(),
                filesystem: def.filesystem.clone(),
                network: def.network.clone(),
                process: def.process.clone(),
                approval: def.approval.clone(),
                tools: def.tools.clone(),
                created_at: def.created_at,
                updated_at: def.updated_at,
            },
        ))
        .map_err(|e| {
            OrbitError::InvalidInput(format!("failed to serialize policy {}: {e}", def.name))
        })?;
        write_atomic(&path, &content)
    }
}

fn parse_policy_def(content: &str, label: String) -> Result<PolicyDef, OrbitError> {
    let doc: PolicyResource = serde_yaml::from_str(content)
        .map_err(|e| OrbitError::InvalidInput(format!("failed to parse {}: {e}", label)))?;
    if doc.kind != ResourceKind::Policy {
        return Err(OrbitError::InvalidInput(format!(
            "failed to parse {}: expected kind Policy, found {}",
            label, doc.kind
        )));
    }
    if doc.schema_version != RESOURCE_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "failed to parse {}: unsupported schemaVersion {}",
            label, doc.schema_version
        )));
    }
    Ok(PolicyDef {
        name: doc.metadata.name,
        description: doc.spec.description,
        filesystem: doc.spec.filesystem,
        network: doc.spec.network,
        process: doc.spec.process,
        approval: doc.spec.approval,
        tools: doc.spec.tools,
        created_at: doc.spec.created_at,
        updated_at: doc.spec.updated_at,
    })
}
