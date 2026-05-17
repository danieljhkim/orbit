use serde_json::{Value, json};

use orbit_common::types::{
    AgentModelPair, OrbitError, ReviewThread, all_agent_families, normalize_attribution_label,
};
use orbit_store::pr_scoreboard;

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use crate::executor::automation::input::required_job_run_id;

use super::client::{GhClient, RealGhClient};
use super::patch_match::{PrFilePatchMap, patch_supports_right_side_line};

pub(crate) fn sync_batch_review_to_github<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = required_job_run_id(input, "sync_batch_review_to_github")?;

    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id), None, None)?;
    let mut total: u64 = 0;

    for task in &batch_tasks {
        if task.github_pr_number().is_none() {
            continue;
        }
        if host.get_task_review_threads(&task.id)?.is_empty() {
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

    let Some(pr_number) = task.github_pr_number() else {
        return Ok(0);
    };

    let mut threads = host.get_task_review_threads(task_id)?;
    if threads.is_empty() {
        return Ok(0);
    }

    let repo_root = host.repo_root()?;

    let owner_repo = gh.get_owner_repo(&repo_root)?;
    let head_sha = gh.get_pr_head_sha(&repo_root, pr_number)?;
    // If patch metadata can't be resolved, fall back to general PR comments
    // instead of failing the entire review sync run.
    let pr_file_patches = gh
        .load_pr_file_patches(&repo_root, &owner_repo, pr_number)
        .unwrap_or_default();

    let mut synced_count: u64 = 0;

    for thread in threads.iter_mut() {
        let pending_labels = pending_sync_message_labels(thread);
        let thread_synced = sync_thread(
            gh,
            &repo_root,
            &owner_repo,
            pr_number,
            &head_sha,
            &pr_file_patches,
            thread,
        )?;
        synced_count += thread_synced;

        if host.scoring_enabled() {
            for label in pending_labels {
                if let Some(model) = scoreable_review_model(host, &label)
                    && let Err(error) =
                        pr_scoreboard::record_pr_review_comment(host.scoreboard_dir(), &model)
                {
                    tracing::warn!(
                        target: "orbit.scoreboard.pr",
                        model = %model,
                        error = %error,
                        "failed to record PR review comment scoreboard message",
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

fn scoreable_review_model<H: RuntimeHost + ?Sized>(host: &H, label: &str) -> Option<String> {
    if let Some((agent, model)) = parse_agent_model_label(label) {
        let model = host
            .canonical_model_name(agent, Some(model))
            .unwrap_or_else(|| model.to_string());
        return scoreable_configured_model(host.resolved_agent_model_pair(agent), &model);
    }

    let label = label.trim();
    if label.is_empty()
        || label.eq_ignore_ascii_case("human")
        || label.eq_ignore_ascii_case("system")
    {
        return None;
    }
    let model = normalize_attribution_label(label, None);
    scoreable_known_model(host, &model)
}

fn scoreable_known_model<H: RuntimeHost + ?Sized>(host: &H, model: &str) -> Option<String> {
    all_agent_families().into_iter().find_map(|family| {
        scoreable_configured_model(host.resolved_agent_model_pair(family), model)
    })
}

fn scoreable_configured_model(pair: Option<AgentModelPair>, model: &str) -> Option<String> {
    let pair = pair?;
    let model = model.trim();
    if model.eq_ignore_ascii_case(&pair.orchestrator) {
        return Some(pair.orchestrator);
    }
    if model.eq_ignore_ascii_case(&pair.helper) {
        return Some(pair.helper);
    }
    None
}

fn render_general_comment_body(path: Option<&str>, line: Option<u64>, body: &str) -> String {
    match (path, line) {
        (Some(path), Some(line)) => format!("On `{path}:{line}`:\n\n{body}"),
        _ => body.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::fs;
    use std::path::{Path, PathBuf};

    use chrono::Utc;
    use orbit_common::types::{
        Activity, ExternalRef, Job, JobTargetType, OrbitEvent, Role, Task, TaskArtifact,
        TaskPriority, TaskStatus, TaskType,
    };
    use orbit_common::types::{ReviewMessage, ReviewThread, ReviewThreadStatus};
    use orbit_store::task_review_scoreboard;
    use orbit_tools::ToolContext;
    use serde_json::Value;
    use tempfile::tempdir;

    use crate::context::{
        JobRunResult, RuntimeHost, TaskActivityUpdate, TaskReadHost, TaskWriteHost,
    };
    use crate::executor::registry::ActivityExecutorRegistry;

    use super::*;

    struct TestHost {
        task: RefCell<Task>,
        review_threads: RefCell<Vec<ReviewThread>>,
        data_root: PathBuf,
        scoreboard_dir: PathBuf,
        registry: ActivityExecutorRegistry,
    }

    impl TestHost {
        fn new(task: Task, data_root: PathBuf, scoreboard_dir: PathBuf) -> Self {
            Self {
                task: RefCell::new(task),
                review_threads: RefCell::new(fixture_review_threads(Utc::now())),
                data_root,
                scoreboard_dir,
                registry: ActivityExecutorRegistry::default(),
            }
        }
    }

    impl TaskReadHost for TestHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            let task = self.task.borrow();
            if task.id == task_id {
                Ok(task.clone())
            } else {
                Err(OrbitError::InvalidInput(format!(
                    "unknown task '{task_id}'"
                )))
            }
        }

        fn get_task_artifacts(&self, _task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
            Ok(Vec::new())
        }

        fn get_task_review_threads(&self, _task_id: &str) -> Result<Vec<ReviewThread>, OrbitError> {
            Ok(self.review_threads.borrow().clone())
        }

        fn list_tasks_filtered(
            &self,
            _status: Option<TaskStatus>,
            _priority: Option<TaskPriority>,
            _parent_id: Option<&str>,
            _batch_id: Option<&str>,
            _external_ref: Option<&orbit_common::types::ExternalRef>,
            _has_external_ref_system: Option<&str>,
        ) -> Result<Vec<Task>, OrbitError> {
            Ok(vec![self.task.borrow().clone()])
        }
    }

    impl TaskWriteHost for TestHost {
        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not needed by review sync tests")
        }

        fn admit_task_for_workflow(
            &self,
            _task_id: &str,
            _workflow: &str,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not needed by review sync tests")
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _update: TaskActivityUpdate,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not needed by review sync tests")
        }

        fn apply_task_automation_update(
            &self,
            _task_id: &str,
            update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            if let Some(review_threads) = update.review_threads {
                *self.review_threads.borrow_mut() = review_threads;
            }
            Ok(())
        }
    }

    impl RuntimeHost for TestHost {
        fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
            Ok(())
        }

        fn repo_root(&self) -> Result<String, OrbitError> {
            Ok(self.data_root.to_str().expect("utf-8 temp dir").to_string())
        }

        fn data_root(&self) -> &Path {
            &self.data_root
        }

        fn activity_executor_registry(&self) -> &ActivityExecutorRegistry {
            &self.registry
        }

        fn run_job_now_with_input_debug(
            &self,
            _job_id: &str,
            _input: Value,
            _debug: bool,
        ) -> Result<JobRunResult, OrbitError> {
            unimplemented!("not needed by review sync tests")
        }

        fn validate_activity_target_exists(
            &self,
            _target_type: JobTargetType,
            _target_id: &str,
        ) -> Result<Activity, OrbitError> {
            unimplemented!("not needed by review sync tests")
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
            unimplemented!("not needed by review sync tests")
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

        fn resolved_agent_model_pair(&self, agent_cli: &str) -> Option<AgentModelPair> {
            match agent_cli {
                "codex" => Some(AgentModelPair::new("gpt-5.4", "gpt-5.4-mini")),
                "claude" => Some(AgentModelPair::new("opus-4.6", "sonnet-4.6")),
                "gemini" => Some(AgentModelPair::new(
                    "gemini-3.1-pro-preview",
                    "gemini-3-flash-preview",
                )),
                "grok" => Some(AgentModelPair::new("grok-4", "grok-3")),
                _ => None,
            }
        }

        fn scoring_enabled(&self) -> bool {
            true
        }

        fn graph_editing(&self) -> bool {
            false
        }

        fn scoreboard_dir(&self) -> &Path {
            &self.scoreboard_dir
        }
    }

    struct TestGhClient {
        next_id: Cell<u64>,
    }

    impl TestGhClient {
        fn new() -> Self {
            Self {
                next_id: Cell::new(10),
            }
        }

        fn next_comment_id(&self) -> u64 {
            let id = self.next_id.get();
            self.next_id.set(id + 1);
            id
        }
    }

    impl GhClient for TestGhClient {
        fn get_owner_repo(&self, _repo_root: &str) -> Result<String, OrbitError> {
            Ok("owner/repo".to_string())
        }

        fn get_pr_head_sha(
            &self,
            _repo_root: &str,
            _pr_number: &str,
        ) -> Result<String, OrbitError> {
            Ok("abc123".to_string())
        }

        fn load_pr_file_patches(
            &self,
            _repo_root: &str,
            _owner_repo: &str,
            _pr_number: &str,
        ) -> Result<PrFilePatchMap, OrbitError> {
            Ok(PrFilePatchMap::default())
        }

        fn create_inline_review_comment(
            &self,
            _repo_root: &str,
            _owner_repo: &str,
            _pr_number: &str,
            _commit_id: &str,
            _path: &str,
            _line: u64,
            _body: &str,
        ) -> Result<u64, OrbitError> {
            Ok(self.next_comment_id())
        }

        fn create_general_comment(
            &self,
            _repo_root: &str,
            _pr_number: &str,
            _body: &str,
        ) -> Result<u64, OrbitError> {
            Ok(self.next_comment_id())
        }

        fn create_reply_comment(
            &self,
            _repo_root: &str,
            _owner_repo: &str,
            _pr_number: &str,
            _parent_comment_id: u64,
            _body: &str,
        ) -> Result<u64, OrbitError> {
            Ok(self.next_comment_id())
        }
    }

    fn fixture_task(_repo_root: &Path) -> Task {
        let now = Utc::now();
        Task {
            id: "T-review-sync".to_string(),
            title: "Review sync".to_string(),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            tags: Vec::new(),
            plan: String::new(),
            execution_summary: String::new(),
            context_files: Vec::new(),
            created_by: Some("gpt-5.4".to_string()),
            planned_by: None,
            implemented_by: None,
            status: TaskStatus::Review,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Chore,
            pr_status: None,
            external_refs: vec![ExternalRef::github_pr("42").expect("github pr ref")],
            relations: Vec::new(),
            job_run_id: None,
            crew: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn fixture_review_threads(now: chrono::DateTime<Utc>) -> Vec<ReviewThread> {
        vec![ReviewThread {
            thread_id: "rt-test".to_string(),
            path: None,
            line: None,
            status: ReviewThreadStatus::Open,
            messages: vec![ReviewMessage {
                message_id: "rm-test".to_string(),
                at: now,
                by: "gpt-5.4".to_string(),
                body: "Review note.".to_string(),
                github_comment_id: None,
            }],
            github_thread_id: None,
        }]
    }

    fn read_scoreboard(scoreboard_dir: &Path, file_name: &str) -> Value {
        let raw = fs::read_to_string(scoreboard_dir.join(file_name)).expect("read scoreboard");
        serde_json::from_str(&raw).expect("parse scoreboard")
    }

    #[test]
    fn github_sync_counts_pr_review_without_incrementing_task_review_again() {
        let temp = tempdir().expect("create tempdir");
        let scoreboard_dir = temp.path().join("scoreboard");
        fs::create_dir_all(&scoreboard_dir).expect("create scoreboard dir");
        // orbit-core covers `add_review_thread` persisting this model-only,
        // pending-sync shape. orbit-engine starts from the persisted shape to
        // avoid a reverse dependency on orbit-core in this crate.
        task_review_scoreboard::record_task_review_thread(&scoreboard_dir, "gpt-5.4")
            .expect("seed local review score");

        let task = fixture_task(temp.path());
        let host = TestHost::new(task, temp.path().to_path_buf(), scoreboard_dir.clone());
        let gh = TestGhClient::new();

        let synced =
            sync_task_review_to_github_with_client(&host, &gh, "T-review-sync").expect("sync");
        assert_eq!(synced, 1);

        let task_review = read_scoreboard(&scoreboard_dir, "task_review.json");
        assert_eq!(
            task_review["task-review-threads"]["gpt-5.4"],
            Value::from(1)
        );
        let pr = read_scoreboard(&scoreboard_dir, "pr.json");
        assert_eq!(pr["pr-review-comments"]["gpt-5.4"], Value::from(1));

        let synced_again =
            sync_task_review_to_github_with_client(&host, &gh, "T-review-sync").expect("resync");
        assert_eq!(synced_again, 0);
        let task_review = read_scoreboard(&scoreboard_dir, "task_review.json");
        assert_eq!(
            task_review["task-review-threads"]["gpt-5.4"],
            Value::from(1)
        );
        let pr = read_scoreboard(&scoreboard_dir, "pr.json");
        assert_eq!(pr["pr-review-comments"]["gpt-5.4"], Value::from(1));
    }

    #[test]
    fn scoreable_review_model_only_scores_configured_models() {
        let temp = tempdir().expect("create tempdir");
        let host = TestHost::new(
            fixture_task(temp.path()),
            temp.path().to_path_buf(),
            temp.path().join("scoreboard"),
        );

        assert_eq!(
            scoreable_review_model(&host, "gpt-5.4").as_deref(),
            Some("gpt-5.4")
        );
        assert_eq!(
            scoreable_review_model(&host, "codex / gpt-5.4").as_deref(),
            Some("gpt-5.4")
        );
        assert_eq!(scoreable_review_model(&host, "gpt-typo"), None);
        assert_eq!(scoreable_review_model(&host, "opus-handle"), None);
        assert_eq!(scoreable_review_model(&host, "codex / gpt-typo"), None);
        assert_eq!(scoreable_review_model(&host, "human"), None);
        assert_eq!(scoreable_review_model(&host, "system"), None);
        assert_eq!(scoreable_review_model(&host, "daniel"), None);
    }

    #[test]
    fn scoreable_review_model_scores_grok_threads() {
        let temp = tempdir().expect("create tempdir");
        let host = TestHost::new(
            fixture_task(temp.path()),
            temp.path().to_path_buf(),
            temp.path().join("scoreboard"),
        );

        assert_eq!(
            scoreable_review_model(&host, "grok-4").as_deref(),
            Some("grok-4")
        );
        assert_eq!(
            scoreable_review_model(&host, "grok / grok-4").as_deref(),
            Some("grok-4")
        );
        assert_eq!(
            scoreable_review_model(&host, "grok / grok-3").as_deref(),
            Some("grok-3")
        );
    }
}
