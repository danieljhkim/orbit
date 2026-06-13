use std::collections::HashMap;
use std::path::Path;

use crate::KnowledgeError;
use crate::commands::GraphCommandContext;
use crate::graph::{GraphIndexReferenceRow, GraphReadOptions};
use crate::service::GraphContextService;
use orbit_graph_extract::Selector;

#[derive(Debug, Clone)]
pub struct RefsInput {
    pub context: GraphCommandContext,
    pub selector: String,
    pub include_simple_name: bool,
    pub include: RefInclude,
    pub limit: usize,
    pub per_file_limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefsResult {
    pub code_refs: Vec<RefMatch>,
    pub doc_refs: Vec<RefMatch>,
    pub config_refs: Vec<RefMatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefMatch {
    pub selector: String,
    pub name: String,
    pub file: String,
    pub kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefKind {
    Code,
    Doc,
    Config,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefInclude {
    code: bool,
    doc: bool,
    config: bool,
}

impl RefInclude {
    pub fn code_only() -> Self {
        Self {
            code: true,
            doc: false,
            config: false,
        }
    }

    pub fn from_names(values: Vec<String>) -> Result<Self, KnowledgeError> {
        if values.is_empty() {
            return Ok(Self::code_only());
        }

        let mut include = Self {
            code: false,
            doc: false,
            config: false,
        };
        for name in values {
            match name.as_str() {
                "code" => include.code = true,
                "doc" => include.doc = true,
                "config" => include.config = true,
                "all" => {
                    include.code = true;
                    include.doc = true;
                    include.config = true;
                }
                other => {
                    return Err(KnowledgeError::invalid_data(format!(
                        "`include` entries must be `code`, `doc`, `config`, or `all`, got `{other}`"
                    )));
                }
            }
        }
        Ok(include)
    }

    // pub(crate) widened for commands/tests/refs.rs during test layout migration
    // (docs/design-patterns/test_layout.md, ORB-00249).
    pub(crate) fn includes(self, kind: RefKind) -> bool {
        match kind {
            RefKind::Code => self.code,
            RefKind::Doc => self.doc,
            RefKind::Config => self.config,
        }
    }
}

pub fn run(input: RefsInput) -> Result<RefsResult, KnowledgeError> {
    let selector: Selector = input
        .selector
        .parse()
        .map_err(|error| KnowledgeError::invalid_data(format!("{error}")))?;
    let search_terms = match &selector {
        Selector::Symbol { symbol, .. } => symbol_search_terms(symbol, input.include_simple_name),
        _ => {
            return Err(KnowledgeError::invalid_data(
                "refs requires a symbol selector (e.g. symbol:path#name:kind)".to_string(),
            ));
        }
    };

    if let Some(result) = try_refs_via_sql_index(
        &input.context,
        &search_terms,
        input.selector.as_str(),
        input.include,
        input.limit,
        input.per_file_limit,
    )? {
        return Ok(result);
    }

    let graph = input.context.read_graph(GraphReadOptions {
        hydrate_leaf_source: true,
        ..Default::default()
    })?;
    let svc = GraphContextService::new(&graph);
    let all_hits = svc.find_references(
        Some(&input.context.knowledge_dir),
        &search_terms,
        Some(input.selector.as_str()),
    );

    let mut code_refs = Vec::new();
    let mut doc_refs = Vec::new();
    let mut config_refs = Vec::new();
    let mut remaining = input.limit;
    let mut file_counts = HashMap::<(RefKind, String), usize>::new();

    for hit in all_hits {
        let ref_kind = classify_ref_kind(&hit.file);
        if !input.include.includes(ref_kind) {
            continue;
        }

        let count = file_counts.entry((ref_kind, hit.file.clone())).or_default();
        if *count >= input.per_file_limit {
            continue;
        }
        if remaining == 0 {
            break;
        }

        *count += 1;
        remaining -= 1;

        let value = RefMatch {
            selector: hit.selector,
            name: hit.name,
            file: hit.file,
            kind: hit.kind,
        };

        match ref_kind {
            RefKind::Code => code_refs.push(value),
            RefKind::Doc => doc_refs.push(value),
            RefKind::Config => config_refs.push(value),
        }
    }

    Ok(RefsResult {
        code_refs,
        doc_refs,
        config_refs,
    })
}

fn try_refs_via_sql_index(
    context: &GraphCommandContext,
    search_terms: &[String],
    definition_selector: &str,
    include: RefInclude,
    limit: usize,
    per_file_limit: usize,
) -> Result<Option<RefsResult>, KnowledgeError> {
    let Some(reader) = context.open_current_graph_index()? else {
        return Ok(None);
    };
    let all_hits = reader.find_references(search_terms, Some(definition_selector))?;
    Ok(Some(refs_result_from_hits(
        all_hits,
        include,
        limit,
        per_file_limit,
    )))
}

fn refs_result_from_hits(
    all_hits: Vec<GraphIndexReferenceRow>,
    include: RefInclude,
    limit: usize,
    per_file_limit: usize,
) -> RefsResult {
    let mut code_refs = Vec::new();
    let mut doc_refs = Vec::new();
    let mut config_refs = Vec::new();
    let mut remaining = limit;
    let mut file_counts = HashMap::<(RefKind, String), usize>::new();

    for hit in all_hits {
        let file = file_for_index_ref_row(&hit);
        let ref_kind = classify_ref_kind(&file);
        if !include.includes(ref_kind) {
            continue;
        }

        let count = file_counts.entry((ref_kind, file.clone())).or_default();
        if *count >= per_file_limit {
            continue;
        }
        if remaining == 0 {
            break;
        }

        *count += 1;
        remaining -= 1;

        let value = RefMatch {
            selector: selector_for_index_ref_row(&hit),
            name: hit.name,
            file,
            kind: hit.kind.unwrap_or_else(|| "file".to_string()),
        };

        match ref_kind {
            RefKind::Code => code_refs.push(value),
            RefKind::Doc => doc_refs.push(value),
            RefKind::Config => config_refs.push(value),
        }
    }

    RefsResult {
        code_refs,
        doc_refs,
        config_refs,
    }
}

fn selector_for_index_ref_row(row: &GraphIndexReferenceRow) -> String {
    row.selector
        .clone()
        .unwrap_or_else(|| match row.node_type.as_str() {
            "file" => format!("file:{}", row.location),
            "leaf" => {
                let kind = row.kind.as_deref().unwrap_or_default();
                format!("symbol:{}:{kind}", row.location)
            }
            "dir" => {
                let path = row.location.trim_end_matches('/');
                format!("dir:{path}")
            }
            _ => row.id.clone(),
        })
}

fn file_for_index_ref_row(row: &GraphIndexReferenceRow) -> String {
    row.location
        .split_once('#')
        .map(|(path, _)| path.to_string())
        .unwrap_or_else(|| row.location.clone())
}

fn symbol_search_terms(symbol: &str, include_simple_name: bool) -> Vec<String> {
    let mut terms = vec![symbol.to_string()];
    if include_simple_name {
        let simple_name = simple_selector_symbol_name(symbol);
        if simple_name != symbol {
            terms.push(simple_name);
        }
    }
    terms
}

// pub(crate) widened for commands/tests/refs.rs access during sibling test
// layout migration (docs/design-patterns/test_layout.md, ORB-00249).
pub(crate) fn simple_selector_symbol_name(symbol: &str) -> String {
    let scoped = symbol
        .rsplit("::")
        .next()
        .unwrap_or(symbol)
        .rsplit('.')
        .next()
        .unwrap_or(symbol);
    strip_numeric_selector_suffixes(scoped).to_string()
}

fn strip_numeric_selector_suffixes(mut symbol: &str) -> &str {
    while let Some((prefix, suffix)) = symbol.rsplit_once('#') {
        if suffix.is_empty() || !suffix.chars().all(|ch| ch.is_ascii_digit()) {
            break;
        }
        symbol = prefix;
    }
    symbol
}

// pub(crate) widened for commands/tests/refs.rs access during sibling test
// layout migration (docs/design-patterns/test_layout.md, ORB-00249).
pub(crate) fn classify_ref_kind(path: &str) -> RefKind {
    let extension = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "md" | "txt" | "rst" | "adoc" => RefKind::Doc,
        "yaml" | "yml" | "toml" | "json" | "jsonc" | "ini" => RefKind::Config,
        _ => RefKind::Code,
    }
}
