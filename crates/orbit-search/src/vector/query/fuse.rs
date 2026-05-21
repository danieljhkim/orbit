use std::collections::BTreeMap;

use super::{Bm25Hit, CosineHit};

pub const RRF_K: f32 = 60.0;

#[derive(Debug, Clone, PartialEq)]
pub struct FusedCandidate {
    pub source_kind: String,
    pub source_id: String,
    pub field: String,
    pub chunk_idx_for_snippet: Option<usize>,
    pub rowid_for_snippet: Option<i64>,
    pub score: f32,
    pub bm25_rank: Option<usize>,
    pub cosine_rank: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CandidateKey {
    source_kind: String,
    source_id: String,
    field: String,
}

pub fn reciprocal_rank_fusion(cosine: &[CosineHit], bm25: &[Bm25Hit]) -> Vec<FusedCandidate> {
    let mut by_key = BTreeMap::<CandidateKey, FusedCandidate>::new();
    for hit in cosine {
        let entry = by_key
            .entry(CandidateKey {
                source_kind: hit.source_kind.clone(),
                source_id: hit.source_id.clone(),
                field: hit.field.clone(),
            })
            .or_insert_with(|| FusedCandidate {
                source_kind: hit.source_kind.clone(),
                source_id: hit.source_id.clone(),
                field: hit.field.clone(),
                chunk_idx_for_snippet: Some(hit.chunk_idx),
                rowid_for_snippet: None,
                score: 0.0,
                bm25_rank: None,
                cosine_rank: None,
            });
        if should_replace_rank(entry.cosine_rank, hit.rank) {
            if let Some(rank) = entry.cosine_rank {
                entry.score -= rrf_contribution(rank);
            }
            entry.score += rrf_contribution(hit.rank);
            entry.cosine_rank = Some(hit.rank);
            entry.chunk_idx_for_snippet = Some(hit.chunk_idx);
        }
    }
    for hit in bm25 {
        let entry = by_key
            .entry(CandidateKey {
                source_kind: hit.source_kind.clone(),
                source_id: hit.source_id.clone(),
                field: hit.field.clone(),
            })
            .or_insert_with(|| FusedCandidate {
                source_kind: hit.source_kind.clone(),
                source_id: hit.source_id.clone(),
                field: hit.field.clone(),
                chunk_idx_for_snippet: None,
                rowid_for_snippet: Some(hit.rowid),
                score: 0.0,
                bm25_rank: None,
                cosine_rank: None,
            });
        if should_replace_rank(entry.bm25_rank, hit.rank) {
            if let Some(rank) = entry.bm25_rank {
                entry.score -= rrf_contribution(rank);
            }
            entry.score += rrf_contribution(hit.rank);
            entry.bm25_rank = Some(hit.rank);
            entry.rowid_for_snippet = Some(hit.rowid);
        }
    }
    let mut candidates = by_key.into_values().collect::<Vec<_>>();
    candidates.sort_by(compare_fused_candidates);
    candidates
}

pub(crate) fn compare_fused_candidates(
    left: &FusedCandidate,
    right: &FusedCandidate,
) -> std::cmp::Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.source_kind.cmp(&right.source_kind))
        .then_with(|| left.source_id.cmp(&right.source_id))
        .then_with(|| left.field.cmp(&right.field))
}

pub(crate) fn rrf_contribution(rank: usize) -> f32 {
    1.0 / (RRF_K + rank as f32)
}

fn should_replace_rank(current: Option<usize>, candidate: usize) -> bool {
    current.is_none_or(|rank| candidate < rank)
}
