//! Unit tests for `learning_fields` — sibling layout under vector/tests/.

use super::super::learning_fields::{LearningEmbeddingSource, learning_embedding_fields};

#[test]
fn learning_embedding_fields_use_stable_section_names() {
    let source = LearningEmbeddingSource {
        id: "L-0001".to_string(),
        summary: "Use async-aware locks".to_string(),
        body: "## Rule\nNever hold std Mutex across await.\n\n## Why\nIt can deadlock.\n\n## How to apply\nDrop the guard before await.\n".to_string(),
        tags: vec!["rust".to_string(), "async".to_string()],
    };

    let field_names = learning_embedding_fields(&source)
        .into_iter()
        .map(|field| field.field)
        .collect::<Vec<_>>();

    assert_eq!(
        field_names,
        vec!["summary", "rule", "why", "how_to_apply", "tags"]
    );
}

#[test]
fn learning_embedding_fields_fallback_to_details_for_plain_body() {
    let source = LearningEmbeddingSource {
        id: "L-0002".to_string(),
        summary: "Keep fixtures tracked".to_string(),
        body: "Avoid .orbit fixture paths.".to_string(),
        tags: Vec::new(),
    };

    let field_names = learning_embedding_fields(&source)
        .into_iter()
        .map(|field| field.field)
        .collect::<Vec<_>>();

    assert_eq!(field_names, vec!["summary", "details"]);
}
