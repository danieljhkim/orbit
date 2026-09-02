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

/// Flushing the buffer ahead of an over-long paragraph must reset the token
/// counter with it. It used to keep the weight of an overlap tail that was
/// then discarded, so the paragraph after the long one was flushed alone as a
/// spurious chunk and embedded twice.
#[test]
fn counter_resets_after_a_long_paragraph_is_split_on_its_own() {
    let embedder = NoopEmbedder::new("noop", 3, 64);
    let long = "l1 l2 l3 l4 l5 l6 l7 l8 l9 l10 l11 l12";
    let text = format!("a b c d\n\n{long}\n\ne f\n\ng h\n\ni j");
    let chunks = chunk_text(&text, &embedder, 5, 3).unwrap();

    assert!(
        !chunks.iter().any(|chunk| chunk == "e f"),
        "the paragraph after the split one was flushed alone: {chunks:?}"
    );
    assert!(
        chunks.iter().any(|chunk| chunk == "e f\n\ng h"),
        "short paragraphs after the split one pack together again: {chunks:?}"
    );
    assert_eq!(chunks[0], "a b c d");
}
