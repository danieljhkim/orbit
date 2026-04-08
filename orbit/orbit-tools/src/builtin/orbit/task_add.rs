use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitTaskAddTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let title = super::required_string(input, &["title"], "title")?;
    let description = super::required_string(input, &["description"], "description")?;
    let workspace = super::required_string(input, &["workspace"], "workspace")?;
    let plan = match input.get("plan") {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Null) | None => None,
        Some(_) => {
            return Err(OrbitError::InvalidInput(
                "`plan` must be a string".to_string(),
            ));
        }
    };

    let mut args = vec![
        "task".to_string(),
        "add".to_string(),
        "--title".to_string(),
        title,
        "--description".to_string(),
        description,
        "--workspace".to_string(),
        workspace,
    ];
    if let Some(plan) = plan {
        args.push("--plan".to_string());
        args.push(plan);
    }
    if let Some(criteria) = super::optional_string_list_alias(
        input,
        &[
            "acceptance_criteria",
            "acceptanceCriteria",
            "acceptance-criteria",
        ],
    )? {
        for criterion in criteria {
            args.push("--acceptance-criteria".to_string());
            args.push(criterion);
        }
    }

    if let Some(comment) = super::optional_string(input, "comment")? {
        args.push("--comment".to_string());
        args.push(comment);
    }
    if let Some(context) = super::optional_string(input, "context")? {
        args.push("--context".to_string());
        args.push(context);
    }
    if let Some(priority) = super::optional_string(input, "priority")? {
        args.push("--priority".to_string());
        args.push(priority);
    }
    if let Some(complexity) = super::optional_string(input, "complexity")? {
        args.push("--complexity".to_string());
        args.push(complexity);
    }
    if let Some(task_type) =
        super::optional_string_alias(input, &["type", "task_type", "taskType"])?
    {
        args.push("--type".to_string());
        args.push(task_type);
    }
    if let Some(source_task) =
        super::optional_string_alias(input, &["source_task_id", "source_task", "sourceTaskId"])?
    {
        args.push("--source-task".to_string());
        args.push(source_task);
    }
    if let Some(parent_id) =
        super::optional_string_alias(input, &["parent_id", "parent", "parentId"])?
    {
        args.push("--parent".to_string());
        args.push(parent_id);
    }
    super::append_identity_flags(&mut args, &identity);

    args.push("--json".to_string());
    Ok(super::orbit_exec_request_with_identity(
        ctx, args, &identity,
    ))
}

impl Tool for OrbitTaskAddTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "title".to_string(),
                description: "Task title".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "description".to_string(),
                description: "Task description markdown".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "acceptance_criteria".to_string(),
                description: "Optional acceptance criteria as a string or array of strings"
                    .to_string(),
                param_type: "array".to_string(),
                required: false,
            },
            ToolParam {
                name: "plan".to_string(),
                description:
                    "Optional task plan markdown. Leave blank for the executing agent to author."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "workspace".to_string(),
                description: "Workspace path for the task".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "comment".to_string(),
                description: "Optional initial task comment".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "context".to_string(),
                description: "Optional comma-separated context file paths".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "priority".to_string(),
                description: "Optional priority level".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "complexity".to_string(),
                description: "Optional task complexity level".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "type".to_string(),
                description: "Optional task type".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "source_task_id".to_string(),
                description: "For bug tasks: originating task ID that introduced the defect"
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "parent_id".to_string(),
                description: "Optional parent task ID for a subtask relationship".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.task.add".to_string(),
            description: "Create an Orbit task and return the created task JSON".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        super::run_orbit_json_command(req, "orbit task add")
    }
}
