//! Shared JSON serializers for the `orbit learning` CLI surface.
//!
//! These mirror the host-side serializers in
//! `orbit_core::runtime::orbit_tool_host::json` so CLI output matches the
//! `orbit.learning.*` MCP tool output byte-for-byte (per the CLI-parity
//! acceptance criterion).

use orbit_core::{EvidenceKind, Learning};
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
        "legacy_ids": learning.legacy_ids,
        "created_at": learning.created_at.to_rfc3339(),
        "updated_at": learning.updated_at.to_rfc3339(),
        "created_by": learning.created_by,
        "priority": learning.priority,
    })
}

pub(crate) fn learning_show_to_json(learning: &Learning) -> Value {
    learning_to_json(learning)
}

fn evidence_kind_str(kind: EvidenceKind) -> &'static str {
    match kind {
        EvidenceKind::Task => "task",
        EvidenceKind::Commit => "commit",
        EvidenceKind::External => "external",
    }
}
