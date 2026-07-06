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

pub(super) fn migrated_default_executor(
    existing: &ExecutorDef,
    seeded: &ExecutorDef,
) -> Option<ExecutorDef> {
    if existing.name != seeded.name {
        return None;
    }

    let mut migrated = existing.clone();
    let mut changed = false;

    if existing.executor_type == ExecutorType::AgentCli
        && seeded.executor_type == ExecutorType::DirectAgent
    {
        migrated.executor_type = ExecutorType::DirectAgent;
        changed = true;
    }

    // Scrub a platform-mismatched sandbox left over from a prior install so
    // re-seeding on a host that can't apply it (e.g. `macos-sandbox-exec` on
    // Linux) drops the declaration instead of leaving dispatch fail-closed.
    // See [ORB-10047].
    if let Some(kind) = existing.sandbox {
        if !kind.is_available_on_current_platform() {
            tracing::warn!(
                executor = %existing.name,
                sandbox = %kind,
                current_platform = std::env::consts::OS,
                target_platform = kind.target_os(),
                "scrubbing platform-mismatched sandbox from installed executor def on re-seed",
            );
            migrated.sandbox = None;
            changed = true;
        }
    }

    if changed { Some(migrated) } else { None }
}

pub(super) fn parse_default_executor(name: &str, yaml: &str) -> Result<ExecutorDef, OrbitError> {
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

    let mut def = ExecutorDef::from_resource_spec(
        resource.metadata.name,
        resource.spec.clone(),
        resource.spec.created_at,
        resource.spec.updated_at,
    );

    // Shipped executor assets today declare a single-OS sandbox primitive
    // (`macos-sandbox-exec`); dispatch fails closed if the runner platform
    // can't apply it. Drop the declaration at parse time on hosts where it
    // doesn't apply so first-install and re-install (overwrite mode) don't
    // persist a platform-mismatched sandbox. See [ORB-10047].
    if let Some(kind) = def.sandbox {
        if !kind.is_available_on_current_platform() {
            tracing::warn!(
                executor = %def.name,
                sandbox = %kind,
                current_platform = std::env::consts::OS,
                target_platform = kind.target_os(),
                "shipped executor asset declares sandbox for another platform; installing without sandbox on this host",
            );
            def.sandbox = None;
        }
    }

    Ok(def)
}
