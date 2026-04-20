use clap::Args;
use orbit_common::types::ResourceKind;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "List or show resources by kind")]
pub struct GetCommand {
    /// Resource reference: kind (e.g. "jobs") or kind/name (e.g. "policy/safe-local-dev")
    pub resource: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for GetCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let (kind, name) = parse_resource_ref(&self.resource)?;

        match (kind, name) {
            (ResourceKind::Job, None) => list_jobs(runtime, self.json),
            (ResourceKind::Job, Some(id)) => show_job(runtime, &id, self.json),
            (ResourceKind::Activity, None) => list_activities(runtime, self.json),
            (ResourceKind::Activity, Some(id)) => show_activity(runtime, &id, self.json),
            (ResourceKind::Policy, None) => list_policies(runtime, self.json),
            (ResourceKind::Policy, Some(name)) => show_policy(runtime, &name, self.json),
            (ResourceKind::Executor, None) => list_executors(runtime, self.json),
            (ResourceKind::Executor, Some(name)) => show_executor(runtime, &name, self.json),
        }
    }
}

/// Parse "jobs", "policy/foo", "executor/bar", etc.
fn parse_resource_ref(s: &str) -> Result<(ResourceKind, Option<String>), OrbitError> {
    if let Some((kind_str, name)) = s.split_once('/') {
        let kind: ResourceKind = kind_str
            .parse()
            .map_err(|e: String| OrbitError::InvalidInput(e))?;
        Ok((kind, Some(name.to_string())))
    } else {
        let kind: ResourceKind = s.parse().map_err(|e: String| OrbitError::InvalidInput(e))?;
        Ok((kind, None))
    }
}

// ── Jobs ──

fn list_jobs(runtime: &OrbitRuntime, as_json: bool) -> Result<(), OrbitError> {
    use orbit_core::command::job::JobCatalogFilter;
    let entries = runtime.list_job_catalog_with_last_run(false, JobCatalogFilter::All)?;
    if as_json {
        let values: Vec<Value> = entries
            .iter()
            .map(|(entry, _)| {
                json!({
                    "job_id": entry.job_id,
                    "kind": entry.kind().to_string(),
                    "state": entry.state().to_string(),
                    "steps": entry.spec.steps.len(),
                })
            })
            .collect();
        crate::output::json::print_pretty(&Value::Array(values))
    } else {
        if entries.is_empty() {
            println!("No jobs found.");
            return Ok(());
        }
        let mut table = crate::output::table::build_table(&["JOB ID", "KIND", "STATE", "STEPS"]);
        for (entry, _) in &entries {
            table.add_row(vec![
                entry.job_id.clone(),
                entry.kind().to_string(),
                entry.state().to_string(),
                entry.spec.steps.len().to_string(),
            ]);
        }
        println!("{table}");
        Ok(())
    }
}

fn show_job(runtime: &OrbitRuntime, job_id: &str, as_json: bool) -> Result<(), OrbitError> {
    let entry = runtime.show_job_catalog_entry(job_id)?;
    if as_json {
        let value = json!({
            "job_id": entry.job_id,
            "kind": entry.kind().to_string(),
            "state": entry.state().to_string(),
            "path": entry.path.display().to_string(),
            "spec": entry.spec,
        });
        crate::output::json::print_pretty(&value)
    } else {
        println!("Job ID:  {}", entry.job_id);
        println!("Kind:    {}", entry.kind());
        println!("State:   {}", entry.state());
        println!("Steps:   {}", entry.spec.steps.len());
        Ok(())
    }
}

// ── Activities (v2 catalog) ──

fn list_activities(runtime: &OrbitRuntime, as_json: bool) -> Result<(), OrbitError> {
    let catalog = runtime
        .v2_activity_catalog()
        .map_err(|err| OrbitError::Store(format!("v2 activity catalog: {err}")))?;
    let names: Vec<&str> = catalog.names().collect();
    if as_json {
        let values: Vec<Value> = names
            .iter()
            .filter_map(|name| {
                catalog.get(name).map(|spec| {
                    json!({
                        "id": name,
                        "type": activity_v2_type_label(spec),
                        "description": spec.description,
                    })
                })
            })
            .collect();
        crate::output::json::print_pretty(&Value::Array(values))
    } else {
        if names.is_empty() {
            println!("No activities found.");
            return Ok(());
        }
        let mut table = crate::output::table::build_table(&["ID", "TYPE", "DESCRIPTION"]);
        for name in &names {
            let Some(spec) = catalog.get(name) else {
                continue;
            };
            table.add_row(vec![
                name.to_string(),
                activity_v2_type_label(spec).to_string(),
                spec.description.clone(),
            ]);
        }
        println!("{table}");
        Ok(())
    }
}

