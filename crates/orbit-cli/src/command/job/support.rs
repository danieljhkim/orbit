use orbit_common::types::{ActivityV2Spec, JobKind, JobV2Step, JobV2StepBody};
use orbit_core::JobRun;
use orbit_core::command::job::{JobCatalogEntry, JobCatalogFilter};
use serde_json::{Value, json};

pub(super) fn format_last_run(last_run: Option<&JobRun>) -> String {
    match last_run {
        None => "never".to_string(),
        Some(run) => {
            let ts = run
                .finished_at
                .or(run.started_at)
                .unwrap_or(run.scheduled_at);
            format!("{} {}", run.state, ts.format("%Y-%m-%dT%H:%M:%SZ"))
        }
    }
}

pub(super) fn job_catalog_filter(
    include_disabled: bool,
    kind: Option<JobKind>,
) -> JobCatalogFilter {
    match kind {
        Some(kind) => JobCatalogFilter::Kind(kind),
        None if include_disabled => JobCatalogFilter::All,
        None => JobCatalogFilter::WorkflowsOnly,
    }
}

pub(super) fn job_catalog_target_summary(job: &JobCatalogEntry) -> (String, String) {
    job.spec
        .steps
        .first()
        .map(v2_step_target_summary)
        .unwrap_or_else(|| ("-".to_string(), "-".to_string()))
}

pub(super) fn job_catalog_to_json_with_last_run(
    job: &JobCatalogEntry,
    last_run: Option<&JobRun>,
) -> Value {
    let mut value = json!({
        "job_id": job.job_id.clone(),
        "kind": job.kind().to_string(),
        "state": job.state().to_string(),
        "default_input": job.spec.default_input,
        "max_active_runs": job.spec.max_active_runs,
        "steps": job.spec.steps.iter().map(job_v2_step_to_json).collect::<Vec<_>>(),
        "path": job.path.display().to_string(),
    });
    value["last_run_state"] = last_run
        .map(|r| serde_json::Value::String(r.state.to_string()))
        .unwrap_or(serde_json::Value::Null);
    value["last_run_at"] = last_run
        .and_then(|r| r.finished_at.or(r.started_at).or(Some(r.scheduled_at)))
        .map(|ts| serde_json::Value::String(ts.to_rfc3339()))
        .unwrap_or(serde_json::Value::Null);
    value
}

pub(super) fn job_catalog_to_signal_json(job: &JobCatalogEntry) -> Value {
    let (_, target_id) = job_catalog_target_summary(job);
    json!({
        "job_id": job.job_id.clone(),
        "target_id": target_id,
        "state": job.state().to_string(),
    })
}

fn job_v2_step_to_json(step: &JobV2Step) -> Value {
    let mut value = json!({
        "id": step.id.clone(),
        "when": step.when,
        "retry": step.retry,
    });
    match &step.body {
        JobV2StepBody::TargetRef(target) => {
            value["body"] = json!({
                "kind": "target_ref",
                "target": target.target.clone(),
                "default_input": target.default_input,
                "timeout_seconds": target.timeout_seconds,
                "session": target.session,
            });
        }
        JobV2StepBody::Target(target) => {
            value["body"] = json!({
                "kind": "target",
                "default_input": target.default_input,
                "timeout_seconds": target.timeout_seconds,
                "session": target.session,
                "spec": target.spec,
            });
        }
        JobV2StepBody::Parallel { parallel } => {
            value["body"] = json!({
                "kind": "parallel",
                "join": parallel.join,
                "branches": parallel.branches.iter().map(job_v2_step_to_json).collect::<Vec<_>>(),
            });
        }
        JobV2StepBody::FanOut { fan_out, fan_in } => {
            value["body"] = json!({
                "kind": "fan_out",
                "items": fan_out.items,
                "max_workers": fan_out.max_workers,
                "worker": job_v2_step_to_json(&fan_out.worker),
                "fan_in": fan_in,
            });
        }
        JobV2StepBody::Loop { loop_ } => {
            value["body"] = json!({
                "kind": "loop",
                "max_iterations": loop_.max_iterations,
                "break_when": loop_.break_when,
                "steps": loop_.steps.iter().map(job_v2_step_to_json).collect::<Vec<_>>(),
            });
        }
    }
    value
}

