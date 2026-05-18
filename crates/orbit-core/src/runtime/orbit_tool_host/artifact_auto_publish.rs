use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use orbit_common::types::{AuditEventStatus, OrbitError, audit_execution_id};
use orbit_tools::OrbitBuiltinAction;
use serde_json::{Value, json};

use crate::{AuditEventInsertParams, OrbitRuntime};

const ORIGIN_REMOTE: &str = "origin";
const PUSH_OUTCOME_PUSHED: &str = "pushed";
const PUSH_OUTCOME_COMMITTED_ONLY: &str = "committed_only";
const PUSH_OUTCOME_FAILED: &str = "failed";
const AUTO_PUBLISH_TARGET: &str = "artifact_auto_publish";

#[derive(Debug, Clone)]
pub(super) struct ArtifactPublishContext {
    action: OrbitBuiltinAction,
    input: Value,
    pre_paths: Vec<String>,
    comment_learning_id: Option<String>,
}

#[derive(Debug, Clone)]
struct PublishPlan {
    tool_name: &'static str,
    artifact_id: String,
    commit_message: String,
    paths: Vec<String>,
}

#[derive(Debug)]
struct PublishSuccess {
    branch: String,
    commit_sha: String,
    target_worktree: PathBuf,
}

#[derive(Debug)]
struct PublishFailure {
    branch: String,
    commit_sha: Option<String>,
    target_worktree: Option<PathBuf>,
    push_outcome: &'static str,
    error: Box<OrbitError>,
}

struct PublishAudit<'a> {
    branch: &'a str,
    commit_sha: Option<&'a str>,
    push_outcome: &'a str,
    worktree: &'a Path,
    agent: Option<&'a str>,
    model: Option<&'a str>,
    error_message: Option<&'a str>,
}

#[derive(Debug)]
struct GitCommandOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

#[derive(Debug)]
struct GitFailure {
    phase: &'static str,
    output: GitCommandOutput,
}

#[derive(Debug)]
struct AutoPublishLock {
    path: PathBuf,
}

