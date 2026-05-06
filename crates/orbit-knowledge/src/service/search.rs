use crate::graph::navigator::GraphNodeRef;

use regex::Regex;

use super::{GraphContextService, MatchedLine, SearchHit, SearchResult, SearchScanLimitExceeded};

const DEFAULT_MATCHED_LINE_LIMIT: usize = 5;
const SNIPPET_CHAR_LIMIT: usize = 240;

struct SearchCriteria<'q> {
    query_lower: &'q str,
    browse: bool,
    node_types: Option<&'q [&'q str]>,
    location_prefix: Option<&'q str>,
    kind_filter: Option<&'q str>,
    source_regex: Option<&'q Regex>,
    matched_line_limit: usize,
    candidate_scan_limit: Option<usize>,
    limit: usize,
}

impl<'a> GraphContextService<'a> {
    pub fn search_with_total(
        &self,
        query: &str,
        node_types: Option<&[&str]>,
        location_prefix: Option<&str>,
        kind_filter: Option<&str>,
        limit: usize,
    ) -> (usize, Vec<GraphNodeRef<'a>>) {
        let (total, hits) = self.search_hits_with_total(
            query,
            node_types,
            location_prefix,
            kind_filter,
            None,
            limit,
        );
        let nodes = hits.into_iter().map(|hit| hit.node).collect();
        (total, nodes)
    }

    pub fn search_hits_with_total(
        &self,
        query: &str,
        node_types: Option<&[&str]>,
        location_prefix: Option<&str>,
        kind_filter: Option<&str>,
        source_regex: Option<&Regex>,
        limit: usize,
    ) -> (usize, Vec<SearchHit<'a>>) {
        self.search_hits_with_total_bounded(
            query,
            node_types,
            location_prefix,
            kind_filter,
            source_regex,
            limit,
            None,
        )
        .expect("unbounded search cannot exceed a candidate scan cap")
    }

    #[allow(clippy::too_many_arguments)]
    pub fn search_hits_with_total_bounded(
        &self,
        query: &str,
        node_types: Option<&[&str]>,
        location_prefix: Option<&str>,
        kind_filter: Option<&str>,
        source_regex: Option<&Regex>,
        limit: usize,
        candidate_scan_limit: Option<usize>,
    ) -> Result<(usize, Vec<SearchHit<'a>>), SearchScanLimitExceeded> {
        let query_lower = query.to_lowercase();
        let criteria = SearchCriteria {
            query_lower: &query_lower,
            browse: query_lower.is_empty(),
            node_types,
            location_prefix,
            kind_filter,
            source_regex,
            matched_line_limit: DEFAULT_MATCHED_LINE_LIMIT,
            candidate_scan_limit,
            limit,
        };
        let mut total = 0usize;
        let mut results = Vec::new();
        let mut scanned_candidates = 0usize;

        for dir in &self.graph.dirs {
            self.collect_search_match(
                GraphNodeRef::Dir(dir),
                "dir",
                &criteria,
                &mut total,
                &mut results,
                &mut scanned_candidates,
            )?;
        }
        for file in &self.graph.files {
            self.collect_search_match(
                GraphNodeRef::File(file),
                "file",
                &criteria,
                &mut total,
                &mut results,
                &mut scanned_candidates,
            )?;
        }
        for leaf in &self.graph.leaves {
            self.collect_search_match(
                GraphNodeRef::Leaf(leaf),
                "symbol",
                &criteria,
                &mut total,
                &mut results,
                &mut scanned_candidates,
            )?;
        }

        Ok((total, results))
    }

