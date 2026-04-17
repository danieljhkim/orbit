use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_types::ResourceKind;
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
    let jobs = runtime.list_jobs(false)?;
    if as_json {
        let values: Vec<Value> = jobs
            .iter()
            .map(|j| {
                json!({
                    "job_id": j.job_id,
                    "state": j.state.to_string(),
                    "steps": j.steps.len(),
                })
            })
            .collect();
        crate::output::json::print_pretty(&Value::Array(values))
    } else {
        if jobs.is_empty() {
            println!("No jobs found.");
            return Ok(());
        }
        let mut table = crate::output::table::build_table(&["JOB ID", "STATE", "STEPS"]);
        for j in &jobs {
            table.add_row(vec![
                j.job_id.to_string(),
                j.state.to_string(),
                j.steps.len().to_string(),
            ]);
        }
        println!("{table}");
        Ok(())
    }
}

fn show_job(runtime: &OrbitRuntime, job_id: &str, as_json: bool) -> Result<(), OrbitError> {
    let job = runtime
        .get_job(job_id)?
        .ok_or_else(|| OrbitError::JobNotFound(job_id.to_string()))?;
    if as_json {
        let value = serde_json::to_value(&job).map_err(|e| OrbitError::Execution(e.to_string()))?;
        crate::output::json::print_pretty(&value)
    } else {
        println!("Job ID:  {}", job.job_id);
        println!("State:   {}", job.state);
        println!("Steps:   {}", job.steps.len());
        Ok(())
    }
}

// ── Activities ──

fn list_activities(runtime: &OrbitRuntime, as_json: bool) -> Result<(), OrbitError> {
    let activities = runtime.list_activities(false)?;
    if as_json {
        let values: Vec<Value> = activities
            .iter()
            .map(|a| {
                json!({
                    "id": a.id,
                    "description": a.description,
                    "is_active": a.is_active,
                })
            })
            .collect();
        crate::output::json::print_pretty(&Value::Array(values))
    } else {
        if activities.is_empty() {
            println!("No activities found.");
            return Ok(());
        }
        let mut table = crate::output::table::build_table(&["ID", "DESCRIPTION", "ACTIVE"]);
        for a in &activities {
            table.add_row(vec![
                a.id.to_string(),
                a.description.clone(),
                if a.is_active { "yes" } else { "no" }.to_string(),
            ]);
        }
        println!("{table}");
        Ok(())
    }
}

fn show_activity(runtime: &OrbitRuntime, id: &str, as_json: bool) -> Result<(), OrbitError> {
    let activity = runtime.show_activity(id)?;
    if as_json {
        let value =
            serde_json::to_value(&activity).map_err(|e| OrbitError::Execution(e.to_string()))?;
        crate::output::json::print_pretty(&value)
    } else {
        println!("ID:          {}", activity.id);
        println!("Description: {}", activity.description);
        println!("Active:      {}", activity.is_active);
        println!("Spec type:   {}", activity.spec_type);
        Ok(())
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
                    "executor_type": d.executor_type,
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
                d.executor_type.clone(),
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
