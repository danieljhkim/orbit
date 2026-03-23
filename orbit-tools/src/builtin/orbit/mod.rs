pub mod activity_show;
pub mod job_run_archive;
pub mod job_run_list;
pub mod job_run_show;
pub mod task_add;
pub mod task_approve;
pub mod task_list;
pub mod task_reject;
pub mod task_show;
pub mod task_start;
pub mod task_update;

use std::collections::HashMap;

use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam};
use serde_json::Value;

use crate::{TIMEOUT_DEFAULT_MS, ToolContext, ToolRegistry};

const ORBIT_TASK_ACTOR_KIND: &str = "ORBIT_TASK_ACTOR_KIND";
const ORBIT_TASK_ACTOR_LABEL: &str = "ORBIT_TASK_ACTOR_LABEL";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct OrbitIdentity {
    pub agent: Option<String>,
    pub model: Option<String>,
    pub actor_label: Option<String>,
}

pub fn register(registry: &mut ToolRegistry) {
    registry.register(task_add::OrbitTaskAddTool);
    registry.register(task_approve::OrbitTaskApproveTool);
    registry.register(task_start::OrbitTaskStartTool);
    registry.register(task_reject::OrbitTaskRejectTool);
    registry.register(task_show::OrbitTaskShowTool);
    registry.register(task_list::OrbitTaskListTool);
    registry.register(task_update::OrbitTaskUpdateTool);
    registry.register(job_run_list::OrbitJobRunListTool);
    registry.register(job_run_show::OrbitJobRunShowTool);
    registry.register(job_run_archive::OrbitJobRunArchiveTool);
    registry.register(activity_show::OrbitActivityShowTool);
}

fn build_actor_label(agent: Option<&str>, model: Option<&str>) -> Option<String> {
    match (agent, model) {
        (Some(agent), Some(model)) => Some(format!("{agent} / {model}")),
        (Some(agent), None) => Some(agent.to_string()),
        (None, Some(model)) => Some(model.to_string()),
        (None, None) => None,
    }
}

pub(super) fn resolve_identity(
    ctx: &ToolContext,
    input: &Value,
) -> Result<OrbitIdentity, OrbitError> {
    let agent = optional_string_alias(input, &["agent"])?.or_else(|| {
        ctx.agent_name
            .clone()
            .filter(|value| !value.trim().is_empty())
    });
    let model = optional_string_alias(input, &["model"])?.or_else(|| {
        ctx.model_name
            .clone()
            .filter(|value| !value.trim().is_empty())
    });
    let actor_label = build_actor_label(agent.as_deref(), model.as_deref());
    Ok(OrbitIdentity {
        agent,
        model,
        actor_label,
    })
}

pub(super) fn identity_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "agent".to_string(),
            description: "Optional agent name for precise Orbit provenance".to_string(),
            param_type: "string".to_string(),
            required: false,
        },
        ToolParam {
            name: "model".to_string(),
            description: "Optional agent model for precise Orbit provenance".to_string(),
            param_type: "string".to_string(),
            required: false,
        },
    ]
}

pub(super) fn append_identity_flags(args: &mut Vec<String>, identity: &OrbitIdentity) {
    if let Some(agent) = &identity.agent {
        args.push("--agent".to_string());
        args.push(agent.clone());
    }
    if let Some(model) = &identity.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }
}

/// Build an [`ExecRequest`] that runs the `orbit` CLI with `args`.
///
/// The environment is deliberately rebuilt from the current process's env vars
/// rather than passed through wholesale, then `ORBIT_TASK_ACTOR_KIND=agent` is
/// injected. This lets the orbit CLI distinguish agent-initiated mutations from
/// human-initiated ones (e.g. for audit attribution and policy checks) without
/// requiring callers to set the variable themselves.
#[cfg(test)]
pub(super) fn orbit_exec_request(ctx: &ToolContext, args: Vec<String>) -> ExecRequest {
    orbit_exec_request_with_identity(ctx, args, &OrbitIdentity::default())
}

pub(super) fn orbit_exec_request_with_identity(
    ctx: &ToolContext,
    args: Vec<String>,
    identity: &OrbitIdentity,
) -> ExecRequest {
    let mut env = std::env::vars().collect::<HashMap<_, _>>();
    env.insert(ORBIT_TASK_ACTOR_KIND.to_string(), "agent".to_string());
    if let Some(actor_label) = &identity.actor_label {
        env.insert(ORBIT_TASK_ACTOR_LABEL.to_string(), actor_label.clone());
    }

    // Inject --root so the spawned orbit CLI resolves to the correct data root
    // regardless of the agent's working directory (e.g. inside a git worktree).
    let args = if let Some(root) = &ctx.orbit_root {
        let mut full = vec!["--root".to_string(), root.to_string_lossy().into_owned()];
        full.extend(args);
        full
    } else {
        args
    };

    ExecRequest {
        program: "orbit".to_string(),
        args,
        current_dir: ctx.cwd.clone(),
        timeout_ms: Some(TIMEOUT_DEFAULT_MS),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::ClearAndSet(env.into_iter().collect()),
        debug: false,
    }
}