    pub fn search(
        &self,
        query: &str,
        node_types: Option<&[&str]>,
        location_prefix: Option<&str>,
        kind_filter: Option<&str>,
        limit: usize,
    ) -> Vec<GraphNodeRef<'a>> {
        self.search_with_total(query, node_types, location_prefix, kind_filter, limit)
            .1
    }

    pub fn search_total(
        &self,
        query: &str,
        node_types: Option<&[&str]>,
        location_prefix: Option<&str>,
        kind_filter: Option<&str>,
    ) -> usize {
        self.search_with_total(query, node_types, location_prefix, kind_filter, 0)
            .0
    }

    /// Search returning structured results with name, kind, and file info.
    pub fn search_structured(
        &self,
        query: &str,
        node_types: Option<&[&str]>,
        location_prefix: Option<&str>,
        kind_filter: Option<&str>,
        limit: usize,
    ) -> Vec<SearchResult> {
        let nodes = self
            .search_with_total(query, node_types, location_prefix, kind_filter, limit)
            .1;
        nodes
            .into_iter()
            .map(|node| {
                let selector = self.selector_for_node(node);
                let name = node.base().name.to_string();
                let kind = match node {
                    GraphNodeRef::Dir(_) => "dir".to_string(),
                    GraphNodeRef::File(_) => "file".to_string(),
                    GraphNodeRef::Leaf(leaf) => leaf.kind.to_string(),
                };
                let file = match node {
                    GraphNodeRef::Leaf(leaf) => leaf
                        .base
                        .location
                        .split_once('#')
                        .map(|(path, _)| path.to_string()),
                    GraphNodeRef::File(file) => Some(file.base.location.clone()),
                    GraphNodeRef::Dir(_) => None,
                };
                SearchResult {
                    selector,
                    name,
                    kind,
                    file,
                }
            })
            .collect()
    }

    fn collect_search_match(
        &self,
        node: GraphNodeRef<'a>,
        node_type: &str,
        criteria: &SearchCriteria<'_>,
        total: &mut usize,
        results: &mut Vec<SearchHit<'a>>,
        scanned_candidates: &mut usize,
    ) -> Result<(), SearchScanLimitExceeded> {
        if !self.node_candidate_matches(node, node_type, criteria) {
            return Ok(());
        }

        let matched_lines = if let Some(regex) = criteria.source_regex {
            let Some((source, first_line)) = source_for_node(node) else {
                return Ok(());
            };
            *scanned_candidates += 1;
            if let Some(limit) = criteria.candidate_scan_limit
                && *scanned_candidates > limit
            {
                return Err(SearchScanLimitExceeded { limit });
            }
            let Some(matched_lines) =
                source_regex_matches_source(source, first_line, regex, criteria.matched_line_limit)
            else {
                return Ok(());
            };
            matched_lines
        } else {
            Vec::new()
        };

        *total += 1;
        if results.len() < criteria.limit {
            results.push(SearchHit {
                node,
                matched_lines,
            });
        }
        Ok(())
    }

    fn node_candidate_matches(
        &self,
        node: GraphNodeRef<'a>,
        node_type: &str,
        criteria: &SearchCriteria<'_>,
    ) -> bool {
        if let Some(types) = criteria.node_types
            && !types.contains(&node_type)
        {
            return false;
        }
        if let Some(prefix) = criteria.location_prefix
            && !node.location().starts_with(prefix)
        {
            return false;
        }
        if let Some(kind_filter) = criteria.kind_filter {
            match node {
                GraphNodeRef::Leaf(leaf) if leaf.kind.to_string() == kind_filter => {}
                GraphNodeRef::Leaf(_) => return false,
                _ => return false,
            }
        }
        criteria.browse
            || node
                .base()
                .name
                .to_lowercase()
                .contains(criteria.query_lower)
            || node
                .location()
                .to_lowercase()
                .contains(criteria.query_lower)
    }
}

fn source_regex_matches_source(
    source: &str,
    first_line: usize,
    regex: &Regex,
    matched_line_limit: usize,
) -> Option<Vec<MatchedLine>> {
    let mut matched_lines = Vec::new();
    let mut matched = false;

    for (line_index, line) in source.lines().enumerate() {
        if !regex.is_match(line) {
            continue;
        }
        matched = true;
        if matched_lines.len() < matched_line_limit {
            matched_lines.push(MatchedLine {
                line_number: first_line + line_index,
                snippet: line_snippet(line),
            });
        }
    }

    matched.then_some(matched_lines)
}

fn source_for_node(node: GraphNodeRef<'_>) -> Option<(&str, usize)> {
    match node {
        GraphNodeRef::File(file) if !file.source.is_empty() => Some((&file.source, 1)),
        GraphNodeRef::Leaf(leaf) if !leaf.source.is_empty() => {
            Some((&leaf.source, leaf.start_line.unwrap_or(1) as usize))
        }
        _ => None,
    }
}

fn line_snippet(line: &str) -> String {
    let trimmed = line.trim_end();
    let mut snippet = String::new();
    for (index, ch) in trimmed.chars().enumerate() {
        if index == SNIPPET_CHAR_LIMIT {
            snippet.push_str("...");
            return snippet;
        }
        snippet.push(ch);
    }
    snippet
}

#[cfg(test)]
mod tests {
    use crate::graph::nodes::{
        BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode,
    };

    use super::*;

