use crate::KnowledgeError;
use crate::commands::GraphCommandContext;
use crate::graph::GraphReadOptions;
use crate::service::GraphContextService;
use crate::service::implementors::{ImplementorHit, trait_implementors};
use orbit_graph_extract::Selector;

#[derive(Debug, Clone)]
pub struct ImplementorsInput {
    pub context: GraphCommandContext,
    pub trait_selector: String,
}

pub struct ImplementorsResult {
    pub trait_selector: String,
    pub implementors: Vec<ImplementorHit>,
}

pub fn run(input: ImplementorsInput) -> Result<ImplementorsResult, KnowledgeError> {
    let selector: Selector = input
        .trait_selector
        .parse()
        .map_err(|error| KnowledgeError::invalid_data(format!("{error}")))?;
    let graph = input.context.read_graph(GraphReadOptions {
        hydrate_leaf_source: true,
        ..Default::default()
    })?;
    let svc = GraphContextService::new(&graph);
    let implementors = trait_implementors(&svc, &graph, &selector)
        .map_err(|error| KnowledgeError::knowledge_unavailable(error.to_string()))?;

    Ok(ImplementorsResult {
        trait_selector: input.trait_selector,
        implementors,
    })
}
