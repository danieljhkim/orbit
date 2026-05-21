use std::collections::HashMap;
use std::path::Path;

use crate::commands::GraphCommandContext;
use crate::graph::GraphReadOptions;
use crate::service::GraphContextService;
use crate::{KnowledgeError, Selector};

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

    fn includes(self, kind: RefKind) -> bool {
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

fn simple_selector_symbol_name(symbol: &str) -> String {
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

fn classify_ref_kind(path: &str) -> RefKind {
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

#[cfg(test)]
mod tests;