impl Drop for AutoPublishLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(super) fn capture_before_action(
    runtime: &OrbitRuntime,
    action: OrbitBuiltinAction,
    input: &Value,
) -> Result<Option<ArtifactPublishContext>, OrbitError> {
    if !runtime.artifact_auto_publish() || !is_auto_publish_action(action) {
        return Ok(None);
    }

    let pre_paths = match action {
        OrbitBuiltinAction::AdrUpdate => input
            .get("id")
            .and_then(Value::as_str)
            .map(|id| existing_adr_paths(runtime, id))
            .unwrap_or_default(),
        OrbitBuiltinAction::AdrSupersede => input
            .get("old_id")
            .or_else(|| input.get("old"))
            .or_else(|| input.get("oldId"))
            .and_then(Value::as_str)
            .map(|id| existing_adr_paths(runtime, id))
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    let comment_learning_id = if action == OrbitBuiltinAction::LearningCommentDelete {
        input
            .get("id")
            .and_then(Value::as_str)
            .map(|id| find_learning_for_comment(runtime, id))
            .transpose()?
            .flatten()
    } else {
        None
    };

    Ok(Some(ArtifactPublishContext {
        action,
        input: input.clone(),
        pre_paths,
        comment_learning_id,
    }))
}

pub(super) fn publish_after_success(
    runtime: &OrbitRuntime,
    response: &Value,
    context: Option<ArtifactPublishContext>,
    agent: Option<&str>,
    model: Option<&str>,
) -> Result<(), OrbitError> {
    let Some(context) = context else {
        return Ok(());
    };
    let plan = build_publish_plan(runtime, response, &context)?;
    let actor = commit_actor(runtime, agent, model);

    match run_auto_publish(runtime, &plan, &actor) {
        Ok(success) => {
            record_auto_publish_audit(
                runtime,
                &plan,
                PublishAudit {
                    branch: &success.branch,
                    commit_sha: Some(&success.commit_sha),
                    push_outcome: PUSH_OUTCOME_PUSHED,
                    worktree: &success.target_worktree,
                    agent,
                    model,
                    error_message: None,
                },
            )?;
            Ok(())
        }
        Err(failure) => {
            let failure_error = failure.error;
            let failure_error_message = failure_error.to_string();
            let audit_worktree = failure
                .target_worktree
                .as_deref()
                .unwrap_or_else(|| runtime.paths().repo_root.as_path());
            let audit_result = record_auto_publish_audit(
                runtime,
                &plan,
                PublishAudit {
                    branch: &failure.branch,
                    commit_sha: failure.commit_sha.as_deref(),
                    push_outcome: failure.push_outcome,
                    worktree: audit_worktree,
                    agent,
                    model,
                    error_message: Some(&failure_error_message),
                },
            );
            match audit_result {
                Ok(()) => Err(*failure_error),
                Err(audit_error) => Err(OrbitError::Execution(format!(
                    "{}; additionally failed to record auto-publish audit: {audit_error}",
                    failure_error
                ))),
            }
        }
    }
}

fn is_auto_publish_action(action: OrbitBuiltinAction) -> bool {
    matches!(
        action,
        OrbitBuiltinAction::AdrAdd
            | OrbitBuiltinAction::AdrUpdate
            | OrbitBuiltinAction::AdrSupersede
            | OrbitBuiltinAction::LearningAdd
            | OrbitBuiltinAction::LearningUpdate
            | OrbitBuiltinAction::LearningSupersede
            | OrbitBuiltinAction::LearningCommentAdd
            | OrbitBuiltinAction::LearningCommentDelete
    )
}

fn build_publish_plan(
    runtime: &OrbitRuntime,
    response: &Value,
    context: &ArtifactPublishContext,
) -> Result<PublishPlan, OrbitError> {
    let mut paths = BTreeSet::new();
    for path in &context.pre_paths {
        paths.insert(path.clone());
    }

    let (tool_name, verb, artifact_id, summary) = match context.action {
        OrbitBuiltinAction::AdrAdd => {
            let id = string_field(response, "id")?;
            let status = string_field(response, "status")?;
            insert_adr_bundle_paths(&mut paths, &status, &id);
            (
                "orbit.adr.add",
                "Add ADR",
                id,
                string_field(response, "title")?,
            )
        }
        OrbitBuiltinAction::AdrUpdate => {
            let id = string_field(response, "id")?;
            let status = string_field(response, "status")?;
            insert_adr_bundle_paths(&mut paths, &status, &id);
            (
                "orbit.adr.update",
                "Update ADR",
                id,
                string_field(response, "title")?,
            )
        }
        OrbitBuiltinAction::AdrSupersede => {
            let old_id = string_field(response, "id")?;
            for path in existing_adr_paths(runtime, &old_id) {
                paths.insert(path);
            }
            let new_id = input_string(&context.input, &["new_id", "new", "newId"])
                .unwrap_or_else(|| "unknown".to_string());
            for path in existing_adr_paths(runtime, &new_id) {
                paths.insert(path);
            }
            (
                "orbit.adr.supersede",
                "Supersede ADR",
                old_id,
                format!("with {new_id}"),
            )
        }
        OrbitBuiltinAction::LearningAdd => {
            let id = string_field(response, "id")?;
            paths.insert(learning_doc_path(&id));
            (
                "orbit.learning.add",
                "Add learning",
                id,
                string_field(response, "summary")?,
            )
        }
        OrbitBuiltinAction::LearningUpdate => {
            let id = string_field(response, "id")?;
            paths.insert(learning_doc_path(&id));
            (
                "orbit.learning.update",
                "Update learning",
                id,
                string_field(response, "summary")?,
            )
        }
        OrbitBuiltinAction::LearningSupersede => {
            let old = response.get("old").ok_or_else(|| {
                OrbitError::Execution("learning supersede response missing old".to_string())
            })?;
            let new = response.get("new").ok_or_else(|| {
                OrbitError::Execution("learning supersede response missing new".to_string())
            })?;
            let old_id = string_field(old, "id")?;
            let new_id = string_field(new, "id")?;
            paths.insert(learning_doc_path(&old_id));
            paths.insert(learning_doc_path(&new_id));
            (
                "orbit.learning.supersede",
                "Supersede learning",
                old_id,
                format!("with {new_id}"),
            )
        }
        OrbitBuiltinAction::LearningCommentAdd => {
            let id = string_field(response, "id")?;
            let learning_id = string_field(response, "learning_id")?;
            paths.insert(learning_comments_path(&learning_id));
            (
                "orbit.learning.comment.add",
                "Add learning comment",
                id,
                summary_from_text(
                    input_string(&context.input, &["body"]).as_deref(),
                    &learning_id,
                ),
            )
        }
        OrbitBuiltinAction::LearningCommentDelete => {
            let id = string_field(response, "id")?;
            let learning_id = context.comment_learning_id.clone().ok_or_else(|| {
                OrbitError::Execution(format!(
                    "auto-publish could not resolve parent learning for comment {id}"
                ))
            })?;
            paths.insert(learning_comments_path(&learning_id));
            (
                "orbit.learning.comment.delete",
                "Delete learning comment",
                id,
                learning_id,
            )
        }
        _ => {
            return Err(OrbitError::Execution(format!(
                "unsupported auto-publish action: {context:?}"
            )));
        }
    };

    let paths: Vec<String> = paths.into_iter().collect();
    if paths.is_empty() {
        return Err(OrbitError::Execution(format!(
            "auto-publish found no artifact paths for {}",
            tool_name
        )));
    }

    Ok(PublishPlan {
        tool_name,
        artifact_id: artifact_id.clone(),
        commit_message: format!(
            "docs: {verb} {artifact_id} — {}",
            sanitize_commit_summary(&summary)
        ),
        paths,
    })
}

fn run_auto_publish(
    runtime: &OrbitRuntime,
    plan: &PublishPlan,
    actor: &str,
) -> Result<PublishSuccess, PublishFailure> {
    let branch = runtime.workflow_base_branch().to_string();
    let target_worktree =
        find_worktree_for_branch(&runtime.paths().repo_root, &branch).map_err(|error| {
            PublishFailure {
                branch: branch.clone(),
                commit_sha: None,
                target_worktree: None,
                push_outcome: PUSH_OUTCOME_FAILED,
                error: Box::new(error),
            }
        })?;

    let _lock = acquire_auto_publish_lock(&target_worktree).map_err(|error| PublishFailure {
        branch: branch.clone(),
        commit_sha: None,
        target_worktree: Some(target_worktree.clone()),
        push_outcome: PUSH_OUTCOME_FAILED,
        error: Box::new(error),
    })?;

    if let Err(error) =
        sync_artifact_paths(&runtime.paths().repo_root, &target_worktree, &plan.paths)
    {
        return Err(PublishFailure {
            branch,
            commit_sha: None,
            target_worktree: Some(target_worktree),
            push_outcome: PUSH_OUTCOME_FAILED,
            error: Box::new(error),
        });
    }

    let commit_sha = match stage_and_commit_with_retry(&target_worktree, plan, actor) {
        Ok(sha) => sha,
        Err(error) => {
            return Err(PublishFailure {
                branch,
                commit_sha: None,
                target_worktree: Some(target_worktree),
                push_outcome: PUSH_OUTCOME_FAILED,
                error: Box::new(error),
            });
        }
    };

    match push_with_rebase_retry(&target_worktree, &branch) {
        Ok(final_sha) => Ok(PublishSuccess {
            branch,
            commit_sha: final_sha.unwrap_or(commit_sha),
            target_worktree,
        }),
        Err(error) => Err(PublishFailure {
            branch,
            commit_sha: Some(commit_sha),
            target_worktree: Some(target_worktree),
            push_outcome: PUSH_OUTCOME_COMMITTED_ONLY,
            error: Box::new(error),
        }),
    }
}

fn stage_and_commit_with_retry(
    worktree: &Path,
    plan: &PublishPlan,
    actor: &str,
) -> Result<String, OrbitError> {
    let mut last_failure: Option<GitFailure> = None;
    for attempt in 0..5 {
        match stage_and_commit_once(worktree, plan, actor) {
            Ok(sha) => return Ok(sha),
            Err(failure) if failure.is_index_lock() && attempt < 4 => {
                last_failure = Some(failure);
                thread::sleep(Duration::from_millis(50_u64 * (1_u64 << attempt)));
            }
            Err(failure) => return Err(failure.into_orbit_error()),
        }
    }
    Err(last_failure.map_or_else(
        || OrbitError::Execution("git index remained locked during auto-publish".to_string()),
        GitFailure::into_orbit_error,
    ))
}

fn stage_and_commit_once(
    worktree: &Path,
    plan: &PublishPlan,
    actor: &str,
) -> Result<String, GitFailure> {
    let paths = effective_git_paths(worktree, &plan.paths)?;
    if paths.is_empty() {
        return Err(GitFailure::message(
            "git add",
            "auto-publish found no existing or tracked artifact paths to commit",
        ));
    }

    let mut add_args = vec!["add".to_string(), "-A".to_string(), "--".to_string()];
    add_args.extend(paths.iter().cloned());
    run_git_success(worktree, &add_args, &[]).map_err(|output| GitFailure {
        phase: "git add",
        output,
    })?;

    let mut diff_args = vec![
        "diff".to_string(),
        "--cached".to_string(),
        "--quiet".to_string(),
        "--".to_string(),
    ];
    diff_args.extend(paths.iter().cloned());
    let diff = run_git(worktree, &diff_args, &[]);
    if diff.code == 0 {
        return Err(GitFailure::message(
            "git diff",
            "auto-publish staged no changes for artifact paths",
        ));
    }
    if diff.code != 1 {
        return Err(GitFailure {
            phase: "git diff",
            output: diff,
        });
    }

    let env = git_identity_env(actor);
    let mut commit_args = vec![
        "commit".to_string(),
        "-m".to_string(),
        plan.commit_message.clone(),
        "--".to_string(),
    ];
    commit_args.extend(paths);
    run_git_success(worktree, &commit_args, &env).map_err(|output| GitFailure {
        phase: "git commit",
        output,
    })?;

    git_head(worktree).map_err(|output| GitFailure {
        phase: "git rev-parse",
        output,
    })
}

fn push_with_rebase_retry(worktree: &Path, branch: &str) -> Result<Option<String>, OrbitError> {
    let push_args = vec![
        "push".to_string(),
        ORIGIN_REMOTE.to_string(),
        branch.to_string(),
    ];
    let first = run_git(worktree, &push_args, &[]);
    if first.success() {
        return Ok(None);
    }
    if !first.is_non_fast_forward() {
        return Err(OrbitError::Execution(format!(
            "auto-publish push to {ORIGIN_REMOTE}/{branch} failed: {}",
            first.summary()
        )));
    }

    let fetch_args = vec![
        "fetch".to_string(),
        ORIGIN_REMOTE.to_string(),
        branch.to_string(),
    ];
    run_git_success(worktree, &fetch_args, &[]).map_err(|output| {
        OrbitError::Execution(format!(
            "auto-publish fetch before rebase for branch '{branch}' failed: {}",
            output.summary()
        ))
    })?;

    let rebase_target = format!("{ORIGIN_REMOTE}/{branch}");
    let rebase_args = vec!["rebase".to_string(), rebase_target.clone()];
    let rebase = run_git(worktree, &rebase_args, &[]);
    if !rebase.success() {
        let abort_args = vec!["rebase".to_string(), "--abort".to_string()];
        let _ = run_git(worktree, &abort_args, &[]);
        return Err(OrbitError::Execution(format!(
            "auto-publish rebase onto {rebase_target} failed for branch '{branch}'; local commit is preserved: {}",
            rebase.summary()
        )));
    }

    let second = run_git(worktree, &push_args, &[]);
    if !second.success() {
        return Err(OrbitError::Execution(format!(
            "auto-publish push to {ORIGIN_REMOTE}/{branch} failed after one rebase retry; local commit is preserved: {}",
            second.summary()
        )));
    }

    git_head(worktree).map(Some).map_err(|output| {
        OrbitError::Execution(format!(
            "read rebased auto-publish HEAD: {}",
            output.summary()
        ))
    })
}

fn sync_artifact_paths(
    source_repo: &Path,
    target_worktree: &Path,
    paths: &[String],
) -> Result<(), OrbitError> {
    for path in paths {
        validate_artifact_path(path)?;
        let source = source_repo.join(path);
        let target = target_worktree.join(path);
        if source == target || same_existing_path(&source, &target) {
            continue;
        }
        if source.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    OrbitError::Io(format!("create auto-publish target dir: {error}"))
                })?;
            }
            fs::copy(&source, &target).map_err(|error| {
                OrbitError::Io(format!(
                    "copy auto-publish artifact '{}' to '{}': {error}",
                    source.display(),
                    target.display()
                ))
            })?;
        } else if target.exists() {
            fs::remove_file(&target).map_err(|error| {
                OrbitError::Io(format!(
                    "remove auto-publish artifact '{}': {error}",
                    target.display()
                ))
            })?;
        }
    }
    Ok(())
}

