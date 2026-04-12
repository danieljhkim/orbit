use std::collections::HashMap;

use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_store::pr_scoreboard;
use orbit_types::{OrbitError, ReviewThread};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

const TIMEOUT_MS: u64 = 15_000;
type PrFilePatchMap = HashMap<String, Option<String>>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ThreadSyncMode {
    Inline { path: String, line: u64 },
    General,
}

trait GhClient {
    fn get_owner_repo(&self, repo_root: &str) -> Result<String, OrbitError>;
    fn get_pr_head_sha(&self, repo_root: &str, pr_number: &str) -> Result<String, OrbitError>;
    fn load_pr_file_patches(
        &self,
        repo_root: &str,
        owner_repo: &str,
        pr_number: &str,
    ) -> Result<PrFilePatchMap, OrbitError>;
    fn create_inline_review_comment(
        &self,
        repo_root: &str,
        owner_repo: &str,
        pr_number: &str,
        commit_id: &str,
        path: &str,
        line: u64,
        body: &str,
    ) -> Result<u64, OrbitError>;
    fn create_general_comment(
        &self,
        repo_root: &str,
        pr_number: &str,
        body: &str,
    ) -> Result<u64, OrbitError>;
    fn create_reply_comment(
        &self,
        repo_root: &str,
        owner_repo: &str,
        pr_number: &str,
        parent_comment_id: u64,
        body: &str,
    ) -> Result<u64, OrbitError>;
}

struct RealGhClient;

impl GhClient for RealGhClient {
    fn get_owner_repo(&self, repo_root: &str) -> Result<String, OrbitError> {
        get_owner_repo(repo_root)
    }

    fn get_pr_head_sha(&self, repo_root: &str, pr_number: &str) -> Result<String, OrbitError> {
        get_pr_head_sha(repo_root, pr_number)
    }

    fn load_pr_file_patches(
        &self,
        repo_root: &str,
        owner_repo: &str,
        pr_number: &str,
    ) -> Result<PrFilePatchMap, OrbitError> {
        load_pr_file_patches(repo_root, owner_repo, pr_number)
    }

    fn create_inline_review_comment(
        &self,
        repo_root: &str,
        owner_repo: &str,
        pr_number: &str,
        commit_id: &str,
        path: &str,
        line: u64,
        body: &str,
    ) -> Result<u64, OrbitError> {
        create_inline_review_comment(
            repo_root, owner_repo, pr_number, commit_id, path, line, body,
        )
    }

    fn create_general_comment(
        &self,
        repo_root: &str,
        pr_number: &str,
        body: &str,
    ) -> Result<u64, OrbitError> {
        create_general_comment(repo_root, pr_number, body)
    }

    fn create_reply_comment(
        &self,
        repo_root: &str,
        owner_repo: &str,
        pr_number: &str,
        parent_comment_id: u64,
        body: &str,
    ) -> Result<u64, OrbitError> {
        create_reply_comment(repo_root, owner_repo, pr_number, parent_comment_id, body)
    }
}

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
    let gh = RealGhClient;
    sync_task_review_to_github_with_client(host, &gh, task_id)
}

fn sync_task_review_to_github_with_client<
    H: RuntimeHost + TaskHost + ?Sized,
    C: GhClient + ?Sized,
