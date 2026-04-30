use chrono::Utc;
use orbit_common::types::{
    OrbitError, ReviewMessage, ReviewThread, ReviewThreadStatus, infer_agent_family_from_model,
};
use orbit_store::task_review_scoreboard;

use crate::OrbitRuntime;

use super::helpers::effective_actor_label;
use super::params::TaskUpdateParams;

impl OrbitRuntime {
    pub fn add_review_thread(
        &self,
        task_id: &str,
        body: String,
        path: Option<String>,
        line: Option<u64>,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<ReviewThread, OrbitError> {
        let (canonical_agent, canonical_model) =
            self.try_canonical_agent_model_identity(agent.as_deref(), model.as_deref())?;
        let actor = self.actor().clone();
        let effective_label = effective_actor_label(
            &actor.label,
            canonical_agent.as_deref(),
            canonical_model.as_deref(),
        );

        let now = Utc::now();
        let nanos_suffix = now.timestamp_subsec_nanos() % 10000;
        let thread_id = format!("rt-{}-{:04}", now.format("%Y%m%d-%H%M%S"), nanos_suffix);
        let message_id = format!("rm-{}-{:04}", now.format("%Y%m%d-%H%M%S"), nanos_suffix);

        let thread = ReviewThread {
            thread_id: thread_id.clone(),
            path,
            line,
            status: ReviewThreadStatus::Open,
            messages: vec![ReviewMessage {
                message_id,
                at: now,
                by: effective_label.clone(),
                body,
                github_comment_id: None,
            }],
            github_thread_id: None,
        };

        self.update_task_with_identity(
            task_id,
            TaskUpdateParams {
                append_review_threads: vec![thread.clone()],
                ..Default::default()
            },
            canonical_agent,
            canonical_model.clone(),
        )?;

        self.record_task_review_message_score(canonical_model.as_deref());
        Ok(thread)
    }

    pub fn list_review_threads(
        &self,
        task_id: &str,
        status_filter: Option<ReviewThreadStatus>,
    ) -> Result<Vec<ReviewThread>, OrbitError> {
        let task = self.get_task(task_id)?;
        let threads = if let Some(status) = status_filter {
            task.review_threads
                .into_iter()
                .filter(|t| t.status == status)
                .collect()
        } else {
            task.review_threads
        };
        Ok(threads)
    }

    pub fn reply_review_thread(
        &self,
        task_id: &str,
        thread_id: &str,
        body: String,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<ReviewThread, OrbitError> {
        let (canonical_agent, canonical_model) =
            self.try_canonical_agent_model_identity(agent.as_deref(), model.as_deref())?;
        let task = self.get_task(task_id)?;
        let existing = task
            .review_threads
            .iter()
            .find(|t| t.thread_id == thread_id)
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "review thread '{thread_id}' not found on task '{task_id}'"
                ))
            })?;

        let actor = self.actor().clone();
        let effective_label = effective_actor_label(
            &actor.label,
            canonical_agent.as_deref(),
            canonical_model.as_deref(),
        );

        let now = Utc::now();
        let nanos_suffix = now.timestamp_subsec_nanos() % 10000;
        let message_id = format!("rm-{}-{:04}", now.format("%Y%m%d-%H%M%S"), nanos_suffix);

        let reply_thread = ReviewThread {
            thread_id: thread_id.to_string(),
            path: None,
            line: None,
            status: existing.status,
            messages: vec![ReviewMessage {
                message_id,
                at: now,
                by: effective_label.clone(),
                body,
                github_comment_id: None,
            }],
            github_thread_id: None,
        };

        self.update_task_with_identity(
            task_id,
            TaskUpdateParams {
                append_review_threads: vec![reply_thread],
                ..Default::default()
            },
            canonical_agent,
            canonical_model.clone(),
        )?;

        self.record_task_review_message_score(canonical_model.as_deref());
        let updated_task = self.get_task(task_id)?;
        updated_task
            .review_threads
            .into_iter()
            .find(|t| t.thread_id == thread_id)
            .ok_or_else(|| {
                OrbitError::Execution("review thread disappeared after reply".to_string())
            })
    }

    pub fn resolve_review_thread(
        &self,
        task_id: &str,
        thread_id: &str,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<ReviewThread, OrbitError> {
        let task = self.get_task(task_id)?;
        let _existing = task
            .review_threads
            .iter()
            .find(|t| t.thread_id == thread_id)
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "review thread '{thread_id}' not found on task '{task_id}'"
                ))
            })?;

        let resolve_thread = ReviewThread {
            thread_id: thread_id.to_string(),
            path: None,
            line: None,
            status: ReviewThreadStatus::Resolved,
            messages: vec![],
            github_thread_id: None,
        };

        self.update_task_with_identity(
            task_id,
            TaskUpdateParams {
                append_review_threads: vec![resolve_thread],
                ..Default::default()
            },
            agent,
            model,
        )?;

        let updated_task = self.get_task(task_id)?;
        updated_task
            .review_threads
            .into_iter()
            .find(|t| t.thread_id == thread_id)
            .ok_or_else(|| {
                OrbitError::Execution("review thread disappeared after resolve".to_string())
            })
    }

    fn record_task_review_message_score(&self, model: Option<&str>) {
        if !self.scoring_enabled() {
            return;
        }
        let Some(model) = model else {
            return;
        };
        if infer_agent_family_from_model(model).is_none() {
            return;
        }
        if let Err(error) =
            task_review_scoreboard::record_task_review_message(&self.paths().scoreboard_dir, model)
        {
            tracing::warn!(
                target: "orbit.scoreboard.task_review",
                model = %model,
                error = %error,
                "failed to record task review scoreboard message",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::Value;
    use tempfile::tempdir;

    use super::super::params::TaskAddParams;
    use super::*;

    fn test_runtime() -> (tempfile::TempDir, OrbitRuntime) {
        let root = tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("create global root");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
        (root, runtime)
    }

    fn read_task_review_scoreboard(runtime: &OrbitRuntime) -> Value {
        let raw = fs::read_to_string(
            runtime
                .data_root()
                .join("state")
                .join("scoreboard")
                .join("task_review.json"),
        )
        .expect("read task review scoreboard");
        serde_json::from_str(&raw).expect("parse task review scoreboard")
    }

    #[test]
    fn add_and_reply_review_threads_score_local_review_messages() {
        let (_root, runtime) = test_runtime();
        let scoreboard_dir = runtime.data_root().join("state").join("scoreboard");
        fs::create_dir_all(&scoreboard_dir).expect("create scoreboard dir");
        let task = runtime
            .add_task(TaskAddParams {
                title: "Review scoring".to_string(),
                description: "Exercise local review-thread scoring.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");

        let thread = runtime
            .add_review_thread(
                &task.id,
                "Initial review note.".to_string(),
                Some("src/lib.rs".to_string()),
                Some(12),
                Some("codex".to_string()),
                Some("gpt-reviewer".to_string()),
            )
            .expect("add review thread");
        runtime
            .reply_review_thread(
                &task.id,
                &thread.thread_id,
                "Follow-up review note.".to_string(),
                Some("codex".to_string()),
                Some("gpt-reviewer".to_string()),
            )
            .expect("reply review thread");

        let scoreboard = read_task_review_scoreboard(&runtime);
        assert_eq!(
            scoreboard["task-review-messages"]["gpt-reviewer"],
            Value::from(2)
        );
        let updated = runtime.get_task(&task.id).expect("reload task");
        let first_message = updated.review_threads[0]
            .messages
            .first()
            .expect("first review message");
        assert_eq!(first_message.by, "gpt-reviewer");
        assert!(first_message.github_comment_id.is_none());

        let summary = runtime
            .generate_scoreboard_summary()
            .expect("generate scoreboard summary");
        let reviewer = summary
            .agents
            .get("gpt-reviewer")
            .expect("reviewer summary");
        assert_eq!(reviewer.task_review.messages, 2);
        assert_eq!(reviewer.pr.review_comments, 0);
    }

    #[test]
    fn human_review_threads_do_not_score_local_review_messages() {
        let (_root, runtime) = test_runtime();
        let scoreboard_dir = runtime.data_root().join("state").join("scoreboard");
        fs::create_dir_all(&scoreboard_dir).expect("create scoreboard dir");
        let task = runtime
            .add_task(TaskAddParams {
                title: "Human review".to_string(),
                description: "Exercise human local review-thread scoring.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");

        runtime
            .add_review_thread(
                &task.id,
                "Human review note.".to_string(),
                None,
                None,
                None,
                None,
            )
            .expect("add review thread");

        assert!(!scoreboard_dir.join("task_review.json").exists());
    }

    #[test]
    fn non_model_labels_do_not_score_local_review_messages() {
        let (_root, runtime) = test_runtime();
        let scoreboard_dir = runtime.data_root().join("state").join("scoreboard");
        fs::create_dir_all(&scoreboard_dir).expect("create scoreboard dir");
        let task = runtime
            .add_task(TaskAddParams {
                title: "Non-model review".to_string(),
                description: "Exercise non-model label scoring skip.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");

        runtime
            .add_review_thread(
                &task.id,
                "Human-attributed review.".to_string(),
                None,
                None,
                None,
                Some("human".to_string()),
            )
            .expect("add review thread with model=human");

        runtime
            .add_review_thread(
                &task.id,
                "Person-attributed review.".to_string(),
                None,
                None,
                None,
                Some("daniel".to_string()),
            )
            .expect("add review thread with model=daniel");

        assert!(!scoreboard_dir.join("task_review.json").exists());

        let updated = runtime.get_task(&task.id).expect("reload task");
        assert_eq!(updated.review_threads.len(), 2);
        assert_eq!(updated.review_threads[0].messages[0].by, "human");
        assert_eq!(updated.review_threads[1].messages[0].by, "daniel");
    }
}
