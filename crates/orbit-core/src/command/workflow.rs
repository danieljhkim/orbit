use serde_json::Value;

use crate::OrbitError;

pub struct Workflow {
    pub alias: &'static str,
    pub job_id: &'static str,
    pub description: &'static str,
    pub supports_tasks: bool,
    pub supports_parallelism: bool,
    pub supports_base: bool,
    pub supports_pr_number: bool,
    pub requires_pr_number: bool,
    /// Upper bound on `--tasks` cardinality. `None` means unbounded (the
    /// historical default). Set to `Some(1)` for single-task workflows like
    /// `duel` that must reject multi-task input with a loud, workflow-
    /// specific error rather than silently taking the first entry.
    pub max_tasks: Option<u32>,
}

pub const WORKFLOWS: &[Workflow] = &[
    Workflow {
        alias: "ship",
        job_id: "job_parallel_task_pipeline",
        description: "Dispatch, implement, open PR, and run review cycle",
        supports_tasks: true,
        supports_parallelism: true,
        supports_base: true,
        supports_pr_number: false,
        requires_pr_number: false,
        max_tasks: None,
    },
    Workflow {
        alias: "ship-local",
        job_id: "job_local_task_pipeline",
        description: "Dispatch, implement, and commit locally (no PR)",
        supports_tasks: true,
        supports_parallelism: true,
        supports_base: true,
        supports_pr_number: false,
        requires_pr_number: false,
        max_tasks: None,
    },
    Workflow {
        alias: "review-pr",
        job_id: "job_batch_review_cycle",
        description: "Review, gate, fix-loop, and merge a batch PR by PR number",
        supports_tasks: false,
        supports_parallelism: false,
        supports_base: true,
        supports_pr_number: true,
        requires_pr_number: true,
        max_tasks: None,
    },
    Workflow {
        alias: "duel",
        job_id: "job_duel_pipeline",
        description: "Single-task cross-agent duel: random implementer/reviewer/arbiter, scored",
        supports_tasks: true,
        supports_parallelism: false,
        supports_base: true,
        supports_pr_number: false,
        requires_pr_number: false,
        max_tasks: Some(1),
    },
    Workflow {
        alias: "duel-plan",
        job_id: "job_duel_plan_pipeline",
        description: "Single-task planning duel: two planners and one arbiter, scored",
        supports_tasks: true,
        supports_parallelism: false,
        supports_base: true,
        supports_pr_number: false,
        requires_pr_number: false,
        max_tasks: Some(1),
    },
];

pub fn find_workflow(name: &str) -> Option<&'static Workflow> {
    WORKFLOWS.iter().find(|w| w.alias == name)
}

pub struct WorkflowInput {
    pub tasks: Option<String>,
    pub parallelism: Option<u32>,
    pub base: Option<String>,
    pub pr_number: Option<String>,
}

pub fn validate_workflow_flags(
    workflow: &Workflow,
    input: &WorkflowInput,
) -> Result<(), OrbitError> {
    if !workflow.supports_tasks && input.tasks.is_some() {
        return Err(OrbitError::InvalidInput(format!(
            "--tasks is not supported by workflow '{}'",
            workflow.alias
        )));
    }
    if !workflow.supports_parallelism && input.parallelism.is_some() {
        return Err(OrbitError::InvalidInput(format!(
            "--parallelism is not supported by workflow '{}'",
            workflow.alias
        )));
    }
    if !workflow.supports_base && input.base.is_some() {
        return Err(OrbitError::InvalidInput(format!(
            "--base is not supported by workflow '{}'",
            workflow.alias
        )));
    }
    if !workflow.supports_pr_number && input.pr_number.is_some() {
        return Err(OrbitError::InvalidInput(format!(
            "--pr-number is not supported by workflow '{}'",
            workflow.alias
        )));
    }
    if workflow.requires_pr_number && input.pr_number.is_none() {
        return Err(OrbitError::InvalidInput(format!(
            "--pr-number is required for workflow '{}'",
            workflow.alias
        )));
    }
    Ok(())
}

pub fn build_workflow_input(input: &WorkflowInput) -> Result<Value, OrbitError> {
    build_workflow_input_for(None, input)
}

/// Variant of [`build_workflow_input`] that also enforces any workflow-
/// specific cardinality constraints such as `Workflow::max_tasks`. Callers
/// that already know the resolved workflow should use this; the legacy
/// `build_workflow_input` is retained for call sites that do not.
pub fn build_workflow_input_for(
    workflow: Option<&Workflow>,
    input: &WorkflowInput,
) -> Result<Value, OrbitError> {
    let mut map = serde_json::Map::new();

    if let Some(tasks) = &input.tasks {
        let task_ids: Vec<Value> = tasks
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| Value::String(s.to_string()))
            .collect();
        if task_ids.is_empty() {
            return Err(OrbitError::InvalidInput(
                "--tasks value must not be empty".to_string(),
            ));
        }
        if let Some(workflow) = workflow
            && let Some(max) = workflow.max_tasks
            && task_ids.len() as u32 > max
        {
            if max == 1 {
                return Err(OrbitError::InvalidInput(format!(
                    "workflow '{}' accepts exactly one task id — got {}",
                    workflow.alias,
                    task_ids.len()
                )));
            }
            return Err(OrbitError::InvalidInput(format!(
                "workflow '{}' accepts at most {} task ids — got {}",
                workflow.alias,
                max,
                task_ids.len()
            )));
        }
        if let Some(workflow) = workflow
            && workflow.max_tasks == Some(1)
            && task_ids.len() == 1
        {
            map.insert("task_id".to_string(), task_ids[0].clone());
        }
        map.insert("task_ids".to_string(), Value::Array(task_ids));
    }

    if let Some(parallelism) = input.parallelism {
        if parallelism == 0 {
            return Err(OrbitError::InvalidInput(
                "--parallelism must be greater than 0".to_string(),
            ));
        }
        map.insert("parallelism".to_string(), Value::Number(parallelism.into()));
    }

    if let Some(base) = &input.base {
        if base.is_empty() {
            return Err(OrbitError::InvalidInput(
                "--base must not be empty".to_string(),
            ));
        }
        map.insert("base".to_string(), Value::String(base.clone()));
    }

    if let Some(pr_number) = &input.pr_number {
        if pr_number.is_empty() {
            return Err(OrbitError::InvalidInput(
                "--pr-number must not be empty".to_string(),
            ));
        }
        map.insert("pr_number".to_string(), Value::String(pr_number.clone()));
    }

    Ok(Value::Object(map))
}
