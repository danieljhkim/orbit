use serde_json::json;

use crate::protocol::ToolDescriptor;
use crate::schema::{
    any_schema, array_of_string_schema, bool_schema, int_schema, obj, schema_with_identity,
    str_schema,
};

pub fn mcp_tools() -> Vec<ToolDescriptor> {
    vec![
        ToolDescriptor {
            name: "orbit.agent.run".to_string(),
            description: "Run agent workflow for a task".to_string(),
            input_schema: schema_with_identity(
                obj(&[
                    ("task_id", str_schema()),
                    ("approve_on_verbal", bool_schema()),
                    ("approved_by", str_schema()),
                    ("approval_note", str_schema()),
                    ("run_identity_id", str_schema()),
                ]),
                &["task_id"],
            ),
        },
        ToolDescriptor {
            name: "orbit.config.show".to_string(),
            description: "Show effective Orbit configuration".to_string(),
            input_schema: schema_with_identity(obj(&[]), &[]),
        },
        ToolDescriptor {
            name: "orbit.job.add".to_string(),
            description: "Create a scheduled job".to_string(),
            input_schema: schema_with_identity(
                obj(&[
                    ("target_id", str_schema()),
                    ("schedule", str_schema()),
                    ("agent_cli", str_schema()),
                    ("timeout", str_schema()),
                    ("retry_max_attempts", int_schema()),
                    ("retry_backoff", str_schema()),
                    ("retry_initial_delay", str_schema()),
                ]),
                &["target_id", "schedule", "agent_cli"],
            ),
        },
        ToolDescriptor {
            name: "orbit.job.delete".to_string(),
            description: "Disable an existing job".to_string(),
            input_schema: schema_with_identity(obj(&[("job_id", str_schema())]), &["job_id"]),
        },
        ToolDescriptor {
            name: "orbit.job.history".to_string(),
            description: "List run history for a job".to_string(),
            input_schema: schema_with_identity(obj(&[("job_id", str_schema())]), &["job_id"]),
        },
        ToolDescriptor {
            name: "orbit.job.list".to_string(),
            description: "List jobs".to_string(),
            input_schema: schema_with_identity(obj(&[("include_disabled", bool_schema())]), &[]),
        },
        ToolDescriptor {
            name: "orbit.job.pause".to_string(),
            description: "Pause a job schedule".to_string(),
            input_schema: schema_with_identity(obj(&[("job_id", str_schema())]), &["job_id"]),
        },
        ToolDescriptor {
            name: "orbit.job.resume".to_string(),
            description: "Resume a paused job".to_string(),
            input_schema: schema_with_identity(obj(&[("job_id", str_schema())]), &["job_id"]),
        },
        ToolDescriptor {
            name: "orbit.job.run".to_string(),
            description: "Run a job immediately".to_string(),
            input_schema: schema_with_identity(obj(&[("job_id", str_schema())]), &["job_id"]),
        },
        ToolDescriptor {
            name: "orbit.job.show".to_string(),
            description: "Show job details".to_string(),
            input_schema: schema_with_identity(obj(&[("job_id", str_schema())]), &["job_id"]),
        },
        ToolDescriptor {
            name: "orbit.skill.doctor".to_string(),
            description: "Validate file-backed skills".to_string(),
            input_schema: schema_with_identity(obj(&[]), &[]),
        },
        ToolDescriptor {
            name: "orbit.skill.list".to_string(),
            description: "List available skills".to_string(),
            input_schema: schema_with_identity(obj(&[]), &[]),
        },
        ToolDescriptor {
            name: "orbit.skill.show".to_string(),
            description: "Show a skill by id".to_string(),
            input_schema: schema_with_identity(obj(&[("skill_id", str_schema())]), &["skill_id"]),
        },
        ToolDescriptor {
            name: "orbit.task.add".to_string(),
            description: "Create a task".to_string(),
            input_schema: schema_with_identity(
                obj(&[
                    ("title", str_schema()),
                    ("description", str_schema()),
                    ("instructions", str_schema()),
                    ("context_files", array_of_string_schema()),
                    ("workspace_path", str_schema()),
                    ("task_identity_id", str_schema()),
                    ("assigned_to", str_schema()),
                    ("created_by", str_schema()),
                    ("priority", str_schema()),
                    ("task_type", str_schema()),
                    ("owner", str_schema()),
                    ("parent_id", str_schema()),
                ]),
                &["title"],
            ),
        },
        ToolDescriptor {
            name: "orbit.task.approve".to_string(),
            description: "Approve a task for agent execution".to_string(),
            input_schema: schema_with_identity(
                obj(&[
                    ("task_id", str_schema()),
                    ("approved_by", str_schema()),
                    ("note", str_schema()),
                ]),
                &["task_id"],
            ),
        },
        ToolDescriptor {
            name: "orbit.task.close".to_string(),
            description: "Close a task".to_string(),
            input_schema: schema_with_identity(obj(&[("task_id", str_schema())]), &["task_id"]),
        },
        ToolDescriptor {
            name: "orbit.task.delete".to_string(),
            description: "Delete a task".to_string(),
            input_schema: schema_with_identity(obj(&[("task_id", str_schema())]), &["task_id"]),
        },
        ToolDescriptor {
            name: "orbit.task.list".to_string(),
            description: "List tasks with optional filters".to_string(),
            input_schema: schema_with_identity(
                obj(&[("status", str_schema()), ("priority", str_schema())]),
                &[],
            ),
        },
        ToolDescriptor {
            name: "orbit.task.reopen".to_string(),
            description: "Reopen a task".to_string(),
            input_schema: schema_with_identity(obj(&[("task_id", str_schema())]), &["task_id"]),
        },
        ToolDescriptor {
            name: "orbit.task.search".to_string(),
            description: "Search tasks".to_string(),
            input_schema: schema_with_identity(obj(&[("query", str_schema())]), &["query"]),
        },
        ToolDescriptor {
            name: "orbit.task.show".to_string(),
            description: "Show task details".to_string(),
            input_schema: schema_with_identity(obj(&[("task_id", str_schema())]), &["task_id"]),
        },
        ToolDescriptor {
            name: "orbit.task.update".to_string(),
            description: "Update a task".to_string(),
            input_schema: schema_with_identity(
                obj(&[
                    ("task_id", str_schema()),
                    ("title", str_schema()),
                    ("description", str_schema()),
                    ("instructions", str_schema()),
                    ("context_files", array_of_string_schema()),
                    ("workspace_path", str_schema()),
                    ("clear_workspace_path", bool_schema()),
                    ("task_identity_id", str_schema()),
                    ("clear_task_identity_id", bool_schema()),
                    ("assigned_to", str_schema()),
                    ("clear_assigned_to", bool_schema()),
                    ("created_by", str_schema()),
                    ("clear_created_by", bool_schema()),
                    ("status", str_schema()),
                    ("priority", str_schema()),
                    ("task_type", str_schema()),
                    ("owner", str_schema()),
                    ("parent_id", str_schema()),
                    ("clear_parent_id", bool_schema()),
                ]),
                &["task_id"],
            ),
        },
        ToolDescriptor {
            name: "orbit.tool.add".to_string(),
            description: "Register an external tool".to_string(),
            input_schema: schema_with_identity(
                obj(&[
                    ("name", str_schema()),
                    ("path", str_schema()),
                    ("description", str_schema()),
                ]),
                &["path"],
            ),
        },
        ToolDescriptor {
            name: "orbit.tool.disable".to_string(),
            description: "Disable a tool".to_string(),
            input_schema: schema_with_identity(obj(&[("tool_name", str_schema())]), &["tool_name"]),
        },
        ToolDescriptor {
            name: "orbit.tool.doctor".to_string(),
            description: "Validate tools".to_string(),
            input_schema: schema_with_identity(obj(&[]), &[]),
        },
        ToolDescriptor {
            name: "orbit.tool.enable".to_string(),
            description: "Enable a tool".to_string(),
            input_schema: schema_with_identity(obj(&[("tool_name", str_schema())]), &["tool_name"]),
        },
        ToolDescriptor {
            name: "orbit.tool.list".to_string(),
            description: "List tools".to_string(),
            input_schema: schema_with_identity(obj(&[]), &[]),
        },
        ToolDescriptor {
            name: "orbit.tool.remove".to_string(),
            description: "Remove an external tool".to_string(),
            input_schema: schema_with_identity(obj(&[("tool_name", str_schema())]), &["tool_name"]),
        },
        ToolDescriptor {
            name: "orbit.tool.run".to_string(),
            description: "Execute a tool".to_string(),
            input_schema: schema_with_identity(
                obj(&[("tool_name", str_schema()), ("input", any_schema())]),
                &["tool_name"],
            ),
        },
        ToolDescriptor {
            name: "orbit.tool.show".to_string(),
            description: "Show tool details".to_string(),
            input_schema: schema_with_identity(obj(&[("tool_name", str_schema())]), &["tool_name"]),
        },
        ToolDescriptor {
            name: "orbit.work.add".to_string(),
            description: "Create a work specification".to_string(),
            input_schema: schema_with_identity(
                obj(&[
                    ("work_id", str_schema()),
                    ("work_type", str_schema()),
                    ("description", str_schema()),
                    ("input_schema_json", json!({ "type": "object" })),
                    ("output_schema_json", json!({ "type": "object" })),
                    ("artifact_path_template", str_schema()),
                    ("skill_refs", array_of_string_schema()),
                    ("work_identity_id", str_schema()),
                    ("assigned_to", str_schema()),
                    ("created_by", str_schema()),
                ]),
                &["work_id", "description"],
            ),
        },
        ToolDescriptor {
            name: "orbit.work.delete".to_string(),
            description: "Delete a work spec".to_string(),
            input_schema: schema_with_identity(obj(&[("work_id", str_schema())]), &["work_id"]),
        },
        ToolDescriptor {
            name: "orbit.work.list".to_string(),
            description: "List works".to_string(),
            input_schema: schema_with_identity(obj(&[("include_inactive", bool_schema())]), &[]),
        },
        ToolDescriptor {
            name: "orbit.work.show".to_string(),
            description: "Show work details".to_string(),
            input_schema: schema_with_identity(obj(&[("work_id", str_schema())]), &["work_id"]),
        },
    ]
}

pub fn find_tool(name: &str) -> Option<ToolDescriptor> {
    mcp_tools().into_iter().find(|tool| tool.name == name)
}

#[cfg(test)]
mod tests {
    use super::mcp_tools;

    #[test]
    fn registry_is_sorted_and_unique() {
        let tools = mcp_tools();
        let mut names = tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(
            names, sorted,
            "tool registry must be sorted for deterministic output"
        );

        names.dedup();
        assert_eq!(names.len(), tools.len(), "tool names must be unique");
    }
}
