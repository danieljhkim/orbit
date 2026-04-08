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
    },
    Workflow {
        alias: "review",
        job_id: "job_review_tasks",
        description: "Review tasks in proposed/review status",
        supports_tasks: false,
        supports_parallelism: false,
        supports_base: false,
        supports_pr_number: false,
        requires_pr_number: false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn find_workflow_returns_match() {
        assert_eq!(
            find_workflow("ship").unwrap().job_id,
            "job_parallel_task_pipeline"
        );
        assert_eq!(
            find_workflow("ship-local").unwrap().job_id,
            "job_local_task_pipeline"
        );
        assert_eq!(find_workflow("review").unwrap().job_id, "job_review_tasks");
        assert_eq!(
            find_workflow("review-pr").unwrap().job_id,
            "job_batch_review_cycle"
        );
    }

    #[test]
    fn find_workflow_returns_none_for_unknown() {
        assert!(find_workflow("nonexistent").is_none());
    }

    #[test]
    fn validate_rejects_unsupported_tasks() {
        let workflow = find_workflow("review").unwrap();
        let input = WorkflowInput {
            tasks: Some("T123".to_string()),
            parallelism: None,
            base: None,
            pr_number: None,
        };
        assert!(validate_workflow_flags(workflow, &input).is_err());
    }

    #[test]
    fn validate_rejects_unsupported_parallelism() {
        let workflow = find_workflow("review").unwrap();
        let input = WorkflowInput {
            tasks: None,
            parallelism: Some(2),
            base: None,
            pr_number: None,
        };
        assert!(validate_workflow_flags(workflow, &input).is_err());
    }

    #[test]
    fn validate_accepts_supported_flags() {
        let workflow = find_workflow("ship").unwrap();
        let input = WorkflowInput {
            tasks: Some("T123,T456".to_string()),
            parallelism: Some(2),
            base: Some("main".to_string()),
            pr_number: None,
        };
        assert!(validate_workflow_flags(workflow, &input).is_ok());
    }

    #[test]
    fn validate_rejects_unsupported_pr_number() {
        let workflow = find_workflow("ship").unwrap();
        let input = WorkflowInput {
            tasks: None,
            parallelism: None,
            base: None,
            pr_number: Some("42".to_string()),
        };
        assert!(validate_workflow_flags(workflow, &input).is_err());
    }

    #[test]
    fn validate_requires_pr_number_for_review_pr() {
        let workflow = find_workflow("review-pr").unwrap();
        let input = WorkflowInput {
            tasks: None,
            parallelism: None,
            base: Some("main".to_string()),
            pr_number: None,
        };
        assert!(validate_workflow_flags(workflow, &input).is_err());
    }

    #[test]
    fn validate_accepts_review_pr_flags() {
        let workflow = find_workflow("review-pr").unwrap();
        let input = WorkflowInput {
            tasks: None,
            parallelism: None,
            base: Some("main".to_string()),
            pr_number: Some("42".to_string()),
        };
        assert!(validate_workflow_flags(workflow, &input).is_ok());
    }

    #[test]
    fn build_input_empty_when_no_flags() {
        let input = WorkflowInput {
            tasks: None,
            parallelism: None,
            base: None,
            pr_number: None,
        };
        assert_eq!(build_workflow_input(&input).unwrap(), json!({}));
    }

    #[test]
    fn build_input_maps_all_flags() {
        let input = WorkflowInput {
            tasks: Some("T123,T456".to_string()),
            parallelism: Some(3),
            base: Some("main".to_string()),
            pr_number: Some("42".to_string()),
        };
        let result = build_workflow_input(&input).unwrap();
        assert_eq!(result["task_ids"], json!(["T123", "T456"]));
        assert_eq!(result["parallelism"], json!(3));
        assert_eq!(result["base"], json!("main"));
        assert_eq!(result["pr_number"], json!("42"));
    }

    #[test]
    fn build_input_rejects_zero_parallelism() {
        let input = WorkflowInput {
            tasks: None,
            parallelism: Some(0),
            base: None,
            pr_number: None,
        };
        assert!(build_workflow_input(&input).is_err());
    }

    #[test]
    fn build_input_rejects_empty_tasks() {
        let input = WorkflowInput {
            tasks: Some("".to_string()),
            parallelism: None,
            base: None,
            pr_number: None,
        };
        assert!(build_workflow_input(&input).is_err());
    }

    #[test]
    fn build_input_rejects_empty_base() {
        let input = WorkflowInput {
            tasks: None,
            parallelism: None,
            base: Some("".to_string()),
            pr_number: None,
        };
        assert!(build_workflow_input(&input).is_err());
    }

    #[test]
    fn build_input_rejects_empty_pr_number() {
        let input = WorkflowInput {
            tasks: None,
            parallelism: None,
            base: None,
            pr_number: Some("".to_string()),
        };
        assert!(build_workflow_input(&input).is_err());
    }
}
