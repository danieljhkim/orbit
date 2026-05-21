//! Unit tests for `adr_fields` — sibling layout under vector/tests/.

use super::super::adr_fields::{AdrEmbeddingSource, adr_embedding_fields};

#[test]
fn adr_embedding_fields_use_stable_section_names() {
    let source = AdrEmbeddingSource {
        id: "ADR-0001".to_string(),
        title: "Keep task FTS task-bound".to_string(),
        body: "## Context\nTasks need isolated lexical rows.\n\n## Decision\nKeep task FTS task-bound.\n\n## Consequences\nDocs use corpus_fts.\n".to_string(),
        tags: vec!["search".to_string(), "adr".to_string()],
    };

    let field_names = adr_embedding_fields(&source)
        .into_iter()
        .map(|field| field.field)
        .collect::<Vec<_>>();

    assert_eq!(
        field_names,
        vec!["title", "decision", "context", "consequences", "tags"]
    );
}

#[test]
fn adr_embedding_fields_skip_status_and_legacy_metadata() {
    let source = AdrEmbeddingSource {
        id: "ADR-0002".to_string(),
        title: "Decision only".to_string(),
        body: "No canonical sections yet.".to_string(),
        tags: Vec::new(),
    };

    let field_names = adr_embedding_fields(&source)
        .into_iter()
        .map(|field| field.field)
        .collect::<Vec<_>>();

    assert_eq!(field_names, vec!["title"]);
}
