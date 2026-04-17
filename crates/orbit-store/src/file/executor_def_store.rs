use std::fs;
use std::path::PathBuf;

use orbit_types::{
    ExecutorDef, ExecutorResource, ExecutorResourceSpec, OrbitError, RESOURCE_SCHEMA_VERSION,
    ResourceKind,
};

use crate::file::fs_utils::write_atomic;

pub struct ExecutorDefFileStore {
    root: PathBuf,
}

impl ExecutorDefFileStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn executors_dir(&self) -> PathBuf {
        self.root.clone()
    }

    pub fn list_executor_defs(&self) -> Result<Vec<ExecutorDef>, OrbitError> {
        let dir = self.executors_dir();
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut defs = Vec::new();
        let entries = fs::read_dir(&dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                let content =
                    fs::read_to_string(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
                let def = parse_executor_def(&content, path.display().to_string())?;
                defs.push(def);
            }
        }
        defs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(defs)
    }

    pub fn get_executor_def(&self, name: &str) -> Result<Option<ExecutorDef>, OrbitError> {
        let path = self.executors_dir().join(format!("{name}.yaml"));
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
        let def = parse_executor_def(&content, path.display().to_string())?;
        Ok(Some(def))
    }

    pub fn upsert_executor_def(&self, def: &ExecutorDef) -> Result<(), OrbitError> {
        let dir = self.executors_dir();
        fs::create_dir_all(&dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        let path = dir.join(format!("{}.yaml", def.name));
        let content = serde_yaml::to_string(&ExecutorResource::new(
            ResourceKind::Executor,
            def.name.clone(),
            ExecutorResourceSpec {
                executor_type: def.executor_type.clone(),
                command: def.command.clone(),
                args: def.args.clone(),
                stdout_format: def.stdout_format.clone(),
                models: def.models.clone(),
                timeout_seconds: def.timeout_seconds,
                env: def.env.clone(),
                created_at: def.created_at,
                updated_at: def.updated_at,
            },
        ))
        .map_err(|e| OrbitError::Execution(format!("failed to serialize executor def: {e}")))?;
        write_atomic(&path, &content)
    }
}

fn parse_executor_def(content: &str, label: String) -> Result<ExecutorDef, OrbitError> {
    let doc: ExecutorResource = serde_yaml::from_str(content)
        .map_err(|e| OrbitError::InvalidInput(format!("invalid executor def at {}: {e}", label)))?;
    if doc.kind != ResourceKind::Executor {
        return Err(OrbitError::InvalidInput(format!(
            "invalid executor def at {}: expected kind Executor, found {}",
            label, doc.kind
        )));
    }
    if doc.schema_version != RESOURCE_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "invalid executor def at {}: unsupported schemaVersion {}",
            label, doc.schema_version
        )));
    }
    Ok(ExecutorDef {
        name: doc.metadata.name,
        executor_type: doc.spec.executor_type,
        command: doc.spec.command,
        args: doc.spec.args,
        stdout_format: doc.spec.stdout_format,
        models: doc.spec.models,
        timeout_seconds: doc.spec.timeout_seconds,
        env: doc.spec.env,
        created_at: doc.spec.created_at,
        updated_at: doc.spec.updated_at,
    })
}
