//! Unit tests for `upsert` — sibling layout under store/tests/.

use crate::NoopEmbedder;
use crate::vector::{EmbeddingField, VectorStore};

#[test]
fn upsert_embeddings_skips_unchanged_content_hashes() {
    let store = VectorStore::open_in_memory().unwrap();
    let embedder = NoopEmbedder::small();
    let fields = vec![EmbeddingField::new("purpose", "same content")];

    let first = store
        .upsert_embeddings("task", "T1", &fields, &embedder, false)
        .unwrap();
    let second = store
        .upsert_embeddings("task", "T1", &fields, &embedder, false)
        .unwrap();

    assert_eq!(first.embedded_chunks, 1);
    assert_eq!(second.embedded_chunks, 0);
    assert_eq!(second.skipped_fields, 1);
}
