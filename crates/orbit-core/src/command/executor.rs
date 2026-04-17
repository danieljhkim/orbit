use orbit_store::ExecutorDefStoreBackend;
use orbit_types::{
    ExecutorDef, ExecutorResource, OrbitError, RESOURCE_SCHEMA_VERSION, ResourceKind,
};

pub(crate) const DEFAULT_EXECUTOR_FILES: &[(&str, &str)] = &[
    ("claude", include_str!("../../assets/executors/claude.yaml")),
    ("codex", include_str!("../../assets/executors/codex.yaml")),
    ("gemini", include_str!("../../assets/executors/gemini.yaml")),
    (
        "local-shell",
        include_str!("../../assets/executors/local-shell.yaml"),
    ),
];

pub(crate) fn seed_default_executors(
    store: &dyn ExecutorDefStoreBackend,
    overwrite: bool,
) -> Result<usize, OrbitError> {
    let mut created = 0usize;
    for (name, yaml) in DEFAULT_EXECUTOR_FILES {
        let def = parse_default_executor(name, yaml)?;
        let existing = store.get_executor_def(&def.name)?;
        if existing.is_none() || overwrite {
            store.upsert_executor_def(&def)?;
            created += 1;
        }
    }
    Ok(created)
}

fn parse_default_executor(name: &str, yaml: &str) -> Result<ExecutorDef, OrbitError> {
    let resource: ExecutorResource = serde_yaml::from_str(yaml).map_err(|e| {
        OrbitError::InvalidInput(format!("invalid embedded executor def '{name}': {e}"))
    })?;
    if resource.schema_version != RESOURCE_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "invalid embedded executor def '{name}': unsupported schemaVersion {}",
            resource.schema_version
        )));
    }
    if resource.kind != ResourceKind::Executor {
        return Err(OrbitError::InvalidInput(format!(
            "invalid embedded executor def '{name}': expected kind Executor, found {}",
            resource.kind
        )));
    }
    if resource.metadata.name != name {
        return Err(OrbitError::InvalidInput(format!(
            "default executor file key '{}' does not match metadata.name '{}'",
            name, resource.metadata.name
        )));
    }

    Ok(ExecutorDef {
        name: resource.metadata.name,
        executor_type: resource.spec.executor_type,
        command: resource.spec.command,
        args: resource.spec.args,
        stdout_format: resource.spec.stdout_format,
        models: resource.spec.models,
        timeout_seconds: resource.spec.timeout_seconds,
        env: resource.spec.env,
        created_at: resource.spec.created_at,
        updated_at: resource.spec.updated_at,
    })
}
