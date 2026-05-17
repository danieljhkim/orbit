// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::fs;

use orbit_common::types::{LearningCommentEvent, OrbitError};

use super::super::layout::{
    comments_jsonl_path, validate_learning_comment_id, validate_learning_id,
};
use super::super::record::read_comment_events;

/// Validate and trim a learning comment body (≤ 500 chars, non-empty after trim).
pub(super) fn validate_learning_comment_body(raw: &str) -> Result<String, OrbitError> {
    let body = raw.trim().to_string();
    if body.is_empty() {
        return Err(OrbitError::InvalidInput(
            "learning comment body must not be empty".to_string(),
        ));
    }
    let count = body.chars().count();
    if count > 500 {
        return Err(OrbitError::InvalidInput(format!(
            "learning comment body must be at most 500 characters (got {count})"
        )));
    }
    Ok(body)
}

/// Validate and trim a learning comment author model (non-empty after trim).
pub(super) fn validate_learning_comment_model(raw: &str) -> Result<String, OrbitError> {
    let model = raw.trim().to_string();
    if model.is_empty() {
        return Err(OrbitError::InvalidInput(
            "learning comment requires a non-empty model".to_string(),
        ));
    }
    Ok(model)
}

/// Validate all comment JSONL files under the learning root (used by reindex).
pub(super) fn validate_comment_files(root: &std::path::Path) -> Result<(), OrbitError> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|err| OrbitError::Io(err.to_string()))? {
        let entry = entry.map_err(|err| OrbitError::Io(err.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|err| OrbitError::Io(err.to_string()))?;
        if !file_type.is_dir() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if validate_learning_id(&id).is_err() {
            continue;
        }
        let path = comments_jsonl_path(root, &id);
        for event in read_comment_events(&path)? {
            match event {
                LearningCommentEvent::Create(comment) => {
                    validate_learning_comment_id(&comment.id)?;
                    if comment.learning_id != id {
                        return Err(OrbitError::Store(format!(
                            "invalid learning comment file {}: comment '{}' belongs to '{}'",
                            path.display(),
                            comment.id,
                            comment.learning_id
                        )));
                    }
                    validate_learning_comment_body(&comment.body)?;
                    validate_learning_comment_model(&comment.author_model)?;
                }
                LearningCommentEvent::Tombstone(tombstone) => {
                    validate_learning_comment_id(&tombstone.id)?;
                    if tombstone.learning_id != id {
                        return Err(OrbitError::Store(format!(
                            "invalid learning comment file {}: tombstone '{}' belongs to '{}'",
                            path.display(),
                            tombstone.id,
                            tombstone.learning_id
                        )));
                    }
                    if tombstone.op != "delete" {
                        return Err(OrbitError::Store(format!(
                            "invalid learning comment file {}: tombstone '{}' has op '{}'",
                            path.display(),
                            tombstone.id,
                            tombstone.op
                        )));
                    }
                    validate_learning_comment_model(&tombstone.deleted_by)?;
                }
            }
        }
    }
    Ok(())
}