fn same_existing_path(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn effective_git_paths(worktree: &Path, paths: &[String]) -> Result<Vec<String>, GitFailure> {
    let mut out = Vec::new();
    for path in paths {
        let full = worktree.join(path);
        if full.exists() || git_path_is_tracked(worktree, path)? {
            out.push(path.clone());
        }
    }
    Ok(out)
}

fn git_path_is_tracked(worktree: &Path, path: &str) -> Result<bool, GitFailure> {
    let args = vec![
        "ls-files".to_string(),
        "--error-unmatch".to_string(),
        "--".to_string(),
        path.to_string(),
    ];
    let output = run_git(worktree, &args, &[]);
    if output.success() {
        return Ok(true);
    }
    if output.code == 1 {
        return Ok(false);
    }
    Err(GitFailure {
        phase: "git ls-files",
        output,
    })
}

fn find_worktree_for_branch(repo_root: &Path, branch: &str) -> Result<PathBuf, OrbitError> {
    let args = vec![
        "worktree".to_string(),
        "list".to_string(),
        "--porcelain".to_string(),
    ];
    let output = run_git(repo_root, &args, &[]);
    if !output.success() {
        return Err(OrbitError::Execution(format!(
            "list git worktrees for auto-publish: {}",
            output.summary()
        )));
    }

    let mut current_worktree: Option<PathBuf> = None;
    for line in output.stdout.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current_worktree = Some(PathBuf::from(path));
            continue;
        }
        if let Some(name) = line.strip_prefix("branch refs/heads/")
            && name == branch
            && let Some(path) = current_worktree.take()
        {
            return Ok(path);
        }
    }

    Err(OrbitError::Execution(format!(
        "auto-publish target branch '{branch}' is not checked out in any worktree"
    )))
}