    struct LanguageCase {
        path: &'static str,
        language: &'static str,
        name: &'static str,
        kind: LeafKind,
        source: &'static str,
        pattern: &'static str,
        start_line: u32,
    }

    #[test]
    fn source_regex_matches_leaf_source_for_indexed_languages() {
        let cases = vec![
            LanguageCase {
                path: "src/lib.rs",
                language: "rust",
                name: "build",
                kind: LeafKind::Function,
                source: "pub const fn build() {}\n",
                pattern: r"^\s*pub\s+const\s+fn\s+build",
                start_line: 7,
            },
            LanguageCase {
                path: "go/main.go",
                language: "go",
                name: "Build",
                kind: LeafKind::Function,
                source: "func Build() {}\n",
                pattern: r"^\s*func\s+Build",
                start_line: 11,
            },
            LanguageCase {
                path: "java/Sink.java",
                language: "java",
                name: "Sink",
                kind: LeafKind::Class,
                source: "class Sink implements ISink {}\n",
                pattern: r"class\s+Sink\s+implements\s+ISink",
                start_line: 13,
            },
            LanguageCase {
                path: "web/sink.js",
                language: "javascript",
                name: "Sink",
                kind: LeafKind::Class,
                source: "class Sink extends BaseSink {}\n",
                pattern: r"class\s+Sink\s+extends\s+BaseSink",
                start_line: 17,
            },
            LanguageCase {
                path: "py/sink.py",
                language: "python",
                name: "Sink",
                kind: LeafKind::Class,
                source: "class Sink(ISink):\n    pass\n",
                pattern: r"^\s*class\s+Sink\(ISink\):",
                start_line: 19,
            },
        ];

        for case in cases {
            let graph = graph_for_case(&case);
            let service = GraphContextService::new(&graph);
            let regex = Regex::new(case.pattern).unwrap();
            let (total, hits) = service.search_hits_with_total(
                "",
                Some(&["symbol"]),
                Some(case.path),
                None,
                Some(&regex),
                10,
            );

            assert_eq!(total, 1, "expected one match for {}", case.language);
            assert_eq!(
                hits.len(),
                1,
                "expected one returned hit for {}",
                case.language
            );
            assert_eq!(hits[0].node.base().language, case.language);
            assert_eq!(
                hits[0].matched_lines[0].line_number,
                case.start_line as usize
            );
        }
    }

    fn graph_for_case(case: &LanguageCase) -> CodebaseGraphV1 {
        let root_id = "dir:.".to_string();
        let file_id = format!("file:{}", case.path);
        let kind_name = case.kind.to_string();
        let leaf_id = format!("symbol:{}#{}:{kind_name}", case.path, case.name);

        CodebaseGraphV1 {
            root_dir_id: root_id.clone(),
            dirs: vec![DirNode {
                base: base_node(&root_id, ".", ".", "", None),
                dir_children: Vec::new(),
                file_children: vec![file_id.clone()],
            }],
            files: vec![FileNode {
                base: base_node(&file_id, case.path, case.path, case.language, Some("dir:.")),
                extension: std::path::Path::new(case.path)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(str::to_string),
                source_blob_hash: None,
                source: String::new(),
                imports: Vec::new(),
                exports: Vec::new(),
                re_exports: Vec::new(),
                leaf_children: vec![leaf_id.clone()],
            }],
            leaves: vec![LeafNode {
                base: base_node(
                    &leaf_id,
                    case.name,
                    &format!("{}#{}", case.path, case.name),
                    case.language,
                    Some(&file_id),
                ),
                kind: case.kind.clone(),
                source: case.source.to_string(),
                source_blob_hash: None,
                source_hash: None,
                file_hash_at_capture: None,
                history: Vec::new(),
                input_signature: Vec::new(),
                output_signature: Vec::new(),
                start_line: Some(case.start_line),
                end_line: Some(case.start_line + case.source.lines().count() as u32 - 1),
                children: Vec::new(),
            }],
        }
    }

    fn base_node(
        id: &str,
        name: &str,
        location: &str,
        language: &str,
        parent_id: Option<&str>,
    ) -> BaseNodeFields {
        BaseNodeFields {
            id: id.to_string(),
            identity_key: id.to_string(),
            object_hash: None,
            name: name.to_string(),
            location: location.to_string(),
            language: language.to_string(),
            description: String::new(),
            parent_id: parent_id.map(str::to_string),
            is_locked: false,
            lineage_locked: false,
            lock_owner: None,
            lock_reason: String::new(),
        }
    }
}