>(
    host: &H,
    gh: &C,
    task_id: &str,
) -> Result<u64, OrbitError> {
    let task = host.get_task(task_id)?;

    if task.pr_number.is_none() {
        return Ok(0);
    }

    if task.review_threads.is_empty() {
        return Ok(0);
    }

    let pr_number = task.pr_number.as_deref().ok_or_else(|| {
        OrbitError::InvalidInput("sync_review_to_github: task missing pr_number".to_string())
    })?;
    let Some(repo_root) = task.repo_root.as_deref().or(task.workspace_path.as_deref()) else {
        eprintln!(
            "orbit: skipping review sync for task {task_id}: \
             missing repo_root and workspace_path"
        );
        return Ok(0);
    };

    let owner_repo = gh.get_owner_repo(repo_root)?;
    let head_sha = gh.get_pr_head_sha(repo_root, pr_number)?;
    // If patch metadata can't be resolved, fall back to general PR comments
    // instead of failing the entire review sync run.
    let pr_file_patches = gh
        .load_pr_file_patches(repo_root, &owner_repo, pr_number)
        .unwrap_or_default();

    let mut threads = task.review_threads.clone();
    let mut synced_count: u64 = 0;

    for thread in threads.iter_mut() {
        let pending_labels = pending_sync_message_labels(thread);
        let thread_synced = sync_thread(
            gh,
            repo_root,
            &owner_repo,
            pr_number,
            &head_sha,
            &pr_file_patches,
            thread,
        )?;
        synced_count += thread_synced;

        if host.scoring_enabled() {
            for label in pending_labels {
                if let Some((agent, model)) = parse_agent_model_label(&label) {
                    let model = host
                        .canonical_model_name(agent, Some(model))
                        .unwrap_or_else(|| model.to_string());
                    let _ = pr_scoreboard::record_pr_review_comment(
                        host.scoreboard_dir(),
                        agent,
                        &model,
                    );
                }
            }
        }
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

fn sync_thread<C: GhClient + ?Sized>(
    gh: &C,
    repo_root: &str,
    owner_repo: &str,
    pr_number: &str,
    head_sha: &str,
    pr_file_patches: &PrFilePatchMap,
    thread: &mut ReviewThread,
) -> Result<u64, OrbitError> {
    let mut synced: u64 = 0;
    let thread_path = thread.path.clone();
    let thread_line = thread.line;
    let sync_mode = sync_mode_for_thread(thread_path.as_deref(), thread_line, pr_file_patches);

    if thread.github_thread_id.is_none() && !thread.messages.is_empty() {
        let first_msg = &thread.messages[0];

        let github_id = match &sync_mode {
            ThreadSyncMode::Inline { path, line } => gh.create_inline_review_comment(
                repo_root,
                owner_repo,
                pr_number,
                head_sha,
                path,
                *line,
                &first_msg.body,
            )?,
            ThreadSyncMode::General => gh.create_general_comment(
                repo_root,
                pr_number,
                &render_general_comment_body(thread_path.as_deref(), thread_line, &first_msg.body),
            )?,
        };

        thread.github_thread_id = Some(github_id);
        thread.messages[0].github_comment_id = Some(github_id);
        synced += 1;
    }

    match &sync_mode {
        ThreadSyncMode::Inline { .. } => {
            if let Some(parent_id) = thread.github_thread_id {
                for msg in thread.messages.iter_mut().skip(1) {
                    if msg.github_comment_id.is_some() {
                        continue;
                    }
                    let reply_id = gh.create_reply_comment(
                        repo_root, owner_repo, pr_number, parent_id, &msg.body,
                    )?;
                    msg.github_comment_id = Some(reply_id);
                    synced += 1;
                }
            }
        }
        ThreadSyncMode::General => {
            for msg in thread.messages.iter_mut().skip(1) {
                if msg.github_comment_id.is_some() {
                    continue;
                }
                let comment_id = gh.create_general_comment(
                    repo_root,
                    pr_number,
                    &render_general_comment_body(thread_path.as_deref(), thread_line, &msg.body),
                )?;
                msg.github_comment_id = Some(comment_id);
                synced += 1;
            }
        }
    }

    Ok(synced)
}

fn sync_mode_for_thread(
    path: Option<&str>,
    line: Option<u64>,
    pr_file_patches: &PrFilePatchMap,
) -> ThreadSyncMode {
    match (path, line) {
        (Some(path), Some(line))
            if pr_file_patches
                .get(path)
                .and_then(|patch| patch.as_deref())
                .is_some_and(|patch| patch_supports_right_side_line(patch, line)) =>
        {
            ThreadSyncMode::Inline {
                path: path.to_string(),
                line,
            }
        }
        _ => ThreadSyncMode::General,
    }
}

fn pending_sync_message_labels(thread: &ReviewThread) -> Vec<String> {
    let mut labels = Vec::new();

    if thread.github_thread_id.is_none()
        && let Some(first) = thread.messages.first()
    {
        labels.push(first.by.clone());
    }

    labels.extend(
        thread
            .messages
            .iter()
            .skip(1)
            .filter(|message| message.github_comment_id.is_none())
            .map(|message| message.by.clone()),
    );

    labels
}

fn parse_agent_model_label(label: &str) -> Option<(&str, &str)> {
    let (agent, model) = label.split_once(" / ")?;
    let agent = agent.trim();
    let model = model.trim();
    if agent.is_empty() || model.is_empty() {
        return None;
    }
    Some((agent, model))
}

fn render_general_comment_body(path: Option<&str>, line: Option<u64>, body: &str) -> String {
    match (path, line) {
        (Some(path), Some(line)) => format!("On `{path}:{line}`:\n\n{body}"),
        _ => body.to_string(),
    }
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

fn load_pr_file_patches(
    repo_root: &str,
    owner_repo: &str,
    pr_number: &str,
) -> Result<PrFilePatchMap, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "api".to_string(),
                format!("repos/{owner_repo}/pulls/{pr_number}/files"),
                "--paginate".to_string(),
                "--slurp".to_string(),
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
            "failed to fetch PR file patches: {}",
            result.stderr.trim()
        )));
    }

    parse_pr_file_patches(&result.stdout)
}

