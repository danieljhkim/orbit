//! Shared JSON serializers for the `orbit learning` CLI surface.
//!
//! These mirror the host-side serializers in
//! `orbit_core::runtime::orbit_tool_host::json` so CLI output matches the
//! `orbit.learning.*` MCP tool output byte-for-byte (per the CLI-parity
//! acceptance criterion).

use orbit_core::{
    EvidenceKind, Learning, LearningComment, LearningSearchResult, LearningVoteSummary,
};
use serde_json::{Value, json};

pub(crate) fn learning_to_json(learning: &Learning) -> Value {
    json!({
        "id": learning.id,
        "status": learning.status.as_str(),
        "scope": {
            "paths": learning.scope.paths,
            "tags": learning.scope.tags,
            "symbols": learning.scope.symbols,
            "semantic_seed": learning.scope.semantic_seed,
        },
        "summary": learning.summary,
        "body": learning.body,
        "evidence": learning
            .evidence
            .iter()
            .map(|e| json!({"kind": evidence_kind_str(e.kind), "ref": e.reference}))
            .collect::<Vec<_>>(),
        "supersedes": learning.supersedes,
        "superseded_by": learning.superseded_by,
        "created_at": learning.created_at.to_rfc3339(),
        "updated_at": learning.updated_at.to_rfc3339(),
        "created_by": learning.created_by,
        "priority": learning.priority,
    })
}

pub(crate) fn learning_show_to_json(
    learning: &Learning,
    vote_summary: &LearningVoteSummary,
) -> Value {
    let mut value = learning_to_json(learning);
    if let Some(object) = value.as_object_mut() {
        object.insert("vote_count".to_string(), json!(vote_summary.vote_count));
        object.insert(
            "last_voted_at".to_string(),
            vote_summary
                .last_voted_at
                .map(|ts| json!(ts.to_rfc3339()))
                .unwrap_or(Value::Null),
        );
    }
    value
}

pub(crate) fn learning_search_result_to_json(result: &LearningSearchResult) -> Value {
    let learning = &result.learning;
    json!({
        "id": learning.id,
        "summary": learning.summary,
        "scope": {
            "paths": learning.scope.paths,
            "tags": learning.scope.tags,
        },
        "updated_at": learning.updated_at.to_rfc3339(),
        "priority": learning.priority,
        "matched_by": result.matched_by,
    })
}

pub(crate) fn learning_comment_to_json(comment: &LearningComment) -> Value {
    json!({
        "id": comment.id,
        "learning_id": comment.learning_id,
        "body": comment.body,
        "author_model": comment.author_model,
        "created_at": comment.created_at.to_rfc3339(),
    })
}

fn evidence_kind_str(kind: EvidenceKind) -> &'static str {
    match kind {
        EvidenceKind::Task => "task",
        EvidenceKind::Commit => "commit",
        EvidenceKind::External => "external",
    }
}
