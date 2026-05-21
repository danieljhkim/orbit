//! Unit tests for `doc_fields` — sibling layout under vector/tests/.

use super::super::doc_fields::{DocEmbeddingSource, doc_embedding_fields};

#[test]
fn doc_embedding_fields_use_stable_names() {
    let source = DocEmbeddingSource {
        path: "docs/example.md".to_string(),
        title: "Example Summary".to_string(),
        tags: vec!["search".to_string(), "docs".to_string()],
        body: "Body text".to_string(),
    };

    let field_names = doc_embedding_fields(&source)
        .into_iter()
        .map(|field| field.field)
        .collect::<Vec<_>>();

    assert_eq!(field_names, vec!["path", "title", "tags", "body"]);
}
