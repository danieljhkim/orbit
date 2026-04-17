use orbit_knowledge::Selector;
use orbit_knowledge::service::GraphContextService;
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeRefsTool;

impl Tool for OrbitKnowledgeRefsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.refs".to_string(),
            description: "Find references to a symbol across the knowledge graph. Returns leaves whose source mentions the symbol name.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Symbol selector to find references for (e.g. `symbol:src/lib.rs#hello:function`)".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "limit".to_string(),
                    description: "Max results (default 20)".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "knowledge_dir".to_string(),
                    description: "Optional knowledge artifact directory; defaults to `<workspace>/.orbit/knowledge`".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let selector_str = super::required_string(&input, &["selector"], "selector")?;
        let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;

        let selector: Selector = selector_str
            .parse()
            .map_err(|e| OrbitError::InvalidInput(format!("{e}")))?;

        // Extract symbol name from selector
        let (symbol_name, search_terms) = match &selector {
            Selector::Symbol { symbol, .. } => {
                let mut search_terms = vec![symbol.clone()];
                if let Some(simple_name) = symbol.rsplit("::").next()
                    && simple_name != symbol
                {
                    search_terms.push(simple_name.to_string());
                }
                (symbol.clone(), search_terms)
            }
            _ => {
                return Err(OrbitError::InvalidInput(
                    "refs requires a symbol selector (e.g. symbol:path#name:kind)".to_string(),
                ));
            }
        };

        // Extract the defining file to exclude self-references
        let graph = super::load_graph_for_read(ctx, &input)?;
        let svc = GraphContextService::new(&graph);
        let all_hits = svc.find_references(&search_terms, Some(selector_str.as_str()));
        let hits: Vec<&_> = all_hits.iter().take(limit).collect();

        let references: Vec<Value> = hits
            .into_iter()
            .map(|h| {
                json!({
                    "selector": h.selector,
                    "name": h.name,
                    "file": h.file,
                    "kind": h.kind,
                })
            })
            .collect();

        Ok(json!({
            "symbol": symbol_name,
            "definition": selector_str,
            "total": all_hits.len(),
            "references": references,
        }))
    }
}