fn parse_pr_file_patches(stdout: &str) -> Result<PrFilePatchMap, OrbitError> {
    let payload: Value = serde_json::from_str(stdout.trim()).map_err(|error| {
        OrbitError::Execution(format!(
            "failed to parse gh api pull request files output: {error}"
        ))
    })?;

    let mut patches = HashMap::new();
    for item in flatten_paginated_items(payload, "pull request files")? {
        let Value::Object(file) = item else {
            return Err(OrbitError::Execution(
                "gh api pull request files returned non-object item".to_string(),
            ));
        };
        let filename = file
            .get("filename")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                OrbitError::Execution(
                    "gh api pull request files returned item without filename".to_string(),
                )
            })?;
        let patch = file.get("patch").and_then(Value::as_str).map(String::from);
        patches.insert(filename.to_string(), patch);
    }

    Ok(patches)
}

fn flatten_paginated_items(payload: Value, label: &str) -> Result<Vec<Value>, OrbitError> {
    match payload {
        Value::Array(items) => {
            let mut flattened = Vec::new();
            for item in items {
                match item {
                    Value::Array(page) => flattened.extend(page),
                    Value::Object(_) => flattened.push(item),
                    other => {
                        return Err(OrbitError::Execution(format!(
                            "gh api {label} returned unexpected item type: {}",
                            json_type_name(&other)
                        )));
                    }
                }
            }
            Ok(flattened)
        }
        other => Err(OrbitError::Execution(format!(
            "gh api {label} returned unexpected payload type: {}",
            json_type_name(&other)
        ))),
    }
}

fn patch_supports_right_side_line(patch: &str, target_line: u64) -> bool {
    if target_line == 0 {
        return false;
    }

    let mut current_new_line: Option<u64> = None;

    for line in patch.lines() {
        if let Some(start_line) = parse_hunk_new_start(line) {
            current_new_line = Some(start_line);
            continue;
        }

        let Some(new_line) = current_new_line.as_mut() else {
            continue;
        };

        match line.as_bytes().first().copied() {
            Some(b' ') | Some(b'+') => {
                if *new_line == target_line {
                    return true;
                }
                *new_line += 1;
            }
            Some(b'-') => {}
            _ => {}
        }
    }

    false
}

