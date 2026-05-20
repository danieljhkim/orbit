use std::path::{Path, PathBuf};

use chrono::Utc;
use clap::Args;
use orbit_common::types::{
    EXECUTOR_RESOURCE_SCHEMA_VERSION, ExecutorDef, ExecutorResource,
    POLICY_RESOURCE_SCHEMA_VERSION, PolicyDef, ResourceHeader, ResourceKind, parse_policy_resource,
};
use orbit_core::{OrbitError, OrbitRuntime};
use serde::de::DeserializeOwned;

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Apply a resource definition from YAML")]
pub struct ApplyCommand {
    /// Path to a YAML file or directory of YAML files
    #[arg(short = 'f', long = "file")]
    pub file: PathBuf,
}

impl Execute for ApplyCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let paths = resolve_paths(&self.file)?;
        if paths.is_empty() {
            return Err(OrbitError::InvalidInput("no YAML files found".to_string()));
        }

        let mut applied = 0u32;
        for path in &paths {
            let content = std::fs::read_to_string(path)
                .map_err(|e| OrbitError::Io(format!("{}: {e}", path.display())))?;
            apply_one(runtime, &content, path)?;
            applied += 1;
        }

        println!("{applied} resource(s) applied.");
        Ok(())
    }
}

fn resolve_paths(path: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    if path.is_dir() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(path)
            .map_err(|e| OrbitError::Io(format!("{}: {e}", path.display())))?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let p = entry.path();
                if p.extension()
                    .is_some_and(|ext| ext == "yaml" || ext == "yml")
                {
                    Some(p)
                } else {
                    None
                }
            })
            .collect();
        files.sort();
        Ok(files)
    } else if path.is_file() {
        Ok(vec![path.to_path_buf()])
    } else {
        Err(OrbitError::InvalidInput(format!(
            "path not found: {}",
            path.display()
        )))
    }
}

fn apply_one(runtime: &OrbitRuntime, content: &str, path: &Path) -> Result<(), OrbitError> {
    let header: ResourceHeader = parse_resource(content, path, "resource header")?;
    validate_header(&header, path)?;

    match header.kind {
        ResourceKind::Policy => apply_policy(runtime, content, path, &header.metadata.name),
        ResourceKind::Executor => apply_executor(runtime, content, path, &header.metadata.name),
        ResourceKind::Job | ResourceKind::Activity => Err(OrbitError::InvalidInput(format!(
            "{}: `orbit apply` no longer supports kind `{}`. Activities and workflows ship \
             as default YAMLs on `orbit init` and resolve via the v2 catalog.",
            path.display(),
            header.kind,
        ))),
    }
}

fn apply_policy(
    runtime: &OrbitRuntime,
    content: &str,
    path: &Path,
    name: &str,
) -> Result<(), OrbitError> {
    let doc = parse_policy_resource(content, &format!("{}: Policy resource", path.display()))?;
    let existing = runtime.get_policy_def(name)?;
    let mut def = PolicyDef {
        name: name.to_string(),
        description: doc.spec.description,
        deny_read: doc.spec.deny_read,
        deny_modify: doc.spec.deny_modify,
        fs_profiles: doc.spec.fs_profiles,
        created_at: existing
            .as_ref()
            .map(|policy| policy.created_at)
            .unwrap_or(doc.spec.created_at),
        updated_at: Utc::now(),
    };
    if existing.is_none() {
        def.created_at = doc.spec.created_at;
    }
    runtime.upsert_policy_def(&def)?;
    println!("policy/{name} applied");
    Ok(())
}

fn apply_executor(
    runtime: &OrbitRuntime,
    content: &str,
    path: &Path,
    name: &str,
) -> Result<(), OrbitError> {
    let doc: ExecutorResource = parse_resource(content, path, "Executor resource")?;
    let existing = runtime.get_executor_def(name)?;
    let created_at = existing
        .as_ref()
        .map(|executor| executor.created_at)
        .unwrap_or(doc.spec.created_at);
    let def = ExecutorDef::from_resource_spec(name.to_string(), doc.spec, created_at, Utc::now());
    runtime.upsert_executor_def(&def)?;
    println!("executor/{name} applied");
    Ok(())
}

fn validate_header(header: &ResourceHeader, path: &Path) -> Result<(), OrbitError> {
    let supported: Vec<u32> = match header.kind {
        ResourceKind::Policy => vec![POLICY_RESOURCE_SCHEMA_VERSION],
        ResourceKind::Executor => vec![EXECUTOR_RESOURCE_SCHEMA_VERSION],
        ResourceKind::Job | ResourceKind::Activity => Vec::new(),
    };
    if !supported.contains(&header.schema_version) {
        let expected = supported
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" or ");
        return Err(OrbitError::InvalidInput(format!(
            "{}: unsupported schemaVersion '{}' (expected {})",
            path.display(),
            header.schema_version,
            expected,
        )));
    }
    Ok(())
}

fn parse_resource<T: DeserializeOwned>(
    content: &str,
    path: &Path,
    label: &str,
) -> Result<T, OrbitError> {
    serde_yaml::from_str(content).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "{}: failed to parse {}: {}",
            path.display(),
            label,
            error
        ))
    })
}
