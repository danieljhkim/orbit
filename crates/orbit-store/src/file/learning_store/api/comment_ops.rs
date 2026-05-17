// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use chrono::{DateTime, Utc};
use orbit_common::types::{
    LearningComment, LearningCommentEvent, LearningCommentTombstone, LearningStatus, NotFoundKind,
    OrbitError,
};

use super::super::layout::{
    comments_jsonl_path, locate_learning, next_learning_comment_id, validate_learning_comment_id,
    validate_learning_id,
};
use super::super::lock::{acquire_learning_allocation_lock, acquire_learning_lock};
use super::super::record::{
    append_jsonl_comment_row, lookup_learning_comment, read_learning_file, scan_learning_comments,
};
use super::store::LearningFileStore;
use super::validation::{validate_learning_comment_body, validate_learning_comment_model};
use crate::backend::{LearningCommentAddParams, LearningCommentDeleteParams};

impl LearningFileStore {
    pub(crate) fn add_learning_comment(
        &self,
        params: LearningCommentAddParams,
    ) -> Result<LearningComment, OrbitError> {
        self.add_learning_comment_at(params, Utc::now())
    }

    #[cfg(test)]
    pub(crate) fn add_learning_comment_at(
        &self,
        params: LearningCommentAddParams,
        now: DateTime<Utc>,
    ) -> Result<LearningComment, OrbitError> {
        self.add_learning_comment_at_impl(params, now)
    }

    #[cfg(not(test))]
    fn add_learning_comment_at(
        &self,
        params: LearningCommentAddParams,
        now: DateTime<Utc>,
    ) -> Result<LearningComment, OrbitError> {
        self.add_learning_comment_at_impl(params, now)
    }

    fn add_learning_comment_at_impl(
        &self,
        params: LearningCommentAddParams,
        now: DateTime<Utc>,
    ) -> Result<LearningComment, OrbitError> {
        validate_learning_id(&params.learning_id)?;
        let body = validate_learning_comment_body(&params.body)?;
        let author_model = validate_learning_comment_model(&params.author_model)?;

        let Some(path) = locate_learning(&self.root, &params.learning_id)? else {
            return Err(OrbitError::not_found(
                NotFoundKind::Learning,
                params.learning_id,
            ));
        };
        let learning = read_learning_file(&path)?;
        if learning.status == LearningStatus::Superseded {
            return Err(OrbitError::InvalidInput(format!(
                "learning '{}' is superseded; use orbit.learning.supersede for the parent-replacement workflow",
                learning.id
            )));
        }

        let _allocation_lock = acquire_learning_allocation_lock(&self.root)?;
        let _lock = acquire_learning_lock(&self.root, &learning.id)?;

        let Some(path) = locate_learning(&self.root, &learning.id)? else {
            return Err(OrbitError::not_found(NotFoundKind::Learning, learning.id));
        };
        let learning = read_learning_file(&path)?;
        if learning.status == LearningStatus::Superseded {
            return Err(OrbitError::InvalidInput(format!(
                "learning '{}' is superseded; use orbit.learning.supersede for the parent-replacement workflow",
                learning.id
            )));
        }

        let comment = LearningComment {
            id: next_learning_comment_id(&self.root, now)?,
            learning_id: learning.id.clone(),
            body,
            author_model,
            created_at: now,
        };
        append_jsonl_comment_row(
            &comments_jsonl_path(&self.root, &learning.id),
            &LearningCommentEvent::Create(comment.clone()),
        )?;
        Ok(comment)
    }

    pub(crate) fn list_learning_comments(
        &self,
        learning_id: &str,
        include_deleted: bool,
    ) -> Result<Vec<LearningComment>, OrbitError> {
        validate_learning_id(learning_id)?;
        if locate_learning(&self.root, learning_id)?.is_none() {
            return Err(OrbitError::not_found(
                NotFoundKind::Learning,
                learning_id.to_string(),
            ));
        }
        scan_learning_comments(
            &comments_jsonl_path(&self.root, learning_id),
            include_deleted,
        )
    }

    pub(crate) fn delete_learning_comment(
        &self,
        params: LearningCommentDeleteParams,
    ) -> Result<(), OrbitError> {
        validate_learning_comment_id(&params.comment_id)?;
        let deleted_by = validate_learning_comment_model(&params.deleted_by)?;
        let Some(parent_id) = self.find_learning_for_comment(&params.comment_id)? else {
            return Err(OrbitError::not_found(
                NotFoundKind::LearningComment,
                params.comment_id,
            ));
        };
        let _lock = acquire_learning_lock(&self.root, &parent_id.learning_id)?;
        let path = comments_jsonl_path(&self.root, &parent_id.learning_id);
        let Some(lookup) = lookup_learning_comment(&path, &params.comment_id)? else {
            return Err(OrbitError::not_found(
                NotFoundKind::LearningComment,
                params.comment_id,
            ));
        };
        if lookup.deleted {
            return Ok(());
        }
        append_jsonl_comment_row(
            &path,
            &LearningCommentEvent::Tombstone(LearningCommentTombstone {
                id: params.comment_id,
                learning_id: lookup.learning_id,
                op: "delete".to_string(),
                deleted_at: Utc::now(),
                deleted_by,
            }),
        )
    }

    fn find_learning_for_comment(
        &self,
        comment_id: &str,
    ) -> Result<Option<super::super::record::LearningCommentLookup>, OrbitError> {
        if !self.root.exists() {
            return Ok(None);
        }
        for entry in std::fs::read_dir(&self.root).map_err(|e| OrbitError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let file_type = entry
                .file_type()
                .map_err(|e| OrbitError::Io(e.to_string()))?;
            if !file_type.is_dir() {
                continue;
            }
            let Some(id) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if validate_learning_id(&id).is_err() {
                continue;
            }
            let path = comments_jsonl_path(&self.root, &id);
            if let Some(lookup) = lookup_learning_comment(&path, comment_id)? {
                return Ok(Some(lookup));
            }
        }
        Ok(None)
    }
}
