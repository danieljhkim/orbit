//! Unit tests for `embedder` — sibling layout under tests/.

use super::super::embedder::ModelSpec;

#[test]
fn model_aliases_parse() {
    assert_eq!(ModelSpec::parse("bge-small").unwrap().dim, 384);
    assert_eq!(
        ModelSpec::parse("NomicEmbedTextV15").unwrap().alias,
        "nomic-v1.5"
    );
    assert!(ModelSpec::parse("unknown").is_err());
}
