use std::collections::HashMap;
use std::path::Path;

use orbit_common::types::{OrbitError, ToolParam, ToolSchema, optional_string_list_alias};
use orbit_knowledge::Selector;
use orbit_knowledge::service::GraphContextService;
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeRefsTool;

const DEFAULT_LIMIT: usize = 20;
const DEFAULT_PER_FILE_LIMIT: usize = 5;

impl Tool for OrbitKnowledgeRefsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.refs".to_string(),
            description: "Use when you need symbol refs. Prefer over grep when raw hits mix code, docs, and config. Behavior: returns `code_refs`; `doc_refs` and `config_refs` stay empty unless `include` asks for them.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Target symbol selector.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "limit".to_string(),
                    description: "Max results.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "include_simple_name".to_string(),
                    description: "Also search the tail name.".to_string(),
                    param_type: "boolean".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "include".to_string(),
                    description: "String/array: `code`, `doc`, `config`, `all`.".to_string(),
                    param_type: "string_list".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "per_file_limit".to_string(),
                    description: "Max refs per file/category.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "knowledge_dir".to_string(),
                    description: "Override knowledge dir.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                super::graph_ref_param(),
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let selector_str = super::required_string(&input, &["selector"], "selector")?;
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_LIMIT as u64) as usize;
        let include_simple_name = input
            .get("include_simple_name")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let include = RefInclude::parse(&input)?;
        let per_file_limit = input
            .get("per_file_limit")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_PER_FILE_LIMIT as u64) as usize;

        let selector: Selector = selector_str
            .parse()
            .map_err(|e| OrbitError::InvalidInput(format!("{e}")))?;

        // Extract symbol name from selector
        let search_terms = match &selector {
            Selector::Symbol { symbol, .. } => {
                let mut search_terms = vec![symbol.clone()];
                if let Some(simple_name) = symbol.rsplit("::").next()
                    && include_simple_name
                    && simple_name != symbol
                {
                    search_terms.push(simple_name.to_string());
                }
                search_terms
            }
            _ => {
                return Err(OrbitError::InvalidInput(
                    "refs requires a symbol selector (e.g. symbol:path#name:kind)".to_string(),
                ));
            }
        };

        // Extract the defining file to exclude self-references
        let knowledge_dir = super::knowledge_write::resolve_knowledge_dir(ctx, &input)?;
        let graph = super::load_graph_for_read(ctx, &input)?;
        let svc = GraphContextService::new(&graph);
        let all_hits = svc.find_references(
            Some(&knowledge_dir),
            &search_terms,
            Some(selector_str.as_str()),
        );
        let mut code_refs = Vec::new();
        let mut doc_refs = Vec::new();
        let mut config_refs = Vec::new();
        let mut remaining = limit;
        let mut file_counts = HashMap::<(RefKind, String), usize>::new();

        for hit in all_hits {
            let kind = classify_ref_kind(&hit.file);
            if !include.includes(kind) {
                continue;
            }

            let count = file_counts.entry((kind, hit.file.clone())).or_default();
            if *count >= per_file_limit {
                continue;
            }
            if remaining == 0 {
                break;
            }

            *count += 1;
            remaining -= 1;

            let value = json!({
                "selector": hit.selector,
                "name": hit.name,
                "file": hit.file,
                "kind": hit.kind,
            });

            match kind {
                RefKind::Code => code_refs.push(value),
                RefKind::Doc => doc_refs.push(value),
                RefKind::Config => config_refs.push(value),
            }
        }

        Ok(json!({
            "code_refs": code_refs,
            "doc_refs": doc_refs,
            "config_refs": config_refs,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RefKind {
    Code,
    Doc,
    Config,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RefInclude {
    code: bool,
    doc: bool,
    config: bool,
}

impl RefInclude {
    fn code_only() -> Self {
        Self {
            code: true,
            doc: false,
            config: false,
        }
    }

    fn parse(input: &Value) -> Result<Self, OrbitError> {
        if input.get("include").is_none() {
            return Ok(Self::code_only());
        }
        let values = optional_string_list_alias(input, &["include"])?.unwrap_or_default();
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
                    return Err(OrbitError::InvalidInput(format!(
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
