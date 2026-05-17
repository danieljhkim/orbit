// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use chrono::DateTime;
use orbit_common::types::{
    LearningStatus, LearningVoteRow, LearningVoteSummary, NotFoundKind, OrbitError,
};

use super::super::layout::{locate_learning, validate_learning_id, votes_jsonl_path};
use super::super::lock::acquire_learning_lock;
use super::super::votes::{append_vote_row, read_vote_rows, summarize_votes};
use super::store::LearningFileStore;
use crate::backend::LearningUpvoteParams;

impl LearningFileStore {
    pub(crate) fn upvote_learning(
        &self,
        params: LearningUpvoteParams,
    ) -> Result<LearningVoteSummary, OrbitError> {
        self.upvote_learning_at(params, chrono::Utc::now())
    }

    #[cfg(test)]
    pub(crate) fn upvote_learning_at(
        &self,
        params: LearningUpvoteParams,
        now: DateTime<chrono::Utc>,
    ) -> Result<LearningVoteSummary, OrbitError> {
        self.upvote_learning_at_impl(params, now)
    }

    #[cfg(not(test))]
    fn upvote_learning_at(
        &self,
        params: LearningUpvoteParams,
        now: DateTime<chrono::Utc>,
    ) -> Result<LearningVoteSummary, OrbitError> {
        self.upvote_learning_at_impl(params, now)
    }

    fn upvote_learning_at_impl(
        &self,
        params: LearningUpvoteParams,
        now: DateTime<chrono::Utc>,
    ) -> Result<LearningVoteSummary, OrbitError> {
        validate_learning_id(&params.learning_id)?;
        let Some(path) = locate_learning(&self.root, &params.learning_id)? else {
            return Err(OrbitError::not_found(
                NotFoundKind::Learning,
                params.learning_id,
            ));
        };
        let learning = super::super::record::read_learning_file(&path)?;
        if learning.status == LearningStatus::Superseded {
            return Err(OrbitError::InvalidInput(format!(
                "learning '{}' is superseded; use the superseding learning before voting",
                learning.id
            )));
        }

        let task_id = params
            .task_id
            .map(|task_id| task_id.trim().to_string())
            .filter(|task_id| !task_id.is_empty())
            .ok_or_else(|| {
                OrbitError::InvalidInput(
                    "learning upvote requires `task_id` in v1; free-floating votes are rejected by policy"
                        .to_string(),
                )
            })?;
        let voter_model = params.voter_model.trim().to_string();
        if voter_model.is_empty() {
            return Err(OrbitError::InvalidInput(
                "learning upvote requires a non-empty voter model".to_string(),
            ));
        }

        let _lock = acquire_learning_lock(&self.root, &learning.id)?;
        let votes_path = votes_jsonl_path(&self.root, &learning.id);
        let rows = read_vote_rows(&votes_path)?;
        let already_voted = rows.iter().any(|row| {
            row.learning_id == learning.id
                && row.voter_model == voter_model
                && row.task_id.as_deref() == Some(task_id.as_str())
        });
        if already_voted {
            return Ok(summarize_votes(&rows));
        }

        let mut next_rows = rows;
        let row = LearningVoteRow {
            learning_id: learning.id,
            voter_model,
            voted_at: now,
            task_id: Some(task_id),
        };
        append_vote_row(&votes_path, &row)?;
        next_rows.push(row);
        Ok(summarize_votes(&next_rows))
    }

    pub(crate) fn learning_vote_summary(
        &self,
        id: &str,
    ) -> Result<LearningVoteSummary, OrbitError> {
        validate_learning_id(id)?;
        if locate_learning(&self.root, id)?.is_none() {
            return Err(OrbitError::not_found(
                NotFoundKind::Learning,
                id.to_string(),
            ));
        }
        let rows = read_vote_rows(&votes_jsonl_path(&self.root, id))?;
        Ok(summarize_votes(&rows))
    }
}
