use serde_json::{Value, json};

use orbit_store::pr_scoreboard;
use orbit_types::{OrbitError, ReviewThread};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use crate::executor::automation::input::required_batch_id;

use super::client::{GhClient, RealGhClient};
use super::patch_match::{PrFilePatchMap, patch_supports_right_side_line};

pub(crate) fn sync_batch_review_to_github<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = required_batch_id(input, "sync_batch_review_to_github")?;

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
                    let _ = pr_scoreboard::record_pr_review_comment(host.scoreboard_dir(), &model);
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum ThreadSyncMode {
    Inline { path: String, line: u64 },
    General,
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

    if thread.github_thread_id.is_none() {
        if let Some(first) = thread.messages.first() {
            labels.push(first.by.clone());
        }
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
