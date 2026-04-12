use orbit_exec::{ExecRequest, NoSandbox, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitTaskUpdateTool;

pub(super) fn build_exec_requests(
    ctx: &ToolContext,
    input: &Value,
) -> Result<(ExecRequest, ExecRequest), OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let id = super::required_string(input, &["id"], "id")?;
    let mut args = vec!["task".to_string(), "update".to_string(), id.clone()];
    let mut changed = false;

    if let Some(title) = super::optional_string(input, "title")? {
        args.push("--title".to_string());
        args.push(title);
        changed = true;
    }
    if let Some(description) = input.get("description") {
        let raw = description.as_str().ok_or_else(|| {
            OrbitError::InvalidInput("`description` must be a string".to_string())
        })?;
        args.push("--description".to_string());
        args.push(raw.to_string());
        changed = true;
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
        changed = true;
    }
    if let Some(status) = super::optional_string(input, "status")? {
        args.push("--status".to_string());
        args.push(status);
        changed = true;
    }
    if let Some(plan) = input.get("plan") {
        let raw = plan
            .as_str()
            .ok_or_else(|| OrbitError::InvalidInput("`plan` must be a string".to_string()))?;
        args.push("--plan".to_string());
        args.push(raw.to_string());
        changed = true;
    }
    if let Some(summary) = super::optional_string(input, "execution_summary")? {
        args.push("--execution-summary".to_string());
        args.push(summary);
        changed = true;
    }
    if let Some(comment) = super::optional_string(input, "comment")? {
        args.push("--comment".to_string());
        args.push(comment);
        changed = true;
    }
    if let Some(pr_status) = super::optional_string(input, "pr_status")? {
        args.push("--pr-status".to_string());
        args.push(pr_status);
        changed = true;
    }
    if let Some(pr_number) = super::optional_string(input, "pr_number")? {
        args.push("--pr-number".to_string());
        args.push(pr_number);
        changed = true;
    }
    if let Some(batch_id) = super::optional_string(input, "batch_id")? {
        args.push("--batch-id".to_string());
        args.push(batch_id);
        changed = true;
    }
    if let Some(context_files) = super::optional_string_list_alias(input, &["context_files"])? {
        args.push("--context".to_string());
        args.push(context_files.join(","));
        changed = true;
    }
    if let Some(artifacts) = optional_artifacts(input)? {
        for (path, content) in artifacts {
            args.push("--artifact".to_string());
            args.push(format!("{path}={content}"));
        }
        changed = true;
    }

    if !changed {
        return Err(OrbitError::InvalidInput(
            "orbit.task.update requires at least one of `title`, `description`, `acceptance_criteria`, `status`, `plan`, `execution_summary`, `comment`, `pr_status`, `pr_number`, `batch_id`, `context_files`, or `artifacts`"
                .to_string(),
        ));
    }

    super::append_identity_flags(&mut args, &identity);

    let update = super::orbit_exec_request_with_identity(ctx, args, &identity);
    let show = super::orbit_exec_request_with_identity(
        ctx,
        vec![
            "task".to_string(),
            "show".to_string(),
            id,
            "--json".to_string(),
        ],
        &identity,
    );
    Ok((update, show))
}

