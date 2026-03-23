use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::OrbitError;
use serde_json::{Value, json};

use super::input::required_input_string;
use crate::context::{RuntimeHost, TaskHost};

const TIMEOUT_MS: u64 = 15_000;

pub(super) fn load_pr_comments<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let task = host.get_task(task_id)?;
    let pr_number = task.pr_number.as_deref().ok_or_else(|| {
        OrbitError::InvalidInput("load_pr_comments requires task.pr_number".to_string())
    })?;

    let repo_root = task
        .repo_root
        .as_deref()
        .or(task.workspace_path.as_deref())
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "load_pr_comments requires task.repo_root or task.workspace_path".to_string(),
            )
        })?;

    let comments = fetch_pr_review_comments(repo_root, pr_number)?;

    // Filter to unresolved comments (not part of a resolved thread).
    // GitHub's REST API doesn't directly flag "resolved" on individual comments,
    // but we can use the review threads endpoint to check.
    let unresolved = filter_unresolved_comments(repo_root, pr_number, &comments)?;

    if unresolved.is_empty() {
        return Ok(json!({
            "loop_exit": true,
            "comments": [],
            "comment_summary": "No unresolved comments.",
        }));
    }

    let summary = build_comment_summary(&unresolved);
    Ok(json!({
        "loop_exit": false,
        "comments": unresolved,
        "comment_summary": summary,
    }))
}

fn fetch_pr_review_comments(repo_root: &str, pr_number: &str) -> Result<Vec<Value>, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "api".to_string(),
                format!("repos/{{owner}}/{{repo}}/pulls/{pr_number}/comments"),
                "--paginate".to_string(),
            ],
            current_dir: Some(repo_root.to_string()),
            timeout_ms: Some(TIMEOUT_MS),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "failed to fetch PR comments for '{}': {}",
            pr_number,
            result.stderr.trim()
        )));
    }

    let comments: Vec<Value> = serde_json::from_str(result.stdout.trim()).unwrap_or_default();
    Ok(comments)
}

fn filter_unresolved_comments(
    repo_root: &str,
    pr_number: &str,
    comments: &[Value],
) -> Result<Vec<Value>, OrbitError> {
    // Fetch review threads to determine which are resolved.
    // Each thread has `isResolved` and a `comments` array whose entries
    // carry a `databaseId` that matches the REST API comment `id`.
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "pr".to_string(),
                "view".to_string(),
                pr_number.to_string(),
                "--json".to_string(),
                "reviewThreads".to_string(),
            ],
            current_dir: Some(repo_root.to_string()),
            timeout_ms: Some(TIMEOUT_MS),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        // If we can't fetch threads, return all comments as unresolved
        // (conservative approach).
        return Ok(comments.to_vec());
    }

    let payload: Value = serde_json::from_str(result.stdout.trim()).unwrap_or_default();

    // Collect databaseIds of all comments that belong to resolved threads.
    let resolved_comment_ids: std::collections::HashSet<u64> = payload
        .get("reviewThreads")
        .and_then(Value::as_array)
        .map(|threads| {
            threads
                .iter()
                .filter(|t| {
                    t.get("isResolved")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
                .flat_map(|t| {
                    t.get("comments")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|c| c.get("databaseId").and_then(Value::as_u64))
                })
                .collect()
        })
        .unwrap_or_default();

    if resolved_comment_ids.is_empty() {
        return Ok(comments.to_vec());
    }

    // Keep only comments whose REST API `id` is NOT in a resolved thread.
    let unresolved: Vec<Value> = comments
        .iter()
        .filter(|c| {
            let id = c.get("id").and_then(Value::as_u64).unwrap_or(0);
            !resolved_comment_ids.contains(&id)
        })
        .cloned()
        .collect();

    Ok(unresolved)
}

fn build_comment_summary(comments: &[Value]) -> String {
    let mut summary = format!("{} unresolved comment(s):\n", comments.len());
    for (i, comment) in comments.iter().enumerate() {
        let path = comment
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let line = comment
            .get("line")
            .or_else(|| comment.get("original_line"))
            .and_then(Value::as_u64)
            .map(|n| n.to_string())
            .unwrap_or_else(|| "?".to_string());
        let body = comment
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(200)
            .collect::<String>();
        let user = comment
            .get("user")
            .and_then(|u| u.get("login"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        summary.push_str(&format!(
            "\n{}. {} ({}:{}) — {}\n",
            i + 1,
            user,
            path,
            line,
            body
        ));
    }
    summary
}
