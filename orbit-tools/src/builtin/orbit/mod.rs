pub mod activity_show;
pub mod task_list;
pub mod task_show;
pub mod task_update;

use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam};
use serde_json::Value;

use crate::{ToolContext, ToolRegistry};

const ORBIT_TIMEOUT_MS: u64 = 15_000;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(task_show::OrbitTaskShowTool);
    registry.register(task_list::OrbitTaskListTool);
    registry.register(task_update::OrbitTaskUpdateTool);
    registry.register(activity_show::OrbitActivityShowTool);
}

pub(super) fn orbit_exec_request(ctx: &ToolContext, args: Vec<String>) -> ExecRequest {
    ExecRequest {
        program: "orbit".to_string(),
        args,
        current_dir: ctx.cwd.clone(),
        timeout_ms: Some(ORBIT_TIMEOUT_MS),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
    }
}

pub(super) fn run_orbit_json_command(
    ctx: &ToolContext,
    args: Vec<String>,
    label: &str,
) -> Result<Value, OrbitError> {
    let req = orbit_exec_request(ctx, args);
    let result = run_process(&req, &NoSandbox)?;
    if !result.success {
        let stderr = result.stderr.trim();
        let detail = if stderr.is_empty() {
            "command returned non-zero exit status"
        } else {
            stderr
        };
        return Err(OrbitError::Execution(format!("{label} failed: {detail}")));
    }

    parse_json_output(label, &result.stdout)
}

pub(super) fn parse_json_output(label: &str, stdout: &str) -> Result<Value, OrbitError> {
    serde_json::from_str(stdout)
        .map_err(|e| OrbitError::Execution(format!("failed to parse {label} output: {e}")))
}

pub(super) fn required_string(
    input: &Value,
    keys: &[&str],
    canonical: &str,
) -> Result<String, OrbitError> {
    for key in keys {
        if let Some(value) = input.get(*key) {
            let raw = value
                .as_str()
                .ok_or_else(|| OrbitError::InvalidInput(format!("`{key}` must be a string")))?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "`{key}` must not be empty"
                )));
            }
            return Ok(trimmed.to_string());
        }
    }

    Err(OrbitError::InvalidInput(format!("missing `{canonical}`")))
}

pub(super) fn optional_string(input: &Value, key: &str) -> Result<Option<String>, OrbitError> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let raw = value
                .as_str()
                .ok_or_else(|| OrbitError::InvalidInput(format!("`{key}` must be a string")))?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "`{key}` must not be empty"
                )));
            }
            Ok(Some(trimmed.to_string()))
        }
    }
}

pub(super) fn orbit_id_params(kind: &str) -> Vec<ToolParam> {
    vec![ToolParam {
        name: "id".to_string(),
        description: format!("{kind} ID"),
        param_type: "string".to_string(),
        required: true,
    }]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{ToolContext, ToolRegistry};

    #[test]
    fn orbit_tools_are_registered() {
        let mut registry = ToolRegistry::new();
        registry.register_builtins();
        let names: Vec<_> = registry.schemas().into_iter().map(|s| s.name).collect();
        for expected in &[
            "orbit.task.show",
            "orbit.task.list",
            "orbit.task.update",
            "orbit.activity.show",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing tool: {expected}"
            );
        }
    }

    #[test]
    fn orbit_exec_request_uses_tool_context_cwd() {
        let req = super::orbit_exec_request(
            &ToolContext {
                cwd: Some("/tmp/orbit-tools".to_string()),
            },
            vec!["task".to_string(), "show".to_string(), "T1".to_string()],
        );

        assert_eq!(req.program, "orbit");
        assert_eq!(req.current_dir.as_deref(), Some("/tmp/orbit-tools"));
    }

    #[test]
    fn task_show_builds_request_from_id() {
        let req = super::task_show::build_exec_request(
            &ToolContext::default(),
            &json!({"id": "T20260315-025432"}),
        )
        .expect("id should be accepted");

        assert_eq!(
            req.args,
            vec![
                "task".to_string(),
                "show".to_string(),
                "T20260315-025432".to_string(),
                "--json".to_string(),
            ]
        );
    }

    #[test]
    fn task_show_rejects_missing_id() {
        let err = super::task_show::build_exec_request(&ToolContext::default(), &json!({}))
            .expect_err("missing id must fail");
        assert!(err.to_string().contains("missing `id`"), "{err}");
    }

    #[test]
    fn task_list_builds_status_filter_when_present() {
        let req = super::task_list::build_exec_request(
            &ToolContext::default(),
            &json!({"status": "backlog"}),
        )
        .expect("valid list input");

        assert_eq!(
            req.args,
            vec![
                "task".to_string(),
                "list".to_string(),
                "--json".to_string(),
                "--status".to_string(),
                "backlog".to_string(),
            ]
        );
    }

    #[test]
    fn task_update_builds_update_and_show_requests() {
        let (update, show) = super::task_update::build_exec_requests(
            &ToolContext::default(),
            &json!({
                "id": "T20260315-025432",
                "status": "review",
                "comment": "ready for review",
            }),
        )
        .expect("valid update input");

        assert_eq!(
            update.args,
            vec![
                "task".to_string(),
                "update".to_string(),
                "T20260315-025432".to_string(),
                "--status".to_string(),
                "review".to_string(),
                "--comment".to_string(),
                "ready for review".to_string(),
            ]
        );
        assert_eq!(
            show.args,
            vec![
                "task".to_string(),
                "show".to_string(),
                "T20260315-025432".to_string(),
                "--json".to_string(),
            ]
        );
    }

    #[test]
    fn task_update_requires_at_least_one_field() {
        let err = super::task_update::build_exec_requests(
            &ToolContext::default(),
            &json!({"id": "T20260315-025432"}),
        )
        .expect_err("missing fields must fail");
        assert!(err.to_string().contains("at least one"), "{err}");
    }

    #[test]
    fn activity_show_builds_request_from_id() {
        let req = super::activity_show::build_exec_request(
            &ToolContext::default(),
            &json!({"id": "open_pr"}),
        )
        .expect("id should be accepted");

        assert_eq!(
            req.args,
            vec![
                "activity".to_string(),
                "show".to_string(),
                "open_pr".to_string(),
                "--json".to_string(),
            ]
        );
    }
}