fn acquire_auto_publish_lock(worktree: &Path) -> Result<AutoPublishLock, OrbitError> {
    let git_dir = git_common_dir(worktree)?;
    let lock_path = git_dir.join("orbit-auto-publish.lock");
    for attempt in 0..20 {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                writeln!(file, "pid={}", std::process::id())
                    .map_err(|error| OrbitError::Io(error.to_string()))?;
                return Ok(AutoPublishLock { path: lock_path });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempt < 19 => {
                thread::sleep(Duration::from_millis(50 * (1 + attempt as u64)));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(OrbitError::Execution(format!(
                    "auto-publish lock '{}' is still held after bounded retry",
                    lock_path.display()
                )));
            }
            Err(error) => {
                return Err(OrbitError::Io(format!(
                    "create auto-publish lock '{}': {error}",
                    lock_path.display()
                )));
            }
        }
    }
    Err(OrbitError::Execution(format!(
        "auto-publish lock '{}' is still held after bounded retry",
        lock_path.display()
    )))
}

fn git_common_dir(worktree: &Path) -> Result<PathBuf, OrbitError> {
    let args = vec!["rev-parse".to_string(), "--git-common-dir".to_string()];
    let output = run_git(worktree, &args, &[]);
    if !output.success() {
        return Err(OrbitError::Execution(format!(
            "resolve git common dir for auto-publish: {}",
            output.summary()
        )));
    }
    let raw = output.stdout.trim();
    if raw.is_empty() {
        return Err(OrbitError::Execution(
            "git returned an empty common dir for auto-publish".to_string(),
        ));
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(worktree.join(path))
    }
}

