use crate::KnowledgeError;
use crate::commands::GraphCommandContext;
use crate::graph::{GraphIndexCallerRow, GraphReadOptions};
use crate::service::GraphContextService;
use crate::service::callers::{CallerHit, MAX_CALLER_DEPTH, transitive_callers};
use orbit_graph_extract::Selector;

const DEFAULT_DEPTH: usize = 2;

#[derive(Debug, Clone)]
pub struct CallersInput {
    pub context: GraphCommandContext,
    pub selector: String,
    pub requested_depth: Option<usize>,
}

pub struct CallersResult {
    pub target: String,
    pub requested_depth: usize,
    pub depth: usize,
    pub callers: Vec<CallerHit>,
}

pub fn run(input: CallersInput) -> Result<CallersResult, KnowledgeError> {
    let selector: Selector = input
        .selector
        .parse()
        .map_err(|error| KnowledgeError::invalid_data(format!("{error}")))?;
    let requested_depth = input.requested_depth.unwrap_or(DEFAULT_DEPTH);
    let depth = requested_depth.min(MAX_CALLER_DEPTH);

    if let Some(callers) =
        try_callers_via_sql_index(&input.context, input.selector.as_str(), depth)?
    {
        return Ok(CallersResult {
            target: input.selector,
            requested_depth,
            depth,
            callers,
        });
    }

    let graph = input.context.read_graph(GraphReadOptions {
        hydrate_leaf_source: true,
        ..Default::default()
    })?;
    let svc = GraphContextService::new(&graph);
    let callers = transitive_callers(&svc, &graph, &selector, depth)
        .map_err(|error| KnowledgeError::knowledge_unavailable(error.to_string()))?;

    Ok(CallersResult {
        target: input.selector,
        requested_depth,
        depth,
        callers,
    })
}

fn try_callers_via_sql_index(
    context: &GraphCommandContext,
    selector: &str,
    depth: usize,
) -> Result<Option<Vec<CallerHit>>, KnowledgeError> {
    let Some(reader) = context.open_current_graph_index()? else {
        return Ok(None);
    };
    let Some(rows) = reader.transitive_callers(selector, depth)? else {
        return Ok(None);
    };
    Ok(Some(
        rows.into_iter().map(caller_hit_from_index_row).collect(),
    ))
}

fn caller_hit_from_index_row(row: GraphIndexCallerRow) -> CallerHit {
    CallerHit {
        selector: row.selector,
        name: row.name,
        file: row.file,
        kind: row.kind,
        distance: row.distance,
        via: row.via,
    }
}
