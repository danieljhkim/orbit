use std::fs;
use std::path::PathBuf;

use orbit_common::types::{
    EXECUTOR_RESOURCE_SCHEMA_VERSION, ExecutorDef, ExecutorResource, ExecutorResourceSpec,
    OrbitError, ResourceKind, ResourceMetadata, validate_resource_name,
};

use orbit_common::utility::fs::atomic_write_text_volatile as write_atomic;

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

    fn executor_path(&self, name: &str) -> Result<PathBuf, OrbitError> {
        validate_resource_name(name)?;
        Ok(self.executors_dir().join(format!("{name}.yaml")))
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
        let path = self.executor_path(name)?;
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
        let def = parse_executor_def(&content, path.display().to_string())?;
        Ok(Some(def))
    }

    pub fn upsert_executor_def(&self, def: &ExecutorDef) -> Result<(), OrbitError> {
        let path = self.executor_path(&def.name)?;
        let dir = self.executors_dir();
        fs::create_dir_all(&dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        let content = serde_yaml::to_string(&ExecutorResource {
            schema_version: EXECUTOR_RESOURCE_SCHEMA_VERSION,
            kind: ResourceKind::Executor,
            metadata: ResourceMetadata::named(def.name.clone()),
            spec: ExecutorResourceSpec {
                executor_type: def.executor_type,
                command: def.command.clone(),
                args: def.args.clone(),
                stdout_format: def.stdout_format,
                model_pair_override: def.model_pair_override.clone(),
                model_flag: def.model_flag.clone(),
                timeout_seconds: def.timeout_seconds,
                env: def.env.clone(),
                sandbox: def.sandbox,
                allow_fallback: def.allow_fallback,
                created_at: def.created_at,
                updated_at: def.updated_at,
            },
        })
        .map_err(|e| OrbitError::Execution(format!("failed to serialize executor def: {e}")))?;
        write_atomic(&path, &content).map_err(Into::into)
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
    if doc.schema_version != EXECUTOR_RESOURCE_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "invalid executor def at {}: unsupported schemaVersion {}",
            label, doc.schema_version
        )));
    }
    doc.metadata.validate_name()?;
    Ok(ExecutorDef::from_resource_spec(
        doc.metadata.name,
        doc.spec.clone(),
        doc.spec.created_at,
        doc.spec.updated_at,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use orbit_common::types::ExecutorSandboxKind;
    use orbit_common::types::ExecutorType;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn baseline_def(name: &str) -> ExecutorDef {
        let now = Utc::now();
        ExecutorDef {
            name: name.to_string(),
            executor_type: ExecutorType::DirectAgent,
            command: Some(name.to_string()),
            args: vec!["--flag".to_string()],
            stdout_format: None,
            model_pair_override: None,
            model_flag: None,
            timeout_seconds: None,
            env: HashMap::new(),
            sandbox: None,
            allow_fallback: false,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn roundtrips_sandbox_and_allow_fallback_fields() {
        let dir = tempdir().expect("tempdir");
        let store = ExecutorDefFileStore::new(dir.path().to_path_buf());

        let mut def = baseline_def("claude");
        def.sandbox = Some(ExecutorSandboxKind::MacosSandboxExec);
        def.allow_fallback = true;
        store.upsert_executor_def(&def).expect("upsert");

        let loaded = store
            .get_executor_def("claude")
            .expect("get")
            .expect("present");
        assert_eq!(loaded.name, "claude");
        assert_eq!(loaded.sandbox, Some(ExecutorSandboxKind::MacosSandboxExec));
        assert!(loaded.allow_fallback);
    }

    #[test]
    fn roundtrips_model_flag_field() {
        let dir = tempdir().expect("tempdir");
        let store = ExecutorDefFileStore::new(dir.path().to_path_buf());

        let mut def = baseline_def("gemini");
        def.model_flag = Some("-m".to_string());
        store.upsert_executor_def(&def).expect("upsert");

        let loaded = store
            .get_executor_def("gemini")
            .expect("get")
            .expect("present");
        assert_eq!(loaded.model_flag.as_deref(), Some("-m"));

        let on_disk = std::fs::read_to_string(dir.path().join("gemini.yaml")).expect("read");
        assert!(
            on_disk.contains("model_flag: -m"),
            "model_flag should be persisted: {on_disk}"
        );
    }

    #[test]
    fn omits_sandbox_fields_when_default() {
        let dir = tempdir().expect("tempdir");
        let store = ExecutorDefFileStore::new(dir.path().to_path_buf());

        let def = baseline_def("codex");
        store.upsert_executor_def(&def).expect("upsert");

        let on_disk = std::fs::read_to_string(dir.path().join("codex.yaml")).expect("read");
        assert!(
            !on_disk.contains("sandbox"),
            "sandbox should be omitted when None: {on_disk}"
        );
        assert!(
            !on_disk.contains("allow_fallback"),
            "allow_fallback should be omitted when false: {on_disk}"
        );
    }

    #[test]
    fn loads_executor_yaml_with_explicit_sandbox_kind() {
        let dir = tempdir().expect("tempdir");
        let yaml = "schemaVersion: 2\nkind: Executor\nmetadata:\n  name: gemini\nspec:\n  executor_type: direct_agent\n  command: gemini\n  args: []\n  sandbox: macos-sandbox-exec\n  allow_fallback: true\n";
        std::fs::write(dir.path().join("gemini.yaml"), yaml).expect("seed");

        let store = ExecutorDefFileStore::new(dir.path().to_path_buf());
        let loaded = store
            .get_executor_def("gemini")
            .expect("get")
            .expect("present");
        assert_eq!(loaded.sandbox, Some(ExecutorSandboxKind::MacosSandboxExec));
        assert!(loaded.allow_fallback);
    }

    #[test]
    fn rejects_traversal_executor_name_without_external_write() {
        let dir = tempdir().expect("tempdir");
        let store = ExecutorDefFileStore::new(dir.path().join("executors"));

        let err = store
            .upsert_executor_def(&baseline_def("../x"))
            .expect_err("traversal name must fail");
        assert!(matches!(err, OrbitError::InvalidInput(_)));
        assert!(!dir.path().join("x.yaml").exists());

        let err = store
            .get_executor_def("../x")
            .expect_err("traversal lookup must fail");
        assert!(matches!(err, OrbitError::InvalidInput(_)));
    }

    #[test]
    fn rejects_traversal_executor_metadata_name_when_loading() {
        let dir = tempdir().expect("tempdir");
        let executors_dir = dir.path().join("executors");
        std::fs::create_dir_all(&executors_dir).expect("mkdir");
        std::fs::write(
            executors_dir.join("bad.yaml"),
            "schemaVersion: 2\nkind: Executor\nmetadata:\n  name: ../x\nspec:\n  executor_type: direct_agent\n",
        )
        .expect("seed");

        let store = ExecutorDefFileStore::new(executors_dir);
        let err = store
            .list_executor_defs()
            .expect_err("traversal metadata name must fail");
        assert!(matches!(err, OrbitError::InvalidInput(_)));
        assert!(!dir.path().join("x.yaml").exists());
    }
}