fn run_git_success(
    worktree: &Path,
    args: &[String],
    env: &[(&'static str, String)],
) -> Result<GitCommandOutput, GitCommandOutput> {
    let output = run_git(worktree, args, env);
    if output.success() {
        Ok(output)
    } else {
        Err(output)
    }
}

fn run_git(worktree: &Path, args: &[String], env: &[(&'static str, String)]) -> GitCommandOutput {
    let mut command = Command::new("git");
    command.arg("-C").arg(worktree).args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    match command.output() {
        Ok(output) => GitCommandOutput {
            code: output.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        },
        Err(error) => GitCommandOutput {
            code: 1,
            stdout: String::new(),
            stderr: format!("spawn git: {error}"),
        },
    }
}

fn git_head(worktree: &Path) -> Result<String, GitCommandOutput> {
    let args = vec!["rev-parse".to_string(), "HEAD".to_string()];
    run_git_success(worktree, &args, &[]).map(|output| output.stdout.trim().to_string())
}

fn git_identity_env(actor: &str) -> Vec<(&'static str, String)> {
    let actor = actor.trim();
    let actor = if actor.is_empty() { "agent" } else { actor };
    vec![
        ("GIT_AUTHOR_NAME", actor.to_string()),
        ("GIT_AUTHOR_EMAIL", format!("{actor}@orbit.local")),
        ("GIT_COMMITTER_NAME", actor.to_string()),
        ("GIT_COMMITTER_EMAIL", format!("{actor}@orbit.local")),
    ]
}

fn commit_actor(runtime: &OrbitRuntime, agent: Option<&str>, model: Option<&str>) -> String {
    agent
        .or(model)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| runtime.actor_label().to_string())
}

fn record_auto_publish_audit(
    runtime: &OrbitRuntime,
    plan: &PublishPlan,
    audit: PublishAudit<'_>,
) -> Result<(), OrbitError> {
    let actor = commit_actor(runtime, audit.agent, audit.model);
    let payload = json!({
        "commit_sha": audit.commit_sha,
        "push_outcome": audit.push_outcome,
        "artifact_id": plan.artifact_id.as_str(),
        "tool_name": plan.tool_name,
        "branch": audit.branch,
        "remote": ORIGIN_REMOTE,
        "paths": &plan.paths,
        "actor": actor,
    });
    let arguments_json = serde_json::to_string(&payload).map_err(|error| {
        OrbitError::Execution(format!("serialize auto-publish audit payload: {error}"))
    })?;
    let status = if audit.push_outcome == PUSH_OUTCOME_PUSHED {
        AuditEventStatus::Success
    } else {
        AuditEventStatus::Failure
    };
    runtime.record_audit_event(&AuditEventInsertParams {
        execution_id: audit_execution_id("artifact-auto-publish"),
        command: "artifact".to_string(),
        subcommand: Some("auto-publish".to_string()),
        tool_name: Some(plan.tool_name.to_string()),
        target_type: Some(AUTO_PUBLISH_TARGET.to_string()),
        target_id: Some(plan.artifact_id.clone()),
        role: actor,
        status,
        exit_code: if status == AuditEventStatus::Success {
            0
        } else {
            1
        },
        duration_ms: 0,
        working_directory: audit.worktree.to_string_lossy().into_owned(),
        arguments_json: Some(arguments_json),
        stdout_truncated: None,
        stderr_truncated: None,
        error_message: audit.error_message.map(ToOwned::to_owned),
        host: std::env::var("HOSTNAME").ok(),
        pid: std::process::id(),
        session_id: None,
        task_id: std::env::var("ORBIT_TASK_ID")
            .ok()
            .filter(|value| !value.is_empty()),
        job_run_id: std::env::var("ORBIT_RUN_ID")
            .ok()
            .filter(|value| !value.is_empty()),
        activity_id: std::env::var("ORBIT_ACTIVITY_ID")
            .ok()
            .filter(|value| !value.is_empty()),
        step_index: std::env::var("ORBIT_STEP_INDEX")
            .ok()
            .and_then(|value| value.parse().ok()),
    })
}

fn existing_adr_paths(runtime: &OrbitRuntime, id: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for status in ["proposed", "accepted", "superseded", "deleted"] {
        let doc = adr_doc_path(status, id);
        if runtime.paths().repo_root.join(&doc).is_file() {
            paths.push(doc);
            paths.push(adr_body_path(status, id));
        }
    }
    paths
}

fn insert_adr_bundle_paths(paths: &mut BTreeSet<String>, status: &str, id: &str) {
    paths.insert(adr_doc_path(status, id));
    paths.insert(adr_body_path(status, id));
}

fn adr_doc_path(status: &str, id: &str) -> String {
    format!(".orbit/adrs/{status}/{id}/adr.yaml")
}

fn adr_body_path(status: &str, id: &str) -> String {
    format!(".orbit/adrs/{status}/{id}/body.md")
}

fn learning_doc_path(id: &str) -> String {
    format!(".orbit/learnings/{id}/learning.yaml")
}

fn learning_comments_path(id: &str) -> String {
    format!(".orbit/learnings/{id}/comments.jsonl")
}

fn find_learning_for_comment(
    runtime: &OrbitRuntime,
    comment_id: &str,
) -> Result<Option<String>, OrbitError> {
    let root = runtime.paths().repo_root.join(".orbit/learnings");
    if !root.is_dir() {
        return Ok(None);
    }
    for entry in fs::read_dir(&root).map_err(|error| OrbitError::Io(error.to_string()))? {
        let entry = entry.map_err(|error| OrbitError::Io(error.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| OrbitError::Io(error.to_string()))?;
        if !file_type.is_dir() {
            continue;
        }
        let Some(learning_id) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        let comments = entry.path().join("comments.jsonl");
        let Ok(raw) = fs::read_to_string(comments) else {
            continue;
        };
        for line in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if value.get("id").and_then(Value::as_str) == Some(comment_id) {
                return Ok(Some(learning_id));
            }
        }
    }
    Ok(None)
}

fn validate_artifact_path(path: &str) -> Result<(), OrbitError> {
    let value = Path::new(path);
    if value.is_absolute() || path.split('/').any(|part| part == "..") {
        return Err(OrbitError::Execution(format!(
            "refusing to auto-publish unsafe artifact path '{path}'"
        )));
    }
    if !(path.starts_with(".orbit/adrs/") || path.starts_with(".orbit/learnings/")) {
        return Err(OrbitError::Execution(format!(
            "refusing to auto-publish non-artifact path '{path}'"
        )));
    }
    Ok(())
}

fn string_field(value: &Value, field: &str) -> Result<String, OrbitError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| OrbitError::Execution(format!("auto-publish response missing `{field}`")))
}

fn input_string(input: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

fn summary_from_text(raw: Option<&str>, fallback: &str) -> String {
    raw.and_then(|value| value.lines().map(str::trim).find(|line| !line.is_empty()))
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| fallback.to_string())
}

fn sanitize_commit_summary(summary: &str) -> String {
    let collapsed = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    let value = if trimmed.is_empty() {
        "artifact update"
    } else {
        trimmed
    };
    let mut out = String::new();
    for ch in value.chars().take(96) {
        out.push(ch);
    }
    out
}

impl GitCommandOutput {
    fn success(&self) -> bool {
        self.code == 0
    }

    fn summary(&self) -> String {
        let stderr = self.stderr.trim();
        if !stderr.is_empty() {
            return stderr.to_string();
        }
        let stdout = self.stdout.trim();
        if !stdout.is_empty() {
            return stdout.to_string();
        }
        format!("git exited with status {}", self.code)
    }

    fn is_non_fast_forward(&self) -> bool {
        let combined = format!("{}\n{}", self.stdout, self.stderr).to_ascii_lowercase();
        combined.contains("non-fast-forward")
            || combined.contains("fetch first")
            || combined.contains("stale info")
    }
}

impl GitFailure {
    fn message(phase: &'static str, message: &str) -> Self {
        Self {
            phase,
            output: GitCommandOutput {
                code: 1,
                stdout: String::new(),
                stderr: message.to_string(),
            },
        }
    }

    fn is_index_lock(&self) -> bool {
        let combined =
            format!("{}\n{}", self.output.stdout, self.output.stderr).to_ascii_lowercase();
        combined.contains("index.lock")
            || combined.contains("unable to create")
            || combined.contains("another git process")
    }

    fn into_orbit_error(self) -> OrbitError {
        OrbitError::Execution(format!(
            "auto-publish {} failed: {}",
            self.phase,
            self.output.summary()
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::thread;

    use orbit_common::types::AuditEventStatus;
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::*;

    const BASE_BRANCH: &str = "agent-main";

    struct GitRepoFixture {
        _temp: TempDir,
        runtime: OrbitRuntime,
        repo: PathBuf,
        remote: PathBuf,
    }

    impl GitRepoFixture {
        fn new(auto_publish: bool) -> Self {
            let temp = tempfile::tempdir().expect("tempdir");
            let global = temp.path().join("global");
            let repo = temp.path().join("repo");
            let remote = temp.path().join("remote.git");
            fs::create_dir_all(&global).expect("global dir");
            fs::create_dir_all(&repo).expect("repo dir");

            git_ok(temp.path(), &["init", "--bare", path_str(&remote)]);
            git_ok(&repo, &["init"]);
            git_ok(&repo, &["checkout", "-b", BASE_BRANCH]);
            git_ok(&repo, &["config", "user.name", "test"]);
            git_ok(&repo, &["config", "user.email", "test@orbit.local"]);

            let orbit_dir = repo.join(".orbit");
            fs::create_dir_all(&orbit_dir).expect("orbit dir");
            fs::write(
                orbit_dir.join("config.toml"),
                format!(
                    "[artifacts]\nauto_publish = {auto_publish}\n\n[workflow]\nbase_branch = \"{BASE_BRANCH}\"\n"
                ),
            )
            .expect("write config");
            fs::write(repo.join("notes.txt"), "clean\n").expect("write notes");

            let runtime = OrbitRuntime::from_roots(&global, &orbit_dir).expect("runtime");
            git_ok(&repo, &["add", "."]);
            git_ok(&repo, &["commit", "-m", "initial"]);
            git_ok(&repo, &["remote", "add", ORIGIN_REMOTE, path_str(&remote)]);
            git_ok(&repo, &["push", "-u", ORIGIN_REMOTE, BASE_BRANCH]);

            Self {
                _temp: temp,
                runtime,
                repo,
                remote,
            }
        }

        fn add_learning(&self, summary: &str) -> Result<Value, OrbitError> {
            self.runtime.execute_tool_command(
                "orbit.learning.add",
                json!({
                    "summary": summary,
                    "scope": { "paths": ["crates/**"] },
                    "model": "codex",
                }),
                None,
                None,
            )
        }

        fn execute_tool_as_codex(&self, name: &str, input: Value) -> Result<Value, OrbitError> {
            self.runtime
                .execute_tool_command(name, input, None, Some("codex".to_string()))
        }

        fn add_accepted_adr(&self, title: &str) -> String {
            let adr = self
                .execute_tool_as_codex(
                    "orbit.adr.add",
                    json!({
                        "title": title,
                        "body": format!(
                            "## Context\n{title} exists.\n\n## Decision\nPublish it.\n\n## Consequences\n- It reaches git history.\n"
                        ),
                        "related_features": ["task-artifacts"],
                    }),
                )
                .expect("add adr");
            let id = adr["id"].as_str().expect("adr id").to_string();
            self.execute_tool_as_codex(
                "orbit.adr.update",
                json!({
                    "id": id,
                    "status": "accepted",
                    "related_tasks": ["ORB-00136"],
                }),
            )
            .expect("accept adr");
            id
        }

        fn head(&self) -> String {
            git_ok(&self.repo, &["rev-parse", "HEAD"])
        }

        fn remote_head(&self) -> String {
            git_dir_ok(&self.remote, &["rev-parse", BASE_BRANCH])
        }
    }

    #[test]
    fn disabled_auto_publish_leaves_learning_unstaged_and_uncommitted() {
        let fixture = GitRepoFixture::new(false);
        let before = fixture.head();

        let response = fixture
            .add_learning("Keep manual publication")
            .expect("add learning");
        let id = response["id"].as_str().expect("learning id");

        assert_eq!(fixture.head(), before);
        assert_eq!(
            git_ok(&fixture.repo, &["diff", "--cached", "--name-only"]),
            ""
        );
        let status = git_ok(&fixture.repo, &["status", "--short", ".orbit/learnings"]);
        assert!(
            status.contains(".orbit/learnings/"),
            "expected unstaged learning dir for {id}, got {status:?}"
        );
    }

    #[test]
    fn enabled_auto_publish_commits_pushes_and_preserves_unrelated_dirty_file() {
        let fixture = GitRepoFixture::new(true);
        fs::write(fixture.repo.join("notes.txt"), "dirty\n").expect("dirty notes");

        let response = fixture
            .add_learning("Publish only the learning path")
            .expect("add learning");
        let id = response["id"].as_str().expect("learning id");
        let local_head = fixture.head();

        assert_eq!(
            git_ok(&fixture.repo, &["branch", "--show-current"]),
            BASE_BRANCH
        );
        assert_eq!(fixture.remote_head(), local_head);
        assert_eq!(
            git_ok(&fixture.repo, &["log", "-1", "--format=%an %ae"]),
            "codex codex@orbit.local"
        );
        let subject = git_ok(&fixture.repo, &["log", "-1", "--format=%s"]);
        assert!(
            subject.starts_with(&format!("docs: Add learning {id} — ")),
            "unexpected subject: {subject}"
        );
        assert_eq!(
            git_ok(&fixture.repo, &["show", "--name-only", "--format=", "HEAD"]),
            format!(".orbit/learnings/{id}/learning.yaml")
        );
        assert_eq!(
            git_ok(&fixture.repo, &["diff", "--cached", "--name-only"]),
            ""
        );
        assert_eq!(
            git_ok(&fixture.repo, &["status", "--short", "notes.txt"]),
            "M notes.txt"
        );

        let events = fixture
            .runtime
            .list_audit_events(
                None,
                Some("orbit.learning.add".to_string()),
                Some(AuditEventStatus::Success),
                None,
                16,
            )
            .expect("audit events");
        let event = events
            .iter()
            .find(|event| event.target_type.as_deref() == Some(AUTO_PUBLISH_TARGET))
            .expect("auto-publish audit");
        let payload: Value =
            serde_json::from_str(event.arguments_json.as_deref().expect("arguments json"))
                .expect("parse audit payload");
        assert_eq!(payload["commit_sha"], local_head);
        assert_eq!(payload["push_outcome"], PUSH_OUTCOME_PUSHED);
        assert_eq!(payload["artifact_id"], id);
        assert_eq!(payload["tool_name"], "orbit.learning.add");
    }

    #[test]
    fn non_fast_forward_push_fetches_rebases_and_retries_once() {
        let fixture = GitRepoFixture::new(true);
        let concurrent = fixture.seed_concurrent_remote_commit();

        fixture
            .add_learning("Retry after concurrent remote publication")
            .expect("add learning after remote race");

        assert_eq!(fixture.remote_head(), fixture.head());
        git_ok(
            &fixture.repo,
            &["merge-base", "--is-ancestor", &concurrent, "HEAD"],
        );
        let subjects = git_ok(&fixture.repo, &["log", "--format=%s", "-2"]);
        assert!(
            subjects.contains("concurrent artifact commit"),
            "expected rebased history to include concurrent commit: {subjects}"
        );
    }

    #[test]
    fn missing_base_branch_worktree_returns_error_after_writing_artifact() {
        let fixture = GitRepoFixture::new(true);
        let before = fixture.head();
        git_ok(&fixture.repo, &["checkout", "-b", "feature-only"]);

        let error = fixture
            .add_learning("No checked out base branch")
            .expect_err("missing base branch worktree should fail");

        let message = error.to_string();
        assert!(message.contains(BASE_BRANCH), "{message}");
        assert!(
            message.contains("not checked out in any worktree"),
            "{message}"
        );
        assert_eq!(fixture.head(), before);
        let status = git_ok(&fixture.repo, &["status", "--short", ".orbit/learnings"]);
        assert!(status.contains(".orbit/learnings/"), "{status}");
    }

    #[test]
    fn failing_pre_commit_hook_leaves_artifact_staged_but_uncommitted() {
        let fixture = GitRepoFixture::new(true);
        let before = fixture.head();
        fixture.write_hook("pre-commit", "echo pre-commit failed >&2\nexit 1\n");

        let error = fixture
            .add_learning("Pre-commit hook should block")
            .expect_err("pre-commit hook should fail");

        let message = error.to_string();
        assert!(message.contains("git commit"), "{message}");
        assert!(message.contains("pre-commit failed"), "{message}");
        assert_eq!(fixture.head(), before);
        let staged = git_ok(&fixture.repo, &["diff", "--cached", "--name-only"]);
        assert!(staged.contains(".orbit/learnings/"), "{staged}");
    }

    #[test]
    fn failing_pre_push_hook_leaves_local_commit_unpushed() {
        let fixture = GitRepoFixture::new(true);
        let before_local = fixture.head();
        let before_remote = fixture.remote_head();
        fixture.write_hook("pre-push", "echo pre-push failed >&2\nexit 1\n");

        let error = fixture
            .add_learning("Pre-push hook should block")
            .expect_err("pre-push hook should fail");

        let message = error.to_string();
        assert!(message.contains("push"), "{message}");
        assert!(message.contains("pre-push failed"), "{message}");
        assert_ne!(fixture.head(), before_local);
        assert_eq!(fixture.remote_head(), before_remote);
    }

    #[test]
    fn remote_rejection_leaves_local_commit_unpushed() {
        let fixture = GitRepoFixture::new(true);
        let before_local = fixture.head();
        let before_remote = fixture.remote_head();
        fixture.write_remote_hook("pre-receive", "echo auth failed >&2\nexit 1\n");

        let error = fixture
            .add_learning("Remote auth failure keeps local commit")
            .expect_err("remote hook should reject push");

        let message = error.to_string();
        assert!(message.contains("push"), "{message}");
        assert!(message.contains("auth failed"), "{message}");
        assert_ne!(fixture.head(), before_local);
        assert_eq!(fixture.remote_head(), before_remote);
    }

    #[test]
    fn rebase_conflict_preserves_local_commit_and_reports_branch() {
        let fixture = GitRepoFixture::new(true);
        let learning = fixture
            .add_learning("Conflict base summary")
            .expect("seed learning");
        let id = learning["id"].as_str().expect("learning id").to_string();
        let remote_commit = fixture.seed_conflicting_remote_learning_update(&id);

        let error = fixture
            .execute_tool_as_codex(
                "orbit.learning.update",
                json!({
                    "id": id,
                    "summary": "Local conflict summary",
                }),
            )
            .expect_err("rebase conflict should fail auto-publish");

        let message = error.to_string();
        assert!(message.contains("rebase"), "{message}");
        assert!(message.contains(BASE_BRANCH), "{message}");
        assert!(message.contains("local commit is preserved"), "{message}");
        assert_eq!(fixture.remote_head(), remote_commit);
        let subject = git_ok(&fixture.repo, &["log", "-1", "--format=%s"]);
        assert!(
            subject.starts_with(&format!("docs: Update learning {id} — ")),
            "unexpected local subject: {subject}"
        );
    }

    #[test]
    fn learning_supersede_and_comments_publish_expected_paths() {
        let fixture = GitRepoFixture::new(true);
        let old = fixture.add_learning("Old learning").expect("old learning");
        let old_id = old["id"].as_str().expect("old learning id").to_string();
        let new = fixture.add_learning("New learning").expect("new learning");
        let new_id = new["id"].as_str().expect("new learning id").to_string();

        let comment = fixture
            .execute_tool_as_codex(
                "orbit.learning.comment.add",
                json!({
                    "learning_id": old_id,
                    "body": "Comment auto-publishes with the parent learning.",
                    "model": "codex",
                }),
            )
            .expect("add comment");
        let comment_id = comment["id"].as_str().expect("comment id").to_string();
        assert_eq!(
            git_ok(&fixture.repo, &["show", "--name-only", "--format=", "HEAD"]),
            format!(".orbit/learnings/{old_id}/comments.jsonl")
        );

        fixture
            .execute_tool_as_codex(
                "orbit.learning.comment.delete",
                json!({
                    "id": comment_id,
                    "model": "codex",
                }),
            )
            .expect("delete comment");
        assert_eq!(
            git_ok(&fixture.repo, &["show", "--name-only", "--format=", "HEAD"]),
            format!(".orbit/learnings/{old_id}/comments.jsonl")
        );

        fixture
            .execute_tool_as_codex(
                "orbit.learning.supersede",
                json!({
                    "id": old_id,
                    "with": new_id,
                }),
            )
            .expect("supersede learning");
        assert_eq!(fixture.remote_head(), fixture.head());
        let names = git_ok(&fixture.repo, &["show", "--name-only", "--format=", "HEAD"]);
        for expected in [
            format!(".orbit/learnings/{old_id}/learning.yaml"),
            format!(".orbit/learnings/{new_id}/learning.yaml"),
        ] {
            assert!(
                names.lines().any(|line| line == expected),
                "missing {expected} from changed paths: {names}"
            );
        }
    }

    #[test]
    fn adr_supersede_publishes_old_and_new_bundles() {
        let fixture = GitRepoFixture::new(true);
        let old_id = fixture.add_accepted_adr("Old auto-publish ADR");
        let new_id = fixture.add_accepted_adr("New auto-publish ADR");

        fixture
            .execute_tool_as_codex(
                "orbit.adr.supersede",
                json!({
                    "old_id": old_id,
                    "new_id": new_id,
                }),
            )
            .expect("supersede adr");

        assert_eq!(fixture.remote_head(), fixture.head());
        let names = git_ok(
            &fixture.repo,
            &["show", "--name-only", "--format=", "--no-renames", "HEAD"],
        );
        for expected in [
            format!(".orbit/adrs/accepted/{old_id}/adr.yaml"),
            format!(".orbit/adrs/accepted/{old_id}/body.md"),
            format!(".orbit/adrs/superseded/{old_id}/adr.yaml"),
            format!(".orbit/adrs/superseded/{old_id}/body.md"),
            format!(".orbit/adrs/accepted/{new_id}/adr.yaml"),
        ] {
            assert!(
                names.lines().any(|line| line == expected),
                "missing {expected} from changed paths: {names}"
            );
        }
    }

    #[test]
    fn parallel_invocations_serialize_commits_and_pushes() {
        let fixture = GitRepoFixture::new(true);
        let first = fixture
            .add_learning("Parallel learning one")
            .expect("first learning");
        let first_id = first["id"].as_str().expect("first learning id").to_string();
        let second = fixture
            .add_learning("Parallel learning two")
            .expect("second learning");
        let second_id = second["id"]
            .as_str()
            .expect("second learning id")
            .to_string();
        let before = fixture.head();

        let runtime_one = fixture.runtime.clone();
        let update_one = thread::spawn(move || {
            runtime_one.execute_tool_command(
                "orbit.learning.update",
                json!({
                    "id": first_id,
                    "summary": "Parallel learning one updated",
                }),
                None,
                Some("codex".to_string()),
            )
        });
        let runtime_two = fixture.runtime.clone();
        let update_two = thread::spawn(move || {
            runtime_two.execute_tool_command(
                "orbit.learning.update",
                json!({
                    "id": second_id,
                    "summary": "Parallel learning two updated",
                }),
                None,
                Some("codex".to_string()),
            )
        });

        update_one
            .join()
            .expect("first thread")
            .expect("first update");
        update_two
            .join()
            .expect("second thread")
            .expect("second update");

        assert_eq!(fixture.remote_head(), fixture.head());
        assert_eq!(
            git_ok(
                &fixture.repo,
                &["rev-list", "--count", &format!("{before}..HEAD")]
            ),
            "2"
        );
        assert!(!fixture.repo.join(".git/index.lock").exists());
    }

    #[test]
    fn adr_status_update_commits_moved_bundle_paths() {
        let fixture = GitRepoFixture::new(true);
        let adr = fixture
            .runtime
            .execute_tool_command(
                "orbit.adr.add",
                json!({
                    "title": "Auto-publish ADR moves",
                    "body": "## Context\nA decision exists.\n\n## Decision\nPublish it.\n\n## Consequences\n- The ADR is visible.\n- Cost: Git history gets a commit.\n",
                    "related_features": ["task-artifacts"],
                    "model": "codex",
                }),
                None,
                None,
            )
            .expect("add adr");
        let id = adr["id"].as_str().expect("adr id").to_string();

        fixture
            .runtime
            .execute_tool_command(
                "orbit.adr.update",
                json!({
                    "id": id,
                    "status": "accepted",
                    "related_tasks": ["ORB-00136"],
                    "model": "codex",
                }),
                None,
                None,
            )
            .expect("accept adr");

        assert_eq!(fixture.remote_head(), fixture.head());
        let names = git_ok(
            &fixture.repo,
            &["show", "--name-only", "--format=", "--no-renames", "HEAD"],
        );
        for expected in [
            format!(".orbit/adrs/accepted/{id}/adr.yaml"),
            format!(".orbit/adrs/accepted/{id}/body.md"),
            format!(".orbit/adrs/proposed/{id}/adr.yaml"),
            format!(".orbit/adrs/proposed/{id}/body.md"),
        ] {
            assert!(
                names.lines().any(|line| line == expected),
                "missing {expected} from changed paths: {names}"
            );
        }
    }

    impl GitRepoFixture {
        fn seed_concurrent_remote_commit(&self) -> String {
            let clone = self
                ._temp
                .path()
                .join(format!("concurrent-{}", std::process::id()));
            git_ok(
                self._temp.path(),
                &[
                    "clone",
                    "--branch",
                    BASE_BRANCH,
                    path_str(&self.remote),
                    path_str(&clone),
                ],
            );
            git_ok(&clone, &["config", "user.name", "other"]);
            git_ok(&clone, &["config", "user.email", "other@orbit.local"]);
            fs::write(clone.join("concurrent.txt"), "remote\n").expect("write concurrent");
            git_ok(&clone, &["add", "concurrent.txt"]);
            git_ok(&clone, &["commit", "-m", "concurrent artifact commit"]);
            git_ok(&clone, &["push", ORIGIN_REMOTE, BASE_BRANCH]);
            git_ok(&clone, &["rev-parse", "HEAD"])
        }

        fn seed_conflicting_remote_learning_update(&self, id: &str) -> String {
            let clone = self
                ._temp
                .path()
                .join(format!("conflict-{}", std::process::id()));
            git_ok(
                self._temp.path(),
                &[
                    "clone",
                    "--branch",
                    BASE_BRANCH,
                    path_str(&self.remote),
                    path_str(&clone),
                ],
            );
            git_ok(&clone, &["config", "user.name", "other"]);
            git_ok(&clone, &["config", "user.email", "other@orbit.local"]);
            let rel_path = format!(".orbit/learnings/{id}/learning.yaml");
            let path = clone.join(&rel_path);
            let raw = fs::read_to_string(&path).expect("read learning yaml");
            let updated = raw.replacen(
                "summary: Conflict base summary",
                "summary: Remote conflict summary",
                1,
            );
            assert_ne!(raw, updated, "expected summary line in learning yaml");
            fs::write(&path, updated).expect("write remote conflict");
            git_ok(&clone, &["add", &rel_path]);
            git_ok(
                &clone,
                &["commit", "-m", "remote conflicting learning edit"],
            );
            git_ok(&clone, &["push", ORIGIN_REMOTE, BASE_BRANCH]);
            git_ok(&clone, &["rev-parse", "HEAD"])
        }

        fn write_hook(&self, name: &str, body: &str) {
            let hook = self.repo.join(".git/hooks").join(name);
            fs::write(&hook, format!("#!/bin/sh\n{body}")).expect("write hook");
            let mut permissions = fs::metadata(&hook).expect("hook metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&hook, permissions).expect("set hook permissions");
        }

        fn write_remote_hook(&self, name: &str, body: &str) {
            let hook = self.remote.join("hooks").join(name);
            fs::write(&hook, format!("#!/bin/sh\n{body}")).expect("write remote hook");
            let mut permissions = fs::metadata(&hook)
                .expect("remote hook metadata")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&hook, permissions).expect("set remote hook permissions");
        }
    }

    fn git_ok(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git -C {} {} failed\nstdout:\n{}\nstderr:\n{}",
            cwd.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn git_dir_ok(git_dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("--git-dir")
            .arg(git_dir)
            .args(args)
            .output()
            .expect("run git-dir");
        assert!(
            output.status.success(),
            "git --git-dir {} {} failed\nstdout:\n{}\nstderr:\n{}",
            git_dir.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn path_str(path: &Path) -> &str {
        path.to_str().expect("utf8 path")
    }
}
