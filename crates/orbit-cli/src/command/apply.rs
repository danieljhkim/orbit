use std::path::PathBuf;

use chrono::Utc;
use clap::Args;
use orbit_core::command::activity::{
    ActivityAddParams, ActivityUpdateParams as RuntimeActivityUpdateParams,
};
use orbit_core::command::job::JobAddParams;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_types::{
    ActivityResource, ExecutorDef, ExecutorResource, JobResource, PolicyDef, PolicyResource,
    RESOURCE_SCHEMA_VERSION, ResourceHeader, ResourceKind,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

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

fn resolve_paths(path: &PathBuf) -> Result<Vec<PathBuf>, OrbitError> {
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
        Ok(vec![path.clone()])
    } else {
        Err(OrbitError::InvalidInput(format!(
            "path not found: {}",
            path.display()
        )))
    }
}

fn apply_one(runtime: &OrbitRuntime, content: &str, path: &PathBuf) -> Result<(), OrbitError> {
    let header: ResourceHeader = parse_resource(content, path, "resource header")?;
    validate_header(&header, path)?;

    match header.kind {
        ResourceKind::Policy => apply_policy(runtime, content, path, &header.metadata.name),
        ResourceKind::Executor => apply_executor(runtime, content, path, &header.metadata.name),
        ResourceKind::Job => apply_job(runtime, content, path, &header.metadata.name),
        ResourceKind::Activity => apply_activity(runtime, content, path, &header.metadata.name),
    }
}

fn apply_policy(
    runtime: &OrbitRuntime,
    content: &str,
    path: &PathBuf,
    name: &str,
) -> Result<(), OrbitError> {
    let doc: PolicyResource = parse_resource(content, path, "Policy resource")?;
    let existing = runtime.get_policy_def(name)?;
    let mut def = PolicyDef {
        name: name.to_string(),
        description: doc.spec.description,
        filesystem: doc.spec.filesystem,
        network: doc.spec.network,
        process: doc.spec.process,
        approval: doc.spec.approval,
        tools: doc.spec.tools,
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
    path: &PathBuf,
    name: &str,
) -> Result<(), OrbitError> {
    let doc: ExecutorResource = parse_resource(content, path, "Executor resource")?;
    let existing = runtime.get_executor_def(name)?;
    let mut def = ExecutorDef {
        name: name.to_string(),
        executor_type: doc.spec.executor_type,
        command: doc.spec.command,
        args: doc.spec.args,
        stdout_format: doc.spec.stdout_format,
        models: doc.spec.models,
        timeout_seconds: doc.spec.timeout_seconds,
        env: doc.spec.env,
        created_at: existing
            .as_ref()
            .map(|executor| executor.created_at)
            .unwrap_or(doc.spec.created_at),
        updated_at: Utc::now(),
    };
    if existing.is_none() {
        def.created_at = doc.spec.created_at;
    }
    runtime.upsert_executor_def(&def)?;
    println!("executor/{name} applied");
    Ok(())
}

fn apply_job(
    runtime: &OrbitRuntime,
    content: &str,
    path: &PathBuf,
    name: &str,
) -> Result<(), OrbitError> {
    let doc: JobResource = parse_resource(content, path, "Job resource")?;
    let spec = doc.spec;

    if runtime.get_job(name)?.is_some() {
        runtime.update_job_definition(
            name,
            spec.default_input,
            spec.max_active_runs,
            spec.max_iterations,
            spec.steps,
            spec.policy,
            spec.state,
        )?;
    } else {
        runtime.add_job(JobAddParams {
            job_id: Some(name.to_string()),
            default_input: spec.default_input,
            max_active_runs: Some(spec.max_active_runs),
            max_iterations: Some(spec.max_iterations),
            steps: spec.steps,
            policy: spec.policy,
            initial_state_override: Some(spec.state),
        })?;
    }

    println!("job/{name} applied");
    Ok(())
}

fn apply_activity(
    runtime: &OrbitRuntime,
    content: &str,
    path: &PathBuf,
    name: &str,
) -> Result<(), OrbitError> {
    let doc: ActivityResource = parse_resource(content, path, "Activity resource")?;
    let spec_config = Value::Object(doc.spec.spec_config.clone());
    let update_params = RuntimeActivityUpdateParams {
        description: Some(doc.spec.description.clone()),
        input_schema_json: Some(doc.spec.input_schema_json.clone()),
        output_schema_json: Some(doc.spec.output_schema_json.clone()),
        spec_config: Some(spec_config.clone()),
        executor: Some(doc.spec.executor.clone()),
        workspace_path: Some(doc.spec.workspace_path.clone()),
        created_by: Some(doc.spec.created_by.clone()),
        is_active: Some(doc.spec.is_active),
    };

    match runtime.show_activity(name) {
        Ok(_) => {
            runtime.update_activity(name, update_params)?;
        }
        Err(OrbitError::ActivityNotFound(_)) => {
            runtime.add_activity(ActivityAddParams {
                id: name.to_string(),
                spec_type: doc.spec.spec_type,
                description: doc.spec.description,
                input_schema_json: doc.spec.input_schema_json,
                output_schema_json: doc.spec.output_schema_json,
                spec_config,
                executor: doc.spec.executor,
                workspace_path: doc.spec.workspace_path,
                created_by: doc.spec.created_by,
            })?;
            if !doc.spec.is_active {
                runtime.update_activity(
                    name,
                    RuntimeActivityUpdateParams {
                        is_active: Some(false),
                        ..Default::default()
                    },
                )?;
            }
        }
        Err(error) => return Err(error),
    }

    println!("activity/{name} applied");
    Ok(())
}

fn validate_header(header: &ResourceHeader, path: &PathBuf) -> Result<(), OrbitError> {
    if header.schema_version != RESOURCE_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "{}: unsupported schemaVersion '{}' (expected '{}')",
            path.display(),
            header.schema_version,
            RESOURCE_SCHEMA_VERSION,
        )));
    }
    Ok(())
}

fn parse_resource<T: DeserializeOwned>(
    content: &str,
    path: &PathBuf,
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
