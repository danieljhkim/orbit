use std::path::Path;

use crate::graph::navigator::GraphNodeRef;
use crate::graph::nodes::CodebaseGraphV1;
use crate::service::GraphContextService;

use super::search::DEFAULT_RANKING_HARD_CAP;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FuzzyCandidate {
    pub(crate) selector: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) file: Option<String>,
    pub(crate) score: f32,
}

pub(crate) fn fuzzy_name_candidates(
    graph: &CodebaseGraphV1,
    query: &str,
    limit: usize,
) -> Vec<FuzzyCandidate> {
    if limit == 0 {
        return Vec::new();
    }

    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let svc = GraphContextService::new(graph);
    let mut candidates = Vec::new();
    let mut scanned = 0usize;

    for leaf in &graph.leaves {
        if scanned >= DEFAULT_RANKING_HARD_CAP {
            break;
        }
        scanned += 1;
        push_candidate(
            &mut candidates,
            CandidateInput {
                selector: svc.selector_for_node(GraphNodeRef::Leaf(leaf)),
                result_name: leaf.base.name.clone(),
                match_name: leaf.base.name.as_str(),
                kind: leaf.kind.to_string(),
                file: leaf
                    .base
                    .location
                    .split_once('#')
                    .map(|(path, _)| path.to_string()),
            },
            &query,
        );
    }

    for file in &graph.files {
        if scanned >= DEFAULT_RANKING_HARD_CAP {
            break;
        }
        scanned += 1;
        let basename = Path::new(&file.base.location)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(file.base.name.as_str());
        push_candidate(
            &mut candidates,
            CandidateInput {
                selector: svc.selector_for_node(GraphNodeRef::File(file)),
                result_name: file.base.name.clone(),
                match_name: basename,
                kind: "file".to_string(),
                file: Some(file.base.location.clone()),
            },
            &query,
        );
    }

    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.selector.cmp(&right.selector))
    });
    candidates.truncate(limit);
    candidates
}

struct CandidateInput<'a> {
    selector: String,
    result_name: String,
    match_name: &'a str,
    kind: String,
    file: Option<String>,
}

fn push_candidate(candidates: &mut Vec<FuzzyCandidate>, input: CandidateInput<'_>, query: &str) {
    let candidate = input.match_name.to_lowercase();
    let query_len = query.chars().count();
    let candidate_len = candidate.chars().count();
    if candidate_len.abs_diff(query_len) > 2 {
        return;
    }

    let distance = levenshtein_distance(query, &candidate);
    if distance == 0 || distance > 2 {
        return;
    }

    let max_len = candidate_len.max(query_len);
    if max_len == 0 {
        return;
    }

    let score = (1.0 - (distance as f32 / max_len as f32)).clamp(0.0, 1.0);
    candidates.push(FuzzyCandidate {
        selector: input.selector,
        name: input.result_name,
        kind: input.kind,
        file: input.file,
        score,
    });
}

pub(crate) fn levenshtein_distance(left: &str, right: &str) -> usize {
    if left == right {
        return 0;
    }
    if left.is_empty() {
        return right.chars().count();
    }
    if right.is_empty() {
        return left.chars().count();
    }

    let right_chars = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut current = vec![0; right_chars.len() + 1];

    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let substitution = usize::from(left_char != *right_char);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right_chars.len()]
}