fn show_activity(runtime: &OrbitRuntime, id: &str, as_json: bool) -> Result<(), OrbitError> {
    let catalog = runtime
        .v2_activity_catalog()
        .map_err(|err| OrbitError::Store(format!("v2 activity catalog: {err}")))?;
    let activity = catalog
        .get(id)
        .ok_or_else(|| OrbitError::ActivityNotFound(id.to_string()))?;
    if as_json {
        let value = json!({
            "id": id,
            "type": activity_v2_type_label(activity),
            "description": activity.description,
            "input_schema_json": activity.input_schema_json,
            "output_schema_json": activity.output_schema_json,
            "fsProfile": activity.fs_profile,
            "schemaVersion": 2,
        });
        crate::output::json::print_pretty(&value)
    } else {
        println!("ID:          {id}");
        println!("Type:        {}", activity_v2_type_label(activity));
        println!("Description: {}", activity.description);
        Ok(())
    }
}

fn activity_v2_type_label(spec: &orbit_common::types::activity_job::ActivityV2) -> &'static str {
    use orbit_common::types::activity_job::ActivityV2Spec;
    match &spec.spec {
        ActivityV2Spec::AgentLoop(_) => "agent_loop",
        ActivityV2Spec::Groundhog(_) => "groundhog",
        ActivityV2Spec::Deterministic(_) => "deterministic",
        ActivityV2Spec::Shell(_) => "shell",
    }
}

// ── Policies ──

fn list_policies(runtime: &OrbitRuntime, as_json: bool) -> Result<(), OrbitError> {
    let defs = runtime.list_policy_defs()?;
    if as_json {
        let values: Vec<Value> = defs
            .iter()
            .map(|d| {
                json!({
                    "name": d.name,
                    "description": d.description,
                    "updated_at": d.updated_at.to_rfc3339(),
                })
            })
            .collect();
        crate::output::json::print_pretty(&Value::Array(values))
    } else {
        if defs.is_empty() {
            println!("No policies found.");
            return Ok(());
        }
        let mut table = crate::output::table::build_table(&["NAME", "DESCRIPTION", "UPDATED"]);
        for d in &defs {
            table.add_row(vec![
                d.name.clone(),
                d.description.clone().unwrap_or_default(),
                d.updated_at.format("%Y-%m-%d %H:%M").to_string(),
            ]);
        }
        println!("{table}");
        Ok(())
    }
}

fn show_policy(runtime: &OrbitRuntime, name: &str, as_json: bool) -> Result<(), OrbitError> {
    let def = runtime
        .get_policy_def(name)?
        .ok_or_else(|| OrbitError::InvalidInput(format!("policy not found: {name}")))?;
    if as_json {
        let value = serde_json::to_value(&def).map_err(|e| OrbitError::Execution(e.to_string()))?;
        crate::output::json::print_pretty(&value)
    } else {
        println!("Name:        {}", def.name);
        if let Some(desc) = &def.description {
            println!("Description: {desc}");
        }
        println!("Updated:     {}", def.updated_at.to_rfc3339());
        Ok(())
    }
}

// ── Executors ──

fn list_executors(runtime: &OrbitRuntime, as_json: bool) -> Result<(), OrbitError> {
    let defs = runtime.list_executor_defs()?;
    if as_json {
        let values: Vec<Value> = defs
            .iter()
            .map(|d| {
                json!({
                    "name": d.name,
                    "executor_type": d.executor_type.to_string(),
                    "command": d.command,
                })
            })
            .collect();
        crate::output::json::print_pretty(&Value::Array(values))
    } else {
        if defs.is_empty() {
            println!("No executors found.");
            return Ok(());
        }
        let mut table = crate::output::table::build_table(&["NAME", "TYPE", "COMMAND", "TIMEOUT"]);
        for d in &defs {
            table.add_row(vec![
                d.name.clone(),
                d.executor_type.to_string(),
                d.command.clone().unwrap_or_default(),
                d.timeout_seconds
                    .map(|t| format!("{t}s"))
                    .unwrap_or_default(),
            ]);
        }
        println!("{table}");
        Ok(())
    }
}

fn show_executor(runtime: &OrbitRuntime, name: &str, as_json: bool) -> Result<(), OrbitError> {
    let def = runtime
        .get_executor_def(name)?
        .ok_or_else(|| OrbitError::InvalidInput(format!("executor not found: {name}")))?;
    if as_json {
        let value = serde_json::to_value(&def).map_err(|e| OrbitError::Execution(e.to_string()))?;
        crate::output::json::print_pretty(&value)
    } else {
        println!("Name:     {}", def.name);
        println!("Type:     {}", def.executor_type);
        if let Some(cmd) = &def.command {
            println!("Command:  {cmd}");
        }
        if !def.args.is_empty() {
            println!("Args:     {}", def.args.join(" "));
        }
        if let Some(timeout) = def.timeout_seconds {
            println!("Timeout:  {timeout}s");
        }
        Ok(())
    }
}