pub(super) fn print_v2_step(step: &JobV2Step, indent: usize) {
    use crate::output::color::bold;

    let pad = " ".repeat(indent);
    println!("{pad}{} {}", bold("ID:"), step.id.as_str());
    if let Some(when) = &step.when {
        println!("{pad}{} {}", bold("When:"), when);
    }
    if let Some(retry) = &step.retry {
        println!("{pad}{} {:?}", bold("Retry:"), retry);
    }
    match &step.body {
        JobV2StepBody::TargetRef(target) => {
            println!("{pad}{} {}", bold("Target Ref:"), target.target.as_str());
            if let Some(session) = &target.session {
                println!("{pad}{} {}", bold("Session:"), session);
            }
            println!("{pad}{} {}", bold("Timeout (s):"), target.timeout_seconds);
        }
        JobV2StepBody::Target(target) => {
            match &target.spec {
                ActivityV2Spec::AgentLoop(spec) => {
                    println!("{pad}{} agent_loop", bold("Activity Type:"));
                    println!("{pad}{} {}", bold("Provider:"), spec.provider.as_str());
                    println!("{pad}{} {}", bold("Backend:"), spec.backend.as_str());
                    if let Some(model) = &spec.model {
                        println!("{pad}{} {}", bold("Model:"), model);
                    }
                }
                ActivityV2Spec::Groundhog(spec) => {
                    println!("{pad}{} groundhog", bold("Activity Type:"));
                    println!("{pad}{} {}", bold("Provider:"), spec.provider.as_str());
                    println!("{pad}{} http", bold("Backend:"));
                    println!(
                        "{pad}{} {}",
                        bold("Attempt Budget Default:"),
                        spec.attempt_budget_default
                    );
                    if let Some(model) = &spec.model {
                        println!("{pad}{} {}", bold("Model:"), model);
                    }
                }
                ActivityV2Spec::Deterministic(spec) => {
                    println!("{pad}{} deterministic", bold("Activity Type:"));
                    println!("{pad}{} {}", bold("Action:"), spec.action.as_str());
                }
                ActivityV2Spec::Shell(spec) => {
                    println!("{pad}{} shell", bold("Activity Type:"));
                    println!("{pad}{} {}", bold("Program:"), spec.program.as_str());
                }
            }
            if let Some(session) = &target.session {
                println!("{pad}{} {}", bold("Session:"), session);
            }
            println!("{pad}{} {}", bold("Timeout (s):"), target.timeout_seconds);
        }
        JobV2StepBody::Parallel { parallel } => {
            println!("{pad}{} parallel", bold("Body:"));
            println!("{pad}{} {:?}", bold("Join:"), parallel.join);
            println!("{pad}{} {}", bold("Branches:"), parallel.branches.len());
            for branch in &parallel.branches {
                print_v2_step(branch, indent + 2);
            }
        }
        JobV2StepBody::FanOut { fan_out, fan_in } => {
            println!("{pad}{} fan_out", bold("Body:"));
            println!("{pad}{} {}", bold("Items:"), fan_out.items.as_str());
            println!("{pad}{} {}", bold("Max Workers:"), fan_out.max_workers);
            println!("{pad}{} {:?}", bold("Fan In:"), fan_in);
            print_v2_step(&fan_out.worker, indent + 2);
        }
        JobV2StepBody::Loop { loop_ } => {
            println!("{pad}{} loop", bold("Body:"));
            println!("{pad}{} {}", bold("Max Iterations:"), loop_.max_iterations);
            if let Some(break_when) = &loop_.break_when {
                println!("{pad}{} {}", bold("Break When:"), break_when);
            }
            for nested in &loop_.steps {
                print_v2_step(nested, indent + 2);
            }
        }
    }
}

fn v2_step_target_summary(step: &JobV2Step) -> (String, String) {
    match &step.body {
        JobV2StepBody::TargetRef(target) => ("activity_ref".to_string(), target.target.clone()),
        JobV2StepBody::Target(target) => match &target.spec {
            ActivityV2Spec::AgentLoop(spec) => (
                "agent_loop".to_string(),
                spec.model
                    .clone()
                    .unwrap_or_else(|| spec.provider.as_str().to_string()),
            ),
            ActivityV2Spec::Groundhog(spec) => (
                "groundhog".to_string(),
                spec.model
                    .clone()
                    .unwrap_or_else(|| spec.provider.as_str().to_string()),
            ),
            ActivityV2Spec::Deterministic(spec) => {
                ("deterministic".to_string(), spec.action.clone())
            }
            ActivityV2Spec::Shell(spec) => ("shell".to_string(), spec.program.clone()),
        },
        JobV2StepBody::Parallel { .. } => ("parallel".to_string(), step.id.clone()),
        JobV2StepBody::FanOut { .. } => ("fan_out".to_string(), step.id.clone()),
        JobV2StepBody::Loop { .. } => ("loop".to_string(), step.id.clone()),
    }
}
