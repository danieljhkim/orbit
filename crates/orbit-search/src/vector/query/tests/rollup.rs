//! Unit tests for `rollup` — sibling layout under vector/query/tests/.

use super::super::FusedCandidate;
use super::super::rollup::rollup_to_tasks;

fn candidate(id: &str, field: &str, score: f32) -> FusedCandidate {
    FusedCandidate {
        source_kind: "task".to_string(),
        source_id: id.to_string(),
        field: field.to_string(),
        chunk_idx_for_snippet: Some(0),
        rowid_for_snippet: None,
        score,
        bm25_rank: None,
        cosine_rank: Some(1),
    }
}

#[test]
fn rollup_keeps_highest_scoring_field_per_task() {
    let hits = rollup_to_tasks(
        vec![
            candidate("T1", "summary", 0.2),
            candidate("T1", "plan", 0.7),
            candidate("T2", "purpose", 0.3),
        ],
        10,
    );

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].source_id, "T1");
    assert_eq!(hits[0].best_field, "plan");
}