impl Tool for OrbitTaskUpdateTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.extend([
            ToolParam {
                name: "title".to_string(),
                description: "New task title".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "description".to_string(),
                description: "New task description (empty string clears)".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "acceptance_criteria".to_string(),
                description: "New acceptance criteria as an array of strings or a single string"
                    .to_string(),
                param_type: "array".to_string(),
                required: false,
            },
            ToolParam {
                name: "plan".to_string(),
                description: "Replacement task plan text (empty string clears)".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "status".to_string(),
                description: "New task status".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "execution_summary".to_string(),
                description: "Replacement execution summary text".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "comment".to_string(),
                description: "Task comment to append".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "pr_status".to_string(),
                description: "PR review status (e.g. approve, request-changes)".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "pr_number".to_string(),
                description: "Pull request number (empty string clears)".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "batch_id".to_string(),
                description: "Batch ID to associate with the task (empty string clears)"
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "context_files".to_string(),
                description: "Context file paths as a comma-separated string or array of strings"
                    .to_string(),
                param_type: "array".to_string(),
                required: false,
            },
            ToolParam {
                name: "artifacts".to_string(),
                description:
                    "Task artifacts to write under `artifacts/`. Accepts either an object \
                    map of `path -> content` or an array of `{ path, content }` objects."
                        .to_string(),
                param_type: "object".to_string(),
                required: false,
            },
        ]);
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.task.update".to_string(),
            description: "Update an Orbit task and return the fresh task JSON".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let (update_req, show_req) = build_exec_requests(ctx, &input)?;

        let update_result = run_process(&update_req, &NoSandbox)?;
        if !update_result.success {
            let stderr = update_result.stderr.trim();
            let detail = if stderr.is_empty() {
                "command returned non-zero exit status"
            } else {
                stderr
            };
            return Err(OrbitError::Execution(format!(
                "orbit task update failed: {detail}"
            )));
        }

        super::run_orbit_json_command(show_req, "orbit task show")
    }
}

