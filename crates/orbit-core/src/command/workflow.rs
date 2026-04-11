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
        alias: "review",
        job_id: "job_review_tasks",
        description: "Review tasks in proposed/review status",
        supports_tasks: false,
        supports_parallelism: false,
        supports_base: false,
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
    fn duel_workflow_is_registered_with_single_task_cap() {
        let duel = find_workflow("duel").expect("duel workflow");
        assert_eq!(duel.job_id, "job_duel_pipeline");
        assert!(duel.supports_tasks);
        assert!(!duel.supports_parallelism);
        assert!(duel.supports_base);
        assert!(!duel.supports_pr_number);
        assert_eq!(duel.max_tasks, Some(1));
        assert!(
            duel.description.to_lowercase().contains("duel")
                || duel.description.to_lowercase().contains("cross-agent"),
            "description should mention cross-agent evaluation"
        );
    }

    #[test]
    fn duel_rejects_multi_task_input_with_workflow_specific_message() {
        let duel = find_workflow("duel").expect("duel workflow");
        let input = WorkflowInput {
            tasks: Some("T20260409-0310,T20260409-0311".to_string()),
            parallelism: None,
            base: None,
            pr_number: None,
        };
        let err = build_workflow_input_for(Some(duel), &input).unwrap_err();
        match err {
            OrbitError::InvalidInput(msg) => {
                assert!(
                    msg.contains("duel") && msg.contains("exactly one task id"),
                    "error must name the workflow and its constraint, got: {msg}"
                );
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn duel_accepts_a_single_task_id() {
        let duel = find_workflow("duel").expect("duel workflow");
        let input = WorkflowInput {
            tasks: Some("T20260409-0310".to_string()),
            parallelism: None,
            base: Some("main".to_string()),
            pr_number: None,
        };
        let built = build_workflow_input_for(Some(duel), &input).unwrap();
        assert_eq!(built["task_ids"], json!(["T20260409-0310"]));
        assert_eq!(built["task_id"], json!("T20260409-0310"));
        assert_eq!(built["base"], json!("main"));
    }

    #[test]
    fn duel_single_task_input_includes_task_id_and_task_ids() {
        let duel = find_workflow("duel").expect("duel workflow");
        let input = WorkflowInput {
            tasks: Some("T20260409-0310".to_string()),
            parallelism: None,
            base: None,
            pr_number: None,
        };
        let built = build_workflow_input_for(Some(duel), &input).unwrap();
        assert_eq!(
            built,
            json!({
                "task_id": "T20260409-0310",
                "task_ids": ["T20260409-0310"]
            })
        );
    }

    #[test]
    fn non_duel_workflows_are_unbounded_by_max_tasks() {
        let ship = find_workflow("ship").expect("ship workflow");
        assert_eq!(ship.max_tasks, None);
        let input = WorkflowInput {
            tasks: Some("T1,T2,T3,T4,T5".to_string()),
            parallelism: None,
            base: None,
            pr_number: None,
        };
        let built = build_workflow_input_for(Some(ship), &input).unwrap();
        assert_eq!(built["task_ids"].as_array().unwrap().len(), 5);
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
