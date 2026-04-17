use chrono::Utc;
use orbit_store::PolicyDefStoreBackend;
use orbit_types::{OrbitError, PolicyDef, PolicyResource, RESOURCE_SCHEMA_VERSION, ResourceKind};

const DEFAULT_POLICY_FILES: &[(&str, &str)] = &[(
    "safe-local-dev",
    include_str!("../../assets/policies/safe-local-dev.yaml"),
)];

pub(crate) fn seed_default_policies(
    store: &dyn PolicyDefStoreBackend,
    overwrite: bool,
) -> Result<usize, OrbitError> {
    let now = Utc::now();
    let mut count = 0;
    for (name, raw) in DEFAULT_POLICY_FILES {
        let existing = store.get_policy_def(name)?;
        if existing.is_some() && !overwrite {
            continue;
        }
        let def = parse_default_policy(name, raw, now)?;
        store.upsert_policy_def(&def)?;
        count += 1;
    }
    Ok(count)
}

fn parse_default_policy(
    name: &str,
    raw: &str,
    now: chrono::DateTime<Utc>,
) -> Result<PolicyDef, OrbitError> {
    let resource: PolicyResource = serde_yaml::from_str(raw)
        .map_err(|e| OrbitError::InvalidInput(format!("invalid default policy '{name}': {e}")))?;
    if resource.schema_version != RESOURCE_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "invalid default policy '{name}': unsupported schemaVersion {}",
            resource.schema_version
        )));
    }
    if resource.kind != ResourceKind::Policy {
        return Err(OrbitError::InvalidInput(format!(
            "invalid default policy '{name}': expected kind Policy, found {}",
            resource.kind
        )));
    }
    if resource.metadata.name != name {
        return Err(OrbitError::InvalidInput(format!(
            "default policy file key '{}' does not match metadata.name '{}'",
            name, resource.metadata.name
        )));
    }

    Ok(PolicyDef {
        name: resource.metadata.name,
        description: resource.spec.description,
        filesystem: resource.spec.filesystem,
        network: resource.spec.network,
        process: resource.spec.process,
        approval: resource.spec.approval,
        tools: resource.spec.tools,
        created_at: now,
        updated_at: now,
    })
}