fn optional_artifacts(input: &Value) -> Result<Option<Vec<(String, String)>>, OrbitError> {
    let Some(value) = input.get("artifacts").or_else(|| input.get("artifact")) else {
        return Ok(None);
    };

    match value {
        Value::Null => Ok(None),
        Value::Object(map) => {
            let mut artifacts = Vec::with_capacity(map.len());
            for (path, content) in map {
                let path = path.trim();
                if path.is_empty() {
                    return Err(OrbitError::InvalidInput(
                        "`artifacts` keys must not be empty".to_string(),
                    ));
                }
                let content = content.as_str().ok_or_else(|| {
                    OrbitError::InvalidInput("`artifacts` values must be strings".to_string())
                })?;
                artifacts.push((path.to_string(), content.to_string()));
            }
            Ok(Some(artifacts))
        }
        Value::Array(items) => {
            let mut artifacts = Vec::with_capacity(items.len());
            for item in items {
                let path = item.get("path").and_then(Value::as_str).ok_or_else(|| {
                    OrbitError::InvalidInput(
                        "`artifacts` entries must include string `path` values".to_string(),
                    )
                })?;
                let content = item.get("content").and_then(Value::as_str).ok_or_else(|| {
                    OrbitError::InvalidInput(
                        "`artifacts` entries must include string `content` values".to_string(),
                    )
                })?;
                let path = path.trim();
                if path.is_empty() {
                    return Err(OrbitError::InvalidInput(
                        "`artifacts` entry paths must not be empty".to_string(),
                    ));
                }
                artifacts.push((path.to_string(), content.to_string()));
            }
            Ok(Some(artifacts))
        }
        _ => Err(OrbitError::InvalidInput(
            "`artifacts` must be an object or array".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::path::PathBuf;

    use crate::ToolContext;

    use super::build_exec_requests;

    fn test_context() -> ToolContext {
        ToolContext {
            cwd: Some("/tmp/orbit".to_string()),
            orbit_root: Some(PathBuf::from("/tmp/orbit-root")),
            agent_name: Some("codex".to_string()),
            model_name: Some("gpt-5.4".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn build_exec_requests_uses_context_flag_for_context_files() {
        let (update, show) = build_exec_requests(
            &test_context(),
            &json!({
                "id": "T20260330-002312",
                "context_files": "crates/orbit-cli/src/command/task.rs,crates/orbit-tools/src/builtin/orbit/task_update.rs"
            }),
        )
        .expect("request should build");

        assert_eq!(update.program, "orbit");
        assert!(
            update.args.contains(&"--context".to_string()),
            "expected `--context` in {:?}",
            update.args
        );
        assert!(
            !update.args.contains(&"--context-files".to_string()),
            "legacy flag should not be emitted: {:?}",
            update.args
        );
        assert_eq!(
            show.args,
            vec![
                "--root",
                "/tmp/orbit-root",
                "task",
                "show",
                "T20260330-002312",
                "--json",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn build_exec_requests_accepts_context_files_array() {
        let (update, _) = build_exec_requests(
            &test_context(),
            &json!({
                "id": "T20260330-002312",
                "context_files": [
                    "crates/orbit-cli/src/command/task.rs",
                    "crates/orbit-tools/src/builtin/orbit/task_update.rs"
                ]
            }),
        )
        .expect("request should build");

        let context_index = update
            .args
            .iter()
            .position(|arg| arg == "--context")
            .expect("expected `--context` in request");
        assert_eq!(
            update.args.get(context_index + 1).map(String::as_str),
            Some(
                "crates/orbit-cli/src/command/task.rs,crates/orbit-tools/src/builtin/orbit/task_update.rs"
            )
        );
    }

    #[test]
    fn build_exec_requests_error_mentions_context_files() {
        let err = build_exec_requests(&test_context(), &json!({ "id": "T20260330-002312" }))
            .expect_err("missing fields should fail");
        let message = err.to_string();

        assert!(message.contains("title"));
        assert!(message.contains("description"));
        assert!(message.contains("acceptance_criteria"));
        assert!(message.contains("context_files"));
        assert!(message.contains("artifacts"));
    }

    #[test]
    fn build_exec_requests_supports_metadata_fields() {
        let (update, _) = build_exec_requests(
            &test_context(),
            &json!({
                "id": "T20260330-002312",
                "title": "Updated title",
                "description": "",
                "acceptance_criteria": ["first", "second"]
            }),
        )
        .expect("request should build");

        assert!(update.args.contains(&"--title".to_string()));
        assert!(update.args.contains(&"Updated title".to_string()));
        let description_index = update
            .args
            .iter()
            .position(|arg| arg == "--description")
            .expect("expected `--description` in request");
        assert_eq!(
            update.args.get(description_index + 1).map(String::as_str),
            Some("")
        );

        let criteria_flags = update
            .args
            .iter()
            .filter(|arg| arg.as_str() == "--acceptance-criteria")
            .count();
        assert_eq!(criteria_flags, 2);
    }

    #[test]
    fn build_exec_requests_rejects_non_string_context_files_entries() {
        let err = build_exec_requests(
            &test_context(),
            &json!({
                "id": "T20260330-002312",
                "context_files": ["crates/orbit-cli/src/command/task.rs", 7]
            }),
        )
        .expect_err("non-string entries should fail");

        assert!(err.to_string().contains("entries must be strings"));
    }

    #[test]
    fn build_exec_requests_supports_artifact_map() {
        let (update, _) = build_exec_requests(
            &test_context(),
            &json!({
                "id": "T20260330-002312",
                "artifacts": {
                    "planning-duel/codex-gpt-5.4.md": "*authored by: codex / gpt-5.4*"
                }
            }),
        )
        .expect("request should build");

        let artifact_index = update
            .args
            .iter()
            .position(|arg| arg == "--artifact")
            .expect("expected `--artifact` in request");
        assert_eq!(
            update.args.get(artifact_index + 1).map(String::as_str),
            Some("planning-duel/codex-gpt-5.4.md=*authored by: codex / gpt-5.4*")
        );
    }

    #[test]
    fn build_exec_requests_supports_artifact_array() {
        let (update, _) = build_exec_requests(
            &test_context(),
            &json!({
                "id": "T20260330-002312",
                "artifacts": [
                    {
                        "path": "planning-duel/codex-gpt-5.4.md",
                        "content": "*authored by: codex / gpt-5.4*"
                    }
                ]
            }),
        )
        .expect("request should build");

        assert!(update.args.contains(&"--artifact".to_string()));
        assert!(update.args.contains(
            &"planning-duel/codex-gpt-5.4.md=*authored by: codex / gpt-5.4*".to_string()
        ));
    }
}
