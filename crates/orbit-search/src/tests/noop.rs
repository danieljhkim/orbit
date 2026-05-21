//! Unit tests for `noop` — sibling layout under tests/.

use super::super::noop::NoopEmbedder;
use crate::embedder::Embedder;

#[test]
fn noop_embedder_is_deterministic_and_normalized() {
    let embedder = NoopEmbedder::small();
    let vectors = embedder.embed(&["alpha", "alpha", "beta"]).unwrap();
    assert_eq!(vectors[0], vectors[1]);
    assert_ne!(vectors[0], vectors[2]);
    let norm = vectors[0]
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    assert!((norm - 1.0).abs() < 0.0001);
}
