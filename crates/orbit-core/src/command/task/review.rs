use chrono::Utc;
use orbit_types::{OrbitError, ReviewMessage, ReviewThread, ReviewThreadStatus};

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
        let actor = self.actor().clone();
        let effective_label =
            effective_actor_label(&actor.label, agent.as_deref(), model.as_deref());

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
            agent,
            model,
        )?;

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
        let effective_label =
            effective_actor_label(&actor.label, agent.as_deref(), model.as_deref());

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
            agent,
            model,
        )?;

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
}
