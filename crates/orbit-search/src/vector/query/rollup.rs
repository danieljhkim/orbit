use std::collections::BTreeMap;

use super::FusedCandidate;

#[derive(Debug, Clone, PartialEq)]
pub struct TaskHit {
    pub source_kind: String,
    pub source_id: String,
    pub best_field: String,
    pub best_chunk_idx: Option<usize>,
    pub best_rowid: Option<i64>,
    pub score: f32,
    pub bm25_rank: Option<usize>,
    pub cosine_rank: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TaskKey {
    source_kind: String,
    source_id: String,
}

pub fn rollup_to_tasks(candidates: Vec<FusedCandidate>, limit: usize) -> Vec<TaskHit> {
    if limit == 0 {
        return Vec::new();
    }

    let mut by_task = BTreeMap::<TaskKey, TaskHit>::new();
    for candidate in candidates {
        let key = TaskKey {
            source_kind: candidate.source_kind.clone(),
            source_id: candidate.source_id.clone(),
        };
        let hit = TaskHit {
            source_kind: candidate.source_kind,
            source_id: candidate.source_id,
            best_field: candidate.field,
            best_chunk_idx: candidate.chunk_idx_for_snippet,
            best_rowid: candidate.rowid_for_snippet,
            score: candidate.score,
            bm25_rank: candidate.bm25_rank,
            cosine_rank: candidate.cosine_rank,
        };
        by_task
            .entry(key)
            .and_modify(|current| {
                if compare_task_hits(&hit, current).is_lt() {
                    *current = hit.clone();
                }
            })
            .or_insert(hit);
    }

    let mut hits = by_task.into_values().collect::<Vec<_>>();
    hits.sort_by(compare_task_hits);
    hits.truncate(limit);
    hits
}

fn compare_task_hits(left: &TaskHit, right: &TaskHit) -> std::cmp::Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.source_kind.cmp(&right.source_kind))
        .then_with(|| left.source_id.cmp(&right.source_id))
        .then_with(|| left.best_field.cmp(&right.best_field))
}
