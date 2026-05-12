use clap::Args;
use orbit_common::types::ResourceKind;
use orbit_core::{NotFoundKind, OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Show detailed resource description")]
pub struct DescribeCommand {
    /// Resource reference: kind/name (e.g. "policy/safe-local-dev")
    pub resource: String,
}

impl Execute for DescribeCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let (kind, name) = parse_resource_ref(&self.resource)?;

        match kind {
            ResourceKind::Job => describe_job(runtime, &name),
            ResourceKind::Activity => describe_activity(runtime, &name),
            ResourceKind::Policy => describe_policy(runtime, &name),
            ResourceKind::Executor => describe_executor(runtime, &name),
        }
    }
}

fn parse_resource_ref(s: &str) -> Result<(ResourceKind, String), OrbitError> {
    let (kind_str, name) = s
        .split_once('/')
        .ok_or_else(|| OrbitError::InvalidInput("expected kind/name (e.g. policy/foo)".into()))?;
    let kind: ResourceKind = kind_str
        .parse()
        .map_err(|e: String| OrbitError::InvalidInput(e))?;
    Ok((kind, name.to_string()))
}

fn describe_job(runtime: &OrbitRuntime, job_id: &str) -> Result<(), OrbitError> {
    let entry = runtime.show_job_catalog_entry(job_id)?;

    println!("Job ID:          {}", entry.job_id);
    println!("Kind:            {}", entry.kind());
    println!("State:           {}", entry.state());
    println!("Max Active Runs: {}", entry.max_active_runs());
    if let Some(input) = entry.default_input() {
        println!(
            "Default Input:   {}",
            serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string())
        );
    }
    println!("Path:            {}", entry.path.display());
    println!();
    println!("Steps ({}):", entry.spec.steps.len());
    for (i, step) in entry.spec.steps.iter().enumerate() {
        println!("  Step {}: {}", i + 1, step.id);
    }
    Ok(())
}

fn describe_activity(runtime: &OrbitRuntime, id: &str) -> Result<(), OrbitError> {
    use orbit_common::types::activity_job::ActivityV2Spec;
    let catalog = runtime
        .v2_activity_catalog()
        .map_err(|err| OrbitError::Store(format!("v2 activity catalog: {err}")))?;
    let activity = catalog
        .get(id)
        .ok_or_else(|| OrbitError::not_found(NotFoundKind::Activity, id.to_string()))?;
    let type_label = match &activity.spec {
        ActivityV2Spec::AgentLoop(_) => "agent_loop",
        ActivityV2Spec::Groundhog(_) => "groundhog",
        ActivityV2Spec::Deterministic(_) => "deterministic",
        ActivityV2Spec::Shell(_) => "shell",
    };

    println!("ID:           {id}");
    println!("Type:         {type_label}");
    println!("Description:  {}", activity.description);
    if let Some(ref profile) = activity.fs_profile {
        println!("FS Profile:   {profile}");
    }
    match &activity.spec {
        ActivityV2Spec::AgentLoop(spec) => {
            println!("Backend:      {}", spec.backend.as_str());
            println!("Provider:     {}", spec.provider.as_str());
            if !spec.tools.is_empty() {
                println!("Tools:        {}", spec.tools.join(", "));
            }
        }
        ActivityV2Spec::Groundhog(spec) => {
            println!("Backend:      http");
            println!("Provider:     {}", spec.provider.as_str());
            println!("Attempts:     {}", spec.attempt_budget_default);
            if !spec.tools.is_empty() {
                println!("Tools:        {}", spec.tools.join(", "));
            }
        }
        ActivityV2Spec::Deterministic(spec) => {
            println!("Action:       {}", spec.action);
        }
        ActivityV2Spec::Shell(spec) => {
            println!("Program:      {}", spec.program);
            if !spec.args.is_empty() {
                println!("Args:         {}", spec.args.join(" "));
            }
        }
    }
    Ok(())
}

fn describe_policy(runtime: &OrbitRuntime, name: &str) -> Result<(), OrbitError> {
    let def = runtime
        .get_policy_def(name)?
        .ok_or_else(|| OrbitError::InvalidInput(format!("policy not found: {name}")))?;

    println!("Name:        {}", def.name);
    if let Some(ref desc) = def.description {
        println!("Description: {desc}");
    }
    println!("Created:     {}", def.created_at.to_rfc3339());
    println!("Updated:     {}", def.updated_at.to_rfc3339());

    println!();
    println!("Global Denies:");
    println!(
        "  denyRead:   {}",
        if def.deny_read.is_empty() {
            "[]".to_string()
        } else {
            def.deny_read.join(", ")
        }
    );
    println!(
        "  denyModify: {}",
        if def.deny_modify.is_empty() {
            "[]".to_string()
        } else {
            def.deny_modify.join(", ")
        }
    );

    println!();
    println!("fsProfiles:");
    let mut names: Vec<String> = def.fs_profiles.keys().cloned().collect();
    names.sort();
    if !names
        .iter()
        .any(|name| name == orbit_common::types::UNRESTRICTED_FS_PROFILE)
    {
        names.push(orbit_common::types::UNRESTRICTED_FS_PROFILE.to_string());
    }

    for profile_name in names {
        let profile = def.effective_profile(&profile_name)?;
        println!("  {}:", profile_name);
        println!(
            "    read:   {}",
            if profile.read.is_empty() {
                "[]".to_string()
            } else {
                profile.read.join(", ")
            }
        );
        println!(
            "    modify: {}",
            if profile.modify.is_empty() {
                "[]".to_string()
            } else {
                profile.modify.join(", ")
            }
        );
    }
    Ok(())
}

fn describe_executor(runtime: &OrbitRuntime, name: &str) -> Result<(), OrbitError> {
    let def = runtime
        .get_executor_def(name)?
        .ok_or_else(|| OrbitError::InvalidInput(format!("executor not found: {name}")))?;

    println!("Name:     {}", def.name);
    println!("Type:     {}", def.executor_type);
    if let Some(ref cmd) = def.command {
        println!("Command:  {cmd}");
    }
    if !def.args.is_empty() {
        println!("Args:     {}", def.args.join(" "));
    }
    if let Some(ref fmt) = def.stdout_format {
        println!("Stdout:   {fmt}");
    }
    if let Some(timeout) = def.timeout_seconds {
        println!("Timeout:  {timeout}s");
    }
    if !def.env.is_empty() {
        println!();
        println!("Env:");
        for (k, v) in &def.env {
            println!("  {k}={v}");
        }
    }
    println!("Created:  {}", def.created_at.to_rfc3339());
    println!("Updated:  {}", def.updated_at.to_rfc3339());
    Ok(())
}