fn parse_hunk_new_start(line: &str) -> Option<u64> {
    if !line.starts_with("@@") {
        return None;
    }

    line.split_whitespace()
        .find(|segment| segment.starts_with('+'))
        .and_then(|segment| segment.trim_start_matches('+').split(',').next())
        .and_then(|start| start.parse::<u64>().ok())
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
        "side": "RIGHT",
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

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use chrono::Utc;
    use orbit_tools::ToolContext;
    use orbit_types::{
        Activity, ActorIdentity, Job, JobTargetType, OrbitError, OrbitEvent, ReviewMessage,
        ReviewThread, ReviewThreadStatus, Role, Task, TaskPriority, TaskStatus, TaskType,
    };
    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::{
        GhClient, PrFilePatchMap, ThreadSyncMode, parse_agent_model_label, parse_pr_file_patches,
        patch_supports_right_side_line, pending_sync_message_labels, render_general_comment_body,
        sync_mode_for_thread, sync_task_review_to_github_with_client,
    };
    use crate::context::{JobRunResult, RuntimeHost, TaskAutomationUpdate, TaskHost};

    struct TestHost {
        task: Task,
        scoreboard_dir: PathBuf,
        scoring_enabled: bool,
        applied_updates: Mutex<Vec<TaskAutomationUpdate>>,
    }

    impl TestHost {
        fn with_scoreboard(task: Task, scoreboard_dir: PathBuf) -> Self {
            Self {
                task,
                scoreboard_dir,
                scoring_enabled: true,
                applied_updates: Mutex::new(Vec::new()),
            }
        }
    }

    impl TaskHost for TestHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            if self.task.id == task_id {
                Ok(self.task.clone())
            } else {
                Err(OrbitError::TaskNotFound(task_id.to_string()))
            }
        }

        fn get_task_artifacts(
            &self,
            _task_id: &str,
        ) -> Result<Vec<orbit_types::TaskArtifact>, OrbitError> {
            Ok(Vec::new())
        }

        fn list_tasks_filtered(
            &self,
            _status: Option<TaskStatus>,
            _priority: Option<TaskPriority>,
            _parent_id: Option<&str>,
            _batch_id: Option<&str>,
        ) -> Result<Vec<Task>, OrbitError> {
            unimplemented!("not used in sync_review tests")
        }

        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not used in sync_review tests")
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not used in sync_review tests")
        }

        fn apply_task_automation_update(
            &self,
            _task_id: &str,
            update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            self.applied_updates
                .lock()
                .expect("applied updates lock")
                .push(update);
            Ok(())
        }
    }

    impl RuntimeHost for TestHost {
        fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
            Ok(())
        }

        fn repo_root(&self) -> Result<String, OrbitError> {
            Ok(self.task.repo_root.clone().unwrap_or_default())
        }

        fn data_root(&self) -> &Path {
            Path::new(".")
        }

        fn run_job_now_with_input_debug(
            &self,
            _job_id: &str,
            _input: Value,
            _debug: bool,
        ) -> Result<JobRunResult, OrbitError> {
            unimplemented!("not used in sync_review tests")
        }

        fn validate_activity_target_exists(
            &self,
            _target_type: JobTargetType,
            _target_id: &str,
        ) -> Result<Activity, OrbitError> {
            unimplemented!("not used in sync_review tests")
        }

        fn get_job(&self, _job_id: &str) -> Result<Option<Job>, OrbitError> {
            Ok(None)
        }

        fn run_tool_with_context_and_role(
            &self,
            _name: &str,
            _input: Value,
            _role: Role,
            _tool_context: ToolContext,
        ) -> Result<Value, OrbitError> {
            unimplemented!("not used in sync_review tests")
        }

        fn maybe_create_failure_task(
            &self,
            _job_id: &str,
            _run_id: &str,
            _error_code: &str,
            _error_message: &str,
            _agent: Option<&str>,
            _model: Option<&str>,
        ) -> Result<(), OrbitError> {
            Ok(())
        }

        fn scoring_enabled(&self) -> bool {
            self.scoring_enabled
        }

        fn graph_editing(&self) -> bool {
            false
        }

        fn scoreboard_dir(&self) -> &Path {
            &self.scoreboard_dir
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum GhCall {
        InlineComment {
            commit_id: String,
            path: String,
            line: u64,
            body: String,
        },
        GeneralComment {
            body: String,
        },
        ReplyComment {
            parent_comment_id: u64,
            body: String,
        },
    }

    struct FakeGhClient {
        owner_repo: String,
        head_sha: String,
        pr_file_patches: Result<PrFilePatchMap, String>,
        inline_comment_ids: Mutex<VecDeque<u64>>,
        general_comment_ids: Mutex<VecDeque<u64>>,
        reply_comment_ids: Mutex<VecDeque<u64>>,
        calls: Mutex<Vec<GhCall>>,
    }

    impl FakeGhClient {
        fn new(pr_file_patches: Result<PrFilePatchMap, String>) -> Self {
            Self {
                owner_repo: "orbit/orbit".to_string(),
                head_sha: "deadbeef".to_string(),
                pr_file_patches,
                inline_comment_ids: Mutex::new(VecDeque::new()),
                general_comment_ids: Mutex::new(VecDeque::new()),
                reply_comment_ids: Mutex::new(VecDeque::new()),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn with_inline_comment_ids(self, ids: impl IntoIterator<Item = u64>) -> Self {
            *self.inline_comment_ids.lock().expect("inline ids lock") = ids.into_iter().collect();
            self
        }

        fn with_general_comment_ids(self, ids: impl IntoIterator<Item = u64>) -> Self {
            *self.general_comment_ids.lock().expect("general ids lock") = ids.into_iter().collect();
            self
        }

        fn with_reply_comment_ids(self, ids: impl IntoIterator<Item = u64>) -> Self {
            *self.reply_comment_ids.lock().expect("reply ids lock") = ids.into_iter().collect();
            self
        }

        fn take_next_id(queue: &Mutex<VecDeque<u64>>, label: &str) -> Result<u64, OrbitError> {
            queue
                .lock()
                .expect("comment id queue lock")
                .pop_front()
                .ok_or_else(|| OrbitError::Execution(format!("missing fake {label} id")))
        }

        fn calls(&self) -> Vec<GhCall> {
            self.calls.lock().expect("calls lock").clone()
        }
    }

    impl GhClient for FakeGhClient {
        fn get_owner_repo(&self, _repo_root: &str) -> Result<String, OrbitError> {
            Ok(self.owner_repo.clone())
        }

        fn get_pr_head_sha(
            &self,
            _repo_root: &str,
            _pr_number: &str,
        ) -> Result<String, OrbitError> {
            Ok(self.head_sha.clone())
        }

        fn load_pr_file_patches(
            &self,
            _repo_root: &str,
            _owner_repo: &str,
            _pr_number: &str,
        ) -> Result<PrFilePatchMap, OrbitError> {
            self.pr_file_patches.clone().map_err(OrbitError::Execution)
        }

        fn create_inline_review_comment(
            &self,
            _repo_root: &str,
            _owner_repo: &str,
            _pr_number: &str,
            commit_id: &str,
            path: &str,
            line: u64,
            body: &str,
        ) -> Result<u64, OrbitError> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(GhCall::InlineComment {
                    commit_id: commit_id.to_string(),
                    path: path.to_string(),
                    line,
                    body: body.to_string(),
                });
            Self::take_next_id(&self.inline_comment_ids, "inline review comment")
        }

        fn create_general_comment(
            &self,
            _repo_root: &str,
            _pr_number: &str,
            body: &str,
        ) -> Result<u64, OrbitError> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(GhCall::GeneralComment {
                    body: body.to_string(),
                });
            Self::take_next_id(&self.general_comment_ids, "general comment")
        }

        fn create_reply_comment(
            &self,
            _repo_root: &str,
            _owner_repo: &str,
            _pr_number: &str,
            parent_comment_id: u64,
            body: &str,
        ) -> Result<u64, OrbitError> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(GhCall::ReplyComment {
                    parent_comment_id,
                    body: body.to_string(),
                });
            Self::take_next_id(&self.reply_comment_ids, "reply comment")
        }
    }

    fn sample_task(task_id: &str, repo_root: &Path) -> Task {
        let now = Utc::now();
        Task {
            id: task_id.to_string(),
            parent_id: None,
            title: format!("Task {task_id}"),
            description: "test".to_string(),
            acceptance_criteria: vec![],
            plan: "plan".to_string(),
            execution_summary: String::new(),
            context_files: vec![],
            workspace_path: Some(repo_root.display().to_string()),
            repo_root: Some(repo_root.display().to_string()),
            assigned_to: None,
            created_by: None,
            actor_identity: ActorIdentity::default(),
            status: TaskStatus::Review,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Feature,
            pr_number: Some("91".to_string()),
            pr_status: Some("REQUEST_CHANGES".to_string()),
            proposed_by: None,
            source_task_id: None,
            batch_id: Some("batch-1".to_string()),
            comments: vec![],
            history: vec![],
            review_threads: vec![],
            created_at: now,
            updated_at: now,
        }
    }

    fn scoreboard_value(scoreboard_dir: &Path) -> Value {
        serde_json::from_str(
            &fs::read_to_string(scoreboard_dir.join("pr.json")).expect("read scoreboard"),
        )
        .expect("valid scoreboard json")
    }

    fn review_message(
        message_id: &str,
        by: &str,
        body: &str,
        github_comment_id: Option<u64>,
    ) -> ReviewMessage {
        ReviewMessage {
            message_id: message_id.to_string(),
            at: Utc::now(),
            by: by.to_string(),
            body: body.to_string(),
            github_comment_id,
        }
    }

    fn review_thread(
        thread_id: &str,
        path: Option<&str>,
        line: Option<u64>,
        messages: Vec<ReviewMessage>,
        github_thread_id: Option<u64>,
    ) -> ReviewThread {
        ReviewThread {
            thread_id: thread_id.to_string(),
            path: path.map(str::to_string),
            line,
            status: ReviewThreadStatus::Open,
            messages,
            github_thread_id,
        }
    }

    fn sample_patch_map(path: &str) -> PrFilePatchMap {
        HashMap::from([(
            path.to_string(),
            Some("@@ -9,2 +9,3 @@\n context\n-old\n+new\n+more".to_string()),
        )])
    }

    fn updated_threads(host: &TestHost) -> Vec<ReviewThread> {
        let updates = host.applied_updates.lock().expect("applied updates");
        assert_eq!(updates.len(), 1);
        updates[0]
            .review_threads
            .clone()
            .expect("review threads updated")
    }

    #[test]
    fn patch_supports_right_side_line_matches_context_and_added_lines() {
        let patch = "\
@@ -6,4 +6,5 @@ use std::time::Duration;
 use orbit_types::{JobRunState, OrbitError, Task, TaskStatus};
 use serde_json::{Value, json};
 
-use super::git::{git_command_success, git_output, git_success, resolve_worktree_start_point};
+use super::git::{git_output, git_success, resolve_worktree_start_point};
+use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};";

        assert!(patch_supports_right_side_line(patch, 6));
        assert!(patch_supports_right_side_line(patch, 7));
        assert!(patch_supports_right_side_line(patch, 8));
        assert!(patch_supports_right_side_line(patch, 9));
        assert!(patch_supports_right_side_line(patch, 10));
    }

    #[test]
    fn patch_supports_right_side_line_rejects_lines_outside_the_diff() {
        let patch = "\
@@ -6,4 +6,4 @@ use std::time::Duration;
 use orbit_types::{JobRunState, OrbitError, Task, TaskStatus};
 use serde_json::{Value, json};
 
-use super::git::{git_command_success, git_output, git_success, resolve_worktree_start_point};
+use super::git::{git_output, git_success, resolve_worktree_start_point};";

        assert!(!patch_supports_right_side_line(patch, 391));
        assert!(!patch_supports_right_side_line(patch, 0));
    }

    #[test]
    fn sync_mode_falls_back_to_general_for_non_diff_line() {
        let mut patches = HashMap::new();
        patches.insert(
            "crates/orbit-engine/src/executor/automation/parallel.rs".to_string(),
            Some(
                "\
@@ -6,4 +6,4 @@ use std::time::Duration;
 use orbit_types::{JobRunState, OrbitError, Task, TaskStatus};
 use serde_json::{Value, json};
 
-use super::git::{git_command_success, git_output, git_success, resolve_worktree_start_point};
+use super::git::{git_output, git_success, resolve_worktree_start_point};"
                    .to_string(),
            ),
        );

        assert_eq!(
            sync_mode_for_thread(
                Some("crates/orbit-engine/src/executor/automation/parallel.rs"),
                Some(391),
                &patches,
            ),
            ThreadSyncMode::General
        );
    }

    #[test]
    fn render_general_comment_body_preserves_inline_location_context() {
        assert_eq!(
            render_general_comment_body(Some("src/lib.rs"), Some(42), "Needs attention"),
            "On `src/lib.rs:42`:\n\nNeeds attention"
        );
        assert_eq!(
            render_general_comment_body(None, None, "Summary"),
            "Summary".to_string()
        );
    }

    #[test]
    fn parse_pr_file_patches_handles_slurped_pages() {
        let payload = json!([
            [
                {
                    "filename": "src/lib.rs",
                    "patch": "@@ -1 +1 @@\n-old\n+new"
                }
            ],
            [
                {
                    "filename": "README.md"
                }
            ]
        ]);

        let parsed = parse_pr_file_patches(&payload.to_string()).expect("parse patches");

        assert_eq!(
            parsed.get("src/lib.rs").and_then(|value| value.as_deref()),
            Some("@@ -1 +1 @@\n-old\n+new")
        );
        assert!(parsed.get("README.md").is_some_and(|value| value.is_none()));
    }

    #[test]
    fn pending_sync_message_labels_collects_only_unsynced_messages() {
        let thread = ReviewThread {
            thread_id: "rt-1".to_string(),
            path: Some("src/lib.rs".to_string()),
            line: Some(42),
            status: ReviewThreadStatus::Open,
            messages: vec![
                ReviewMessage {
                    message_id: "rm-1".to_string(),
                    at: Utc::now(),
                    by: "claude / sonnet".to_string(),
                    body: "Needs a test.".to_string(),
                    github_comment_id: None,
                },
                ReviewMessage {
                    message_id: "rm-2".to_string(),
                    at: Utc::now(),
                    by: "codex / gpt-5.4".to_string(),
                    body: "Added coverage.".to_string(),
                    github_comment_id: None,
                },
                ReviewMessage {
                    message_id: "rm-3".to_string(),
                    at: Utc::now(),
                    by: "claude / sonnet".to_string(),
                    body: "Looks good.".to_string(),
                    github_comment_id: Some(33),
                },
            ],
            github_thread_id: None,
        };

        assert_eq!(
            pending_sync_message_labels(&thread),
            vec!["claude / sonnet".to_string(), "codex / gpt-5.4".to_string()]
        );
    }

    #[test]
    fn parse_agent_model_label_requires_agent_and_model() {
        assert_eq!(
            parse_agent_model_label("claude / sonnet"),
            Some(("claude", "sonnet"))
        );
        assert_eq!(parse_agent_model_label("human reviewer"), None);
        assert_eq!(parse_agent_model_label("claude / "), None);
    }

    #[test]
    fn sync_general_comment_persists_github_id_and_task_update() {
        let repo_dir = tempdir().expect("repo dir");
        let scoreboard_dir = tempdir().expect("scoreboard dir");
        let mut task = sample_task("T20260402-0425", repo_dir.path());
        task.review_threads = vec![review_thread(
            "rt-1",
            None,
            None,
            vec![review_message(
                "rm-1",
                "claude / sonnet",
                "Please add a behavior-level sync test.",
                None,
            )],
            None,
        )];

        let host = TestHost::with_scoreboard(task, scoreboard_dir.path().to_path_buf());
        let gh = FakeGhClient::new(Ok(HashMap::new())).with_general_comment_ids([123]);

        let synced = sync_task_review_to_github_with_client(&host, &gh, "T20260402-0425")
            .expect("sync succeeds");

        assert_eq!(synced, 1);
        assert_eq!(
            gh.calls(),
            vec![GhCall::GeneralComment {
                body: "Please add a behavior-level sync test.".to_string(),
            }]
        );
        let scoreboard = scoreboard_value(scoreboard_dir.path());
        assert_eq!(scoreboard["pr-review-comments"]["claude"]["sonnet"], 1);

        let threads = updated_threads(&host);
        assert_eq!(threads[0].github_thread_id, Some(123));
        assert_eq!(threads[0].messages[0].github_comment_id, Some(123));
    }

    #[test]
    fn sync_general_comment_skips_scoreboard_when_scoring_disabled() {
        let repo_dir = tempdir().expect("repo dir");
        let scoreboard_dir = tempdir().expect("scoreboard dir");
        let mut task = sample_task("T20260402-0425", repo_dir.path());
        task.review_threads = vec![review_thread(
            "rt-1",
            None,
            None,
            vec![review_message(
                "rm-1",
                "claude / sonnet",
                "Please add a behavior-level sync test.",
                None,
            )],
            None,
        )];

        let host = TestHost {
            task,
            scoreboard_dir: scoreboard_dir.path().to_path_buf(),
            scoring_enabled: false,
            applied_updates: Mutex::new(Vec::new()),
        };
        let gh = FakeGhClient::new(Ok(HashMap::new())).with_general_comment_ids([123]);

        let synced = sync_task_review_to_github_with_client(&host, &gh, "T20260402-0425")
            .expect("sync succeeds");

        assert_eq!(synced, 1);
        assert!(!scoreboard_dir.path().join("pr.json").exists());

        let threads = updated_threads(&host);
        assert_eq!(threads[0].github_thread_id, Some(123));
        assert_eq!(threads[0].messages[0].github_comment_id, Some(123));
    }

    #[test]
    fn sync_inline_comment_persists_github_ids() {
        let repo_dir = tempdir().expect("repo dir");
        let scoreboard_dir = tempdir().expect("scoreboard dir");
        let mut task = sample_task("T20260402-0501", repo_dir.path());
        task.review_threads = vec![review_thread(
            "rt-1",
            Some("src/lib.rs"),
            Some(10),
            vec![
                review_message("rm-1", "human reviewer", "Inline note", None),
                review_message("rm-2", "human reviewer", "Follow-up", None),
            ],
            None,
        )];

        let host = TestHost::with_scoreboard(task, scoreboard_dir.path().to_path_buf());
        let gh = FakeGhClient::new(Ok(sample_patch_map("src/lib.rs")))
            .with_inline_comment_ids([200])
            .with_reply_comment_ids([201]);

        let synced = sync_task_review_to_github_with_client(&host, &gh, "T20260402-0501")
            .expect("sync succeeds");

        assert_eq!(synced, 2);
        assert_eq!(
            gh.calls(),
            vec![
                GhCall::InlineComment {
                    commit_id: "deadbeef".to_string(),
                    path: "src/lib.rs".to_string(),
                    line: 10,
                    body: "Inline note".to_string(),
                },
                GhCall::ReplyComment {
                    parent_comment_id: 200,
                    body: "Follow-up".to_string(),
                },
            ]
        );

        let threads = updated_threads(&host);
        assert_eq!(threads[0].github_thread_id, Some(200));
        assert_eq!(threads[0].messages[0].github_comment_id, Some(200));
        assert_eq!(threads[0].messages[1].github_comment_id, Some(201));
    }

    #[test]
    fn sync_reply_for_inline_thread_already_has_thread_id() {
        let repo_dir = tempdir().expect("repo dir");
        let scoreboard_dir = tempdir().expect("scoreboard dir");
        let mut task = sample_task("T20260402-0502", repo_dir.path());
        task.review_threads = vec![review_thread(
            "rt-1",
            Some("src/lib.rs"),
            Some(10),
            vec![
                review_message("rm-1", "human reviewer", "Original note", Some(200)),
                review_message("rm-2", "human reviewer", "New reply", None),
            ],
            Some(200),
        )];

        let host = TestHost::with_scoreboard(task, scoreboard_dir.path().to_path_buf());
        let gh =
            FakeGhClient::new(Ok(sample_patch_map("src/lib.rs"))).with_reply_comment_ids([102]);

        let synced = sync_task_review_to_github_with_client(&host, &gh, "T20260402-0502")
            .expect("sync succeeds");

        assert_eq!(synced, 1);
        assert_eq!(
            gh.calls(),
            vec![GhCall::ReplyComment {
                parent_comment_id: 200,
                body: "New reply".to_string(),
            }]
        );

        let threads = updated_threads(&host);
        assert_eq!(threads[0].github_thread_id, Some(200));
        assert_eq!(threads[0].messages[0].github_comment_id, Some(200));
        assert_eq!(threads[0].messages[1].github_comment_id, Some(102));
    }

    #[test]
    fn patch_lookup_failure_falls_back_to_general_comment() {
        let repo_dir = tempdir().expect("repo dir");
        let scoreboard_dir = tempdir().expect("scoreboard dir");
        let mut task = sample_task("T20260402-0503", repo_dir.path());
        task.review_threads = vec![review_thread(
            "rt-1",
            Some("src/lib.rs"),
            Some(10),
            vec![review_message(
                "rm-1",
                "human reviewer",
                "Fallback note",
                None,
            )],
            None,
        )];

        let host = TestHost::with_scoreboard(task, scoreboard_dir.path().to_path_buf());
        let gh = FakeGhClient::new(Err("patch lookup failed".to_string()))
            .with_general_comment_ids([321]);

        let synced = sync_task_review_to_github_with_client(&host, &gh, "T20260402-0503")
            .expect("sync succeeds");

        assert_eq!(synced, 1);
        assert_eq!(
            gh.calls(),
            vec![GhCall::GeneralComment {
                body: "On `src/lib.rs:10`:\n\nFallback note".to_string(),
            }]
        );

        let threads = updated_threads(&host);
        assert_eq!(threads[0].github_thread_id, Some(321));
        assert_eq!(threads[0].messages[0].github_comment_id, Some(321));
    }
}
