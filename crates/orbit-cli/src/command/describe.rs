use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_types::ResourceKind;

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
    let job = runtime
        .get_job(job_id)?
        .ok_or_else(|| OrbitError::JobNotFound(job_id.to_string()))?;

    println!("Job ID:          {}", job.job_id);
    println!("State:           {}", job.state);
    println!("Max Active Runs: {}", job.max_active_runs);
    println!("Max Iterations:  {}", job.max_iterations);
    if let Some(ref policy) = job.policy {
        println!("Policy:          {policy}");
    }
    if let Some(ref input) = job.default_input {
        println!(
            "Default Input:   {}",
            serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string())
        );
    }
    println!("Created:         {}", job.created_at.to_rfc3339());
    println!("Updated:         {}", job.updated_at.to_rfc3339());
    println!();
    println!("Steps ({}):", job.steps.len());
    for (i, step) in job.steps.iter().enumerate() {
        println!("  Step {}:", i + 1);
        println!("    Target Type: {}", step.target_type);
        println!("    Target ID:   {}", step.target_id);
        println!("    Agent CLI:   {}", step.agent_cli);
        if let Some(ref model) = step.model {
            println!("    Model:       {model}");
        }
        if step.timeout_seconds > 0 {
            println!("    Timeout:     {}s", step.timeout_seconds);
        }
        if step.retry_max_attempts > 0 {
            println!("    Retries:     {}", step.retry_max_attempts);
        }
    }
    Ok(())
}

fn describe_activity(runtime: &OrbitRuntime, id: &str) -> Result<(), OrbitError> {
    let activity = runtime.show_activity(id)?;

    println!("ID:          {}", activity.id);
    println!("Description: {}", activity.description);
    println!("Spec Type:   {}", activity.spec_type);
    println!("Active:      {}", activity.is_active);
    if let Some(ref executor) = activity.executor {
        println!("Executor:    {executor}");
    }
    if let Some(ref ws) = activity.workspace_path {
        println!("Workspace:   {ws}");
    }
    if !activity.tools.is_empty() {
        println!("Tools:       {}", activity.tools.join(", "));
    }
    if !activity.proc_allowed_programs.is_empty() {
        println!("Proc Allow:  {}", activity.proc_allowed_programs.join(", "));
    }
    if let Some(ref created_by) = activity.created_by {
        println!("Created By:  {created_by}");
    }
    println!("Created:     {}", activity.created_at.to_rfc3339());
    println!("Updated:     {}", activity.updated_at.to_rfc3339());

    if !activity.spec_config.is_null() {
        println!();
        println!("Spec Config:");
        println!(
            "{}",
            serde_json::to_string_pretty(&activity.spec_config)
                .unwrap_or_else(|_| activity.spec_config.to_string())
        );
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

    if let Some(ref fs) = def.filesystem {
        println!();
        println!("Filesystem:");
        if !fs.allow_write.is_empty() {
            println!("  allow_write: {}", fs.allow_write.join(", "));
        }
        if !fs.deny_write.is_empty() {
            println!("  deny_write:  {}", fs.deny_write.join(", "));
        }
    }
    if let Some(ref net) = def.network {
        println!();
        println!("Network:");
        println!("  mode: {:?}", net.mode);
        if !net.allow_hosts.is_empty() {
            println!("  allow_hosts: {}", net.allow_hosts.join(", "));
        }
        if !net.deny_hosts.is_empty() {
            println!("  deny_hosts:  {}", net.deny_hosts.join(", "));
        }
    }
    if let Some(ref proc) = def.process {
        println!();
        println!("Process:");
        if !proc.allow_commands.is_empty() {
            println!("  allow_commands: {}", proc.allow_commands.join(", "));
        }
        if !proc.deny_commands.is_empty() {
            println!("  deny_commands:  {}", proc.deny_commands.join(", "));
        }
    }
    if let Some(ref approval) = def.approval {
        println!();
        println!("Approval:");
        if !approval.require_for.is_empty() {
            println!("  require_for: {}", approval.require_for.join(", "));
        }
    }
    if let Some(ref tools) = def.tools {
        println!();
        println!("Tools:");
        if !tools.allow.is_empty() {
            println!("  allow: {}", tools.allow.join(", "));
        }
        if !tools.deny.is_empty() {
            println!("  deny:  {}", tools.deny.join(", "));
        }
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
