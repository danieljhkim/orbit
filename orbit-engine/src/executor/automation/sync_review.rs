use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ReviewThread};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

const TIMEOUT_MS: u64 = 15_000;

pub(super) fn sync_batch_review_to_github<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = input
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "sync_batch_review_to_github requires input.run_id".to_string(),
            )
        })?;

    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id))?;
    let mut total: u64 = 0;

    for task in &batch_tasks {
        if task.pr_number.is_none() {
            continue;
        }
        if task.review_threads.is_empty() {
            continue;
        }
        total += sync_task_review_to_github(host, &task.id)?;
    }

    Ok(json!({ "synced_count": total }))
}

fn sync_task_review_to_github<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    task_id: &str,
) -> Result<u64, OrbitError> {
    let task = host.get_task(task_id)?;

    if task.pr_number.is_none() {
        return Ok(0);
    }

    if task.review_threads.is_empty() {
        return Ok(0);
    }

    let pr_number = task.pr_number.as_deref().unwrap();
    let repo_root = task
        .repo_root
        .as_deref()
        .or(task.workspace_path.as_deref())
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "sync_review_to_github requires task.repo_root or task.workspace_path".to_string(),
            )
        })?;

    let owner_repo = get_owner_repo(repo_root)?;
    let head_sha = get_pr_head_sha(repo_root, pr_number)?;

    let mut threads = task.review_threads.clone();
    let mut synced_count: u64 = 0;

    for thread in threads.iter_mut() {
        let thread_synced = sync_thread(repo_root, &owner_repo, pr_number, &head_sha, thread)?;
        synced_count += thread_synced;
    }

    if synced_count > 0 {
        host.apply_task_automation_update(
            task_id,
            TaskAutomationUpdate {
                review_threads: Some(threads),
                ..TaskAutomationUpdate::default()
            },
        )?;
    }

    Ok(synced_count)
}

fn sync_thread(
    repo_root: &str,
    owner_repo: &str,
    pr_number: &str,
    head_sha: &str,
    thread: &mut ReviewThread,
) -> Result<u64, OrbitError> {
    let mut synced: u64 = 0;

    if thread.github_thread_id.is_none() && !thread.messages.is_empty() {
        let first_msg = &thread.messages[0];

        let github_id = if let (Some(path), Some(line)) = (thread.path.as_deref(), thread.line) {
            // Inline review comment
            create_inline_review_comment(
                repo_root,
                owner_repo,
                pr_number,
                head_sha,
                path,
                line,
                &first_msg.body,
            )?
        } else {
            // General PR comment
            create_general_comment(repo_root, pr_number, &first_msg.body)?
        };

        thread.github_thread_id = Some(github_id);
        thread.messages[0].github_comment_id = Some(github_id);
        synced += 1;
    }

    // Sync reply messages on already-synced threads
    if let Some(parent_id) = thread.github_thread_id {
        for msg in thread.messages.iter_mut().skip(1) {
            if msg.github_comment_id.is_some() {
                continue;
            }
            let reply_id =
                create_reply_comment(repo_root, owner_repo, pr_number, parent_id, &msg.body)?;
            msg.github_comment_id = Some(reply_id);
            synced += 1;
        }
    }

    Ok(synced)
}

fn get_owner_repo(repo_root: &str) -> Result<String, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "repo".to_string(),
                "view".to_string(),
                "--json".to_string(),
                "nameWithOwner".to_string(),
                "-q".to_string(),
                ".nameWithOwner".to_string(),
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
            "failed to get repo owner/name: {}",
            result.stderr.trim()
        )));
    }

    Ok(result.stdout.trim().to_string())
}

fn get_pr_head_sha(repo_root: &str, pr_number: &str) -> Result<String, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "pr".to_string(),
                "view".to_string(),
                pr_number.to_string(),
                "--json".to_string(),
                "headRefOid".to_string(),
                "-q".to_string(),
                ".headRefOid".to_string(),
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
            "failed to get PR head SHA: {}",
            result.stderr.trim()
        )));
    }

    Ok(result.stdout.trim().to_string())
}

fn create_inline_review_comment(
    repo_root: &str,
    owner_repo: &str,
    pr_number: &str,
    commit_id: &str,
    path: &str,
    line: u64,
    body: &str,
) -> Result<u64, OrbitError> {
    let payload = json!({
        "body": body,
        "commit_id": commit_id,
        "path": path,
        "line": line,
    });

    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "api".to_string(),
                format!("repos/{owner_repo}/pulls/{pr_number}/comments"),
                "--method".to_string(),
                "POST".to_string(),
                "--input".to_string(),
                "-".to_string(),
            ],
            current_dir: Some(repo_root.to_string()),
            timeout_ms: Some(TIMEOUT_MS),
            stdin_mode: StdinMode::Bytes(payload.to_string().into_bytes()),
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "failed to create inline review comment: {}",
            result.stderr.trim()
        )));
    }

    parse_comment_id(&result.stdout)
}

fn create_general_comment(repo_root: &str, pr_number: &str, body: &str) -> Result<u64, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "pr".to_string(),
                "comment".to_string(),
                pr_number.to_string(),
                "--body".to_string(),
                body.to_string(),
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
            "failed to create PR comment: {}",
            result.stderr.trim()
        )));
    }

    // gh pr comment outputs a URL like https://github.com/owner/repo/pull/1#issuecomment-123
    // but doesn't return structured JSON by default. Use the API instead.
    // Actually, let's use the API for general comments too for consistency.
    // Fall back: parse the URL for the comment ID, or return 0 if unparseable.
    // For general comments via `gh pr comment`, the output is a URL.
    // Extract comment ID from the URL fragment.
    let output = result.stdout.trim();
    if let Some(id_str) = output.rsplit("issuecomment-").next()
        && let Ok(id) = id_str.trim().parse::<u64>()
    {
        return Ok(id);
    }

    // If we can't parse the ID from the URL, return an error rather than silently losing it
    Err(OrbitError::Execution(format!(
        "could not parse comment ID from gh pr comment output: {output}"
    )))
}

fn create_reply_comment(
    repo_root: &str,
    owner_repo: &str,
    pr_number: &str,
    parent_comment_id: u64,
    body: &str,
) -> Result<u64, OrbitError> {
    let payload = json!({ "body": body });

    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "api".to_string(),
                format!(
                    "repos/{owner_repo}/pulls/{pr_number}/comments/{parent_comment_id}/replies"
                ),
                "--method".to_string(),
                "POST".to_string(),
                "--input".to_string(),
                "-".to_string(),
            ],
            current_dir: Some(repo_root.to_string()),
            timeout_ms: Some(TIMEOUT_MS),
            stdin_mode: StdinMode::Bytes(payload.to_string().into_bytes()),
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "failed to create reply comment: {}",
            result.stderr.trim()
        )));
    }

    parse_comment_id(&result.stdout)
}

fn parse_comment_id(json_output: &str) -> Result<u64, OrbitError> {
    let value: Value = serde_json::from_str(json_output.trim())
        .map_err(|e| OrbitError::Execution(format!("failed to parse GitHub API response: {e}")))?;

    value
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| OrbitError::Execution("GitHub API response missing 'id' field".to_string()))
}