pub(super) fn run_orbit_json_command(req: ExecRequest, label: &str) -> Result<Value, OrbitError> {
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

/// Extract an optional string from the first matching key in `keys`.
///
/// Tools accept multiple key names for the same logical field to stay
/// friendly to agents that may use slightly different naming conventions
/// (e.g. `"type"`, `"task_type"`, `"taskType"` all map to the task type
/// parameter). The first non-absent key wins; absence of all keys returns
/// `None`. An explicitly empty value is rejected as an error.
pub(super) fn optional_string_alias(
    input: &Value,
    keys: &[&str],
) -> Result<Option<String>, OrbitError> {
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
            return Ok(Some(trimmed.to_string()));
        }
    }

    Ok(None)
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
            "orbit.task.add",
            "orbit.task.approve",
            "orbit.task.start",
            "orbit.task.reject",
            "orbit.task.show",
            "orbit.task.list",
            "orbit.task.update",
            "orbit.job_run.list",
            "orbit.job_run.show",
            "orbit.job_run.archive",
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
                allowed_tools: vec![],
                ..Default::default()
            },
            vec!["task".to_string(), "show".to_string(), "T1".to_string()],
        );

        assert_eq!(req.program, "orbit");
        assert_eq!(req.current_dir.as_deref(), Some("/tmp/orbit-tools"));
    }

    #[test]
    fn orbit_exec_request_injects_root_when_set() {
        let req = super::orbit_exec_request(
            &ToolContext {
                orbit_root: Some(std::path::PathBuf::from("/repo/.orbit")),
                ..Default::default()
            },
            vec!["task".to_string(), "list".to_string(), "--json".to_string()],
        );

        assert_eq!(
            req.args,
            vec![
                "--root".to_string(),
                "/repo/.orbit".to_string(),
                "task".to_string(),
                "list".to_string(),
                "--json".to_string(),
            ]
        );
    }

    #[test]
    fn orbit_exec_request_omits_root_when_none() {
        let req = super::orbit_exec_request(
            &ToolContext::default(),
            vec!["task".to_string(), "list".to_string()],
        );

        assert_eq!(req.args, vec!["task".to_string(), "list".to_string()]);
    }

    #[test]
    fn orbit_exec_request_with_identity_sets_actor_label_env() {
        let req = super::orbit_exec_request_with_identity(
            &ToolContext::default(),
            vec!["task".to_string(), "list".to_string()],
            &super::OrbitIdentity {
                agent: Some("codex".to_string()),
                model: Some("gpt-5.4".to_string()),
                actor_label: Some("codex / gpt-5.4".to_string()),
            },
        );

        let orbit_exec::EnvironmentMode::ClearAndSet(env) = req.environment_mode else {
            panic!("expected clear-and-set env");
        };
        let env_map = env.into_iter().collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            env_map.get("ORBIT_TASK_ACTOR_LABEL").map(String::as_str),
            Some("codex / gpt-5.4")
        );
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
    fn task_start_builds_request_with_optional_fields() {
        let req = super::task_start::build_exec_request(
            &ToolContext::default(),
            &json!({
                "id": "T20260316-010101",
                "note": "approved and ready",
                "comment": "starting implementation",
                "agent": "codex",
                "model": "gpt-5.4",
            }),
        )
        .expect("valid start input");

        assert_eq!(
            req.args,
            vec![
                "task".to_string(),
                "start".to_string(),
                "T20260316-010101".to_string(),
                "--note".to_string(),
                "approved and ready".to_string(),
                "--comment".to_string(),
                "starting implementation".to_string(),
                "--agent".to_string(),
                "codex".to_string(),
                "--model".to_string(),
                "gpt-5.4".to_string(),
                "--json".to_string(),
            ]
        );
    }

    #[test]
    fn task_add_builds_request_with_optional_fields() {
        let req = super::task_add::build_exec_request(
            &ToolContext::default(),
            &json!({
                "title": "Add a tool",
                "description": "Details",
                "plan": "Plan",
                "workspace": "/tmp/orbit",
                "parent_id": "T20260316-000001",
                "comment": "seed comment",
                "context": "a.rs,b.rs",
                "priority": "high",
                "complexity": "hard",
                "type": "feature",
                "agent": "codex",
                "model": "gpt-5.4",
            }),
        )
        .expect("valid add input");

        assert_eq!(
            req.args,
            vec![
                "task".to_string(),
                "add".to_string(),
                "--title".to_string(),
                "Add a tool".to_string(),
                "--description".to_string(),
                "Details".to_string(),
                "--plan".to_string(),
                "Plan".to_string(),
                "--workspace".to_string(),
                "/tmp/orbit".to_string(),
                "--comment".to_string(),
                "seed comment".to_string(),
                "--context".to_string(),
                "a.rs,b.rs".to_string(),
                "--priority".to_string(),
                "high".to_string(),
                "--complexity".to_string(),
                "hard".to_string(),
                "--type".to_string(),
                "feature".to_string(),
                "--parent".to_string(),
                "T20260316-000001".to_string(),
                "--agent".to_string(),
                "codex".to_string(),
                "--model".to_string(),
                "gpt-5.4".to_string(),
                "--json".to_string(),
            ]
        );
    }

    #[test]
    fn task_approve_builds_request_with_optional_comment() {
        let req = super::task_approve::build_exec_request(
            &ToolContext::default(),
            &json!({
                "id": "T20260315-205817",
                "note": "lgtm",
                "comment": "ship it",
                "agent": "codex",
                "model": "gpt-5.4",
            }),
        )
        .expect("valid approve input");

        assert_eq!(
            req.args,
            vec![
                "task".to_string(),
                "approve".to_string(),
                "T20260315-205817".to_string(),
                "--note".to_string(),
                "lgtm".to_string(),
                "--comment".to_string(),
                "ship it".to_string(),
                "--agent".to_string(),
                "codex".to_string(),
                "--model".to_string(),
                "gpt-5.4".to_string(),
                "--json".to_string(),
            ]
        );
    }

    #[test]
    fn task_reject_builds_request_with_optional_comment() {
        let req = super::task_reject::build_exec_request(
            &ToolContext::default(),
            &json!({
                "id": "T20260315-205817",
                "note": "needs work",
                "comment": "please revise",
                "agent": "codex",
                "model": "gpt-5.4",
            }),
        )
        .expect("valid reject input");

        assert_eq!(
            req.args,
            vec![
                "task".to_string(),
                "reject".to_string(),
                "T20260315-205817".to_string(),
                "--note".to_string(),
                "needs work".to_string(),
                "--comment".to_string(),
                "please revise".to_string(),
                "--agent".to_string(),
                "codex".to_string(),
                "--model".to_string(),
                "gpt-5.4".to_string(),
                "--json".to_string(),
            ]
        );
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
    fn task_list_builds_parent_filter_when_present() {
        let req = super::task_list::build_exec_request(
            &ToolContext::default(),
            &json!({"parent_id": "T20260316-000001"}),
        )
        .expect("valid list input");

        assert_eq!(
            req.args,
            vec![
                "task".to_string(),
                "list".to_string(),
                "--json".to_string(),
                "--parent".to_string(),
                "T20260316-000001".to_string(),
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
                "agent": "codex",
                "model": "gpt-5.4",
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
                "--agent".to_string(),
                "codex".to_string(),
                "--model".to_string(),
                "gpt-5.4".to_string(),
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
    fn job_run_list_builds_all_supported_filters() {
        let req = super::job_run_list::build_exec_request(
            &ToolContext::default(),
            &json!({
                "job": "job_review_tasks",
                "status": "running",
                "since": "2026-03-15T00:00:00Z",
                "limit": "10",
            }),
        )
        .expect("valid list input");

        assert_eq!(
            req.args,
            vec![
                "job-run".to_string(),
                "list".to_string(),
                "--json".to_string(),
                "--job".to_string(),
                "job_review_tasks".to_string(),
                "--status".to_string(),
                "running".to_string(),
                "--since".to_string(),
                "2026-03-15T00:00:00Z".to_string(),
                "--limit".to_string(),
                "10".to_string(),
            ]
        );
    }

    #[test]
    fn job_run_show_builds_request_from_id() {
        let req = super::job_run_show::build_exec_request(
            &ToolContext::default(),
            &json!({"id": "jrun-123"}),
        )
        .expect("id should be accepted");

        assert_eq!(
            req.args,
            vec![
                "job-run".to_string(),
                "show".to_string(),
                "jrun-123".to_string(),
                "--json".to_string(),
            ]
        );
    }

    #[test]
    fn job_run_archive_builds_request_from_id() {
        let req = super::job_run_archive::build_exec_request(
            &ToolContext::default(),
            &json!({"id": "jrun-123"}),
        )
        .expect("id should be accepted");

        assert_eq!(
            req.args,
            vec![
                "job-run".to_string(),
                "archive".to_string(),
                "jrun-123".to_string(),
            ]
        );
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
