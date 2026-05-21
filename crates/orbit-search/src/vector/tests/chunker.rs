//! Unit tests for `chunker` — sibling layout under vector/tests/.

use super::super::chunker::chunk_text;
use crate::NoopEmbedder;

#[test]
fn paragraph_chunker_overlaps_at_boundaries() {
    let embedder = NoopEmbedder::new("noop", 3, 64);
    let text = "one two three\n\nfour five six\n\nseven eight nine";
    let chunks = chunk_text(text, &embedder, 5, 3).unwrap();

    assert_eq!(chunks.len(), 3);
    assert!(chunks[0].contains("one two three"));
    assert!(chunks[1].contains("one two three"));
    assert!(chunks[1].contains("four five six"));
    assert!(chunks[2].contains("four five six"));
}
