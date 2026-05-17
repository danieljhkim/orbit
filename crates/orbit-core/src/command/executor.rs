use orbit_common::types::{
    EXECUTOR_RESOURCE_SCHEMA_VERSION, ExecutorDef, ExecutorResource, ExecutorType, OrbitError,
    ResourceKind,
};
use orbit_store::ExecutorDefStoreBackend;

pub(crate) const DEFAULT_EXECUTOR_FILES: &[(&str, &str)] = &[
    ("claude", include_str!("../../assets/executors/claude.yaml")),
    ("codex", include_str!("../../assets/executors/codex.yaml")),
    ("gemini", include_str!("../../assets/executors/gemini.yaml")),
    ("grok", include_str!("../../assets/executors/grok.yaml")),
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
        match existing {
            None => {
                store.upsert_executor_def(&def)?;
                created += 1;
            }
            Some(_) if overwrite => {
                store.upsert_executor_def(&def)?;
                created += 1;
            }
            Some(existing) => {
                if let Some(migrated) = migrated_default_executor(&existing, &def) {
                    store.upsert_executor_def(&migrated)?;
                    created += 1;
                }
            }
        }
    }
    Ok(created)
}

fn migrated_default_executor(existing: &ExecutorDef, seeded: &ExecutorDef) -> Option<ExecutorDef> {
    if existing.name != seeded.name {
        return None;
    }

    if existing.executor_type != ExecutorType::AgentCli
        || seeded.executor_type != ExecutorType::DirectAgent
    {
        return None;
    }

    let mut migrated = existing.clone();
    migrated.executor_type = ExecutorType::DirectAgent;
    Some(migrated)
}

fn parse_default_executor(name: &str, yaml: &str) -> Result<ExecutorDef, OrbitError> {
    let resource: ExecutorResource = serde_yaml::from_str(yaml).map_err(|e| {
        OrbitError::InvalidInput(format!("invalid embedded executor def '{name}': {e}"))
    })?;
    if resource.schema_version != EXECUTOR_RESOURCE_SCHEMA_VERSION {
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

    Ok(ExecutorDef::from_resource_spec(
        resource.metadata.name,
        resource.spec.clone(),
        &format!("embedded:{name}"),
        resource.spec.created_at,
        resource.spec.updated_at,
    ))
}
