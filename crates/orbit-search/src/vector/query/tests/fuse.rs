//! Unit tests for `fuse` — sibling layout under vector/query/tests/.

use std::collections::BTreeMap;

use super::super::fuse::reciprocal_rank_fusion;
use super::super::{Bm25Hit, CosineHit};

fn cosine(id: &str, rank: usize) -> CosineHit {
    CosineHit {
        source_kind: "task".to_string(),
        source_id: id.to_string(),
        field: "purpose".to_string(),
        chunk_idx: 0,
        score: 1.0 / rank as f32,
        rank,
    }
}

fn bm25(id: &str, rank: usize) -> Bm25Hit {
    Bm25Hit {
        source_kind: "task".to_string(),
        source_id: id.to_string(),
        field: "purpose".to_string(),
        rowid: rank as i64,
        rank,
    }
}

#[test]
fn rrf_reproduces_hand_computed_rank_example() {
    let fused = reciprocal_rank_fusion(
        &[cosine("A", 1), cosine("B", 2), cosine("C", 3)],
        &[bm25("B", 1), bm25("A", 2), bm25("D", 3)],
    );

    let by_id = fused
        .iter()
        .map(|hit| (hit.source_id.as_str(), hit.score))
        .collect::<BTreeMap<_, _>>();
    let ab_expected = (1.0 / 61.0) + (1.0 / 62.0);
    let cd_expected = 1.0 / 63.0;
    assert!((by_id["A"] - ab_expected).abs() < 0.000001);
    assert!((by_id["B"] - ab_expected).abs() < 0.000001);
    assert!((by_id["C"] - cd_expected).abs() < 0.000001);
    assert!((by_id["D"] - cd_expected).abs() < 0.000001);
    assert_eq!(
        fused
            .iter()
            .map(|hit| hit.source_id.as_str())
            .collect::<Vec<_>>(),
        vec!["A", "B", "C", "D"]
    );
}

#[test]
fn rrf_counts_one_rank_per_retriever_per_field() {
    let mut lower_chunk = cosine("A", 2);
    lower_chunk.chunk_idx = 1;
    let fused = reciprocal_rank_fusion(&[cosine("A", 1), lower_chunk], &[]);

    assert_eq!(fused.len(), 1);
    assert!((fused[0].score - (1.0 / 61.0)).abs() < 0.000001);
    assert_eq!(fused[0].cosine_rank, Some(1));
    assert_eq!(fused[0].chunk_idx_for_snippet, Some(0));
}
